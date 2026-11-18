use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::playback_result::PlaybackResult;
use crate::core::swiftytube_log::SwiftyTubeLog;

/// Stream URLs with TTL expiry.
#[derive(Default)]
pub struct StreamCache {
    cache: Mutex<HashMap<String, (PlaybackResult, Instant)>>,
}

impl StreamCache {
    /// TTL when the response reports no usable expiry.
    const DEFAULT_TTL_SECONDS: i64 = 5 * 60;

    pub fn shared() -> &'static StreamCache {
        static SHARED: LazyLock<StreamCache> = LazyLock::new(StreamCache::default);
        &SHARED
    }

    fn cache(&self) -> MutexGuard<'_, HashMap<String, (PlaybackResult, Instant)>> {
        self.cache.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get(&self, video_id: &str) -> Option<PlaybackResult> {
        let mut cache = self.cache();
        match cache.get(video_id) {
            Some((result, expires_at)) if *expires_at > Instant::now() => Some(result.clone()),
            _ => {
                cache.remove(video_id);
                None
            }
        }
    }

    pub fn get_expired(&self, video_id: &str) -> Option<PlaybackResult> {
        self.cache().get(video_id).map(|(result, _)| result.clone())
    }

    pub fn set(&self, video_id: &str, result: PlaybackResult) {
        let reported = result.expires_in_seconds.max(0);
        let ttl = if reported > 60 {
            (reported - 60).max(60)
        } else {
            Self::DEFAULT_TTL_SECONDS
        };
        let now = Instant::now();
        // An absurd `expiresInSeconds` would overflow `Instant`; use the default TTL instead.
        let expires_at = now
            .checked_add(Duration::from_secs(ttl as u64))
            .unwrap_or(now + Duration::from_secs(Self::DEFAULT_TTL_SECONDS as u64));
        self.cache()
            .insert(video_id.to_string(), (result, expires_at));
        SwiftyTubeLog::debug(&format!("Cached videoId={video_id} expires in {ttl}s"));
    }

    pub fn remove(&self, video_id: &str) {
        self.cache().remove(video_id);
    }

    pub fn clear(&self) {
        self.cache().clear();
    }
}
