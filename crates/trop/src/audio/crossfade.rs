//! `Trop/Audio/Crossfade.swift`, decision logic only: transitions between
//! songs on two mpv decks. Near the end of a song the next one starts on the
//! spare deck and the two are mixed.
//!
//! - Smart Crossfade: a timed equal-power fade placed by `mix_planner::smart`.
//! - AutoMix: a beat-matched mix placed by `mix_planner::auto_mix`. The
//!   incoming song plays at the outgoing tempo, its beats are kept on the
//!   outgoing beats by small speed corrections, and the bass of the two songs
//!   is swapped on the bar halfway through. Each mix lets the outgoing song
//!   go in a `Style`, set live on named filters with `af-command`.
//!
//! Flow, driven by `Crossfade::tick` every 0.25 s:
//! idle → planning (analyse both songs) → planned → prepared (next song
//! loaded, paused, silent) → fading (the decks swap halfway).
//!
//! The player owns the decks, timers and tasks. This module returns what to
//! do (`Action`), the volumes and filter values per step (`fade_gains`,
//! `BeatMix::step`), and the player sends them to mpv.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use swiftytube::sync::incremental_sync_service::IncrementalSyncService;

use super::mix_planner::{self, Cues, Mix, Style};
use super::track_analyzer::{BEATS_PER_BAR, SAMPLE_RATE, TrackAnalysis};

/// Seconds a plan needs before its start to load the next song.
pub const MIN_LEAD: f64 = 3.0;
/// Seconds of audio analysed at the end of the outgoing song and the start
/// of the incoming one.
pub const TAIL_SECONDS: f64 = 60.0;
pub const HEAD_SECONDS: f64 = 45.0;
/// How far into a song analysis starts. Late enough that a song skipped at
/// once costs no download.
pub const PLAN_DELAY: f64 = 10.0;
/// How long before the transition the next song is loaded so it is buffered in time.
pub const PREPARE_LEAD: f64 = 12.0;
/// Time to ease a changed tempo back to normal after the transition.
pub const SPEED_EASE_SECONDS: f64 = 8.0;
/// AutoMix beat lock: how fast a timing error is corrected (per second), the
/// largest speed correction, and the bass cut on the quieter deck.
pub const LOCK_GAIN: f64 = 0.5;
pub const MAX_LOCK_CORRECTION: f64 = 0.02;
pub const BASS_CUT: f64 = -24.0;
/// Filter sweep range: the incoming low-pass opens over the first half, the
/// outgoing high-pass rises over the second.
pub const LOW_PASS_SWEEP: (f64, f64) = (300.0, 16_000.0);
pub const HIGH_PASS_SWEEP: (f64, f64) = (20.0, 1_500.0);

/// Labels of the filters AutoMix changes live with `af-command`
/// (Swift `PlayerController.bassFilterLabel` and siblings).
// ponytail: owned by PlayerController in Swift; move there with the R6 filter chain.
pub const BASS_FILTER_LABEL: &str = "automixbass";
pub const HIGH_PASS_LABEL: &str = "automixhp";
pub const LOW_PASS_LABEL: &str = "automixlp";
pub const EFFECTS_LABEL: &str = "automixfx";

/// Swift `SettingsStore` transition settings, with its defaults and its
/// `UserDefaults` keys. Missing keys read as the defaults.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TransitionSettings {
    #[serde(rename = "settings.autoMixEnabled")]
    pub auto_mix_enabled: bool,
    #[serde(rename = "settings.crossfadeEnabled")]
    pub crossfade_enabled: bool,
    /// Smart Crossfade length, 1...12 s.
    #[serde(rename = "settings.crossfadeSeconds")]
    pub crossfade_seconds: f64,
    /// Strength of the AutoMix effects (echo, wash, filter sweep), 0...1.
    #[serde(rename = "settings.transitionEffects")]
    pub transition_effects: f64,
}

impl TransitionSettings {
    /// `transition_defaults.json` next to `IncrementalSyncService::default_defaults_path()`.
    pub fn default_path() -> Option<PathBuf> {
        IncrementalSyncService::default_defaults_path()
            .map(|path| path.with_file_name("transition_defaults.json"))
    }

    /// A missing or unreadable file reads as the defaults.
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec(self)?)
    }
}

impl Default for TransitionSettings {
    fn default() -> Self {
        Self {
            auto_mix_enabled: false,
            crossfade_enabled: false,
            crossfade_seconds: 6.0,
            transition_effects: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub from_id: String,
    pub to_id: String,
    pub url: String,
    pub loudness: Option<f64>,
    pub mix: Mix,
    pub style: Style,
    /// The incoming song's beat, which its echo filter is built for.
    pub echo_delay: Option<f64>,
    /// Length of the outgoing song when the plan was made.
    pub duration: f64,
    /// Why an AutoMix became a crossfade, `None` for a mix or with AutoMix off.
    pub fallback: Option<String>,
}

impl Plan {
    /// Bass gain for the incoming deck's filter chain: cut for a beat mix.
    pub fn incoming_bass_gain(&self) -> Option<f64> {
        self.mix.is_beat_matched().then_some(BASS_CUT)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Planning,
    Planned,
    Prepared,
    Fading,
}

/// What `tick` reads from NowPlaying.
#[derive(Clone, Debug, Default)]
pub struct NowPlayingState<'a> {
    pub video_id: Option<&'a str>,
    /// The queue's next song (`NowPlaying.nextSong`).
    pub next_video_id: Option<&'a str>,
    pub is_playing: bool,
    pub current_time: f64,
    pub duration: f64,
    pub is_repeat_on: bool,
    /// The sleep timer stops playback at the end of this song.
    pub stops_at_song_end: bool,
}

/// What the player does after a `tick`.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    /// The transition was cancelled; put the decks back as `restore` says
    /// for the phase it was in.
    Reset {
        was: Phase,
        reason: String,
    },
    /// Resolve `to`'s source, decode `TAIL_SECONDS` from the end of the
    /// playing file and `HEAD_SECONDS` from the start of `to`, analyse both,
    /// then call `plan_ready` (or `plan_failed`) with `token`.
    Plan {
        token: u64,
        from: String,
        to: String,
        duration: f64,
    },
    /// Load the next song on the spare deck, paused and silent
    /// (`incoming_properties`), then report `deck_loaded` or `deck_failed`.
    LoadIncoming(Plan),
    /// Sleep `wait` seconds, call `begin`, unpause the incoming deck, then
    /// run `fade_gains` (no beats) or a `BeatMix` (beats) every 50 ms.
    StartMix {
        wait: f64,
        plan: Plan,
    },
}

/// What `finish_now` asks of the player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Finish {
    Nothing,
    /// The mix had not begun: put the decks back for `was`.
    Reset {
        was: Phase,
    },
    /// Stop the mix loop, promote the spare deck when `promote`, then do
    /// what `complete` asks with `final_speed`.
    Complete {
        promote: bool,
        final_speed: f64,
    },
}

/// Swift `Crossfade` state, without the decks and tasks.
#[derive(Debug, Default)]
pub struct Crossfade {
    phase: Phase,
    plan: Option<Plan>,
    /// Song, next song and duration of the planning run.
    planning: Option<(String, String, f64)>,
    /// Identifies the current planning run; a reset makes older runs stale.
    token: u64,
    incoming_ready: bool,
    /// The mix loop has started (Swift `outgoing != nil`).
    begun: bool,
    promoted: bool,
    outgoing_finished: bool,
    /// Song whose transition failed, so the tick doesn't retry it every 0.25 s.
    failed_for: Option<String>,
    /// Transition settings read when the playing song started, so changing
    /// them takes effect from the next song.
    song_settings: Option<(String, TransitionSettings)>,
    /// AutoMixes planned so far, to rotate between styles.
    mix_count: i64,
}

impl Crossfade {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn plan(&self) -> Option<&Plan> {
        self.plan.as_ref()
    }

    /// The song a running mix is fading into: a Listen Together guest lets
    /// its own mix finish instead of jumping to the room's song.
    pub fn mixing_into(&self) -> Option<&str> {
        (self.phase == Phase::Fading)
            .then(|| self.plan.as_ref().map(|p| p.to_id.as_str()))
            .flatten()
    }

    /// The decks have swapped: an ended song is the incoming one, shorter than the mix.
    pub fn has_promoted(&self) -> bool {
        self.promoted
    }

    /// The outgoing song ran out during the mix: `BeatMix::step` gets no positions.
    pub fn outgoing_finished(&self) -> bool {
        self.outgoing_finished
    }

    fn is_enabled(&self) -> bool {
        self.song_settings
            .as_ref()
            .is_some_and(|(_, s)| s.auto_mix_enabled || s.crossfade_enabled)
    }

    /// Called every 0.25 s with the playing song and the live settings.
    pub fn tick(&mut self, np: &NowPlayingState, settings: &TransitionSettings) -> Action {
        if self.failed_for.is_some() && self.failed_for.as_deref() != np.video_id {
            self.failed_for = None;
        }
        if let Some(id) = np.video_id
            && self.song_settings.as_ref().map(|(s, _)| s.as_str()) != Some(id)
        {
            self.song_settings = Some((id.to_string(), settings.clone()));
        }
        if self.phase == Phase::Fading {
            // Play Next or a reorder changed what follows: the mix can still be called off.
            if !self.promoted
                && let Some(plan) = &self.plan
                && np.next_video_id != Some(plan.to_id.as_str())
            {
                return self.reset_action("queue changed");
            }
            return Action::None;
        }
        let (Some(current), Some(next)) = (np.video_id, np.next_video_id) else {
            return self.stop_action();
        };
        if !self.is_enabled()
            || np.is_repeat_on
            || np.stops_at_song_end
            || np.duration <= 30.0
            || np.duration >= 600.0
        {
            return self.stop_action();
        }
        // Swift goes on to plan in the same tick; here planning starts on the next one.
        if let Some(plan) = &self.plan
            && (plan.from_id != current || plan.to_id != next)
        {
            return self.reset_action("queue changed");
        }
        if !np.is_playing {
            return Action::None;
        }
        let position = np.current_time;

        match self.phase {
            Phase::Idle => {
                if position < PLAN_DELAY || self.failed_for.as_deref() == Some(current) {
                    return Action::None;
                }
                self.phase = Phase::Planning;
                self.planning = Some((current.to_string(), next.to_string(), np.duration));
                Action::Plan {
                    token: self.token,
                    from: current.to_string(),
                    to: next.to_string(),
                    duration: np.duration,
                }
            }
            Phase::Planned => match &self.plan {
                Some(plan) if position >= plan.mix.fade_start - PREPARE_LEAD => {
                    let plan = plan.clone();
                    self.phase = Phase::Prepared;
                    self.incoming_ready = false;
                    Action::LoadIncoming(plan)
                }
                _ => Action::None,
            },
            Phase::Prepared => {
                let Some(plan) = &self.plan else {
                    return Action::None;
                };
                let fade_start = plan.mix.fade_start;
                // The next song is still loading past the start: a mix begun
                // late would jump straight to its end. Let the song end on its own.
                if !self.incoming_ready && position > fade_start + 1.0 {
                    let late = (position - fade_start) as i64;
                    let reason = format!("next song not loaded {late}s after the mix start");
                    let was = self.fail(&reason);
                    return Action::Reset { was, reason };
                }
                if !self.incoming_ready || position < fade_start - 0.4 {
                    return Action::None;
                }
                let plan = plan.clone();
                self.phase = Phase::Fading;
                // The tick is coarse; sleep the rest so the start lands on the beat.
                Action::StartMix {
                    wait: (fade_start - position).max(0.0),
                    plan,
                }
            }
            Phase::Planning | Phase::Fading => Action::None,
        }
    }

    fn stop_action(&mut self) -> Action {
        if self.phase == Phase::Idle {
            return Action::None;
        }
        self.reset_action("transitions turned off, repeat, sleep timer or no next song")
    }

    fn reset_action(&mut self, reason: &str) -> Action {
        Action::Reset {
            was: self.reset(reason),
            reason: reason.to_string(),
        }
    }

    /// Cancels any planned or running transition and returns the phase it
    /// was in. Called when a new song is loaded the normal way or playback
    /// stops. The player then cancels its plan, mix and speed-ease tasks and
    /// puts back the decks: for any phase but `Idle`, the spare deck that took
    /// part is paused at volume 100, speed 1, `clear_effects`; for `Fading`
    /// the active deck also gets volume 100 and `clear_effects`.
    pub fn reset(&mut self, reason: &str) -> Phase {
        let was = self.phase;
        if was != Phase::Idle {
            log::info!("Transition reset: {reason}");
        }
        self.token += 1;
        self.clear();
        was
    }

    /// Gives up on the planned transition: the song ends on its own and the
    /// next one loads the normal way. Returns the phase for `reset`.
    pub fn fail(&mut self, reason: &str) -> Phase {
        log::warn!("Transition failed: {reason}; the song ends normally");
        self.failed_for = self
            .plan
            .as_ref()
            .map(|p| p.from_id.clone())
            .or_else(|| self.planning.as_ref().map(|(from, _, _)| from.clone()));
        self.reset(reason)
    }

    /// The next song's source could not be resolved: the song ends normally.
    pub fn plan_failed(&mut self, token: u64) {
        if token != self.token || self.phase != Phase::Planning {
            return;
        }
        log::warn!("Transition plan failed: no source for the next song; the song ends normally");
        self.failed_for = self.planning.take().map(|(from, _, _)| from);
        self.phase = Phase::Idle;
    }

    /// Finishes a planning run: picks the mix and its style. `url` and
    /// `loudness` are the next song's source, `now` the playing position,
    /// `current_echo_delay` the beat the playing deck's echo was built for.
    /// `None` when the run is stale or the plan came too late (the song then
    /// ends normally).
    #[allow(clippy::too_many_arguments)]
    pub fn plan_ready(
        &mut self,
        token: u64,
        url: &str,
        loudness: Option<f64>,
        tail: Option<&TrackAnalysis>,
        head: Option<&TrackAnalysis>,
        now: f64,
        has_effect_filters: bool,
        current_echo_delay: Option<f64>,
    ) -> Option<&Plan> {
        if token != self.token || self.phase != Phase::Planning {
            return None;
        }
        let (from, to, duration) = self.planning.take()?;
        let settings = self
            .song_settings
            .as_ref()
            .map(|(_, s)| s.clone())
            .unwrap_or_default();
        let tail_start = (duration - TAIL_SECONDS).max(0.0);
        let wants_auto_mix = settings.auto_mix_enabled;
        let mut auto_mix = if wants_auto_mix {
            mix_planner::auto_mix(duration, tail, tail_start, head)
        } else {
            None
        };
        let mut fallback =
            (wants_auto_mix && auto_mix.is_none()).then(|| auto_mix_blocker(tail, head));
        // Analysis took too long: a mix begun past its start would jump to its end.
        if auto_mix
            .as_ref()
            .is_some_and(|late| late.fade_start < now + MIN_LEAD)
        {
            fallback = Some("finished too late for the mix".to_string());
            auto_mix = None;
        }
        let mix = auto_mix.unwrap_or_else(|| {
            mix_planner::smart(duration, settings.crossfade_seconds, tail, tail_start, head)
        });
        if mix.fade_start < now + MIN_LEAD {
            log::warn!(
                "Transition plan too late: {}s left in the song; it ends normally",
                (duration - now) as i64
            );
            self.failed_for = Some(from);
            self.phase = Phase::Idle;
            return None;
        }
        if let Some(fallback) = &fallback {
            log::warn!("AutoMix fell back to a crossfade: {fallback}");
        }
        let mut style = Style::Blend;
        if mix.is_beat_matched() && has_effect_filters {
            self.mix_count += 1;
            let room = duration - mix.fade_start - mix.fade_length;
            let cues = match (tail, head) {
                (Some(tail), Some(head)) => mix_planner::cues(&mix, tail, tail_start, head),
                _ => Cues::default(),
            };
            style = mix_planner::style(&mix, current_echo_delay, room, cues, self.mix_count);
        }
        log::info!(
            "Transition plan: {} at {:.1}s for {:.1}s, next from {:.1}s, speed {:.3}",
            if mix.is_beat_matched() {
                style.as_str()
            } else {
                "crossfade"
            },
            mix.fade_start,
            mix.fade_length,
            mix.in_start,
            mix.speed
        );
        self.phase = Phase::Planned;
        self.plan = Some(Plan {
            from_id: from,
            to_id: to,
            url: url.to_string(),
            loudness,
            echo_delay: head.and_then(TrackAnalysis::beat_period),
            mix,
            style,
            duration,
            fallback,
        });
        self.plan.as_ref()
    }

    /// The spare deck loaded the next song. True when the player should set
    /// its `start` property back to `none`.
    pub fn deck_loaded(&mut self) -> bool {
        if self.phase != Phase::Prepared || self.plan.is_none() {
            return false;
        }
        self.incoming_ready = true;
        true
    }

    /// The spare deck failed to load. `Some(phase)` when the transition was
    /// given up and the decks need `reset`'s restore.
    pub fn deck_failed(&mut self) -> Option<Phase> {
        let fails =
            self.phase == Phase::Prepared || (self.phase == Phase::Fading && !self.promoted);
        (fails && self.plan.is_some())
            .then(|| self.fail("next song failed to load on the spare deck"))
    }

    /// The mix loop starts: `StartMix`'s wait is over.
    pub fn begin(&mut self) {
        self.begun = true;
        self.promoted = false;
        self.outgoing_finished = false;
    }

    /// Marks the decks swapped. True the first time: the player then calls
    /// `promoteSpare(plan.to_id, plan.loudness, plan.echo_delay)` and moves
    /// NowPlaying to the next song. Call it once the mix progress reaches 0.5.
    pub fn promote(&mut self) -> bool {
        if self.promoted || self.plan.is_none() {
            return false;
        }
        self.promoted = true;
        true
    }

    /// The outgoing song ran out before the transition ended. True when the
    /// player must promote now.
    pub fn outgoing_ended(&mut self) -> bool {
        if self.phase != Phase::Fading {
            return false;
        }
        self.outgoing_finished = true;
        self.promote()
    }

    /// Ends a running transition at once (pause, seek or speed change).
    pub fn finish_now(&mut self) -> Finish {
        if self.phase != Phase::Fading {
            return Finish::Nothing;
        }
        if !self.begun {
            // The transition was scheduled but hadn't started; keep the song going.
            return Finish::Reset {
                was: self.reset("pause, seek or speed change"),
            };
        }
        let promote = self.promote();
        Finish::Complete {
            promote,
            final_speed: self.complete(None),
        }
    }

    /// The mix is over. Returns the incoming deck's final speed. The player
    /// pauses the outgoing deck (never stops it) at volume 100, speed 1 and
    /// `clear_effects`, sets the incoming deck to volume 100 and
    /// `clear_effects`, and when the speed is not 1 eases it with
    /// `eased_speed` every 200 ms. `speed` is `BeatMix::final_speed`, `None`
    /// for a timed fade.
    pub fn complete(&mut self, speed: Option<f64>) -> f64 {
        let final_speed = speed
            .or_else(|| self.plan.as_ref().map(|p| p.mix.speed))
            .unwrap_or(1.0);
        self.clear();
        final_speed
    }

    fn clear(&mut self) {
        self.phase = Phase::Idle;
        self.plan = None;
        self.planning = None;
        self.incoming_ready = false;
        self.begun = false;
        self.promoted = false;
        self.outgoing_finished = false;
    }
}

/// Why `mix_planner::auto_mix` found no mix, for the log.
pub fn auto_mix_blocker(tail: Option<&TrackAnalysis>, head: Option<&TrackAnalysis>) -> String {
    let Some(tail) = tail else {
        return "the outgoing song could not be analysed".into();
    };
    let Some(head) = head else {
        return "the next song could not be analysed".into();
    };
    let Some(out_bpm) = tail.bpm else {
        return "no steady beat in the outgoing song".into();
    };
    let Some(in_bpm) = head.bpm else {
        return "no steady beat in the next song".into();
    };
    let gap = (out_bpm / in_bpm - 1.0).abs();
    if gap > mix_planner::MAX_TEMPO_MATCH {
        return format!("tempo gap of {}%", (gap * 100.0).round() as i64);
    }
    "no room for a mix before the song goes quiet".into()
}

/// Properties the player sets on the spare deck before `loadfile plan.url
/// replace`, in order. The `af` chain comes from `PlayerController::filterChain`
/// with `plan.loudness`, `plan.incoming_bass_gain()` and `plan.echo_delay`.
pub fn incoming_properties(plan: &Plan) -> [(&'static str, String); 5] {
    [
        ("pause", "yes".into()),
        ("volume", "0".into()),
        ("speed", fmt(plan.mix.speed)),
        // Keep the audio output open between files.
        ("gapless-audio", "yes".into()),
        ("start", fmt(plan.mix.in_start)),
    ]
}

// MARK: - Fades

/// Smart Crossfade progress (0...1) after `elapsed` seconds of the fade.
pub fn fade_progress(elapsed: f64, fade_length: f64) -> f64 {
    (elapsed / fade_length).clamp(0.0, 1.0)
}

/// Equal-power fade at progress `t`: (outgoing, incoming) linear gains.
pub fn fade_gains(t: f64) -> (f64, f64) {
    let t = t.clamp(0.0, 1.0) * std::f64::consts::FRAC_PI_2;
    (t.cos(), t.sin())
}

/// mpv's volume is cubic (amplitude = (volume / 100)³), so a linear gain
/// needs a cube root to fade smoothly.
pub fn mpv_volume(gain: f64) -> String {
    fmt(100.0 * gain.max(0.0).cbrt())
}

/// Numbers as mpv properties and filter arguments take them (Swift `PlayerController.fmt`).
pub fn fmt(value: f64) -> String {
    format!("{value:.4}")
}

/// Incoming speed `elapsed` seconds into easing `from` back to 1×. Done at `SPEED_EASE_SECONDS`.
pub fn eased_speed(from: f64, elapsed: f64) -> f64 {
    let t = (elapsed / SPEED_EASE_SECONDS).min(1.0);
    from + (1.0 - from) * t
}

/// Incoming level factor at progress `t`: held at `DUCK_LEVEL` while the
/// outgoing song still sings over it, then released over 2 beats.
pub fn duck(mix: &Mix, t: f64) -> f64 {
    let (Some(until), Some(beats)) = (mix.duck_until, mix.beats) else {
        return 1.0;
    };
    let release = ((t - until) * beats as f64 / 2.0).clamp(0.0, 1.0);
    mix_planner::DUCK_LEVEL + (1.0 - mix_planner::DUCK_LEVEL) * release
}

/// Where the incoming low-pass starts: at full strength 300 Hz, at zero strength already open.
pub fn low_pass_start(strength: f64) -> f64 {
    mix_planner::sweep(LOW_PASS_SWEEP.1, LOW_PASS_SWEEP.0, strength)
}

/// Where the outgoing high-pass ends up.
pub fn high_pass_end(strength: f64) -> f64 {
    mix_planner::sweep(HIGH_PASS_SWEEP.0, HIGH_PASS_SWEEP.1, strength)
}

/// How long a style's tail rings after the mix, `None` when it has none.
pub fn tail_seconds(style: Style, period: f64) -> Option<f64> {
    match style {
        Style::Echo => Some(mix_planner::echo_tail(period)),
        Style::Wash => Some(mix_planner::WASH_TAIL),
        Style::Blend | Style::Sweep | Style::Loop => None,
    }
}

/// Incoming speed that pulls its beats back onto the outgoing beats: where
/// the incoming song should be, and a small correction to get there.
pub fn lock_speed(mix: &Mix, outgoing: f64, incoming: f64) -> f64 {
    let expected = mix.in_start + (outgoing - mix.fade_start) * mix.speed;
    let error = (incoming - expected) / mix.speed;
    let correction = (-LOCK_GAIN * error).clamp(-MAX_LOCK_CORRECTION, MAX_LOCK_CORRECTION);
    mix.speed * (1.0 + correction)
}

// MARK: - Effects

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Deck {
    Outgoing,
    Incoming,
}

/// One live filter change: `deck.command("af-command", &effect.args())`.
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub deck: Deck,
    pub label: &'static str,
    /// The filter inside the label's lavfi graph: with mpv's default ("all")
    /// the command fails on the graph's other filters.
    pub target: &'static str,
    pub name: &'static str,
    pub value: String,
}

impl Effect {
    fn new(
        deck: Deck,
        label: &'static str,
        target: &'static str,
        name: &'static str,
        value: f64,
    ) -> Self {
        Self {
            deck,
            label,
            target,
            name,
            value: fmt(value),
        }
    }

    /// Arguments of mpv's `af-command`.
    pub fn args(&self) -> [&str; 4] {
        [self.label, self.name, &self.value, self.target]
    }
}

fn bass(deck: Deck, gain: f64) -> Effect {
    Effect::new(deck, BASS_FILTER_LABEL, "bass", "g", gain)
}

/// Filter values that put a deck's AutoMix filters back to rest. The player
/// also sets `ab-loop-a` and `ab-loop-b` to `no` and `ab-loop-count` to `inf`.
pub fn clear_effects(deck: Deck, has_bass_filter: bool, has_effect_filters: bool) -> Vec<Effect> {
    let mut effects = Vec::new();
    if has_bass_filter {
        effects.push(bass(deck, 0.0));
    }
    if has_effect_filters {
        effects.extend([
            Effect::new(deck, HIGH_PASS_LABEL, "highpass", "m", 0.0),
            Effect::new(deck, LOW_PASS_LABEL, "lowpass", "m", 0.0),
            Effect::new(deck, EFFECTS_LABEL, "volume@dry", "volume", 1.0),
            Effect::new(deck, EFFECTS_LABEL, "volume@echosend", "volume", 0.0),
            Effect::new(deck, EFFECTS_LABEL, "volume@verbsend", "volume", 0.0),
        ]);
    }
    effects
}

/// Deck gains (linear, for `mpv_volume`) and filter values at one moment of a mix.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub outgoing: f64,
    pub incoming: f64,
    pub effects: Vec<Effect>,
}

/// Swift `applyStyle`: volumes and effect levels at mix progress `t` (0...1).
/// The outgoing song fades out over the second half: by its deck volume, or
/// for echo and wash by its dry path only, so their tail outlives it.
/// `duck` is `duck(mix, t)`, `strength` the Transition Effects setting.
pub fn frame(style: Style, t: f64, beats: usize, duck: f64, strength: f64) -> Frame {
    use std::f64::consts::PI;
    let strength = strength.clamp(0.0, 1.0);
    let out_gain = if t < 0.5 { 1.0 } else { ((t - 0.5) * PI).cos() };
    let in_gain = if t < 0.5 { (t * PI).sin() } else { 1.0 } * duck;
    let fx = |name: &'static str, value: f64| {
        Effect::new(Deck::Outgoing, EFFECTS_LABEL, name, "volume", value)
    };
    // Gains and filter mixes change in 50 ms steps: a big step is an audible
    // click, so every send and filter switch ramps over many ticks.
    match style {
        Style::Echo => {
            // The last bar feeds the echo, rising to its middle and back to 0 at its end.
            let bar_beats = BEATS_PER_BAR as f64;
            let into_bar = (t - 1.0) * beats as f64 + bar_beats;
            let send = into_bar.min(bar_beats - into_bar).max(0.0) / (bar_beats / 2.0) * strength;
            Frame {
                outgoing: 1.0,
                incoming: in_gain,
                effects: vec![fx("volume@dry", out_gain), fx("volume@echosend", send)],
            }
        }
        Style::Wash => {
            // Back to 0 over the last 2 beats, so the reverb keeps a tail, not a cut.
            let fade_out = (1.0 - t) * beats as f64 / 2.0;
            let send = ((t - 0.5) * 2.0).min(fade_out).clamp(0.0, 1.0) * strength;
            Frame {
                outgoing: 1.0,
                incoming: in_gain,
                effects: vec![fx("volume@dry", out_gain), fx("volume@verbsend", send)],
            }
        }
        Style::Sweep => {
            // A filter at the edge of hearing still shifts the bass phase, so
            // switching it on or off at once clicks: its mix fades instead.
            let lp =
                |name, value| Effect::new(Deck::Incoming, LOW_PASS_LABEL, "lowpass", name, value);
            let hp =
                |name, value| Effect::new(Deck::Outgoing, HIGH_PASS_LABEL, "highpass", name, value);
            let effects = if t < 0.5 {
                let cutoff =
                    mix_planner::sweep(low_pass_start(strength), LOW_PASS_SWEEP.1, t * 2.0);
                vec![lp("f", cutoff), hp("m", t * 2.0)]
            } else {
                let cutoff =
                    mix_planner::sweep(HIGH_PASS_SWEEP.0, high_pass_end(strength), (t - 0.5) * 2.0);
                vec![
                    lp("m", 1.0 - (t - 0.5) * 2.0),
                    hp("f", cutoff),
                    hp("m", 1.0),
                ]
            };
            Frame {
                outgoing: out_gain,
                incoming: in_gain,
                effects,
            }
        }
        Style::Blend | Style::Loop => Frame {
            outgoing: out_gain,
            incoming: in_gain,
            effects: Vec::new(),
        },
    }
}

/// The filter values that cut the outgoing song after an echo or wash mix,
/// leaving only its tail.
pub fn tail_cut_effects() -> [Effect; 3] {
    let fx = |name| Effect::new(Deck::Outgoing, EFFECTS_LABEL, name, "volume", 0.0);
    [
        fx("volume@dry"),
        fx("volume@echosend"),
        fx("volume@verbsend"),
    ]
}

/// What the player does at one 50 ms step of a beat mix.
#[derive(Clone, Debug, PartialEq)]
pub struct BeatStep {
    /// Mix progress (0...1); call `Crossfade::promote` at 0.5.
    pub t: f64,
    pub outgoing_volume: f64,
    pub incoming_volume: f64,
    /// Filter values that changed since the last step.
    pub effects: Vec<Effect>,
    /// New incoming speed from the beat lock.
    pub speed: Option<f64>,
    /// The mix is over: wait out `BeatMix::tail`, then `Crossfade::complete`.
    pub done: bool,
}

/// Swift `runBeatMix`: progress is counted in outgoing beats, read from the
/// music itself, so every step lands on the beat.
/// - First half: the incoming song fades in with its bass cut.
/// - Halfway (a bar line): the bass swaps from the outgoing song to the
///   incoming one, and the incoming song becomes the current song.
/// - Second half: the outgoing song fades out.
#[derive(Debug)]
pub struct BeatMix {
    mix: Mix,
    beats: usize,
    period: f64,
    style: Style,
    /// The bar the loop style repeats and how many extra times: the player
    /// sets `ab-loop-a`, `ab-loop-b` and `ab-loop-count` on the outgoing deck.
    pub loop_bar: Option<(std::ops::Range<f64>, usize)>,
    bass_swapped: bool,
    last_speed: f64,
    last_lock_update: Option<f64>,
    /// Seconds the loop has jumped the outgoing song back, added to its
    /// position so the mix keeps counting forward.
    loop_offset: f64,
    last_outgoing: Option<f64>,
    /// Effect values sent during this mix, so unchanged ones aren't resent every 50 ms.
    sent: HashMap<(Deck, &'static str, &'static str, &'static str), String>,
}

impl BeatMix {
    /// `None` for a timed fade (no beats).
    pub fn new(plan: &Plan) -> Option<Self> {
        let (beats, period) = (plan.mix.beats?, plan.mix.beat_period?);
        Some(Self {
            mix: plan.mix.clone(),
            beats,
            period,
            style: plan.style,
            loop_bar: (plan.style == Style::Loop)
                .then(|| mix_planner::loop_bar(&plan.mix))
                .flatten(),
            bass_swapped: false,
            last_speed: plan.mix.speed,
            last_lock_update: None,
            loop_offset: 0.0,
            last_outgoing: None,
            sent: HashMap::new(),
        })
    }

    pub fn style(&self) -> Style {
        self.style
    }

    /// Filter values sent before the incoming deck is heard. The incoming
    /// deck is still at volume 0, so the low-pass is in place first.
    pub fn start_effects(&mut self, strength: f64) -> Vec<Effect> {
        if self.style != Style::Sweep {
            return Vec::new();
        }
        let start = [
            Effect::new(
                Deck::Incoming,
                LOW_PASS_LABEL,
                "lowpass",
                "f",
                low_pass_start(strength),
            ),
            Effect::new(Deck::Incoming, LOW_PASS_LABEL, "lowpass", "m", 1.0),
            // Off (m=0) while it moves from its 1 kHz rest to the sweep start.
            Effect::new(
                Deck::Outgoing,
                HIGH_PASS_LABEL,
                "highpass",
                "f",
                HIGH_PASS_SWEEP.0,
            ),
        ];
        self.unsent(start.into())
    }

    fn unsent(&mut self, effects: Vec<Effect>) -> Vec<Effect> {
        effects
            .into_iter()
            .filter(|e| {
                let key = (e.deck, e.label, e.target, e.name);
                self.sent.insert(key, e.value.clone()).as_ref() != Some(&e.value)
            })
            .collect()
    }

    /// One step. `elapsed` is wall seconds since the mix began, `positions`
    /// the (outgoing, incoming) deck positions (`None` once the outgoing song
    /// ended), `has_effect_filters` whether mpv still has them, `strength`
    /// the live Transition Effects setting.
    pub fn step(
        &mut self,
        elapsed: f64,
        positions: Option<(f64, f64)>,
        has_effect_filters: bool,
        strength: f64,
    ) -> BeatStep {
        // mpv dropped the effect filters mid-song: the mix carries on by volume alone.
        if !matches!(self.style, Style::Blend | Style::Loop) && !has_effect_filters {
            log::warn!(
                "AutoMix style {} lost its filters mid-mix; finishing as a blend",
                self.style.as_str()
            );
            self.style = Style::Blend;
        }
        if let (Some((bar, _)), Some((position, _))) = (&self.loop_bar, positions) {
            let length = bar.end - bar.start;
            if self
                .last_outgoing
                .is_some_and(|last| position < last - length / 2.0)
            {
                self.loop_offset += length;
            }
            self.last_outgoing = Some(position);
        }
        let out_position = positions.map(|(out, _)| out + self.loop_offset);
        // Once the outgoing song has ended, its position stops; count by the clock.
        let elapsed_beats = out_position.map_or(elapsed / self.period, |p| {
            (p - self.mix.fade_start) / self.period
        });
        let t = (elapsed_beats / self.beats as f64).clamp(0.0, 1.0);

        let frame = frame(self.style, t, self.beats, duck(&self.mix, t), strength);
        let mut effects = self.unsent(frame.effects);
        if !self.bass_swapped && t >= 0.5 {
            self.bass_swapped = true;
            effects.push(bass(Deck::Incoming, 0.0));
            effects.push(bass(Deck::Outgoing, BASS_CUT));
        }

        let mut speed = None;
        if let (Some((_, incoming)), Some(out_position)) = (positions, out_position)
            && self
                .last_lock_update
                .is_none_or(|last| elapsed - last >= 0.25)
        {
            self.last_lock_update = Some(elapsed);
            let locked = lock_speed(&self.mix, out_position, incoming);
            if (locked - self.last_speed).abs() > 0.0005 {
                self.last_speed = locked;
                speed = Some(locked);
            }
        }

        BeatStep {
            t,
            outgoing_volume: frame.outgoing,
            incoming_volume: frame.incoming,
            effects,
            speed,
            // Safety net if positions stop moving for another reason.
            done: t >= 1.0 || elapsed > self.mix.fade_length / self.mix.speed + 5.0,
        }
    }

    /// Seconds to let an echo or wash ring after `tail_cut_effects`, `None`
    /// when the style has no tail. Never past the song's end: a deck that runs
    /// out goes idle. Skip it when the outgoing song already ended.
    pub fn tail(&self, outgoing_position: Option<f64>, song_duration: f64) -> Option<f64> {
        let tail = tail_seconds(self.style, self.period)?;
        let left = outgoing_position.map_or(0.0, |p| song_duration - p - 0.5);
        if left < tail {
            log::warn!(
                "AutoMix {} tail cut to {}s: the song ends",
                self.style.as_str(),
                left.max(0.0)
            );
        }
        Some(tail.min(left).max(0.0))
    }

    /// The incoming speed the mix ended at, for `Crossfade::complete`.
    pub fn final_speed(&self) -> f64 {
        self.last_speed
    }
}

// MARK: - Analysis input

/// FFmpeg arguments that decode `seconds` of audio at the start or end of
/// `url` to mono 32-bit floats at `SAMPLE_RATE` in `output` (`pipe:1` for
/// stdout). The bytes go through `samples_from_f32le` to
/// `track_analyzer::analyze`. Desktop FFmpeg uses the system CA store, so
/// Swift's `-cafile` is not needed.
pub fn ffmpeg_args(url: &str, seconds: f64, from_end: bool, output: &str) -> Vec<String> {
    let mut args: Vec<String> = ["-y", "-v", "error", "-rw_timeout", "15000000"]
        .map(String::from)
        .into();
    if url.starts_with("http") {
        args.extend(["-headers".into(), "User-Agent: Mozilla/5.0\r\n".into()]);
    }
    let seconds = seconds as i64;
    if from_end {
        args.extend(["-sseof".into(), format!("-{seconds}")]);
    }
    args.extend([
        "-i".into(),
        url.into(),
        "-t".into(),
        seconds.to_string(),
        "-vn".into(),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        SAMPLE_RATE.to_string(),
        "-f".into(),
        "f32le".into(),
        output.into(),
    ]);
    args
}

/// Little-endian 32-bit float samples; a trailing partial sample is dropped.
pub fn samples_from_f32le(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}
