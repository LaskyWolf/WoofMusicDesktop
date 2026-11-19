//! The queue part of `Trop/Audio/NowPlaying.swift` and `PlaybackQueue.swift`:
//! the songs, the current one, Play Next, shuffle and repeat. Loading and
//! showing the song stays in the app.

use std::sync::Arc;

use crate::networking::models::yt_item::SongItem;

#[derive(Debug, Default, Clone)]
pub struct Queue {
    /// Behind an `Arc` so the queue panel can hold it without copying.
    pub songs: Arc<Vec<SongItem>>,
    pub index: usize,
    pub is_shuffle_on: bool,
    pub is_repeat_on: bool,
    /// The last song added with Play Next that has not played yet.
    play_next_end: Option<usize>,
    /// The order before shuffle, so turning it off restores it.
    original: Option<Vec<SongItem>>,
}

impl Queue {
    pub fn song(&self) -> Option<&SongItem> {
        self.songs.get(self.index)
    }

    pub fn has_next(&self) -> bool {
        self.index + 1 < self.songs.len()
    }

    pub fn has_previous(&self) -> bool {
        self.index > 0
    }

    pub fn next_song(&self) -> Option<&SongItem> {
        self.songs.get(self.index + 1)
    }

    /// Swift `PlaybackQueue.play` + `setQueue`: the songs without the
    /// unavailable ones; the start stays on its song, or moves to the next
    /// available one. Turns shuffle off.
    pub fn set(&mut self, songs: &[SongItem], start: usize) {
        let available: Vec<SongItem> = songs.iter().filter(|s| s.is_available()).cloned().collect();
        let before = songs[..start.min(songs.len())]
            .iter()
            .filter(|s| s.is_available())
            .count();
        self.index = before.min(available.len().saturating_sub(1));
        self.songs = Arc::new(available);
        self.play_next_end = None;
        self.original = None;
        self.is_shuffle_on = false;
    }

    /// Adds `song` after the current one and the songs already added with Play Next.
    pub fn enqueue_next(&mut self, song: SongItem) {
        if !song.is_available() {
            return;
        }
        let after = self.play_next_end.unwrap_or(self.index).max(self.index) + 1;
        let at = after.min(self.songs.len());
        Arc::make_mut(&mut self.songs).insert(at, song);
        self.play_next_end = Some(at);
    }

    /// Adds songs at the end, of the unshuffled order too, so turning
    /// shuffle off keeps them.
    pub fn append(&mut self, songs: &[SongItem]) {
        let songs: Vec<SongItem> = songs.iter().filter(|s| s.is_available()).cloned().collect();
        if let Some(original) = &mut self.original {
            original.extend(songs.iter().cloned());
        }
        Arc::make_mut(&mut self.songs).extend(songs);
    }

    fn pending_play_next(&self) -> Vec<SongItem> {
        match self.play_next_end {
            Some(end) if end > self.index && end < self.songs.len() => {
                self.songs[self.index + 1..=end].to_vec()
            }
            _ => Vec::new(),
        }
    }

    fn reinsert_play_next(&mut self, pending: Vec<SongItem>) {
        let at = (self.index + 1).min(self.songs.len());
        let count = pending.len();
        Arc::make_mut(&mut self.songs).splice(at..at, pending);
        self.play_next_end = (count > 0).then(|| self.index + count);
    }

    /// A radio replaces the queue but keeps the songs added with Play Next.
    pub fn replace_with_radio(&mut self, songs: &[SongItem], current: usize) {
        let pending = self.pending_play_next();
        self.set(songs, current);
        self.reinsert_play_next(pending);
    }

    /// The user picked a queue entry. Jumping back, or past the Play Next
    /// songs, ends the Play Next block.
    pub fn move_to(&mut self, index: usize) {
        if index < self.index || index >= self.play_next_end.unwrap_or(index) {
            self.play_next_end = None;
        }
        self.index = index;
    }

    /// Removes an upcoming or past song; the current one stays.
    pub fn remove(&mut self, index: usize) {
        if index >= self.songs.len() || index == self.index {
            return;
        }
        Arc::make_mut(&mut self.songs).remove(index);
        if index < self.index {
            self.index -= 1;
        }
        if let Some(end) = self.play_next_end
            && index <= end
        {
            self.play_next_end = end.checked_sub(1).filter(|end| *end > self.index);
        }
    }

    /// Moves the song at `from` to `to` (both positions in the queue before the move).
    pub fn move_song(&mut self, from: usize, to: usize) {
        let len = self.songs.len();
        if from >= len || to >= len || from == to {
            return;
        }
        self.play_next_end = None;
        let songs = Arc::make_mut(&mut self.songs);
        let song = songs.remove(from);
        songs.insert(to, song);
        self.index = if from == self.index {
            to
        } else if from < self.index && to >= self.index {
            self.index - 1
        } else if from > self.index && to <= self.index {
            self.index + 1
        } else {
            self.index
        };
    }

    /// Shuffles the upcoming songs, keeping the current one; calling it again re-shuffles.
    pub fn shuffle(&mut self) {
        self.play_next_end = None;
        self.is_shuffle_on = true;
        if self.songs.len() < 2 {
            return;
        }
        if self.original.is_none() {
            self.original = Some(self.songs.to_vec());
        }
        let start = self.index + 1;
        let songs = Arc::make_mut(&mut self.songs);
        if start < songs.len() {
            fastrand::shuffle(&mut songs[start..]);
        }
    }

    /// Restores the order from before shuffle, still on the current song.
    pub fn unshuffle(&mut self) {
        let pending = self.pending_play_next();
        self.play_next_end = None;
        self.is_shuffle_on = false;
        let Some(original) = self.original.take() else {
            return;
        };
        let Some(current) = self.song().map(|song| song.video_id.clone()) else {
            return;
        };
        self.index = original
            .iter()
            .position(|song| song.video_id == current)
            .unwrap_or(self.index)
            .min(original.len().saturating_sub(1));
        self.songs = Arc::new(original);
        self.reinsert_play_next(pending);
    }

    /// True when `saved` holds this queue.
    pub fn is_saved_as(&self, saved: &SavedQueue) -> bool {
        *self.songs == saved.songs
            && self.index == saved.index
            && self.play_next_end == saved.play_next_end
            && self.is_shuffle_on == saved.is_shuffle_on
    }

    pub fn saved(&self) -> SavedQueue {
        SavedQueue {
            songs: self.songs.to_vec(),
            index: self.index,
            play_next_end: self.play_next_end,
            is_shuffle_on: self.is_shuffle_on,
        }
    }

    /// Swift `restoreQueueIfNeeded`: the index clamped, Play Next kept only
    /// when it is still ahead. `None` for an empty queue. The order from
    /// before shuffle is not saved, so turning shuffle off keeps this order.
    pub fn restored(saved: &SavedQueue) -> Option<Self> {
        let last = saved.songs.len().checked_sub(1)?;
        let index = saved.index.min(last);
        Some(Self {
            songs: Arc::new(saved.songs.clone()),
            index,
            is_shuffle_on: saved.is_shuffle_on,
            is_repeat_on: false,
            play_next_end: saved
                .play_next_end
                .filter(|end| *end > index && *end <= last),
            original: None,
        })
    }
}

/// Swift's `nowPlaying.queue`, `queueIndex`, `playNextEnd` and `shuffle`.
/// Repeat stays in `PlayerDefaults`, which the desktop app always keeps.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SavedQueue {
    pub songs: Vec<SongItem>,
    pub index: usize,
    pub play_next_end: Option<usize>,
    pub is_shuffle_on: bool,
}

impl SavedQueue {
    pub fn default_path() -> Option<std::path::PathBuf> {
        PlayerDefaults::default_path().map(|path| path.with_file_name("queue.json"))
    }

    /// A missing or unreadable file reads as an empty queue.
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Writes the queue, or removes the file for an empty one (Swift
    /// `persistQueueState`).
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if self.songs.is_empty() {
            return match std::fs::remove_file(path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec(self)?)
    }
}

/// What the player remembers between launches. Swift keeps repeat in
/// `UserDefaults` and uses the system volume; the desktop app keeps its own.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PlayerDefaults {
    #[serde(rename = "nowPlaying.repeat")]
    pub repeat: bool,
    /// The volume slider, 0...1.
    pub volume: f64,
    #[serde(rename = "settings.equalizerEnabled")]
    pub equalizer_enabled: bool,
    /// dB per band of `equalizer::FREQUENCIES`.
    #[serde(rename = "settings.equalizerGains")]
    pub equalizer_gains: Vec<f64>,
    /// A preset's id, or `equalizer::CUSTOM`.
    #[serde(rename = "settings.equalizerPresetID")]
    pub equalizer_preset: String,
    /// The speed last picked for a podcast episode.
    #[serde(rename = "playback.podcastSpeed")]
    pub podcast_speed: f64,
    /// Skip the talking and skits of music videos with SponsorBlock.
    #[serde(rename = "settings.skipNonMusic")]
    pub skip_non_music: bool,
}

impl Default for PlayerDefaults {
    fn default() -> Self {
        Self {
            repeat: false,
            volume: 1.0,
            equalizer_enabled: false,
            equalizer_gains: vec![0.0; crate::audio::equalizer::FREQUENCIES.len()],
            equalizer_preset: "flat".into(),
            podcast_speed: 1.0,
            skip_non_music: true,
        }
    }
}

impl PlayerDefaults {
    /// The gains mpv applies: `None` while the equalizer is off.
    pub fn equalizer(&self) -> Option<Vec<f64>> {
        self.equalizer_enabled.then(|| self.equalizer_gains.clone())
    }

    pub fn default_path() -> Option<std::path::PathBuf> {
        crate::audio::crossfade::TransitionSettings::default_path()
            .map(|path| path.with_file_name("player_defaults.json"))
    }

    /// A missing or unreadable file reads as the defaults.
    pub fn load(path: &std::path::Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec(self)?)
    }
}
