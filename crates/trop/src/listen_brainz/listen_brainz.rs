// Trop/ListenBrainz/ListenBrainz.swift.
//
// Swift's `NowPlaying` timer calls `tick()` 4 times a second and reads
// `NowPlaying.shared`. Here the app calls `tick` as often, passing what is
// playing and the time. Swift sends from a detached `Task`; `tick` awaits the
// send instead.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use swiftytube::auth::secure_store::{SecureStore, SecureStoreError};
use swiftytube::networking::http_client::Session;

use super::listen_brainz_defaults::{ListenBrainzDefaults, ListenBrainzSettings};
use super::listen_brainz_service::{
    AdditionalInfo, ListenBrainzError, ListenBrainzListen, ListenBrainzService, ListenType,
    TrackMetadata,
};
use crate::last_fm::last_fm_integration::PlayingSong;

/// Sends the songs you play to ListenBrainz, the open alternative to Last.fm.
/// A song counts once half of it, or 4 minutes, has played: ListenBrainz's
/// own rule. The user token lives in the keyring.
pub struct ListenBrainz {
    pub session: Session,
    /// The `ListenBrainzSettings` file.
    pub defaults_path: PathBuf,
    /// Swift `SettingsStore.isIncognito`: nothing is sent while true.
    pub incognito: bool,
    store: SecureStore,
    token: Option<String>,
    user_name: Option<String>,

    // Playback of the current song.
    video_id: Option<String>,
    started_at: SystemTime,
    listened: f64,
    last_tick: Option<SystemTime>,
    sent_now_playing: bool,
    submitted: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ListenBrainzConnectError {
    #[error(transparent)]
    Service(#[from] ListenBrainzError),
    #[error(transparent)]
    Store(#[from] SecureStoreError),
}

impl ListenBrainz {
    /// `defaults_path`: usually `ListenBrainzSettings::default_path()`.
    pub fn new(session: Session, defaults_path: PathBuf) -> Self {
        let mut listen_brainz = Self {
            session,
            defaults_path,
            incognito: false,
            store: SecureStore::new(ListenBrainzDefaults::KEYCHAIN_SERVICE),
            token: None,
            user_name: None,
            video_id: None,
            started_at: UNIX_EPOCH,
            listened: 0.0,
            last_tick: None,
            sent_now_playing: false,
            submitted: false,
        };
        listen_brainz.reload_for_active_profile();
        listen_brainz
    }

    pub fn user_name(&self) -> Option<&str> {
        self.user_name.as_deref()
    }

    /// Reads the token from the keyring again.
    pub fn reload_for_active_profile(&mut self) {
        self.token = self
            .store
            .load(ListenBrainzDefaults::TOKEN_ACCOUNT)
            .ok()
            .and_then(|data| String::from_utf8(data).ok());
        self.user_name = self
            .token
            .as_ref()
            .and_then(|_| ListenBrainzSettings::load(&self.defaults_path).user_name);
        self.video_id = None;
    }

    /// Checks `raw_token` with ListenBrainz, then saves it.
    pub async fn connect(&mut self, raw_token: &str) -> Result<(), ListenBrainzConnectError> {
        let token = raw_token.trim();
        let name = ListenBrainzService::validate(&self.session, token).await?;
        self.store
            .save(token.as_bytes(), ListenBrainzDefaults::TOKEN_ACCOUNT)?;
        self.save_user_name(Some(name.clone()));
        self.token = Some(token.to_string());
        self.user_name = Some(name);
        Ok(())
    }

    pub fn disconnect(&mut self) {
        if let Err(error) = self.store.delete(ListenBrainzDefaults::TOKEN_ACCOUNT) {
            log::error!("Could not delete the ListenBrainz token: {error}");
        }
        self.save_user_name(None);
        self.token = None;
        self.user_name = None;
    }

    fn save_user_name(&self, user_name: Option<String>) {
        let settings = ListenBrainzSettings { user_name };
        if let Err(error) = settings.save(&self.defaults_path) {
            log::error!("Could not save the ListenBrainz user name: {error}");
        }
    }

    /// Counts the time the song actually plays. `now_playing`: `None` when
    /// nothing is loaded.
    pub async fn tick(&mut self, now_playing: Option<&PlayingSong>, now: SystemTime) {
        let Some(token) = self.token.clone() else {
            return;
        };
        let Some(np) = now_playing else {
            return;
        };
        if self.video_id.as_deref() != Some(np.video_id.as_str()) {
            self.video_id = Some(np.video_id.clone());
            self.started_at = now;
            self.listened = 0.0;
            self.last_tick = None;
            self.sent_now_playing = false;
            self.submitted = false;
        }
        if np.is_playing
            && let Some(last_tick) = self.last_tick
        {
            // Capped so a stalled timer can't count time that didn't play.
            let elapsed = now.duration_since(last_tick).unwrap_or_default();
            self.listened += elapsed.min(Duration::from_secs(1)).as_secs_f64();
        }
        self.last_tick = np.is_playing.then_some(now);

        if !self.sent_now_playing
            && self.listened >= ListenBrainzDefaults::NOW_PLAYING_DELAY_SECONDS
        {
            self.sent_now_playing = true;
            self.send(np, ListenType::PlayingNow, None, &token).await;
        }
        let duration = np.duration;
        if !self.submitted
            && duration >= ListenBrainzDefaults::MIN_SONG_DURATION
            && self.listened >= (duration / 2.0).min(ListenBrainzDefaults::MAX_LISTEN_SECONDS)
        {
            self.submitted = true;
            self.send(np, ListenType::Single, Some(self.started_at), &token)
                .await;
        }
    }

    // ponytail: a listen that fails to send is dropped, not queued; add an
    // offline queue if listens go missing on bad connections.
    async fn send(
        &self,
        np: &PlayingSong,
        listen_type: ListenType,
        listened_at: Option<SystemTime>,
        token: &str,
    ) {
        if self.incognito || np.title.is_empty() || np.artist.is_empty() {
            return;
        }
        let listen = ListenBrainzListen {
            listened_at: listened_at.map(|time| {
                time.duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64
            }),
            track_metadata: TrackMetadata {
                artist_name: np.artist.clone(),
                track_name: np.title.clone(),
                release_name: np.album.clone().filter(|album| !album.is_empty()),
                additional_info: AdditionalInfo {
                    media_player: ListenBrainzDefaults::CLIENT_NAME.into(),
                    submission_client: ListenBrainzDefaults::CLIENT_NAME.into(),
                    music_service: "music.youtube.com".into(),
                    origin_url: format!("https://music.youtube.com/watch?v={}", np.video_id),
                    duration_ms: (np.duration * 1000.0) as i64,
                },
            },
        };
        let kind = listen_type.raw_value();
        match ListenBrainzService::submit(&self.session, &listen, listen_type, token).await {
            // Server not reached: usually offline.
            None => log::warn!("Submitting {kind} got no response"),
            Some(200) => {}
            Some(status) => log::error!("Submitting {kind} failed with status {status}"),
        }
    }
}
