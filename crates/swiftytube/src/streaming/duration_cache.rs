use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex, MutexGuard};

#[derive(Default)]
struct State {
    cache: HashMap<String, i64>,
    pending: HashSet<String>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(Mutex::default);

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Thread-safe video durations with in-flight marking.
// ponytail: Swift also posts `Notification.Name.durationDidUpdate` from `set`;
// add a channel here when a screen needs to refresh rows on it (R4).
pub struct DurationCache;

impl DurationCache {
    pub fn is_pending(video_id: &str) -> bool {
        state().pending.contains(video_id)
    }

    pub fn get(video_id: &str) -> Option<i64> {
        state().cache.get(video_id).copied()
    }

    pub fn set(video_id: &str, duration: i64) {
        let mut state = state();
        state.cache.insert(video_id.to_string(), duration);
        state.pending.remove(video_id);
    }

    pub fn mark_pending(video_id: &str) {
        state().pending.insert(video_id.to_string());
    }

    pub fn clear_pending(video_id: &str) {
        state().pending.remove(video_id);
    }
}
