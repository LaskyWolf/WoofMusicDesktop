// Trop/LastFM/LastFMDefaults.swift. The `UserDefaults` keys become the JSON
// keys of `LastFMSettings`, stored in its own file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use swiftytube::sync::incremental_sync_service::IncrementalSyncService;

pub struct LastFMDefaults;

impl LastFMDefaults {
    pub const BASE_URL: &str = "https://ws.audioscrobbler.com/2.0/";
    pub const AUTH_URL_BASE: &str = "https://www.last.fm/api/auth/";

    // Scrobble defaults (mirrors Metrolist).
    pub const DEFAULT_SCROBBLE_DELAY_PERCENT: f32 = 0.5;
    pub const DEFAULT_SCROBBLE_MIN_DURATION: i64 = 30;
    pub const DEFAULT_SCROBBLE_DELAY_SECONDS: i64 = 180;

    /// The `LASTFM_API_KEY` environment variable at run time, else at build
    /// time (Swift reads Info.plist in release builds).
    pub fn api_key() -> String {
        Self::credential(
            std::env::var("LASTFM_API_KEY").ok(),
            option_env!("LASTFM_API_KEY"),
        )
    }

    /// `LASTFM_SECRET`, read like `api_key`.
    pub fn secret() -> String {
        Self::credential(
            std::env::var("LASTFM_SECRET").ok(),
            option_env!("LASTFM_SECRET"),
        )
    }

    /// The page where the user approves `token`. Swift reads `api_key` itself;
    /// here the caller passes `LastFMService::api_key`.
    pub fn auth_url(api_key: &str, token: &str) -> String {
        format!("{}?api_key={api_key}&token={token}", Self::AUTH_URL_BASE)
    }

    fn credential(runtime: Option<String>, build_time: Option<&str>) -> String {
        [runtime.as_deref(), build_time]
            .into_iter()
            .flatten()
            .map(str::trim)
            .find(|value| !value.is_empty())
            .unwrap_or_default()
            .to_string()
    }
}

/// The Last.fm settings Swift keeps in `UserDefaults`, under the same keys
/// (compatible with Metrolist naming). Missing keys read as the Swift defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LastFMSettings {
    #[serde(rename = "lastfmUsername", skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(rename = "lastfmScrobblingEnable")]
    pub scrobbling_enabled: bool,
    #[serde(rename = "lastfmUseNowPlaying")]
    pub use_now_playing: bool,
    #[serde(rename = "lastfmUseSendLikes")]
    pub use_send_likes: bool,
    #[serde(rename = "scrobbleDelayPercent")]
    pub scrobble_delay_percent: f32,
    #[serde(rename = "scrobbleMinSongDuration")]
    pub scrobble_min_duration: i64,
    #[serde(rename = "scrobbleDelaySeconds")]
    pub scrobble_delay_seconds: i64,
}

impl Default for LastFMSettings {
    fn default() -> Self {
        Self {
            username: None,
            scrobbling_enabled: false,
            use_now_playing: false,
            use_send_likes: false,
            scrobble_delay_percent: LastFMDefaults::DEFAULT_SCROBBLE_DELAY_PERCENT,
            scrobble_min_duration: LastFMDefaults::DEFAULT_SCROBBLE_MIN_DURATION,
            scrobble_delay_seconds: LastFMDefaults::DEFAULT_SCROBBLE_DELAY_SECONDS,
        }
    }
}

impl LastFMSettings {
    /// `last_fm_defaults.json` next to `IncrementalSyncService::default_defaults_path()`.
    pub fn default_path() -> Option<PathBuf> {
        IncrementalSyncService::default_defaults_path()
            .map(|path| path.with_file_name("last_fm_defaults.json"))
    }

    /// A missing or unreadable file reads as the defaults, like unset keys.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec(self)?)
    }
}
