// Parsing half of Trop/Detail/PlaylistDetailView.swift. The Swift file mixes
// the SwiftUI view, its view model and this parser; only the parser and the
// network path of `PlaylistDetailViewModel.load()` are translated. Local and
// auto playlists, saving rows to the database and the header cache land with R5.

use std::collections::HashMap;

use swiftytube::formatting::duration_format::DurationFormat;
use swiftytube::networking::inner_tube_decode::{
    InnerTubeDecode, JsonDict, as_dict, as_dict_list, as_string,
};
use swiftytube::networking::inner_tube_error::InnerTubeError;
use swiftytube::networking::inner_tube_json::InnerTubeJSON;

use super::album_detail_parser::first_number;
use super::detail_models::PlaylistDetailInfo;
use crate::from_swiftytube::sync::browse_lens::BrowseLens;
use crate::from_swiftytube::sync::playlist_detail_service::PlaylistDetailService;
use crate::networking::models::yt_item::{PlaylistContributor, SongItem};

const BROWSE_ID_PATH: &[&str] = &["navigationEndpoint", "browseEndpoint", "browseId"];

pub struct PlaylistDetailParser;

impl PlaylistDetailParser {
    /// Network path of `PlaylistDetailViewModel.load()`: every page through
    /// `PlaylistDetailService::fetch_playlist_rows`, then the parser.
    pub async fn load(
        service: &PlaylistDetailService<'_>,
        playlist_id: &str,
    ) -> Result<PlaylistDetailInfo, InnerTubeError> {
        let result = service.fetch_playlist_rows(playlist_id, 100, false).await?;
        let rows: Vec<&JsonDict> = result.rows.iter().collect();
        Ok(Self::parse_playlist_detail(
            &result.first_page,
            &rows,
            playlist_id,
        ))
    }

    /// videoId → playlistSetVideoId, the per-row id YouTube needs to remove a song.
    pub fn set_video_ids(rows: &[&JsonDict]) -> HashMap<String, String> {
        let mut ids = HashMap::new();
        for row in rows {
            let item = InnerTubeDecode::dict(
                &["musicResponsiveListItemRenderer", "playlistItemData"],
                row,
            );
            let video_id = as_string(item.and_then(|i| i.get("videoId")));
            let set_video_id = as_string(item.and_then(|i| i.get("playlistSetVideoId")));
            if let (Some(video_id), Some(set_video_id)) = (video_id, set_video_id) {
                ids.entry(video_id.to_string())
                    .or_insert_with(|| set_video_id.to_string());
            }
        }
        ids
    }

    /// Parses InnerTube browse JSON into a PlaylistDetailInfo.
    /// Header metadata (title, author, song count, duration, thumbnail, description)
    /// comes from the first page, songs from the track rows collected across
    /// every continuation page.
    pub fn parse_playlist_detail(
        json: &JsonDict,
        rows: &[&JsonDict],
        playlist_id: &str,
    ) -> PlaylistDetailInfo {
        let mut info = PlaylistDetailInfo {
            title: "Unknown Playlist".to_string(),
            playlist_id: playlist_id.to_string(),
            ..PlaylistDetailInfo::default()
        };

        let first_tab_section_inner = BrowseLens::first_section_item(json);
        let wrapped = first_tab_section_inner
            .and_then(|s| as_dict(s.get("itemSectionRenderer")))
            .and_then(|r| as_dict_list(r.get("contents")));
        let first_tab_section = wrapped
            .and_then(|w| w.first().copied())
            .or(first_tab_section_inner);

        let editable_header = first_tab_section.and_then(|s| {
            InnerTubeDecode::dict(&["musicEditablePlaylistDetailHeaderRenderer", "header"], s)
        });
        let top_header = as_dict(json.get("header"));
        fn detail_or_responsive(dict: &JsonDict) -> Option<&JsonDict> {
            as_dict(dict.get("musicDetailHeaderRenderer"))
                .or_else(|| as_dict(dict.get("musicResponsiveHeaderRenderer")))
        }
        let header_renderer = first_tab_section
            .and_then(|s| as_dict(s.get("musicResponsiveHeaderRenderer")))
            .or_else(|| editable_header.and_then(detail_or_responsive))
            .or_else(|| top_header.and_then(detail_or_responsive));

        let stack = header_renderer
            .and_then(|h| InnerTubeDecode::dict(&["facepile", "avatarStackViewModel"], h));
        let avatar_urls: Vec<String> = stack
            .and_then(|s| as_dict_list(s.get("avatars")))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|avatar| {
                let sources =
                    InnerTubeDecode::array(&["avatarViewModel", "image", "sources"], avatar)?;
                as_string(sources.first()?.get("url")).map(str::to_string)
            })
            .collect();

        if let Some(detail_header) = header_renderer {
            if let Some(title) = InnerTubeJSON::runs_text(as_dict(detail_header.get("title"))) {
                info.title = title;
            }
            info.thumbnail_url = InnerTubeJSON::music_thumbnail_url(detail_header);
            info.description_text =
                InnerTubeJSON::runs_text(as_dict(detail_header.get("description")));

            // Author from straplineTextOne
            if let Some(first_run) =
                InnerTubeDecode::array(&["straplineTextOne", "runs"], detail_header)
                    .and_then(|s| s.first().copied())
                && let Some(text) = as_string(first_run.get("text"))
            {
                info.author_name = Some(text.to_string());
                info.author_browse_id =
                    InnerTubeDecode::string(BROWSE_ID_PATH, first_run).map(str::to_string);
            }

            // Author from facepile (editable/self-authored playlists)
            if info.author_name.is_none()
                && let Some(stack) = stack
            {
                info.author_name =
                    InnerTubeDecode::string(&["text", "content"], stack).map(str::to_string);
                info.author_browse_id = InnerTubeDecode::string(
                    &[
                        "rendererContext",
                        "commandContext",
                        "onTap",
                        "innertubeCommand",
                        "browseEndpoint",
                        "browseId",
                    ],
                    stack,
                )
                .map(str::to_string);
                info.author_avatar_url = avatar_urls.first().cloned();
            }

            // Author from subtitle runs (fallback)
            if info.author_name.is_none()
                && let Some(subtitle) = InnerTubeDecode::array(&["subtitle", "runs"], detail_header)
            {
                for run in subtitle {
                    let Some(text) = as_string(run.get("text")) else {
                        continue;
                    };
                    let trimmed = text.trim();
                    if trimmed.is_empty() || trimmed == "•" {
                        continue;
                    }
                    if let Some(bid) = InnerTubeDecode::string(BROWSE_ID_PATH, run) {
                        info.author_name = Some(trimmed.to_string());
                        info.author_browse_id = Some(bid.to_string());
                        break;
                    }
                    info.author_name.get_or_insert_with(|| trimmed.to_string());
                }
            }

            // Song count / duration from secondSubtitle
            for run in InnerTubeDecode::array(&["secondSubtitle", "runs"], detail_header)
                .unwrap_or_default()
            {
                let Some(text) = as_string(run.get("text")) else {
                    continue;
                };
                let trimmed = text.trim();
                if ["song", "Song", "track", "Track", "video", "Video"]
                    .iter()
                    .any(|w| trimmed.contains(w))
                {
                    if let Some(count) = first_number(trimmed) {
                        info.song_count = count;
                    }
                } else if let Some(parsed) = DurationFormat::parse_clock(trimmed)
                    .or_else(|| DurationFormat::parse_spoken(trimmed))
                {
                    // "1:14:05", "30 minutes" or "1 hour, 14 minutes"
                    info.duration = parsed;
                }
            }
        }

        // --- Songs ---
        // Rows were collected across every continuation page by the caller;
        // continuation markers are already stripped.
        let owner_avatar_url = avatar_urls.first().map(String::as_str);
        let panel_endpoint = stack.and_then(|s| {
            InnerTubeDecode::dict(
                &[
                    "rendererContext",
                    "commandContext",
                    "onTap",
                    "innertubeCommand",
                    "showEngagementPanelEndpoint",
                ],
                s,
            )
        });
        info.collaborators_panel_params = panel_endpoint
            .and_then(|p| InnerTubeDecode::string(&["globalConfiguration", "params"], p))
            .map(str::to_string);
        // Same check as ytmusicapi's parse_playlist_header_meta.
        info.is_collaborative = panel_endpoint
            .and_then(|p| InnerTubeDecode::string(&["identifier", "tag"], p))
            == Some("PAplaylist_collaborate");
        for row in rows {
            let Some(renderer) = as_dict(row.get("musicResponsiveListItemRenderer")) else {
                continue;
            };
            let Some(mut song) = SongItem::from(renderer) else {
                continue;
            };
            song.added_by = PlaylistContributor::from(renderer, owner_avatar_url);
            info.songs.push(song);
        }

        // Use actual song data for total duration (more precise than header)
        if info.song_count == 0 {
            info.song_count = info.songs.len() as i64;
        }
        let song_duration: i64 = info.songs.iter().map(|s| s.duration).sum();
        if song_duration > 0 {
            info.duration = song_duration;
        }

        if info.is_collaborative || avatar_urls.len() > 1 {
            info.collaborator_avatar_urls = avatar_urls;
        }
        let has_editable = |dict: Option<&JsonDict>| {
            dict.and_then(|d| d.get("musicEditablePlaylistDetailHeaderRenderer"))
                .is_some_and(|v| !v.is_null())
        };
        info.is_editable = has_editable(first_tab_section) || has_editable(top_header);
        info
    }
}
