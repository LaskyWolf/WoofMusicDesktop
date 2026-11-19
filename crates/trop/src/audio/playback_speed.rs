//! `Trop/Audio/PlaybackSpeed.swift`: the speed of the playing item. A podcast
//! episode starts at the speed last picked for podcasts; anything else at 1×.
//! Reloading the same item keeps its speed.

pub const LOWEST: f64 = 0.5;
pub const HIGHEST: f64 = 3.0;
pub const STEP: f64 = 0.05;
pub const PRESETS: [f64; 5] = [0.75, 1.0, 1.25, 1.5, 2.0];

#[derive(Clone, Debug, PartialEq)]
pub struct PlaybackSpeed {
    pub rate: f64,
    video_id: Option<String>,
    is_episode: bool,
    /// The speed last picked for an episode.
    // ponytail: one speed for every podcast; Swift keeps one per show, which
    // needs the episode's show id (stored with the episode on iOS).
    pub podcast_rate: f64,
}

impl Default for PlaybackSpeed {
    fn default() -> Self {
        Self {
            rate: 1.0,
            video_id: None,
            is_episode: false,
            podcast_rate: 1.0,
        }
    }
}

impl PlaybackSpeed {
    /// Picks the speed for the item about to load and returns it.
    pub fn select(&mut self, video_id: &str, is_episode: bool) -> f64 {
        if self.video_id.as_deref() == Some(video_id) {
            return self.rate;
        }
        self.video_id = Some(video_id.to_owned());
        self.is_episode = is_episode;
        self.rate = if is_episode && self.podcast_rate.is_finite() {
            self.podcast_rate.clamp(LOWEST, HIGHEST)
        } else {
            1.0
        };
        self.rate
    }

    /// Sets the speed the user chose, snapped to `STEP`; an episode's speed
    /// is remembered for the next episodes. `None` when nothing changed.
    pub fn set(&mut self, value: f64) -> Option<f64> {
        if value.is_nan() {
            return None;
        }
        // n / 20 rather than n × 0.05, so 1.25 and 1.5 come out exact.
        let steps = (1.0 / STEP).round();
        let snapped = ((value * steps).round() / steps).clamp(LOWEST, HIGHEST);
        if snapped == self.rate {
            return None;
        }
        self.rate = snapped;
        if self.is_episode {
            self.podcast_rate = snapped;
        }
        Some(snapped)
    }

    /// "1×", "1.25×".
    pub fn label(rate: f64) -> String {
        let text = format!("{rate:.2}");
        let text = text.trim_end_matches('0').trim_end_matches('.');
        format!("{text}×")
    }
}
