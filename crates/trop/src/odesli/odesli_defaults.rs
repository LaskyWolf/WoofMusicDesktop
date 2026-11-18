// Trop/Odesli/OdesliDefaults.swift, plus `OdesliSettings` (desktop only).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use swiftytube::sync::incremental_sync_service::IncrementalSyncService;

pub struct OdesliDefaults;

impl OdesliDefaults {
    pub const API_URL: &str = "https://api.song.link/v1-alpha.1/links";
    pub const KEYCHAIN_SERVICE: &str = "com.woofmusic.odesli";
    pub const KEY_ACCOUNT: &str = "key";
    /// A public song, sent to check a new key so no link of the user's is involved.
    pub const VALIDATION_LINK: &str = "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT";
    /// The song.link page of a YouTube video: it lists the song on every service.
    pub const SONG_PAGE_URL: &str = "https://song.link/y/";

    pub fn song_page_url(video_id: &str) -> String {
        format!("{}{video_id}", Self::SONG_PAGE_URL)
    }
}

/// Desktop-only setting, stored like the Swift `UserDefaults` keys of the
/// other integrations. Missing keys read as the defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OdesliSettings {
    /// The player bar offers "Copy song link".
    #[serde(rename = "odesliShareLinks")]
    pub share_links: bool,
}

impl Default for OdesliSettings {
    fn default() -> Self {
        Self { share_links: true }
    }
}

impl OdesliSettings {
    /// `odesli_defaults.json` next to `IncrementalSyncService::default_defaults_path()`.
    pub fn default_path() -> Option<PathBuf> {
        IncrementalSyncService::default_defaults_path()
            .map(|path| path.with_file_name("odesli_defaults.json"))
    }

    /// A missing or unreadable file reads as the defaults.
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
