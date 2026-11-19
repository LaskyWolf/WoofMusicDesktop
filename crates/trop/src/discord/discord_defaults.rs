// Trop/Discord/DiscordDefaults.swift, plus the `UserDefaults` keys that
// DiscordIntegration.swift and DiscordSettingsView.swift read. The keys become
// the JSON keys of `DiscordSettings`, stored in its own file.
//
// Not ported: `oauthToken` and `clientSecret`, used only by the iOS sign-in.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use swiftytube::sync::incremental_sync_service::IncrementalSyncService;

pub struct DiscordDefaults;

impl DiscordDefaults {
    pub const APP_ID: &str = "1563918327238107187";

    // Activity / presence defaults
    pub const YOUTUBE_WATCH_URL: &str = "https://music.youtube.com/watch?v=";
    pub const BUTTON1_LABEL: &str = "Listen on YouTube Music";
    pub const BUTTON1_URL_TEMPLATE: &str = "https://music.youtube.com/watch?v={song.id}";
    pub const BUTTON2_LABEL: &str = "Visit Woof Music";
    pub const BUTTON2_URL: &str = "https://github.com/LaskyWolf/WoofMusic";
    pub const STATE_TEMPLATE: &str = "{artist.name}";
    pub const DETAILS_TEMPLATE: &str = "{song.name}";
    /// `ActivityType::Listening`, as text like the Swift picker stores it.
    pub const ACTIVITY_TYPE: &str = "2";
    pub const ACTIVITY_NAME: &str = "";
    pub const UNKNOWN_ARTIST: &str = "Unknown Artist";
    pub const UNKNOWN_ALBUM: &str = "Unknown Album";
}

/// The Discord settings Swift keeps in `UserDefaults`, under the same keys.
/// Missing keys read as the Swift defaults. Not ported: `discordUserStatus`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscordSettings {
    #[serde(rename = "discordRPCEnable")]
    pub enabled: bool,
    #[serde(rename = "discordAdvancedMode")]
    pub advanced_mode: bool,
    #[serde(rename = "discordActivityType")]
    pub activity_type: String,
    #[serde(rename = "discordActivityName")]
    pub activity_name: String,
    #[serde(rename = "discordStateTemplate")]
    pub state_template: String,
    #[serde(rename = "discordDetailsTemplate")]
    pub details_template: String,
    #[serde(rename = "discordButton1Enabled")]
    pub button1_enabled: bool,
    #[serde(rename = "discordButton1Label")]
    pub button1_label: String,
    #[serde(rename = "discordButton1Url")]
    pub button1_url: String,
    #[serde(rename = "discordButton2Enabled")]
    pub button2_enabled: bool,
    #[serde(rename = "discordButton2Label")]
    pub button2_label: String,
    #[serde(rename = "discordButton2Url")]
    pub button2_url: String,
}

impl Default for DiscordSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            advanced_mode: false,
            activity_type: DiscordDefaults::ACTIVITY_TYPE.into(),
            activity_name: DiscordDefaults::ACTIVITY_NAME.into(),
            state_template: DiscordDefaults::STATE_TEMPLATE.into(),
            details_template: DiscordDefaults::DETAILS_TEMPLATE.into(),
            button1_enabled: true,
            button1_label: DiscordDefaults::BUTTON1_LABEL.into(),
            button1_url: DiscordDefaults::BUTTON1_URL_TEMPLATE.into(),
            button2_enabled: true,
            button2_label: DiscordDefaults::BUTTON2_LABEL.into(),
            button2_url: DiscordDefaults::BUTTON2_URL.into(),
        }
    }
}

impl DiscordSettings {
    /// `discord_defaults.json` next to `IncrementalSyncService::default_defaults_path()`.
    pub fn default_path() -> Option<PathBuf> {
        IncrementalSyncService::default_defaults_path()
            .map(|path| path.with_file_name("discord_defaults.json"))
    }

    /// A missing or unreadable file reads as the defaults, like unset keys.
    pub fn load(path: &Path) -> Self {
        crate::settings::settings_store::load_keeping_valid_keys(path)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec(self)?)
    }
}
