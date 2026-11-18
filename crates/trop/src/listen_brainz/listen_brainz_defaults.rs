// Trop/ListenBrainz/ListenBrainzDefaults.swift. The `UserDefaults` key becomes
// the JSON key of `ListenBrainzSettings`, stored in its own file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use swiftytube::sync::incremental_sync_service::IncrementalSyncService;

pub struct ListenBrainzDefaults;

impl ListenBrainzDefaults {
    pub const API_BASE: &str = "https://api.listenbrainz.org/1/";
    pub const KEYCHAIN_SERVICE: &str = "com.woofmusic.listenbrainz";
    pub const TOKEN_ACCOUNT: &str = "token";

    // A song counts once half of it, or 4 minutes, has played: ListenBrainz's own rule.
    pub const MIN_SONG_DURATION: f64 = 30.0;
    pub const MAX_LISTEN_SECONDS: f64 = 240.0;
    // A couple of seconds in, so the title and artist are final.
    pub const NOW_PLAYING_DELAY_SECONDS: f64 = 2.0;

    pub const CLIENT_NAME: &str = "Woof Music";
}

/// What Swift keeps in `UserDefaults` for ListenBrainz, under the same key.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListenBrainzSettings {
    #[serde(
        rename = "listenBrainz.userName",
        skip_serializing_if = "Option::is_none"
    )]
    pub user_name: Option<String>,
}

impl ListenBrainzSettings {
    /// `listen_brainz_defaults.json` next to `IncrementalSyncService::default_defaults_path()`.
    pub fn default_path() -> Option<PathBuf> {
        IncrementalSyncService::default_defaults_path()
            .map(|path| path.with_file_name("listen_brainz_defaults.json"))
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
