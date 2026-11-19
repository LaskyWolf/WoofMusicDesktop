//! `Trop/Playback/EpisodePlaybackStore.swift`: where each podcast episode
//! (or long video) stopped, so it resumes there.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Swift keeps the 200 most recently updated.
const LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodePosition {
    pub position: f64,
    pub duration: f64,
    /// Seconds since 1970.
    pub updated_at: f64,
}

impl EpisodePosition {
    pub fn remaining(&self) -> f64 {
        (self.duration - self.position).max(0.0)
    }

    /// Finished when under 30 s remain or over 95 % played.
    pub fn is_finished(&self) -> bool {
        self.duration > 0.0 && (self.remaining() < 30.0 || self.position / self.duration > 0.95)
    }

    /// Where a new play starts: past 10 s and not finished (Swift `playEpisode`).
    pub fn resume_at(&self) -> Option<f64> {
        (self.position > 10.0 && !self.is_finished()).then_some(self.position)
    }
}

/// Swift's positions key, as a JSON file next to the other defaults.
#[derive(Debug, Default)]
pub struct EpisodePlaybackStore {
    positions: HashMap<String, EpisodePosition>,
}

pub fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

impl EpisodePlaybackStore {
    pub fn default_path() -> Option<PathBuf> {
        crate::audio::crossfade::TransitionSettings::default_path()
            .map(|path| path.with_file_name("episode_positions.json"))
    }

    /// A missing or unreadable file reads as empty.
    pub fn load(path: &Path) -> Self {
        let positions = std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { positions }
    }

    /// The JSON to write after a change, capped to the newest `LIMIT`.
    pub fn encoded(&mut self) -> Vec<u8> {
        if self.positions.len() > LIMIT {
            let mut newest: Vec<(String, EpisodePosition)> = self.positions.drain().collect();
            newest.sort_by(|a, b| b.1.updated_at.total_cmp(&a.1.updated_at));
            newest.truncate(LIMIT);
            self.positions = newest.into_iter().collect();
        }
        serde_json::to_vec(&self.positions).unwrap_or_default()
    }

    pub fn position(&self, video_id: &str) -> Option<EpisodePosition> {
        self.positions.get(video_id).copied()
    }

    /// Records where `video_id` is. True when it changed enough to write:
    /// writes are at least 5 s apart unless the episode just finished.
    pub fn save_position(
        &mut self,
        video_id: &str,
        position: f64,
        duration: f64,
        now: f64,
    ) -> bool {
        if duration <= 0.0 || position < 0.0 {
            return false;
        }
        let updated = EpisodePosition {
            position,
            duration,
            updated_at: now,
        };
        if let Some(existing) = self.positions.get(video_id)
            && (existing.position - position).abs() < 5.0
            && now - existing.updated_at < 5.0
            && existing.is_finished() == updated.is_finished()
        {
            return false;
        }
        self.positions.insert(video_id.to_owned(), updated);
        true
    }

    /// Swift `markPlayed`: a finished episode starts from the top next time.
    pub fn clear(&mut self, video_id: &str) -> bool {
        self.positions.remove(video_id).is_some()
    }
}

/// Swift `EpisodePlaybackStore.syncKnownEpisodes`: remembers the episodes of
/// a podcast page and returns the ones never seen before.
pub fn sync_known_episodes(
    db: &crate::database::database_service::DatabaseService,
    episodes: &[crate::networking::models::yt_item::EpisodeItem],
    podcast_id: &str,
    podcast_name: &str,
) -> rusqlite::Result<Vec<crate::networking::models::yt_item::EpisodeItem>> {
    use crate::database::entities::episode_entity::EpisodeEntity;
    let mut fresh = Vec::new();
    for episode in episodes {
        if db.fetch_one::<EpisodeEntity>(&episode.video_id)?.is_some() {
            continue;
        }
        db.insert_or_replace(&EpisodeEntity {
            id: episode.video_id.clone(),
            title: episode.title.clone(),
            duration: episode.duration,
            thumbnail_url: episode.thumbnail_url.clone(),
            podcast_id: Some(podcast_id.to_owned()),
            podcast_name: Some(podcast_name.to_owned()),
            is_played: false,
        })?;
        fresh.push(episode.clone());
    }
    Ok(fresh)
}
