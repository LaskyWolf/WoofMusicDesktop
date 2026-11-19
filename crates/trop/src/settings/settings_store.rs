//! Trop/Settings/SettingsStore.swift: the settings the desktop app uses, with
//! Swift's defaults and `UserDefaults` keys. Missing keys read as the defaults.
//! The transition settings live in `audio::crossfade::TransitionSettings`.
//!
//! Not ported: theme mode and the left-handed tab bar (the desktop app has
//! one dark look and no tab bar), player style, artwork swipe, Listen
//! Together, downloads, translation, incognito and search history. The
//! equalizer lives in `audio::now_playing::PlayerDefaults`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use swiftytube::streaming::playback_quality::AudioQuality;
use swiftytube::sync::incremental_sync_service::IncrementalSyncService;
use swiftytube::sync::library_sync_models::LibrarySyncOptions;

use crate::audio::lyrics::lyrics_manager::LyricsSettings;
use crate::services::personalization_service::{RadioOptions, RadioStyle};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LyricsAlignment {
    Left,
    #[default]
    Center,
    Right,
}

impl LyricsAlignment {
    pub const ALL: [Self; 3] = [Self::Left, Self::Center, Self::Right];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Center => "Center",
            Self::Right => "Right",
        }
    }
}

/// How Library shows playlists, albums, artists and podcasts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryLayout {
    #[default]
    Grid,
    List,
}

impl LibraryLayout {
    pub const ALL: [Self; 2] = [Self::Grid, Self::List];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Grid => "Grid",
            Self::List => "List",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayerBackgroundStyle {
    #[default]
    Dynamic,
    Solid,
}

impl PlayerBackgroundStyle {
    pub const ALL: [Self; 2] = [Self::Dynamic, Self::Solid];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Dynamic => "Album Art",
            Self::Solid => "Solid",
        }
    }
}

/// `AudioQuality`'s Swift `displayName`s, in menu order.
pub const AUDIO_QUALITIES: [(AudioQuality, &str); 4] = [
    (AudioQuality::Auto, "Auto"),
    (AudioQuality::High, "High"),
    (AudioQuality::Medium, "Medium"),
    (AudioQuality::Low, "Low"),
];

/// Swift `QuickPicksSize`: columns x rows of square cards on Home.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuickPicksSize {
    #[serde(rename = "3x2")]
    Grid3x2,
    #[default]
    #[serde(rename = "3x3")]
    Grid3x3,
}

impl QuickPicksSize {
    pub const ALL: [Self; 2] = [Self::Grid3x2, Self::Grid3x3];

    pub fn label(self) -> &'static str {
        match self {
            Self::Grid3x2 => "3×2",
            Self::Grid3x3 => "3×3",
        }
    }

    pub fn columns(self) -> usize {
        3
    }

    pub fn rows(self) -> usize {
        if self == Self::Grid3x2 { 2 } else { 3 }
    }

    pub fn per_page(self) -> usize {
        self.columns() * self.rows()
    }
}

/// Swift `ContentSettingsView.countries`.
pub const COUNTRIES: [&str; 14] = [
    "US", "GB", "CA", "AU", "FR", "DE", "ES", "IT", "PT", "JP", "KR", "MX", "BR", "IN",
];

/// Swift `defaultTab`: Home, Library, Explore, Search.
pub const DEFAULT_TABS: [&str; 4] = ["Home", "Library", "Explore", "Search"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsStore {
    // Appearance
    /// The name of an accent preset; the app keeps the colours.
    #[serde(rename = "settings.accentName")]
    pub accent_name: String,
    #[serde(rename = "settings.libraryLayout")]
    pub library_layout: LibraryLayout,
    #[serde(rename = "settings.playerBackgroundStyle")]
    pub player_background_style: PlayerBackgroundStyle,
    /// Index into `DEFAULT_TABS`.
    #[serde(rename = "settings.defaultTab")]
    pub default_tab: usize,

    // Lyrics appearance
    #[serde(rename = "settings.lyricsAlignment")]
    pub lyrics_alignment: LyricsAlignment,
    /// Shifts lyric timing by this many seconds (positive = later, negative = earlier).
    #[serde(rename = "settings.lyricsOffsetSeconds")]
    pub lyrics_offset_seconds: f64,
    /// Shows a progress bar during long instrumental gaps in synced lyrics.
    #[serde(rename = "settings.showIntervalIndicator")]
    pub show_interval_indicator: bool,
    #[serde(rename = "settings.romanizeCurrentTrack")]
    pub romanize_current_track: bool,
    /// Karaoke starts so far; every third one brings the wolf.
    #[serde(rename = "karaoke.greetingCount")]
    pub karaoke_greeting_count: u64,

    // Playback
    #[serde(rename = "settings.audioQuality")]
    pub audio_quality: AudioQuality,
    #[serde(rename = "settings.audioNormalization")]
    pub audio_normalization: bool,
    #[serde(rename = "settings.gaplessPlayback")]
    pub gapless_playback: bool,
    /// A song picked while paused stays paused.
    #[serde(rename = "settings.persistentPlayPause")]
    pub persistent_play_pause: bool,
    /// A radio of the last song plays when the queue runs out.
    #[serde(rename = "settings.autoplaySimilar")]
    pub autoplay_similar: bool,
    /// The queue, shuffle and repeat come back at launch.
    #[serde(rename = "settings.persistQueue")]
    pub persist_queue: bool,

    // Radio: the options Start Radio last used.
    #[serde(rename = "settings.lastRadioStyle")]
    pub last_radio_style: RadioStyle,
    #[serde(rename = "settings.lastRadioLimit")]
    pub last_radio_limit: usize,
    #[serde(rename = "settings.lastRadioAllowExplicit")]
    pub last_radio_allow_explicit: bool,

    // Content
    #[serde(rename = "settings.hideExplicit")]
    pub hide_explicit: bool,
    #[serde(rename = "settings.showQuickPicks")]
    pub show_quick_picks: bool,
    #[serde(rename = "settings.quickPicksSize")]
    pub quick_picks_size: QuickPicksSize,
    #[serde(rename = "settings.contentCountry")]
    pub content_country: String,

    // Lyrics providers
    #[serde(rename = "settings.disabledLyricsProviders")]
    pub disabled_lyrics_providers: Vec<String>,
    /// Swift `LyricsSettings.providerOrder`; empty means the default order.
    #[serde(rename = "lyricsProviderOrder")]
    pub lyrics_provider_order: Vec<String>,

    // Privacy
    #[serde(rename = "settings.trackPlayHistory")]
    pub track_play_history: bool,
    /// "2026-9": the month whose Recap card ✕ hid from Home.
    #[serde(rename = "recap.dismissedMonth")]
    pub recap_dismissed_month: String,

    // Listen Together
    #[serde(rename = "settings.listenTogetherServerURL")]
    pub listen_together_server_url: String,
    /// Empty: the account's name.
    #[serde(rename = "settings.listenTogetherUsername")]
    pub listen_together_username: String,

    // Sync
    #[serde(rename = "settings.syncArtists")]
    pub sync_artists: bool,
    #[serde(rename = "settings.syncPlaylists")]
    pub sync_playlists: bool,
    #[serde(rename = "settings.syncAlbums")]
    pub sync_albums: bool,
    #[serde(rename = "settings.syncPodcasts")]
    pub sync_podcasts: bool,
    #[serde(rename = "settings.syncSongs")]
    pub sync_songs: bool,
}

impl Default for SettingsStore {
    fn default() -> Self {
        Self {
            accent_name: "Default".to_owned(),
            library_layout: LibraryLayout::Grid,
            player_background_style: PlayerBackgroundStyle::Dynamic,
            default_tab: 0,
            lyrics_alignment: LyricsAlignment::Center,
            lyrics_offset_seconds: 0.0,
            show_interval_indicator: true,
            romanize_current_track: true,
            karaoke_greeting_count: 0,
            audio_quality: AudioQuality::Auto,
            audio_normalization: false,
            gapless_playback: true,
            persistent_play_pause: false,
            autoplay_similar: true,
            persist_queue: false,
            last_radio_style: RadioStyle::Similar,
            last_radio_limit: 25,
            last_radio_allow_explicit: true,
            hide_explicit: false,
            show_quick_picks: true,
            quick_picks_size: QuickPicksSize::Grid3x3,
            content_country: "US".to_owned(),
            disabled_lyrics_providers: Vec::new(),
            lyrics_provider_order: Vec::new(),
            track_play_history: true,
            recap_dismissed_month: String::new(),
            listen_together_server_url: crate::listen_together::DEFAULT_SERVER_URL.to_owned(),
            listen_together_username: String::new(),
            sync_artists: true,
            sync_playlists: true,
            sync_albums: true,
            sync_podcasts: true,
            sync_songs: true,
        }
    }
}

impl SettingsStore {
    /// `settings_defaults.json` next to `IncrementalSyncService::default_defaults_path()`.
    pub fn default_path() -> Option<PathBuf> {
        IncrementalSyncService::default_defaults_path()
            .map(|path| path.with_file_name("settings_defaults.json"))
    }

    /// A missing or unreadable file reads as the defaults. A key whose value
    /// this version cannot read (wrong type, a newer app's enum case) reads as
    /// its default; the other keys stay.
    pub fn load(path: &Path) -> Self {
        let Some(serde_json::Value::Object(mut map)) = std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        else {
            return Self::default();
        };
        let keys: Vec<String> = map.keys().cloned().collect();
        for key in keys {
            let one = serde_json::Map::from_iter([(key.clone(), map[&key].clone())]);
            if serde_json::from_value::<Self>(one.into()).is_err() {
                map.remove(&key);
            }
        }
        serde_json::from_value(map.into()).unwrap_or_default()
    }

    /// Writes a temporary file and renames it over `path`, so a crash or a
    /// second save never leaves half a file. Saves are serialised.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        static SAVING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = SAVING.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self)?)?;
        std::fs::rename(&tmp, path)
    }

    /// The lyrics providers to ask, in order: the saved order with providers it
    /// does not name appended (Swift `LyricsProviderOrderView.onAppear`).
    pub fn lyrics_settings(&self) -> LyricsSettings {
        LyricsSettings::from_saved(
            self.lyrics_provider_order.clone(),
            self.disabled_lyrics_providers.iter().cloned().collect(),
        )
    }

    /// Swift `RadioOptions.saved`.
    pub fn radio_options(&self) -> RadioOptions {
        RadioOptions {
            style: self.last_radio_style,
            limit: self.last_radio_limit,
            allow_explicit: self.last_radio_allow_explicit,
        }
    }

    /// Swift `RadioOptions.save`.
    pub fn save_radio_options(&mut self, options: RadioOptions) {
        self.last_radio_style = options.style;
        self.last_radio_limit = options.limit;
        self.last_radio_allow_explicit = options.allow_explicit;
    }

    /// Swift `SyncBridge` maps the five sync toggles onto the sync options.
    pub fn sync_options(&self) -> LibrarySyncOptions {
        LibrarySyncOptions {
            sync_artists: self.sync_artists,
            sync_playlists: self.sync_playlists,
            sync_albums: self.sync_albums,
            sync_podcasts: self.sync_podcasts,
            sync_songs: self.sync_songs,
        }
    }

    /// Swift `syncTropLocale`: the region, English text.
    pub fn locale(&self) -> (String, String) {
        let country = if self.content_country.is_empty() {
            "US"
        } else {
            &self.content_country
        };
        (country.to_owned(), "en".to_owned())
    }

    /// "Off", or a signed value like "+1.5s" (Swift `offsetDescription`).
    pub fn offset_description(&self) -> String {
        let value = self.lyrics_offset_seconds;
        if value == 0.0 {
            return "Off".to_owned();
        }
        let digits = format!("{:.2}", value.abs());
        let digits = digits.trim_end_matches('0').trim_end_matches('.');
        format!("{}{digits}s", if value < 0.0 { "-" } else { "+" })
    }

    /// Moves provider `id` one place up (`-1`) or down (`1`) in the full order.
    pub fn move_lyrics_provider(&mut self, id: &str, step: isize) {
        let mut order = self.lyrics_settings().provider_order;
        if let Some(from) = order.iter().position(|p| p == id) {
            let to = from.saturating_add_signed(step).min(order.len() - 1);
            order.swap(from, to);
        }
        self.lyrics_provider_order = order;
    }

    pub fn set_lyrics_provider_enabled(&mut self, id: &str, enabled: bool) {
        self.disabled_lyrics_providers.retain(|p| p != id);
        if !enabled {
            self.disabled_lyrics_providers.push(id.to_owned());
            self.disabled_lyrics_providers.sort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_keys_read_as_swift_defaults() {
        let settings: SettingsStore =
            serde_json::from_str(r#"{"settings.hideExplicit": true}"#).unwrap();
        assert!(settings.hide_explicit);
        assert!(settings.show_quick_picks);
        assert_eq!(settings.content_country, "US");
        assert_eq!(settings.lyrics_alignment, LyricsAlignment::Center);
    }

    #[test]
    fn saves_and_loads_with_swift_keys() {
        let dir = std::env::temp_dir().join(format!("woof-settings-{}", std::process::id()));
        let path = dir.join("settings_defaults.json");
        let settings = SettingsStore {
            audio_quality: AudioQuality::Low,
            lyrics_alignment: LyricsAlignment::Right,
            ..SettingsStore::default()
        };
        settings.save(&path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains(r#""settings.audioQuality":"low""#));
        assert_eq!(SettingsStore::load(&path), settings);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn one_unreadable_key_keeps_the_others() {
        let dir = std::env::temp_dir().join(format!("woof-settings-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings_defaults.json");
        std::fs::write(
            &path,
            r#"{"settings.lyricsAlignment":"justify","settings.hideExplicit":true,"settings.defaultTab":"x","settings.lyricsOffsetSeconds":null}"#,
        )
        .unwrap();
        let loaded = SettingsStore::load(&path);
        assert!(loaded.hide_explicit);
        assert_eq!(loaded.lyrics_alignment, LyricsAlignment::Center);
        assert_eq!(loaded.default_tab, 0);
        std::fs::write(&path, "[1,2").unwrap();
        assert_eq!(SettingsStore::load(&path), SettingsStore::default());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parallel_saves_leave_one_whole_file() {
        let dir = std::env::temp_dir().join(format!("woof-settings-par-{}", std::process::id()));
        let path = dir.join("settings_defaults.json");
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let settings = SettingsStore {
                        accent_name: i.to_string().repeat(4000),
                        ..SettingsStore::default()
                    };
                    for _ in 0..20 {
                        settings.save(&path).unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let loaded: SettingsStore = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(loaded.accent_name.len() >= 4000);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn moving_a_provider_keeps_every_provider() {
        let mut settings = SettingsStore::default();
        settings.move_lyrics_provider("lrclib", -1);
        assert_eq!(
            settings.lyrics_settings().provider_order,
            ["lrclib", "musixmatch", "netease", "kugou", "genius"]
        );
        settings.move_lyrics_provider("lrclib", -1);
        assert_eq!(settings.lyrics_provider_order[0], "lrclib");
        settings.move_lyrics_provider("genius", 1);
        assert_eq!(settings.lyrics_provider_order[4], "genius");
    }

    #[test]
    fn quick_picks_size_uses_swift_values() {
        let settings: SettingsStore =
            serde_json::from_str(r#"{"settings.quickPicksSize": "3x2"}"#).unwrap();
        assert_eq!(settings.quick_picks_size.per_page(), 6);
        assert_eq!(SettingsStore::default().quick_picks_size.per_page(), 9);
        let raw = serde_json::to_string(&settings).unwrap();
        assert!(raw.contains(r#""settings.quickPicksSize":"3x2""#));
    }

    #[test]
    fn offset_reads_like_swift() {
        let mut settings = SettingsStore::default();
        assert_eq!(settings.offset_description(), "Off");
        settings.lyrics_offset_seconds = 1.5;
        assert_eq!(settings.offset_description(), "+1.5s");
        settings.lyrics_offset_seconds = -0.25;
        assert_eq!(settings.offset_description(), "-0.25s");
    }
}
