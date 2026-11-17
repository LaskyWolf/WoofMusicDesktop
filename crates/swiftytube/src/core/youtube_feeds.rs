use serde_json::Value;

use crate::networking::inner_tube_decode::{JsonDict, as_dict, as_dict_list, as_string};
use crate::networking::inner_tube_json::InnerTubeJSON;

/// Main-YouTube browse ids and channel-tab helpers. Used with the `.youtube`
/// service host; YouTube Music browse ids live with the `Sync` module.
pub struct YouTubeFeeds;

impl YouTubeFeeds {
    // MARK: - Top-level feeds (`browseId`)

    /// Home recommendations.
    pub const HOME: &str = "FEwhat_to_watch";
    /// Trending / Explore.
    pub const TRENDING: &str = "FEtrending";
    /// Subscription uploads (signed in).
    pub const SUBSCRIPTIONS: &str = "FEsubscriptions";
    /// Watch history (signed in).
    pub const HISTORY: &str = "FEhistory";
    /// Shorts feed.
    pub const SHORTS: &str = "FEshorts";

    // MARK: - Library playlists

    /// Liked videos. Browse with `playlist_browse_id(LIKED_VIDEOS_PLAYLIST_ID)`.
    pub const LIKED_VIDEOS_PLAYLIST_ID: &str = "LL";
    /// Watch later. Browse with `playlist_browse_id(WATCH_LATER_PLAYLIST_ID)`.
    pub const WATCH_LATER_PLAYLIST_ID: &str = "WL";

    /// Playlist `browseId` (`VL` + id); accepts an already-prefixed id.
    pub fn playlist_browse_id(playlist_id: &str) -> String {
        if playlist_id.starts_with("VL") {
            playlist_id.to_string()
        } else {
            format!("VL{playlist_id}")
        }
    }

    // MARK: - Channel tabs

    /// Parse the tab chips off a channel browse response. Reusing these
    /// `params` is safer than hardcoding tab protobufs that YouTube rotates.
    pub fn channel_tabs(json: &JsonDict) -> Vec<YouTubeChannelTab> {
        let mut result = Vec::new();
        for tab in Self::tab_renderers(json) {
            let Some(title) = Self::tab_title(tab) else {
                continue;
            };
            let browse =
                as_dict(as_dict(tab.get("endpoint")).and_then(|e| e.get("browseEndpoint")));
            result.push(YouTubeChannelTab {
                title,
                params: as_string(browse.and_then(|b| b.get("params"))).map(str::to_string),
                browse_id: as_string(browse.and_then(|b| b.get("browseId"))).map(str::to_string),
                selected: tab
                    .get("selected")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }
        result
    }

    pub fn tab_renderers(json: &JsonDict) -> Vec<&JsonDict> {
        let Some(contents) = as_dict(json.get("contents")) else {
            return Vec::new();
        };
        for key in [
            "twoColumnBrowseResultsRenderer",
            "singleColumnBrowseResultsRenderer",
        ] {
            let Some(tabs) = as_dict_list(as_dict(contents.get(key)).and_then(|r| r.get("tabs")))
            else {
                continue;
            };
            let renderers: Vec<&JsonDict> = tabs
                .iter()
                .filter_map(|t| as_dict(t.get("tabRenderer")))
                .collect();
            if !renderers.is_empty() {
                return renderers;
            }
        }
        Vec::new()
    }

    pub fn tab_title(tab: &JsonDict) -> Option<String> {
        if let Some(title) = as_string(tab.get("title")).filter(|t| !t.is_empty()) {
            return Some(title.to_string());
        }
        InnerTubeJSON::runs_text(as_dict(tab.get("title")))
    }
}

/// A chip on a channel page (Videos / Shorts / Live / Playlists / ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct YouTubeChannelTab {
    pub title: String,
    pub params: Option<String>,
    pub browse_id: Option<String>,
    pub selected: bool,
}
