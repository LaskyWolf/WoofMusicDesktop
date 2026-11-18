//! Trop/Playback/PlaybackStateService.swift: counts how long each song plays.
//!
//! Every 5 s of listening is banked into the song's total play time (My Top
//! 100 sorts by it), so skipped songs still count. Past 30 s the play also
//! becomes a history event and a monthly play count.
//!
//! Swift is an actor with its own 5 s timer. Here the caller passes the time
//! in and saves what each call returns with [`save`], off the UI thread.
// ponytail: no `YouTubeHistoryReport`: plays reach this database, not the
// account's YouTube Music history. Add it with the YouTube history pings.

use std::time::{Duration, Instant, SystemTime};

use swiftytube::networking::artwork_urls::ArtworkURLs;

use crate::database::database_service::{ConflictResolution, DatabaseService};
use crate::database::entities::event::Event;
use crate::database::entities::song_entity::SongEntity;
use crate::networking::models::yt_item::SongItem;

/// A play counts as listened, for history, from this long.
pub const HISTORY_THRESHOLD: Duration = Duration::from_secs(30);
/// Play time is banked in chunks of at least this many milliseconds.
const BANK_STEP_MS: i64 = 5_000;
/// The remainder banked at stop, down to this many milliseconds.
const MIN_REMAINDER_MS: i64 = 1_000;

/// A write the caller saves with [`save`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlayRecord {
    /// Adds to the song's total play time.
    PlayTime { video_id: String, ms: i64 },
    /// A listened play: a history event and the month's play count.
    Playback { video_id: String, ms: i64 },
}

#[derive(Debug, Default)]
pub struct PlaybackStateService {
    current: Option<String>,
    /// Play time before the last resume; paused time never counts.
    played_before: Duration,
    /// When playback last resumed; `None` while paused or still loading.
    resumed_at: Option<Instant>,
    banked_ms: i64,
    recorded: bool,
}

impl PlaybackStateService {
    fn elapsed(&self, now: Instant) -> Duration {
        self.played_before + self.resumed_at.map_or(Duration::ZERO, |at| now - at)
    }

    /// Starts paused; `set_playing(true)` begins counting once audio runs.
    pub fn start_tracking(&mut self, video_id: &str) {
        *self = Self {
            current: Some(video_id.to_owned()),
            ..Self::default()
        };
    }

    pub fn set_playing(&mut self, playing: bool, now: Instant) {
        if self.current.is_none() {
            return;
        }
        match (playing, self.resumed_at) {
            (true, None) => self.resumed_at = Some(now),
            (false, Some(at)) => {
                self.played_before += now - at;
                self.resumed_at = None;
            }
            _ => {}
        }
    }

    /// Swift `periodicCheck`: banks each full 5 s chunk and records history
    /// once past the threshold. Call it as often as wanted.
    pub fn tick(&mut self, now: Instant) -> Vec<PlayRecord> {
        let Some(video_id) = self.current.clone() else {
            return Vec::new();
        };
        let elapsed = self.elapsed(now);
        let elapsed_ms = elapsed.as_millis() as i64;
        let mut records = Vec::new();
        if elapsed_ms - self.banked_ms >= BANK_STEP_MS {
            records.push(PlayRecord::PlayTime {
                video_id: video_id.clone(),
                ms: elapsed_ms - self.banked_ms,
            });
            self.banked_ms = elapsed_ms;
        }
        if elapsed >= HISTORY_THRESHOLD && !self.recorded {
            self.recorded = true;
            records.push(PlayRecord::Playback {
                video_id,
                ms: elapsed_ms,
            });
        }
        records
    }

    /// Banks the rest of the song (down to 1 s, so short plays count) and its
    /// history if it has not been recorded yet.
    pub fn stop_tracking(&mut self, now: Instant) -> Vec<PlayRecord> {
        let Some(video_id) = self.current.take() else {
            return Vec::new();
        };
        let elapsed = self.elapsed(now);
        let elapsed_ms = elapsed.as_millis() as i64;
        let mut records = Vec::new();
        if elapsed_ms - self.banked_ms >= MIN_REMAINDER_MS {
            records.push(PlayRecord::PlayTime {
                video_id: video_id.clone(),
                ms: elapsed_ms - self.banked_ms,
            });
        }
        if elapsed >= HISTORY_THRESHOLD && !self.recorded {
            records.push(PlayRecord::Playback {
                video_id,
                ms: elapsed_ms,
            });
        }
        *self = Self::default();
        records
    }
}

/// Writes `records`. `song` names the row when the song is not in the
/// database yet (Swift `ensureSongEntity` reads it from the queue).
pub fn save(
    db: &DatabaseService,
    song: Option<&SongItem>,
    records: &[PlayRecord],
) -> rusqlite::Result<()> {
    for record in records {
        let (PlayRecord::PlayTime { video_id, .. } | PlayRecord::Playback { video_id, .. }) =
            record;
        if video_id.is_empty() {
            continue;
        }
        let song = song.filter(|song| song.video_id == *video_id);
        ensure_song(db, video_id, song)?;
        match record {
            PlayRecord::PlayTime { ms, .. } => db.increment_total_play_time(video_id, *ms)?,
            PlayRecord::Playback { ms, .. } => {
                let now = SystemTime::now();
                db.insert(&Event {
                    id: None,
                    song_id: video_id.clone(),
                    timestamp: now,
                    play_time: *ms,
                })?;
                let (year, month) = year_month(now);
                db.increment_play_count(video_id, year, month)?;
            }
        }
    }
    Ok(())
}

/// Inserts the song's row when missing, so banking never hits a missing row.
fn ensure_song(
    db: &DatabaseService,
    video_id: &str,
    song: Option<&SongItem>,
) -> rusqlite::Result<()> {
    let now = SystemTime::now();
    let entity = SongEntity {
        id: video_id.to_owned(),
        title: song.map_or_else(|| video_id.to_owned(), |song| song.title.clone()),
        artist_name: song.and_then(|song| song.artists.first().map(|artist| artist.name.clone())),
        album_name: song.and_then(|song| song.album.clone()),
        duration: song.map_or(0, |song| song.duration),
        thumbnail_url: Some(
            song.and_then(|song| song.thumbnail_url.clone())
                .unwrap_or_else(|| ArtworkURLs::fallback(video_id)),
        ),
        liked: false,
        total_play_time: 0,
        in_library: None,
        library_add_token: String::new(),
        library_remove_token: String::new(),
        is_episode: false,
        is_uploaded: false,
        is_video: false,
        create_date: now,
        modify_date: now,
        is_unavailable: false,
    };
    db.insert_on_conflict(&entity, ConflictResolution::Ignore)
        .map(|_| ())
}

/// The UTC year and month of `date`.
// ponytail: UTC; Swift uses the local calendar, so a play near midnight on a
// month's last day can count for the next month.
fn year_month(date: SystemTime) -> (i64, i64) {
    let text = crate::database::date_text(date);
    let year = text[..4].parse().unwrap_or(1970);
    let month = text[5..7].parse().unwrap_or(1);
    (year, month)
}
