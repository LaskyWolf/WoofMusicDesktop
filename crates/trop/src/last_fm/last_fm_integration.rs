// Trop/LastFM/LastFMIntegration.swift: when a song counts as a scrobble
// (mirrors Metrolist's ScrobbleManager.kt).
//
// Swift observes `NowPlaying`, the play state and a 1-second timer itself. Here
// the app calls `sync_state` every second and on each track or play/pause
// change, passing what is playing and the time. The scrobble timer is a
// deadline checked by `sync_state`, so a scrobble is sent up to one call late.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use swiftytube::networking::http_client::Session;

use super::last_fm_defaults::LastFMSettings;
use super::last_fm_service::LastFMService;
use super::last_fm_token_store::LastFMTokenStore;

/// What the scrobblers read from Swift's `NowPlaying.shared`. The app fills
/// it from its player; ListenBrainz uses it too.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayingSong {
    pub video_id: String,
    pub title: String,
    /// Artist names joined with ", ".
    pub artist: String,
    pub album: Option<String>,
    /// Seconds; 0 while unknown.
    pub duration: f64,
    pub is_playing: bool,
}

pub struct LastFMIntegration {
    pub service: LastFMService,
    /// The `LastFMSettings` file.
    pub defaults_path: PathBuf,
    /// Swift `SettingsStore.isIncognito`: nothing is sent while true.
    pub incognito: bool,
    scrobble_remaining_millis: i64,
    /// 0 while the scrobble timer is stopped.
    scrobble_timer_started_at_ms: i64,
    song_started_at_sec: i64,
    song_started: bool,
    /// Swift `currentVideoId` + `currentSong`.
    current_song: Option<PlayingSong>,
    was_playing: bool,
    last_scrobbled_video_id: Option<String>,
}

impl LastFMIntegration {
    /// `defaults_path`: usually `LastFMSettings::default_path()`.
    pub fn new(session: Session, defaults_path: PathBuf) -> Self {
        Self {
            service: LastFMService::new(session, LastFMTokenStore::new(defaults_path.clone())),
            defaults_path,
            incognito: false,
            scrobble_remaining_millis: 0,
            scrobble_timer_started_at_ms: 0,
            song_started_at_sec: 0,
            song_started: false,
            current_song: None,
            was_playing: false,
            last_scrobbled_video_id: None,
        }
    }

    // MARK: - Public

    pub fn settings(&self) -> LastFMSettings {
        LastFMSettings::load(&self.defaults_path)
    }

    pub fn is_enabled(&self) -> bool {
        self.settings().scrobbling_enabled
    }

    pub fn set_enabled(&self, enabled: bool) -> std::io::Result<()> {
        let mut settings = self.settings();
        settings.scrobbling_enabled = enabled;
        settings.save(&self.defaults_path)
    }

    pub fn is_logged_in(&self) -> bool {
        self.service.token_store.retrieve_session_key().is_some()
    }

    // MARK: - Sync

    /// `now_playing`: `None` when nothing is loaded.
    pub async fn sync_state(&mut self, now_playing: Option<&PlayingSong>, now: SystemTime) {
        let settings = self.settings();
        if !settings.scrobbling_enabled || !self.is_logged_in() {
            return;
        }
        let Some(song) = now_playing else {
            self.on_song_stop();
            self.current_song = None;
            return;
        };

        let current_id = self.current_song.as_ref().map(|s| s.video_id.as_str());
        if current_id != Some(song.video_id.as_str()) {
            self.on_song_stop();
            self.current_song = Some(song.clone());
            self.was_playing = song.is_playing;
            self.on_song_start(&settings, song.is_playing, now).await;
        } else if song.is_playing != self.was_playing {
            self.was_playing = song.is_playing;
            self.handle_play_state(&settings, song.is_playing, now)
                .await;
        }

        // Swift's scrobble `Task` wakes once the remaining time has played.
        if self.scrobble_timer_started_at_ms != 0
            && millis(now) - self.scrobble_timer_started_at_ms >= self.scrobble_remaining_millis
        {
            self.scrobble_timer_started_at_ms = 0;
            self.scrobble_remaining_millis = 0;
            self.scrobble_song(&settings).await;
        }
    }

    async fn handle_play_state(
        &mut self,
        settings: &LastFMSettings,
        is_playing: bool,
        now: SystemTime,
    ) {
        if !is_playing {
            self.on_song_pause(now);
        } else if !self.song_started {
            // Started without `song_started` (a short song, or loaded paused).
            self.on_song_start(settings, true, now).await;
        } else {
            self.on_song_resume(now);
        }
    }

    // MARK: - Scrobble lifecycle

    // Settings are typed into free-form text fields; clamp to the ranges the
    // settings screen states (10–60s, 30–95%, 30–360s).
    fn min_song_duration(settings: &LastFMSettings) -> i64 {
        settings.scrobble_min_duration.clamp(10, 60)
    }

    fn scrobble_delay_percent(settings: &LastFMSettings) -> f32 {
        settings.scrobble_delay_percent.clamp(0.30, 0.95)
    }

    fn scrobble_delay_seconds(settings: &LastFMSettings) -> i64 {
        settings.scrobble_delay_seconds.clamp(30, 360)
    }

    async fn on_song_start(
        &mut self,
        settings: &LastFMSettings,
        is_playing: bool,
        now: SystemTime,
    ) {
        let Some(song) = self.current_song.clone() else {
            return;
        };
        let duration_sec = song.duration as i64;
        if duration_sec <= Self::min_song_duration(settings) {
            if settings.use_now_playing {
                self.update_now_playing(&song).await;
            }
            return;
        }

        self.song_started_at_sec = millis(now) / 1000;
        self.song_started = true;
        // A new play of the same song may scrobble again.
        self.last_scrobbled_video_id = None;

        let threshold_ms =
            (duration_sec as f32 * 1000.0 * Self::scrobble_delay_percent(settings)) as i64;
        self.scrobble_remaining_millis =
            threshold_ms.min(Self::scrobble_delay_seconds(settings) * 1000);

        if self.scrobble_remaining_millis <= 0 {
            self.scrobble_song(settings).await;
            return;
        }
        self.scrobble_timer_started_at_ms = millis(now);

        if settings.use_now_playing {
            self.update_now_playing(&song).await;
        }
        // A track loaded while paused must not count until it plays.
        if !is_playing {
            self.on_song_pause(now);
        }
    }

    fn on_song_pause(&mut self, now: SystemTime) {
        if self.scrobble_timer_started_at_ms != 0 {
            let elapsed = millis(now) - self.scrobble_timer_started_at_ms;
            self.scrobble_remaining_millis = (self.scrobble_remaining_millis - elapsed).max(0);
            self.scrobble_timer_started_at_ms = 0;
        }
    }

    fn on_song_resume(&mut self, now: SystemTime) {
        if self.scrobble_remaining_millis <= 0 {
            return;
        }
        self.scrobble_timer_started_at_ms = millis(now);
    }

    fn on_song_stop(&mut self) {
        self.scrobble_remaining_millis = 0;
        self.scrobble_timer_started_at_ms = 0;
        self.song_started_at_sec = 0;
        self.song_started = false;
    }

    async fn scrobble_song(&mut self, settings: &LastFMSettings) {
        let Some(song) = self.current_song.clone() else {
            return;
        };
        if !settings.scrobbling_enabled
            || self.incognito
            || song.artist.is_empty()
            || song.title.is_empty()
        {
            return;
        }
        if self.last_scrobbled_video_id.as_deref() == Some(song.video_id.as_str()) {
            return;
        }
        self.last_scrobbled_video_id = Some(song.video_id.clone());
        let duration = song.duration as i64;
        let sent = self
            .service
            .scrobble(
                &song.artist,
                &song.title,
                self.song_started_at_sec,
                song.album.as_deref(),
                None,
                (duration > 0).then_some(duration),
            )
            .await;
        if sent.is_err() {
            self.last_scrobbled_video_id = None;
        }
    }

    async fn update_now_playing(&self, song: &PlayingSong) {
        if self.incognito || song.artist.is_empty() || song.title.is_empty() {
            return;
        }
        let duration = song.duration as i64;
        let _ = self
            .service
            .update_now_playing(
                &song.artist,
                &song.title,
                song.album.as_deref(),
                None,
                (duration > 0).then_some(duration),
            )
            .await;
    }

    /// Called when the user likes or unlikes a song.
    pub async fn handle_love(&self, artist: &str, track: &str, love: bool) {
        let settings = self.settings();
        if !settings.use_send_likes || !settings.scrobbling_enabled || !self.is_logged_in() {
            return;
        }
        if artist.is_empty() || track.is_empty() {
            return;
        }
        let _ = self.service.set_love_status(artist, track, love).await;
    }

    pub fn logout(&mut self) {
        self.service.token_store.clear();
        self.on_song_stop();
        self.current_song = None;
        self.last_scrobbled_video_id = None;
    }
}

fn millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
