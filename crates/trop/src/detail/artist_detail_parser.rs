// Parsing half of Trop/Detail/ArtistDetailView.swift. The Swift file mixes the
// SwiftUI view, its view model and this parser; only the parser and the
// browse call of `ArtistDetailViewModel.load()` are translated. The hero
// artwork preload (Nuke) is a UI concern and stays out.

use swiftytube::networking::inner_tube_client::{BrowseArgs, InnerTubeClient};
use swiftytube::networking::inner_tube_decode::{
    InnerTubeDecode, JsonDict, as_dict, as_dict_list, as_string,
};
use swiftytube::networking::inner_tube_error::InnerTubeError;
use swiftytube::networking::inner_tube_json::InnerTubeJSON;

use super::detail_models::{ArtistDetailCache, ArtistDetailInfo};
use crate::networking::models::home_page_parser::HomePageParser;
use crate::networking::models::yt_item::{AlbumItem, ArtistItem, PlaylistItem, SongItem};
use swiftytube::sync::browse_lens::BrowseLens;

/// Singles & EPs share the album page type, so the shelf title
/// ("Singles & EPs") or subtitle ("Single • 2024") decides.
const SINGLE_OR_EP_WORDS: &[&str] = &["single", "singles", "ep", "eps"];

fn has_single_or_ep_word(text: &str) -> bool {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|word| SINGLE_OR_EP_WORDS.contains(&word))
}

pub struct ArtistDetailParser;

impl ArtistDetailParser {
    /// `ArtistDetailViewModel.load()` without the view state: browse + parse,
    /// stored in `ArtistDetailCache`.
    pub async fn load(
        inner_tube: &InnerTubeClient,
        browse_id: &str,
    ) -> Result<ArtistDetailInfo, InnerTubeError> {
        let json = inner_tube.browse(BrowseArgs::id(browse_id)).await?;
        let parsed = Self::parse_artist_detail(&json, browse_id);
        ArtistDetailCache::store(parsed.clone());
        Ok(parsed)
    }

    /// Parses InnerTube browse JSON into an ArtistDetailInfo.
    /// Handles both musicImmersiveHeaderRenderer and musicResponsiveHeaderRenderer for the header,
    /// then extracts top songs from musicShelfRenderer and albums, singles, videos,
    /// playlists and related artists from musicCarouselShelfRenderer shelves.
    pub fn parse_artist_detail(json: &JsonDict, browse_id: &str) -> ArtistDetailInfo {
        let mut info = ArtistDetailInfo {
            name: "Unknown Artist".to_string(),
            browse_id: browse_id.to_string(),
            ..ArtistDetailInfo::default()
        };

        // --- Header ---
        // Artist pages can use either an immersive header (large background image)
        // or a responsive header (smaller, more compact).
        let header = as_dict(json.get("header"));
        let header_renderer = header.and_then(|h| {
            as_dict(h.get("musicImmersiveHeaderRenderer"))
                .or_else(|| as_dict(h.get("musicResponsiveHeaderRenderer")))
        });
        if let Some(renderer) = header_renderer {
            info.name = InnerTubeJSON::runs_text(as_dict(renderer.get("title")))
                .unwrap_or_else(|| "Unknown Artist".to_string());
            info.thumbnail_url = InnerTubeJSON::music_thumbnail_url(renderer);
            let (is_subscribed, subscriber_count_text) =
                Self::extract_subscription_details(renderer);
            info.is_subscribed = is_subscribed.unwrap_or(false);
            info.subscriber_count_text = subscriber_count_text;
        }

        if (info.name == "Unknown Artist" || info.thumbnail_url.is_none())
            && let Some(mf_renderer) =
                InnerTubeDecode::dict(&["microformat", "microformatDataRenderer"], json)
        {
            if info.name == "Unknown Artist"
                && let Some(mf_title) = as_string(mf_renderer.get("title"))
            {
                info.name = mf_title.to_string();
            }
            if info.thumbnail_url.is_none() {
                info.thumbnail_url = InnerTubeJSON::nested_thumbnail_url(Some(mf_renderer));
            }
            if info.description_text.is_none() {
                info.description_text =
                    as_string(mf_renderer.get("description")).map(str::to_string);
            }
        }

        // --- Sections ---
        for section in BrowseLens::browse_sections(json).unwrap_or_default() {
            // musicShelfRenderer typically contains a list of songs
            for item in InnerTubeDecode::array(&["musicShelfRenderer", "contents"], section)
                .unwrap_or_default()
            {
                if let Some(song) =
                    as_dict(item.get("musicResponsiveListItemRenderer")).and_then(SongItem::from)
                {
                    info.songs.push(song);
                }
            }

            // musicDescriptionShelfRenderer is the "About" shelf; it beats the microformat text.
            if let Some(about) = InnerTubeJSON::runs_text(InnerTubeDecode::dict(
                &["musicDescriptionShelfRenderer", "description"],
                section,
            ))
            .filter(|a| !a.is_empty())
            {
                info.description_text = Some(about);
            }

            // musicCarouselShelfRenderer holds albums, singles, videos,
            // playlists and related artists — classified per shelf below.
            if let Some(carousel) = as_dict(section.get("musicCarouselShelfRenderer"))
                && let Some(items) = as_dict_list(carousel.get("contents"))
            {
                let shelf_title = Self::extract_carousel_title(carousel);
                for item in items {
                    if let Some(two_row) = as_dict(item.get("musicTwoRowItemRenderer")) {
                        Self::classify_carousel_item(two_row, &shelf_title, &mut info);
                    }
                }
            }
        }
        info
    }

    /// Routes one carousel tile into albums, singles, videos, playlists or
    /// related artists based on its page type (and shelf/subtitle hints for
    /// singles & EPs, which share the album page type).
    fn classify_carousel_item(two_row: &JsonDict, shelf_title: &str, info: &mut ArtistDetailInfo) {
        let page_type = HomePageParser::extract_page_type(two_row);
        match page_type {
            Some("MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK") => {
                let Some(album) = AlbumItem::from(two_row) else {
                    return;
                };
                if Self::is_single_or_ep(shelf_title, two_row) {
                    info.singles.push(album);
                } else {
                    info.albums.push(album);
                }
            }
            Some("MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL") => {
                info.related_artists.extend(ArtistItem::from(two_row));
            }
            Some("MUSIC_PAGE_TYPE_PLAYLIST") => {
                info.playlists.extend(PlaylistItem::from(two_row));
            }
            _ => {
                // Music videos expose a watch endpoint instead of a browse page.
                if page_type == Some("MUSIC_PAGE_TYPE_VIDEO")
                    || HomePageParser::has_watch_endpoint(two_row)
                {
                    info.videos.extend(SongItem::from(two_row));
                }
            }
        }
    }

    fn is_single_or_ep(shelf_title: &str, renderer: &JsonDict) -> bool {
        has_single_or_ep_word(shelf_title)
            || InnerTubeJSON::runs_texts(as_dict(renderer.get("subtitle")))
                .iter()
                .any(|t| has_single_or_ep_word(t))
    }

    /// YouTube Music subscription payloads vary: the subscribed state is in `subscriptionButton`,
    /// the count in `subscriptionButton2`, and older responses include a notification toggle.
    fn extract_subscription_details(header: &JsonDict) -> (Option<bool>, Option<String>) {
        let mut is_subscribed = None;
        let mut subscriber_count_text = None;
        for key in ["subscriptionButton2", "subscriptionButton"] {
            let Some(button) = as_dict(header.get(key)) else {
                continue;
            };
            if let Some(subscribe) = as_dict(button.get("subscribeButtonRenderer")) {
                is_subscribed = is_subscribed.or_else(|| subscribe.get("subscribed")?.as_bool());
                if subscriber_count_text.is_none() {
                    subscriber_count_text = [
                        "subscriberCountWithSubscribeText",
                        "longSubscriberCountText",
                        "shortSubscriberCountText",
                    ]
                    .iter()
                    .find_map(|count_key| {
                        InnerTubeJSON::runs_text(as_dict(subscribe.get(*count_key)))
                            .filter(|c| !c.is_empty())
                    });
                }
            }
            if let Some(toggle) =
                as_dict(button.get("subscriptionNotificationToggleButtonRenderer"))
            {
                is_subscribed = is_subscribed.or_else(|| toggle.get("subscribed")?.as_bool());
                if subscriber_count_text.is_none() {
                    subscriber_count_text =
                        InnerTubeJSON::runs_text(as_dict(toggle.get("subscribedText")))
                            .filter(|c| !c.is_empty());
                }
            }
        }
        (is_subscribed, subscriber_count_text)
    }

    /// Extracts the title from a musicCarouselShelfBasicHeaderRenderer.
    fn extract_carousel_title(carousel: &JsonDict) -> String {
        InnerTubeJSON::runs_text(InnerTubeDecode::dict(
            &["header", "musicCarouselShelfBasicHeaderRenderer", "title"],
            carousel,
        ))
        .unwrap_or_else(|| "Unknown".to_string())
    }
}
