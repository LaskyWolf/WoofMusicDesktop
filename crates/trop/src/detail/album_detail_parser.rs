// Parsing half of Trop/Detail/AlbumDetailView.swift. The Swift file mixes the
// SwiftUI view, its view model and this parser; only the parser and the
// browse call of `AlbumDetailViewModel.load()` are translated. Saving the
// album (SyncBridge.mutations) and the download bookkeeping land with R5.

use swiftytube::formatting::duration_format::DurationFormat;
use swiftytube::networking::inner_tube_client::{BrowseArgs, InnerTubeClient};
use swiftytube::networking::inner_tube_decode::{InnerTubeDecode, JsonDict, as_dict, as_string};
use swiftytube::networking::inner_tube_error::InnerTubeError;
use swiftytube::networking::inner_tube_json::InnerTubeJSON;

use super::detail_models::AlbumDetailInfo;
use crate::from_swiftytube::sync::artist_credits::ArtistCredits;
use crate::from_swiftytube::sync::browse_lens::BrowseLens;
use crate::networking::models::yt_item::{SongItem, YTArtist};

/// First run of ASCII digits in `text` ("13 songs" → 13).
pub(crate) fn first_number(text: &str) -> Option<i64> {
    let start = text.find(|c: char| c.is_ascii_digit())?;
    let digits: String = text[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// A bare 4-digit year between 1900 and 2100 (exclusive).
fn year_of(trimmed: &str) -> Option<i64> {
    let year: i64 = trimmed.parse().ok()?;
    (trimmed.len() == 4 && year > 1900 && year < 2100).then_some(year)
}

fn artists(runs: &[&JsonDict]) -> Vec<YTArtist> {
    ArtistCredits::parse(runs)
        .into_iter()
        .map(YTArtist::from)
        .collect()
}

pub struct AlbumDetailParser;

impl AlbumDetailParser {
    /// `AlbumDetailViewModel.load()` without the view state: browse + parse.
    pub async fn load(
        inner_tube: &InnerTubeClient,
        browse_id: &str,
    ) -> Result<AlbumDetailInfo, InnerTubeError> {
        let json = inner_tube.browse(BrowseArgs::id(browse_id)).await?;
        Ok(Self::parse_album_detail(&json, browse_id))
    }

    /// Parses InnerTube browse JSON into an AlbumDetailInfo.
    /// Header metadata (title, artists, year, song count, duration, thumbnail)
    /// comes from the responsive or legacy detail header, songs from the shelves.
    pub fn parse_album_detail(json: &JsonDict, browse_id: &str) -> AlbumDetailInfo {
        let mut info = AlbumDetailInfo {
            title: "Unknown Album".to_string(),
            browse_id: browse_id.to_string(),
            ..AlbumDetailInfo::default()
        };

        // Resolve first tab section (exists in both layouts)
        let first_tab_section = BrowseLens::first_section_item(json);

        // --- Header: musicResponsiveHeaderRenderer (modern two-column albums) ---
        if let Some(responsive_header) =
            first_tab_section.and_then(|s| as_dict(s.get("musicResponsiveHeaderRenderer")))
        {
            if let Some(title) = InnerTubeJSON::runs_text(as_dict(responsive_header.get("title"))) {
                info.title = title;
            }
            // Artists from straplineTextOne
            if let Some(strapline) =
                InnerTubeDecode::array(&["straplineTextOne", "runs"], responsive_header)
            {
                info.artists = artists(&strapline);
            }
            // Artist fallback from subtitle runs
            if info.artists.is_empty()
                && let Some(subtitle) =
                    InnerTubeDecode::array(&["subtitle", "runs"], responsive_header)
            {
                info.artists = artists(&subtitle);
            }
            Self::apply_second_subtitle(responsive_header, &mut info);
            Self::apply_subtitle_year(responsive_header, &mut info);
            info.thumbnail_url = InnerTubeJSON::music_thumbnail_url(responsive_header);
        }

        // --- Header fallback: musicDetailHeaderRenderer (legacy / single-column) ---
        if info.title == "Unknown Album"
            && let Some(detail_header) =
                InnerTubeDecode::dict(&["header", "musicDetailHeaderRenderer"], json)
        {
            if let Some(title) = InnerTubeJSON::runs_text(as_dict(detail_header.get("title"))) {
                info.title = title;
            }
            if let Some(subtitle) = InnerTubeDecode::array(&["subtitle", "runs"], detail_header) {
                info.artists.extend(artists(&subtitle));
            }
            Self::apply_second_subtitle(detail_header, &mut info);
            Self::apply_subtitle_year(detail_header, &mut info);
            if info.thumbnail_url.is_none() {
                info.thumbnail_url = InnerTubeJSON::music_thumbnail_url(detail_header);
            }

            // Extract playlistId from menu items (needed for playback queue)
            for item in InnerTubeDecode::array(&["menu", "menuRenderer", "items"], detail_header)
                .unwrap_or_default()
            {
                if let Some(pid) = InnerTubeDecode::string(
                    &[
                        "menuNavigationItemRenderer",
                        "navigationEndpoint",
                        "watchEndpoint",
                        "playlistId",
                    ],
                    item,
                ) {
                    info.playlist_id = Some(pid.to_string());
                }
            }
        }

        // Fallback: extract playlistId from microformat URL query parameter
        // URL format: https://music.youtube.com/playlist?list=OLAK5uy_...
        if info.playlist_id.is_none()
            && let Some(url_canonical) = InnerTubeDecode::string(
                &["microformat", "microformatDataRenderer", "urlCanonical"],
                json,
            )
            && let Ok(url) = reqwest::Url::parse(url_canonical)
        {
            info.playlist_id = url
                .query_pairs()
                .find(|(key, _)| key == "list")
                .map(|(_, value)| value.into_owned());
        }

        // --- Songs ---
        // Album track rows carry no per-row thumbnail; fall back to the album art (same as Metrolist).
        let fallback_thumbnail = info.thumbnail_url.clone();
        let songs_from_shelf = |shelf: &JsonDict| -> Vec<SongItem> {
            InnerTubeDecode::array(&["musicPlaylistShelfRenderer", "contents"], shelf)
                .or_else(|| InnerTubeDecode::array(&["musicShelfRenderer", "contents"], shelf))
                .unwrap_or_default()
                .into_iter()
                .filter_map(|item| as_dict(item.get("musicResponsiveListItemRenderer")))
                .filter_map(SongItem::from)
                .map(|mut song| {
                    if song.thumbnail_url.is_none() {
                        song.thumbnail_url.clone_from(&fallback_thumbnail);
                    }
                    song
                })
                .collect()
        };

        if let Some(two_col) =
            InnerTubeDecode::dict(&["contents", "twoColumnBrowseResultsRenderer"], json)
        {
            // Songs are in secondaryContents for two-column albums
            for section in InnerTubeDecode::array(
                &["secondaryContents", "sectionListRenderer", "contents"],
                two_col,
            )
            .unwrap_or_default()
            {
                info.songs.extend(songs_from_shelf(section));
            }
            // Fallback: songs might still be in first tab section (e.g. musicShelfRenderer)
            if info.songs.is_empty()
                && let Some(first) = first_tab_section
            {
                info.songs.extend(songs_from_shelf(first));
            }
        } else if let Some(sections) = BrowseLens::browse_sections(json) {
            for section in sections {
                info.songs.extend(songs_from_shelf(section));
            }
        }

        // Set songCount from actual count if header value wasn't parsed
        if info.song_count == 0 {
            info.song_count = info.songs.len() as i64;
        }
        info
    }

    /// Year / songCount / duration from secondSubtitle.
    fn apply_second_subtitle(header: &JsonDict, info: &mut AlbumDetailInfo) {
        let Some(runs) = InnerTubeDecode::array(&["secondSubtitle", "runs"], header) else {
            return;
        };
        for run in runs {
            let Some(text) = as_string(run.get("text")) else {
                continue;
            };
            let trimmed = text.trim();
            if let Some(year) = year_of(trimmed) {
                info.year = Some(year);
            } else if ["song", "Song", "track", "Track"]
                .iter()
                .any(|w| trimmed.contains(w))
            {
                if let Some(count) = first_number(trimmed) {
                    info.song_count = count;
                }
            } else if let Some(parsed) = DurationFormat::parse_clock(trimmed)
                .or_else(|| DurationFormat::parse_spoken(trimmed))
            {
                // "1:14:05" or "1 hour, 14 minutes"
                info.duration = parsed;
            }
        }
    }

    /// Year fallback from subtitle ("Album • 2013"); the year left secondSubtitle.
    fn apply_subtitle_year(header: &JsonDict, info: &mut AlbumDetailInfo) {
        if info.year.is_some() {
            return;
        }
        for run in InnerTubeDecode::array(&["subtitle", "runs"], header).unwrap_or_default() {
            let trimmed = as_string(run.get("text")).unwrap_or_default().trim();
            if let Some(year) = year_of(trimmed) {
                info.year = Some(year);
            }
        }
    }
}
