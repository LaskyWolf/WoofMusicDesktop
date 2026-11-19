//! Swift `Crossfade`'s player half: runs the transitions `trop::audio::crossfade`
//! decides on, on the player's two decks. The app calls `tick` with the
//! playing song; near its end the next song starts on the spare deck and the
//! two are mixed, then `take_promoted` tells the app the decks swapped.

use std::sync::{Arc, Mutex, MutexGuard, Once};
use std::time::{Duration, Instant};

use iced::Task;
use iced::task::Handle;
use trop::audio::crossfade::{
    self, Action, BeatMix, Crossfade, Deck, Effect, Finish, HEAD_SECONDS, NowPlayingState, Phase,
    Plan, SPEED_EASE_SECONDS, TAIL_SECONDS, TransitionSettings, fmt,
};
use trop::audio::player_controller::player_controller::{MpvDeck, PlayerController};
use trop::audio::track_analyzer::{self, TrackAnalysis};

use crate::Message;
use crate::services::Services;

/// What the running tasks share with `Transitions`.
#[derive(Clone)]
struct Shared {
    player: Arc<PlayerController>,
    state: Arc<Mutex<Crossfade>>,
    /// Song the decks swapped to, until the app takes it.
    promoted: Arc<Mutex<Option<String>>>,
    /// Beat the playing deck's echo filter was built for (Swift `currentEchoDelay`).
    echo_delay: Arc<Mutex<Option<f64>>>,
}

pub struct Transitions {
    shared: Shared,
    /// Planning, mix and speed-ease tasks, cancelled by a reset.
    running: Vec<Handle>,
    pub settings: TransitionSettings,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl Transitions {
    pub fn new(player: Arc<PlayerController>, settings: TransitionSettings) -> Self {
        Self {
            shared: Shared {
                player,
                state: Arc::default(),
                promoted: Arc::default(),
                echo_delay: Arc::default(),
            },
            running: Vec::new(),
            settings,
        }
    }

    /// The song the decks swapped to since the last call: the app moves its
    /// queue there without loading it.
    pub fn take_promoted(&self) -> Option<String> {
        lock(&self.shared.promoted).take()
    }

    /// Swift `Crossfade.tick`, every app tick while a song is loaded.
    pub fn tick(&mut self, np: &NowPlayingState, services: &Arc<Services>) -> Task<Message> {
        let shared = &self.shared;
        let events = shared.player.take_deck_events();
        let (action, failed) = {
            let mut state = lock(&shared.state);
            if events.spare_loaded
                && state.deck_loaded()
                && let Some(spare) = shared.player.spare()
            {
                let _ = spare.set("start", "none");
            }
            let failed = if events.spare_failed {
                state.deck_failed()
            } else {
                None
            };
            // `outgoing_ended` marks the mix promoted itself: swap the decks here.
            if events.outgoing_ended
                && let Some(plan) = state.plan().cloned()
                && state.outgoing_ended()
            {
                promoted(shared, &plan);
            }
            (state.tick(np, &self.settings), failed)
        };
        if let Some(was) = failed {
            self.restore(was);
        }
        match action {
            Action::None => Task::none(),
            Action::Reset { was, .. } => {
                self.restore(was);
                Task::none()
            }
            Action::Plan { token, to, .. } => {
                let shared = self.shared.clone();
                let services = services.clone();
                let playing = shared.player.active_path();
                self.spawn(async move { make_plan(shared, services, token, to, playing).await })
            }
            Action::LoadIncoming(plan) => {
                self.load_incoming(&plan);
                Task::none()
            }
            Action::StartMix { wait, plan } => {
                let shared = self.shared.clone();
                let strength = self.settings.transition_effects;
                self.spawn(async move { run_mix(shared, plan, wait, strength).await })
            }
        }
    }

    /// Cancels any planned or running transition: a song is loaded the normal way.
    pub fn reset(&mut self, reason: &str) {
        let was = lock(&self.shared.state).reset(reason);
        *lock(&self.shared.echo_delay) = None;
        self.restore(was);
    }

    /// Ends a running transition at once: the user paused or seeked.
    pub fn finish_now(&mut self) -> Task<Message> {
        let (plan, finish) = {
            let mut state = lock(&self.shared.state);
            (state.plan().cloned(), state.finish_now())
        };
        match finish {
            Finish::Nothing => Task::none(),
            Finish::Reset { was } => {
                self.restore(was);
                Task::none()
            }
            Finish::Complete {
                promote,
                final_speed,
            } => {
                self.abort();
                if promote && let Some(plan) = &plan {
                    promoted(&self.shared, plan);
                }
                settle(&self.shared.player, true);
                let deck = self.shared.player.active_deck();
                self.spawn(ease_speed(deck, final_speed))
            }
        }
    }

    fn load_incoming(&mut self, plan: &Plan) {
        let player = &self.shared.player;
        let deck = match player.spare_deck() {
            Ok(deck) => deck,
            Err(error) => {
                let was = lock(&self.shared.state).fail(&format!("no spare deck: {error}"));
                self.restore(was);
                return;
            }
        };
        for (name, value) in crossfade::incoming_properties(plan) {
            let _ = deck.set(name, &value);
        }
        player.apply_filters(
            &deck,
            plan.incoming_bass_gain(),
            plan.echo_delay,
            plan.loudness,
        );
        let _ = deck.command("loadfile", &[&plan.url, "replace"]);
    }

    /// Puts back the decks that took part in a transition that was in `was`.
    fn restore(&mut self, was: Phase) {
        self.abort();
        if was != Phase::Idle {
            settle(&self.shared.player, was == Phase::Fading);
        }
    }

    fn abort(&mut self) {
        for handle in self.running.drain(..) {
            handle.abort();
        }
    }

    fn spawn(&mut self, work: impl Future<Output = ()> + Send + 'static) -> Task<Message> {
        let (task, handle) = Task::future(work).discard().abortable();
        self.running.push(handle);
        task
    }
}

/// Resolves the next song and analyses the end of this one and the start of
/// the next, then hands both to `plan_ready`.
async fn make_plan(
    shared: Shared,
    services: Arc<Services>,
    token: u64,
    next: String,
    playing: Option<String>,
) {
    let stream = services.resolve(next).await.ok();
    let (Some((url, loudness)), Some(playing)) = (stream, playing) else {
        lock(&shared.state).plan_failed(token);
        return;
    };
    let (tail, head) = iced::futures::join!(
        analyze(playing, TAIL_SECONDS, true),
        analyze(url.clone(), HEAD_SECONDS, false),
    );
    let now = shared.player.current_time();
    let has_effect_filters = shared.player.has_effect_filters();
    let echo_delay = *lock(&shared.echo_delay);
    lock(&shared.state).plan_ready(
        token,
        &url,
        loudness,
        tail.as_ref(),
        head.as_ref(),
        now,
        has_effect_filters,
        echo_delay,
    );
}

/// Decodes `seconds` at the start or end of `url` with the `ffmpeg` command
/// (Swift uses FFmpegKit) and analyses them. `None` without ffmpeg: a
/// crossfade is then placed by time alone and AutoMix falls back to it.
// ponytail: a reset doesn't kill a running ffmpeg; it ends on its own within `seconds` of audio or its 15 s network timeout.
async fn analyze(url: String, seconds: f64, from_end: bool) -> Option<TrackAnalysis> {
    static MISSING: Once = Once::new();
    let work = move || {
        let args = crossfade::ffmpeg_args(&url, seconds, from_end, "pipe:1");
        let output = match std::process::Command::new("ffmpeg")
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
        {
            Ok(output) => output,
            Err(_) => {
                MISSING.call_once(|| {
                    eprintln!("ffmpeg was not found: crossfades are timed without analysing the songs, and AutoMix falls back to them. Install ffmpeg for beat-matched mixes.");
                });
                return None;
            }
        };
        if !output.status.success() {
            return None;
        }
        let samples = crossfade::samples_from_f32le(&output.stdout);
        track_analyzer::analyze(&samples, track_analyzer::SAMPLE_RATE)
    };
    tokio::task::spawn_blocking(work).await.ok().flatten()
}

/// Swift `runFade` / `runBeatMix`, then `complete`.
async fn run_mix(shared: Shared, plan: Plan, wait: f64, strength: f64) {
    tokio::time::sleep(Duration::from_secs_f64(wait)).await;
    let player = &shared.player;
    let Some(incoming) = player.spare() else {
        let was = lock(&shared.state).fail("a deck went missing at the mix start");
        settle(player, was == Phase::Fading);
        return;
    };
    let outgoing = player.active_deck();
    lock(&shared.state).begin();
    player.set_crossfading(true);
    let _ = incoming.set("pause", "no");

    let speed = match BeatMix::new(&plan) {
        None => {
            fade(&shared, &plan, &outgoing, &incoming).await;
            None
        }
        Some(beat) => Some(beat_mix(&shared, &plan, beat, strength, &outgoing, &incoming).await),
    };
    let final_speed = lock(&shared.state).complete(speed);
    settle(player, true);
    ease_speed(player.active_deck(), final_speed).await;
}

/// Smart Crossfade: an equal-power fade timed by the clock.
async fn fade(shared: &Shared, plan: &Plan, outgoing: &MpvDeck, incoming: &MpvDeck) {
    let start = Instant::now();
    loop {
        let t = crossfade::fade_progress(start.elapsed().as_secs_f64(), plan.mix.fade_length);
        let (out_gain, in_gain) = crossfade::fade_gains(t);
        set_volumes(
            outgoing,
            incoming,
            out_gain,
            in_gain,
            shared.player.volume(),
        );
        // Halfway the next song becomes the one on screen.
        if t >= 0.5 {
            promote_now(shared, plan);
        }
        if t >= 1.0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// AutoMix: progress counted in outgoing beats, read from the music itself.
/// Returns the incoming speed it ended at.
async fn beat_mix(
    shared: &Shared,
    plan: &Plan,
    mut beat: BeatMix,
    strength: f64,
    outgoing: &MpvDeck,
    incoming: &MpvDeck,
) -> f64 {
    let player = &shared.player;
    apply(&beat.start_effects(strength), outgoing, incoming);
    if let Some((bar, repeats)) = &beat.loop_bar {
        let _ = outgoing.set("ab-loop-a", &fmt(bar.start));
        let _ = outgoing.set("ab-loop-b", &fmt(bar.end));
        let _ = outgoing.set("ab-loop-count", &repeats.to_string());
    }
    let start = Instant::now();
    loop {
        let finished = lock(&shared.state).outgoing_finished();
        let positions = if finished {
            None
        } else {
            player.positions(outgoing, incoming)
        };
        let step = beat.step(
            start.elapsed().as_secs_f64(),
            positions,
            player.has_effect_filters(),
            strength,
        );
        set_volumes(
            outgoing,
            incoming,
            step.outgoing_volume,
            step.incoming_volume,
            player.volume(),
        );
        apply(&step.effects, outgoing, incoming);
        if let Some(speed) = step.speed {
            let _ = incoming.set("speed", &fmt(speed));
        }
        if step.t >= 0.5 {
            promote_now(shared, plan);
        }
        if step.done {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // The song is cut and its sends closed: only the echo or reverb rings on.
    if !lock(&shared.state).outgoing_finished()
        && let Some(tail) = beat.tail(outgoing.position(), plan.duration)
    {
        apply(&crossfade::tail_cut_effects(), outgoing, incoming);
        tokio::time::sleep(Duration::from_secs_f64(tail)).await;
    }
    beat.final_speed()
}

/// Swaps the decks once, halfway through the mix.
fn promote_now(shared: &Shared, plan: &Plan) {
    if lock(&shared.state).promote() {
        promoted(shared, plan);
    }
}

fn promoted(shared: &Shared, plan: &Plan) {
    shared.player.promote_spare(&plan.to_id, plan.loudness);
    *lock(&shared.echo_delay) = plan.echo_delay;
    *lock(&shared.promoted) = Some(plan.to_id.clone());
}

/// Swift `easeSpeed`: back to 1× over `SPEED_EASE_SECONDS` after a tempo-matched mix.
async fn ease_speed(deck: MpvDeck, from: f64) {
    if from == 1.0 {
        return;
    }
    let start = Instant::now();
    loop {
        let elapsed = start.elapsed().as_secs_f64();
        let _ = deck.set("speed", &fmt(crossfade::eased_speed(from, elapsed)));
        if elapsed >= SPEED_EASE_SECONDS {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The spare deck paused and back to rest; with `active`, the playing deck
/// at full volume without effects too. Pause, never stop: the spare keeps its
/// audio output open for the next mix.
fn settle(player: &PlayerController, active: bool) {
    if let Some(spare) = player.spare() {
        let _ = spare.set("pause", "yes");
        let _ = spare.set("volume", &crossfade::fmt(100.0 * player.volume()));
        let _ = spare.set("speed", "1");
        clear_effects(player, &spare);
    }
    if active {
        let deck = player.active_deck();
        let _ = deck.set("volume", &crossfade::fmt(100.0 * player.volume()));
        clear_effects(player, &deck);
    }
    player.set_crossfading(false);
}

fn clear_effects(player: &PlayerController, deck: &MpvDeck) {
    let effects = crossfade::clear_effects(
        Deck::Outgoing,
        player.has_bass_filter(),
        player.has_effect_filters(),
    );
    apply(&effects, deck, deck);
    let _ = deck.set("ab-loop-a", "no");
    let _ = deck.set("ab-loop-b", "no");
    let _ = deck.set("ab-loop-count", "inf");
}

fn apply(effects: &[Effect], outgoing: &MpvDeck, incoming: &MpvDeck) {
    for effect in effects {
        let deck = match effect.deck {
            Deck::Outgoing => outgoing,
            Deck::Incoming => incoming,
        };
        let _ = deck.command("af-command", &effect.args());
    }
}

/// `master` is the volume slider: mpv's volume is cubic, so it scales the
/// linear gains by its cube.
fn set_volumes(outgoing: &MpvDeck, incoming: &MpvDeck, out_gain: f64, in_gain: f64, master: f64) {
    let master = master.powi(3);
    let _ = outgoing.set("volume", &crossfade::mpv_volume(out_gain * master));
    let _ = incoming.set("volume", &crossfade::mpv_volume(in_gain * master));
}
