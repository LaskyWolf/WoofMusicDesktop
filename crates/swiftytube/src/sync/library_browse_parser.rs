// SwiftyTube/Sync/LibraryBrowseParser.swift.

use std::sync::LazyLock;

use regex::Regex;

use super::browse_lens::BrowseLens;
use super::library_parsed_models::{
    ParsedAlbum, ParsedArtist, ParsedPlaylist, ParsedPodcast, ParsedSong,
};
use crate::formatting::duration_format::DurationFormat;
use crate::networking::inner_tube_decode::{
    InnerTubeDecode, JsonDict, as_dict, as_dict_list, as_string,
};
use crate::networking::inner_tube_json::InnerTubeJSON;

/// Swift `(?<=[sh]\d+)-c`; Rust regex has no lookbehind, so the prefix is captured and put back.
static CROP_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([sh]\d+)-c").unwrap());
static VIEWS_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\d+(\.\d+)?[KMBT]?\s*(views|downloads|listeners|subscribers)$").unwrap()
});

pub struct LibraryBrowseParser;

impl LibraryBrowseParser {
    /// The `musicResponsiveListItemRenderer` / two-row items of a library
    /// page. Handles first pages (single/two-column, flat privately-owned
    /// contents) and continuation pages.
    pub fn extract_items(json: &JsonDict) -> Option<Vec<&JsonDict>> {
        fn items_from_content(content: &JsonDict) -> Option<Vec<&JsonDict>> {
            [
                "musicShelfRenderer",
                "gridRenderer",
                "musicPlaylistShelfRenderer",
            ]
            .into_iter()
            .find_map(|key| {
                let renderer = as_dict(content.get(key))?;
                as_dict_list(renderer.get("contents"))
                    .or_else(|| as_dict_list(renderer.get("items")))
            })
        }
        // itemSectionRenderer wrapper first (most library pages).
        fn items_from(section: &JsonDict) -> Option<Vec<&JsonDict>> {
            let wrapped = InnerTubeDecode::array(&["itemSectionRenderer", "contents"], section)
                .and_then(|contents| contents.first().copied());
            match wrapped {
                Some(first) => items_from_content(first),
                None => items_from_content(section),
            }
        }

        if let Some(items) = BrowseLens::first_browse_section(json).and_then(items_from) {
            return Some(items);
        }
        // twoColumnBrowseResultsRenderer (VLSE, VL* playlists).
        if let Some(items) = Self::two_column_sections(json)
            .and_then(|sections| sections.first().copied())
            .and_then(items_from)
        {
            return Some(items);
        }
        // Flat contents (FEmusic_library_privately_owned_*).
        if let Some(items) = as_dict_list(json.get("contents"))
            .and_then(|contents| contents.first().copied())
            .and_then(items_from)
        {
            return Some(items);
        }
        let continuation = as_dict(json.get("continuationContents"))?;
        [
            ("musicShelfContinuation", "contents"),
            ("gridContinuation", "items"),
            ("musicPlaylistShelfContinuation", "contents"),
        ]
        .into_iter()
        .find_map(|(key, list)| InnerTubeDecode::array(&[key, list], continuation))
    }

    pub fn extract_continuation_token(json: &JsonDict) -> Option<String> {
        fn unwrap_section(section: &JsonDict) -> &JsonDict {
            let first = InnerTubeDecode::array(&["itemSectionRenderer", "contents"], section)
                .and_then(|contents| contents.first().copied());
            first
                .and_then(|first| {
                    [
                        "musicShelfRenderer",
                        "gridRenderer",
                        "musicPlaylistShelfRenderer",
                    ]
                    .into_iter()
                    .find_map(|key| as_dict(first.get(key)))
                })
                .unwrap_or(section)
        }
        fn token_in(sections: Option<Vec<&JsonDict>>) -> Option<String> {
            sections?.into_iter().find_map(|section| {
                InnerTubeDecode::continuation_token(Some(unwrap_section(section)))
                    .map(str::to_string)
            })
        }

        token_in(BrowseLens::browse_sections(json))
            .or_else(|| token_in(Self::two_column_sections(json)))
            .or_else(|| token_in(as_dict_list(json.get("contents"))))
            .or_else(|| {
                let continuation = as_dict(json.get("continuationContents"))?;
                [
                    "musicShelfContinuation",
                    "gridContinuation",
                    "musicPlaylistShelfContinuation",
                ]
                .into_iter()
                .find_map(|key| InnerTubeDecode::continuation_token(as_dict(continuation.get(key))))
                .map(str::to_string)
            })
    }

    fn two_column_sections(json: &JsonDict) -> Option<Vec<&JsonDict>> {
        InnerTubeDecode::array(
            &[
                "contents",
                "twoColumnBrowseResultsRenderer",
                "secondaryContents",
                "sectionListRenderer",
                "contents",
            ],
            json,
        )
    }
}

// MARK: Renderer extraction

impl LibraryBrowseParser {
    /// Strips YTMusic's crop parameter (`-c`) from thumbnail URLs so the full
    /// uncropped image is returned instead of a square-cropped version.
    fn uncropped_url(url: &str) -> String {
        CROP_PATTERN.replace_all(url, "$1").into_owned()
    }

    fn responsive_list_item(item: &JsonDict) -> Option<&JsonDict> {
        as_dict(item.get("musicResponsiveListItemRenderer"))
    }

    fn flex_runs(item: &JsonDict, index: usize) -> Vec<&JsonDict> {
        let Some(column) =
            as_dict_list(item.get("flexColumns")).and_then(|columns| columns.get(index).copied())
        else {
            return Vec::new();
        };
        let renderer = as_dict(column.get("musicResponsiveListItemFlexColumnRenderer"))
            .or_else(|| as_dict(column.get("musicResponsiveListItemColumnRenderer")));
        renderer
            .and_then(|r| InnerTubeDecode::array(&["text", "runs"], r))
            .unwrap_or_default()
    }

    fn flex_text(item: &JsonDict, index: usize) -> Option<String> {
        let first = Self::flex_runs(item, index).into_iter().next()?;
        as_string(first.get("text")).map(str::to_string)
    }

    fn all_flex_text_runs(item: &JsonDict, index: usize) -> Vec<String> {
        Self::flex_runs(item, index)
            .into_iter()
            .filter_map(|run| as_string(run.get("text")).map(str::to_string))
            .collect()
    }

    fn thumbnail_url(item: &JsonDict) -> Option<String> {
        let thumbnail = as_dict(item.get("thumbnail"))?;
        let music_thumbnail = as_dict(thumbnail.get("musicThumbnailRenderer"))?;
        let thumb = as_dict(music_thumbnail.get("thumbnail"))
            .or_else(|| as_dict(thumbnail.get("thumbnails")))?;
        let thumbnails = as_dict_list(thumb.get("thumbnails"));
        let url = InnerTubeJSON::last_thumbnail_url(thumbnails.as_deref())?;
        Some(Self::uncropped_url(&url))
    }

    fn menu_items(item: &JsonDict) -> Option<Vec<&JsonDict>> {
        InnerTubeDecode::array(&["menu", "menuRenderer", "items"], item)
    }

    fn two_row_item(item: &JsonDict) -> Option<&JsonDict> {
        as_dict(item.get("musicTwoRowItemRenderer"))
    }

    fn first_run_text(item: &JsonDict, key: &str) -> Option<String> {
        let runs = InnerTubeDecode::array(&[key, "runs"], item)?;
        as_string(runs.first()?.get("text")).map(str::to_string)
    }

    fn two_row_thumbnail_url(item: &JsonDict) -> Option<String> {
        [
            &[
                "thumbnailRenderer",
                "musicThumbnailRenderer",
                "thumbnail",
                "thumbnails",
            ],
            &[
                "thumbnail",
                "musicThumbnailRenderer",
                "thumbnail",
                "thumbnails",
            ],
        ]
        .into_iter()
        .find_map(|path| {
            let thumbnails = InnerTubeDecode::array(path, item);
            InnerTubeJSON::last_thumbnail_url(thumbnails.as_deref())
        })
        .map(|url| Self::uncropped_url(&url))
    }

    fn browse_id(item: &JsonDict) -> Option<String> {
        InnerTubeDecode::string(&["navigationEndpoint", "browseEndpoint", "browseId"], item)
            .map(str::to_string)
    }

    /// Renderer's endpoint first, then the item's: a browse id, else a watch playlist id.
    fn two_row_browse_id(renderer: &JsonDict, item: &JsonDict) -> Option<String> {
        [renderer, item].into_iter().find_map(|dict| {
            let nav = as_dict(dict.get("navigationEndpoint"))?;
            InnerTubeDecode::string(&["browseEndpoint", "browseId"], nav)
                .or_else(|| InnerTubeDecode::string(&["watchEndpoint", "playlistId"], nav))
                .map(str::to_string)
        })
    }

    fn playlist_id(item: &JsonDict) -> Option<String> {
        InnerTubeDecode::string(&["navigationEndpoint", "watchEndpoint", "playlistId"], item)
            .map(str::to_string)
    }
}

// MARK: Parsing helpers

#[derive(Default)]
struct LibraryToggleState {
    add_token: Option<String>,
    remove_token: Option<String>,
    is_toggled: bool,
}

impl LibraryBrowseParser {
    fn library_tokens(item: &JsonDict) -> LibraryToggleState {
        for menu_item in Self::menu_items(item).unwrap_or_default() {
            let Some(toggle) = as_dict(menu_item.get("toggleMenuServiceItemRenderer")) else {
                continue;
            };
            let default_icon_type = InnerTubeDecode::string(&["defaultIcon", "iconType"], toggle);
            let default_token = Self::feedback_token(toggle, "defaultServiceEndpoint");
            let toggled_token = Self::feedback_token(toggle, "toggledServiceEndpoint");
            // The current state follows the default icon: a remove/check icon means toggled.
            let is_toggled = default_icon_type
                .is_some_and(|icon| icon.contains("REMOVE") || icon == "CHECK_CHECK");
            if default_token.is_some() || toggled_token.is_some() {
                return LibraryToggleState {
                    add_token: default_token,
                    remove_token: toggled_token,
                    is_toggled,
                };
            }
        }
        LibraryToggleState::default()
    }

    fn feedback_token(toggle: &JsonDict, endpoint: &str) -> Option<String> {
        InnerTubeDecode::string(&[endpoint, "feedbackEndpoint", "feedbackToken"], toggle)
            .map(str::to_string)
    }

    // Library rows keep the length in fixedColumns; badges hold the explicit mark.
    fn duration_seconds(item: &JsonDict) -> i64 {
        DurationFormat::parse_renderer_duration(item)
    }
}

// MARK: Item parsers

impl LibraryBrowseParser {
    pub fn parse_song(item: &JsonDict) -> Option<ParsedSong> {
        let renderer = Self::responsive_list_item(item)?;
        // videoId at renderer level, in playlistItemData, or in the watch endpoint.
        let video_id = as_string(renderer.get("videoId"))
            .or_else(|| InnerTubeDecode::string(&["playlistItemData", "videoId"], renderer))
            .or_else(|| {
                InnerTubeDecode::string(
                    &["navigationEndpoint", "watchEndpoint", "videoId"],
                    renderer,
                )
            })?
            .to_string();
        let title = Self::flex_text(renderer, 0).unwrap_or_else(|| "Unknown".into());
        const NON_ARTIST_LABELS: [&str; 6] =
            ["song", "video", "track", "music", "podcast", "episode"];
        const CONJUNCTIONS: [&str; 6] = [",", "&", "and", "feat.", "ft.", "featuring"];
        let artists = Self::all_flex_text_runs(renderer, 1)
            .into_iter()
            .filter(|run| {
                let trimmed = run.trim();
                let lower = trimmed.to_lowercase();
                trimmed != " • "
                    && !trimmed.starts_with("http")
                    && !NON_ARTIST_LABELS.contains(&lower.as_str())
                    && !CONJUNCTIONS.contains(&lower.as_str())
                    && !VIEWS_PATTERN.is_match(&lower)
            })
            .collect();
        let album = Self::all_flex_text_runs(renderer, 2)
            .into_iter()
            .find(|run| run != " • " && !run.starts_with("http"));
        let tokens = Self::library_tokens(renderer);
        Some(ParsedSong {
            video_id,
            title,
            artists,
            artist_ids: Vec::new(),
            album,
            album_id: None,
            duration: Self::duration_seconds(renderer),
            thumbnail_url: Self::thumbnail_url(renderer),
            is_liked: tokens.is_toggled,
            library_add_token: tokens.add_token,
            library_remove_token: tokens.remove_token,
        })
    }

    pub fn parse_album(item: &JsonDict) -> Option<ParsedAlbum> {
        // Shelf view.
        if let Some(renderer) = Self::responsive_list_item(item)
            && let Some(browse_id) = Self::browse_id(renderer)
        {
            let title = Self::flex_text(renderer, 0).unwrap_or_else(|| "Unknown".into());
            let subtitle_runs = Self::all_flex_text_runs(renderer, 1);
            let artist = subtitle_runs
                .iter()
                .find(|run| *run != " • " && !run.starts_with("http"))
                .cloned();
            let mut song_count = 0;
            let mut duration = 0;
            for run in &subtitle_runs {
                if run.contains("song") || run.contains("Songs") || run.contains("tracks") {
                    let num = run.trim().split(' ').find(|s| !s.is_empty()).unwrap_or("");
                    song_count = num.parse().unwrap_or(0);
                }
                if run.contains(':') {
                    duration = DurationFormat::parse_clock(run)
                        .or_else(|| run.parse().ok())
                        .unwrap_or(0);
                }
            }
            return Some(ParsedAlbum {
                browse_id,
                title,
                artist,
                thumbnail_url: Self::thumbnail_url(renderer),
                song_count,
                duration,
                playlist_id: Self::playlist_id(renderer),
            });
        }
        // Grid view.
        let renderer = Self::two_row_item(item)?;
        let browse_id = Self::two_row_browse_id(renderer, item)?;
        Some(ParsedAlbum {
            browse_id,
            title: Self::first_run_text(renderer, "title").unwrap_or_else(|| "Unknown".into()),
            artist: Self::first_run_text(renderer, "subtitle"),
            thumbnail_url: Self::two_row_thumbnail_url(renderer),
            song_count: 0,
            duration: 0,
            playlist_id: None,
        })
    }

    pub fn parse_artist(item: &JsonDict) -> Option<ParsedArtist> {
        // Shelf view.
        if let Some(renderer) = Self::responsive_list_item(item)
            && let Some(browse_id) = Self::browse_id(renderer)
        {
            return Some(ParsedArtist {
                name: Self::flex_text(renderer, 0).unwrap_or_else(|| "Unknown".into()),
                thumbnail_url: Self::thumbnail_url(renderer),
                is_subscribed: Self::library_tokens(renderer).is_toggled,
                channel_id: Some(browse_id.clone()),
                browse_id,
            });
        }
        // Grid view.
        let renderer = Self::two_row_item(item)?;
        let browse_id = Self::two_row_browse_id(renderer, item)?;
        Some(ParsedArtist {
            name: Self::first_run_text(renderer, "title").unwrap_or_else(|| "Unknown".into()),
            thumbnail_url: Self::two_row_thumbnail_url(renderer),
            is_subscribed: false,
            channel_id: Some(browse_id.clone()),
            browse_id,
        })
    }

    pub fn parse_playlist(item: &JsonDict) -> Option<ParsedPlaylist> {
        // Shelf view.
        if let Some(renderer) = Self::responsive_list_item(item)
            && let Some(browse_id) = Self::browse_id(renderer)
        {
            return Some(ParsedPlaylist {
                browse_id,
                title: Self::flex_text(renderer, 0).unwrap_or_else(|| "Unknown".into()),
                song_count: Self::all_flex_text_runs(renderer, 1)
                    .iter()
                    .find_map(|run| run.trim().parse().ok()),
                thumbnail_url: Self::thumbnail_url(renderer),
            });
        }
        // Grid view.
        let renderer = Self::two_row_item(item)?;
        let browse_id = Self::two_row_browse_id(renderer, item)?;
        Some(ParsedPlaylist {
            browse_id,
            title: Self::first_run_text(renderer, "title").unwrap_or_else(|| "Unknown".into()),
            song_count: None,
            thumbnail_url: Self::two_row_thumbnail_url(renderer),
        })
    }
}

// MARK: Podcast parser

impl LibraryBrowseParser {
    pub fn parse_podcast(item: &JsonDict) -> Option<ParsedPodcast> {
        if let Some(renderer) = Self::two_row_item(item)
            && let Some(browse_id) = Self::two_row_browse_id(renderer, item)
        {
            return Some(ParsedPodcast {
                browse_id,
                name: Self::first_run_text(renderer, "title").unwrap_or_else(|| "Unknown".into()),
                thumbnail_url: Self::two_row_thumbnail_url(renderer),
                is_subscribed: true,
            });
        }
        let renderer = Self::responsive_list_item(item)?;
        let browse_id = Self::browse_id(renderer)?;
        Some(ParsedPodcast {
            browse_id,
            name: Self::flex_text(renderer, 0).unwrap_or_else(|| "Unknown".into()),
            thumbnail_url: Self::thumbnail_url(renderer),
            is_subscribed: Self::library_tokens(renderer).is_toggled,
        })
    }
}

// MARK: Convenience parsers

impl LibraryBrowseParser {
    fn parse_all<T>(response: &JsonDict, parse: fn(&JsonDict) -> Option<T>) -> Vec<T> {
        Self::extract_items(response)
            .unwrap_or_default()
            .into_iter()
            .filter_map(parse)
            .collect()
    }

    pub fn parse_songs(response: &JsonDict) -> Vec<ParsedSong> {
        Self::parse_all(response, Self::parse_song)
    }

    pub fn parse_albums(response: &JsonDict) -> Vec<ParsedAlbum> {
        Self::parse_all(response, Self::parse_album)
    }

    pub fn parse_artists(response: &JsonDict) -> Vec<ParsedArtist> {
        Self::parse_all(response, Self::parse_artist)
    }

    pub fn parse_playlists(response: &JsonDict) -> Vec<ParsedPlaylist> {
        Self::parse_all(response, Self::parse_playlist)
    }

    pub fn parse_podcasts(response: &JsonDict) -> Vec<ParsedPodcast> {
        Self::parse_all(response, Self::parse_podcast)
    }

    /// Community playlists off an artist page: shelf rows whose browse id is
    /// a plain playlist (no radio, mixes, or album links), capped at 10.
    pub fn extract_community_playlists(json: &JsonDict) -> Vec<ParsedPlaylist> {
        let mut playlists = Vec::new();
        for section in BrowseLens::browse_sections(json).unwrap_or_default() {
            let Some(contents) =
                InnerTubeDecode::array(&["musicShelfRenderer", "contents"], section)
            else {
                continue;
            };
            for entry in contents {
                if playlists.len() >= 10 {
                    return playlists;
                }
                let Some(browse_id) = InnerTubeDecode::string(
                    &[
                        "musicResponsiveListItemRenderer",
                        "navigationEndpoint",
                        "browseEndpoint",
                        "browseId",
                    ],
                    entry,
                ) else {
                    continue;
                };
                let stripped = Self::normalize_playlist_id(browse_id);
                if ["RD", "OLAK", "MPREb_"]
                    .iter()
                    .any(|prefix| stripped.starts_with(prefix))
                {
                    continue;
                }
                playlists.extend(Self::parse_playlist(entry));
            }
        }
        playlists
    }

    /// Strips the `VL` browse prefix down to the raw playlist id.
    pub fn normalize_playlist_id(id: &str) -> &str {
        id.strip_prefix("VL").unwrap_or(id)
    }
}
