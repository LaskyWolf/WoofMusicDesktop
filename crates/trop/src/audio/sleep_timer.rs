//! `Trop/Audio/SleepTimer.swift`: pauses playback after a countdown or when
//! the current song ends. The app checks `due` on its tick.

use std::time::{Duration, Instant};

/// Shown in turn when the timer ends.
pub const MESSAGES: [&str; 3] = ["Good night", "Sleep well", "Sweet dreams"];

/// The countdowns the desktop menu offers.
pub const PRESETS: [u64; 6] = [5, 15, 30, 45, 60, 90];

#[derive(Clone, Debug, Default)]
pub struct SleepTimer {
    ends_at: Option<Instant>,
    /// Pause when the current song ends instead of after a countdown.
    pub stops_at_song_end: bool,
    /// The preset the running countdown started with, for the menu's check mark.
    pub minutes: Option<u64>,
    /// How many times the timer ended, to pick the next message.
    ended: usize,
}

impl SleepTimer {
    pub fn is_active(&self) -> bool {
        self.ends_at.is_some() || self.stops_at_song_end
    }

    pub fn start(&mut self, minutes: u64, now: Instant) {
        self.cancel();
        self.ends_at = Some(now + Duration::from_secs(minutes * 60));
        self.minutes = Some(minutes);
    }

    pub fn stop_at_song_end(&mut self) {
        self.cancel();
        self.stops_at_song_end = true;
    }

    pub fn cancel(&mut self) {
        self.ends_at = None;
        self.minutes = None;
        self.stops_at_song_end = false;
    }

    /// The countdown reached zero: the timer clears itself, the caller pauses.
    pub fn due(&mut self, now: Instant) -> bool {
        if self.ends_at.is_some_and(|end| now >= end) {
            self.cancel();
            return true;
        }
        false
    }

    /// The message for the timer that just ended; the next one comes next time.
    pub fn finish_message(&mut self) -> &'static str {
        let message = MESSAGES[self.ended % MESSAGES.len()];
        self.ended += 1;
        message
    }

    /// "12 min left", "end of song", for the menu label.
    pub fn summary(&self, now: Instant) -> Option<String> {
        if self.stops_at_song_end {
            return Some("end of song".into());
        }
        let left = self.ends_at?.saturating_duration_since(now).as_secs();
        Some(format!("{} min left", left.div_ceil(60).max(1)))
    }
}
