// Trop/Audio/PlayerController/PlayerController.swift: an mpv deck that loads,
// plays, pauses, seeks and stops a stream, and the spare deck crossfades and
// AutoMix play the next song on (`spare_deck`, `promote_spare`, `filter_chain`).
//
// Left out:
// - Equalizer and loudness normalization in `filter_chain`: no settings for them yet.
// - NowPlaying, PlaybackStateService, PlaybackSpeed, lock screen and remote
//   commands live in the app.
// - AVAudioSession, interruptions, route changes and `restoreAudioOutput`:
//   no desktop equivalent (each mpv opens its own output).
// - `pendingURL` parking: the deck is created synchronously in `new`, so mpv
//   is always ready before `play`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use libmpv2::events::Event;
use libmpv2::{Mpv, mpv_end_file_reason};

use crate::audio::crossfade::{
    BASS_FILTER_LABEL, EFFECTS_LABEL, HIGH_PASS_LABEL, LOW_PASS_LABEL, fmt,
};
use crate::audio::mix_planner::ECHO_REPEATS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Stopped,
    Playing,
    Paused,
}

/// Fields Swift reads on the mpv event thread and writes on the main actor.
#[derive(Debug)]
struct Shared {
    play_state: State,
    current_video_id: Option<String>,
    pending_video_id: Option<String>,
    desired_playing: bool,
    /// The playing file ended with an error (Swift `stopped(isEof: false)`),
    /// until `take_failed`.
    failed: bool,
    /// The volume slider, 0...1; mpv's volume is `100 × volume`.
    volume: f64,
    /// While true, the outgoing song's end finishes the crossfade instead of
    /// stopping playback.
    is_crossfading: bool,
    /// Events for the crossfade, taken by `take_deck_events`.
    events: DeckEvents,
    /// False once mpv rejects the AutoMix bass or effect filters.
    has_bass_filter: bool,
    has_effect_filters: bool,
    has_logged_chain_failure: bool,
}

/// What happened on the decks since the crossfade last asked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeckEvents {
    /// The spare deck loaded the next song (Swift `Crossfade.deckLoaded`).
    pub spare_loaded: bool,
    /// The spare deck failed to load it (`deckFailed`).
    pub spare_failed: bool,
    /// The outgoing song ended during the mix (`outgoingEnded`).
    pub outgoing_ended: bool,
}

struct Decks {
    active: MpvDeck,
    /// Second mpv instance that plays the next song during a crossfade.
    /// Created on first use; swapped with `active` halfway through the fade.
    spare: Option<MpvDeck>,
}

/// One mpv instance. mpv handles are thread-safe, and the event thread holds a clone.
#[derive(Clone)]
pub struct MpvDeck {
    handle: Arc<Mpv>,
}

impl PartialEq for MpvDeck {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.handle, &other.handle)
    }
}

impl MpvDeck {
    /// Swift `deck(_:set:_:)`.
    pub fn set(&self, name: &str, value: &str) -> Result<(), libmpv2::Error> {
        self.handle.set_property(name, value)
    }

    /// Swift `deck(_:command:)`.
    pub fn command(&self, name: &str, args: &[&str]) -> Result<(), libmpv2::Error> {
        self.handle.command(name, args)
    }

    /// Playback position; `audio-pts` accounts for the audio output delay.
    pub fn position(&self) -> Option<f64> {
        ["audio-pts", "time-pos"]
            .iter()
            .find_map(|name| self.handle.get_property::<f64>(name).ok())
    }

    /// A number property; 0 when unset or not ready.
    fn double_property(&self, name: &str) -> f64 {
        self.handle.get_property(name).unwrap_or(0.0)
    }
}

pub struct PlayerController {
    decks: Arc<Mutex<Decks>>,
    shared: Arc<Mutex<Shared>>,
    is_running: Arc<AtomicBool>,
    event_loop_threads: Mutex<Vec<JoinHandle<()>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl PlayerController {
    /// Swift `applyPostInitProperties`: the stream CDN sees the same client as on iOS.
    pub const USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1";

    /// Swift `init` + `startMpv`. Fails when libmpv cannot start.
    pub fn new() -> Result<Self, libmpv2::Error> {
        let active = MpvDeck {
            handle: Arc::new(Self::create_handle()?),
        };
        let shared = Arc::new(Mutex::new(Shared {
            play_state: State::Stopped,
            current_video_id: None,
            pending_video_id: None,
            desired_playing: false,
            failed: false,
            volume: 1.0,
            is_crossfading: false,
            events: DeckEvents::default(),
            has_bass_filter: true,
            has_effect_filters: true,
            has_logged_chain_failure: false,
        }));
        let decks = Arc::new(Mutex::new(Decks {
            active: active.clone(),
            spare: None,
        }));
        let is_running = Arc::new(AtomicBool::new(true));
        let thread =
            Self::start_event_loop(active, decks.clone(), shared.clone(), is_running.clone());
        Ok(Self {
            decks,
            shared,
            is_running,
            event_loop_threads: Mutex::new(vec![thread]),
        })
    }

    pub fn active_deck(&self) -> MpvDeck {
        lock(&self.decks).active.clone()
    }

    pub fn current_time(&self) -> f64 {
        self.active_deck().double_property("time-pos")
    }

    pub fn duration(&self) -> f64 {
        self.active_deck().double_property("duration")
    }

    pub fn play_state(&self) -> State {
        lock(&self.shared).play_state
    }

    pub fn current_video_id(&self) -> Option<String> {
        lock(&self.shared).current_video_id.clone()
    }

    /// File path or stream URL of the playing song.
    pub fn active_path(&self) -> Option<String> {
        self.active_deck().handle.get_property("path").ok()
    }

    // MARK: - Playback

    /// Plays `url` from `start` seconds (Swift `resumeOnLoad`), or from the beginning.
    pub fn play(
        &self,
        url: &str,
        video_id: Option<&str>,
        start: Option<f64>,
    ) -> Result<(), libmpv2::Error> {
        let desired_playing = {
            let mut shared = lock(&self.shared);
            shared.failed = false;
            // ponytail: Swift keeps a paused player paused here when the user
            // turned on Persistent Play/Pause (R6 settings); always plays for now.
            shared.desired_playing = true;
            shared.pending_video_id = video_id.map(str::to_string);
            shared.desired_playing
        };
        log::info!("TRANSITION play videoId={}", video_id.unwrap_or("nil"));
        let deck = self.active_deck();
        let mpv = &deck.handle;
        mpv.set_property("pause", !desired_playing)?;
        // A cancelled crossfade can leave the deck faded or sped up.
        mpv.set_property("volume", 100.0 * self.volume())?;
        mpv.set_property("speed", 1.0)?;
        mpv.set_property("start", start.map_or("none".to_string(), fmt))?;
        self.apply_filters(&deck, None, None);
        if let Err(error) = mpv.command("loadfile", &[url, "replace"]) {
            log::error!("mpv_command loadfile failed result={error}");
            return Err(error);
        }
        if desired_playing {
            lock(&self.shared).play_state = State::Playing;
        }
        Ok(())
    }

    pub fn seek(&self, time: f64) {
        // With nothing loaded the seek fails by design.
        let is_loaded = self.play_state() != State::Stopped;
        if let Err(error) = self.active_deck().handle.set_property("time-pos", time)
            && is_loaded
        {
            log::error!("seek failed: mpv error {error}");
        }
    }

    /// Ends playback with nothing left to resume.
    pub fn stop(&self) {
        {
            let mut shared = lock(&self.shared);
            shared.pending_video_id = None;
            shared.current_video_id = None;
            shared.desired_playing = false;
            shared.play_state = State::Stopped;
        }
        let _ = self.active_deck().command("stop", &[]);
    }

    pub fn set_paused(&self, paused: bool) {
        let mut shared = lock(&self.shared);
        // After the queue ends mpv holds no file: unpausing it would play silence.
        if !paused && shared.play_state == State::Stopped && shared.pending_video_id.is_none() {
            return;
        }
        shared.desired_playing = !paused;
        let currently_paused = shared.play_state != State::Playing;
        if currently_paused == paused {
            return;
        }
        let _ = self.active_deck().handle.set_property("pause", paused);
        shared.play_state = if paused {
            State::Paused
        } else {
            State::Playing
        };
    }

    pub fn toggle_play_pause(&self) {
        self.set_paused(self.play_state() == State::Playing);
    }

    // MARK: - Crossfade decks

    /// The second instance, created and started on first use. It starts
    /// muted when the playing deck is (`WOOF_MUTE`).
    pub fn spare_deck(&self) -> Result<MpvDeck, libmpv2::Error> {
        if let Some(spare) = lock(&self.decks).spare.clone() {
            return Ok(spare);
        }
        let spare = MpvDeck {
            handle: Arc::new(Self::create_handle()?),
        };
        if let Ok(mute) = self.active_deck().handle.get_property::<bool>("mute") {
            let _ = spare.handle.set_property("mute", mute);
        }
        let thread = Self::start_event_loop(
            spare.clone(),
            self.decks.clone(),
            self.shared.clone(),
            self.is_running.clone(),
        );
        lock(&self.event_loop_threads).push(thread);
        lock(&self.decks).spare = Some(spare.clone());
        Ok(spare)
    }

    /// The spare deck once it exists.
    pub fn spare(&self) -> Option<MpvDeck> {
        lock(&self.decks).spare.clone()
    }

    /// Playback positions of two decks read together, so they can be compared.
    pub fn positions(&self, outgoing: &MpvDeck, incoming: &MpvDeck) -> Option<(f64, f64)> {
        Some((outgoing.position()?, incoming.position()?))
    }

    /// Makes the spare deck the playing one. The old deck keeps fading out
    /// as the new spare until the crossfade pauses it.
    pub fn promote_spare(&self, video_id: &str) {
        {
            let mut decks = lock(&self.decks);
            let Some(spare) = decks.spare.take() else {
                return;
            };
            let old = std::mem::replace(&mut decks.active, spare);
            decks.spare = Some(old);
        }
        let mut shared = lock(&self.shared);
        shared.current_video_id = Some(video_id.to_string());
        shared.pending_video_id = None;
    }

    /// Swift `isCrossfading`.
    pub fn set_crossfading(&self, on: bool) {
        lock(&self.shared).is_crossfading = on;
    }

    /// Whether the playing file ended with an error since the last call: the
    /// app retries it instead of moving on.
    pub fn volume(&self) -> f64 {
        lock(&self.shared).volume
    }

    /// Sets the volume slider (0...1). A running crossfade picks it up on
    /// its next step; otherwise the playing deck changes now.
    pub fn set_volume(&self, volume: f64) {
        let crossfading = {
            let mut shared = lock(&self.shared);
            shared.volume = volume.clamp(0.0, 1.0);
            shared.is_crossfading
        };
        if !crossfading {
            let _ = self
                .active_deck()
                .set("volume", &fmt(100.0 * volume.clamp(0.0, 1.0)));
        }
    }

    pub fn take_failed(&self) -> bool {
        std::mem::take(&mut lock(&self.shared).failed)
    }

    /// The deck events since the last call.
    pub fn take_deck_events(&self) -> DeckEvents {
        std::mem::take(&mut lock(&self.shared).events)
    }

    pub fn has_bass_filter(&self) -> bool {
        lock(&self.shared).has_bass_filter
    }

    pub fn has_effect_filters(&self) -> bool {
        lock(&self.shared).has_effect_filters
    }

    /// Swift `applyPlaybackSettings`: sets `deck`'s filter chain. mpv rejects
    /// the whole chain when its FFmpeg build lacks a filter, so the chain is
    /// set again without the AutoMix effects, then without the bass filter,
    /// and mixes skip what is missing.
    pub fn apply_filters(&self, deck: &MpvDeck, bass_gain: Option<f64>, echo_delay: Option<f64>) {
        let (has_bass, has_effects) = {
            let shared = lock(&self.shared);
            (shared.has_bass_filter, shared.has_effect_filters)
        };
        let tiers = [(true, true), (false, true), (true, false), (false, false)]
            .into_iter()
            .filter(|(bass, effects)| (has_bass || !bass) && (has_effects || !effects));
        let mut accepted = (false, false);
        let mut failure = None;
        for (bass, effects) in tiers {
            let chain = filter_chain(bass_gain, echo_delay, bass, effects);
            match deck.set("af", &chain) {
                Ok(()) => {
                    accepted = (bass, effects);
                    failure = None;
                    break;
                }
                Err(error) => failure = Some((chain, error)),
            }
        }
        let mut shared = lock(&self.shared);
        if shared.has_effect_filters && !accepted.1 {
            shared.has_effect_filters = false;
            log::info!("mpv rejected the AutoMix effect filters; AutoMix mixes without effects");
        }
        if shared.has_bass_filter && !accepted.0 {
            shared.has_bass_filter = false;
            log::info!("mpv has no bass filter; AutoMix mixes without the bass dip");
        }
        // Logged once: the same settings fail the same way on every song.
        if let Some((chain, error)) = failure
            && !shared.has_logged_chain_failure
        {
            shared.has_logged_chain_failure = true;
            log::error!("Failed to set af chain '{chain}': mpv error {error}");
        }
    }

    // MARK: - mpv lifecycle & event loop

    fn create_handle() -> Result<Mpv, libmpv2::Error> {
        let mpv = Mpv::with_initializer(|init| {
            Self::apply_mpv_startup_options(&init)?;
            log::info!("mpv initializing...");
            Ok(())
        })
        .inspect_err(|error| log::error!("mpv_initialize failed: {error}"))?;
        log::info!("mpv initialized successfully");
        Self::apply_post_init_properties(&mpv)?;
        Ok(mpv)
    }

    /// Swift `applyMpvStartupOptions`, minus the Apple-only `hwdec` and `audio-exclusive`.
    fn apply_mpv_startup_options(init: &libmpv2::MpvInitializer) -> Result<(), libmpv2::Error> {
        for (name, value) in [
            ("vo", "null"),
            ("vid", "no"),
            ("force-window", "no"),
            ("msg-level", "all=error"),
            ("keep-open", "no"),
            ("cache", "yes"),
            ("cache-secs", "120"),
            ("demuxer-max-bytes", "200M"),
            ("gapless-audio", "yes"),
        ] {
            init.set_option(name, value)?;
        }
        Ok(())
    }

    /// Swift also sets `tls-ca-file` to a bundled cacert.pem; desktop mpv uses the system store.
    fn apply_post_init_properties(mpv: &Mpv) -> Result<(), libmpv2::Error> {
        mpv.set_property("user-agent", Self::USER_AGENT)
    }

    fn start_event_loop(
        deck: MpvDeck,
        decks: Arc<Mutex<Decks>>,
        shared: Arc<Mutex<Shared>>,
        is_running: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        std::thread::Builder::new()
            .name("PlayerController.eventLoop".into())
            .spawn(move || Self::event_loop(&deck, &decks, &shared, &is_running))
            .expect("spawn mpv event thread")
    }

    fn event_loop(
        deck: &MpvDeck,
        decks: &Mutex<Decks>,
        shared: &Mutex<Shared>,
        is_running: &AtomicBool,
    ) {
        while is_running.load(Ordering::Acquire) {
            // Blocks until an event; Drop sends `quit`, which ends with Shutdown.
            let event = deck.handle.wait_event(-1.0);
            if matches!(event, Some(Ok(Event::Shutdown))) {
                break;
            }
            // The spare deck reports to the crossfade only.
            if lock(decks).active != *deck {
                let mut shared = lock(shared);
                match event {
                    Some(Ok(Event::FileLoaded)) => shared.events.spare_loaded = true,
                    Some(Err(error)) => {
                        log::warn!("Spare deck failed to load: {error}");
                        shared.events.spare_failed = true;
                    }
                    Some(Ok(Event::EndFile(reason))) if reason == mpv_end_file_reason::Error => {
                        log::warn!("Spare deck failed to load: end of file with an error");
                        shared.events.spare_failed = true;
                    }
                    _ => {}
                }
                continue;
            }
            match event {
                Some(Ok(Event::FileLoaded)) => Self::handle_file_loaded(deck, shared),
                Some(Ok(Event::StartFile)) => log::debug!("TRANSITION START_FILE"),
                Some(Ok(Event::EndFile(reason))) => Self::handle_end_file(shared, reason),
                // libmpv2 reports an END_FILE with an error as `Err` (we send no async commands).
                Some(Err(error)) => {
                    log::debug!("TRANSITION END_FILE error={error}");
                    Self::handle_end_file(shared, mpv_end_file_reason::Error);
                }
                _ => {}
            }
        }
    }

    fn handle_file_loaded(deck: &MpvDeck, shared: &Mutex<Shared>) {
        let mpv = &deck.handle;
        let mut shared = lock(shared);
        let mut paused = mpv.get_property::<bool>("pause").unwrap_or(true);
        if paused && shared.desired_playing {
            let _ = mpv.set_property("pause", false);
            paused = mpv.get_property::<bool>("pause").unwrap_or(true);
        }
        let actually_playing = !paused;
        log::debug!(
            "TRANSITION FILE_LOADED videoId={} playing={actually_playing}",
            shared.pending_video_id.as_deref().unwrap_or("nil")
        );
        shared.play_state = if actually_playing {
            State::Playing
        } else {
            State::Paused
        };
        // A late event of a skipped song found the next song's id, so this one finds none.
        if let Some(pending) = shared.pending_video_id.take() {
            shared.current_video_id = Some(pending);
        }
    }

    fn handle_end_file(shared: &Mutex<Shared>, reason: libmpv2::EndFileReason) {
        let is_eof = reason == mpv_end_file_reason::Eof;
        let is_error = reason == mpv_end_file_reason::Error;
        let mut shared = lock(shared);
        log::debug!("TRANSITION END_FILE reason={reason} isEof={is_eof} isError={is_error}");
        if is_error {
            shared.current_video_id = None;
            shared.failed = true;
            shared.play_state = State::Stopped;
        } else if is_eof && shared.is_crossfading {
            // The outgoing song ran out before the mix swapped decks.
            shared.events.outgoing_ended = true;
        } else if is_eof {
            // The app sees `Stopped` and advances the queue.
            shared.play_state = State::Stopped;
        } else {
            log::debug!("END_FILE non-EOF ignored for auto-advance");
        }
    }
}

impl Drop for PlayerController {
    /// Swift `deinit`: stop the event loop, then mpv is destroyed with the last handle.
    fn drop(&mut self) {
        self.is_running.store(false, Ordering::Release);
        {
            let decks = lock(&self.decks);
            for deck in std::iter::once(&decks.active).chain(&decks.spare) {
                let _ = deck.command("quit", &[]);
            }
        }
        for thread in lock(&self.event_loop_threads).drain(..) {
            let _ = thread.join();
        }
    }
}

/// Swift `filterChain` without the equalizer and loudness filters: a bass
/// filter (flat unless `bass_gain` is set), a high-pass and a low-pass
/// (bypassed), and the echo and reverb sends of `effects_graph` (silent).
/// Always present, so turning AutoMix on or off never rebuilds the filters of
/// the playing song.
pub fn filter_chain(
    bass_gain: Option<f64>,
    echo_delay: Option<f64>,
    includes_bass: bool,
    includes_effects: bool,
) -> String {
    let mut filters = Vec::new();
    if includes_bass {
        let bass = fmt(bass_gain.unwrap_or(0.0));
        filters.push(format!("@{BASS_FILTER_LABEL}:bass=f=180:g={bass}"));
    }
    if includes_effects {
        // 1 kHz is valid at any sample rate; `m=0` passes the sound through.
        filters.push(format!("@{HIGH_PASS_LABEL}:highpass=f=1000:m=0"));
        filters.push(format!("@{LOW_PASS_LABEL}:lowpass=f=1000:m=0"));
        let graph = effects_graph(echo_delay);
        filters.push(format!(
            "@{EFFECTS_LABEL}:lavfi=graph=%{}%{graph}",
            graph.len()
        ));
    }
    filters.join(",")
}

/// Swift `effectsGraph`: the song split into a dry path and two sends, mixed
/// back together: an echo repeating on the beat (when the tempo is known) and
/// a reverb-like tail of many short echoes. Both sends start at 0. `aecho`
/// delays can't change live, so the echo is built for the song's beat when it
/// loads. `duration=first` ends with the song, so the tails add no silence.
pub fn effects_graph(echo_delay: Option<f64>) -> String {
    let reverb = "[verb]volume@verbsend=0,aecho=0.7:0.7:13|19|29|37:0.5|0.4|0.3|0.2,\
        aecho=0:0.6:41|67|97|131|173|223|281|353|439|547|677|839:\
        0.6|0.5|0.42|0.35|0.29|0.24|0.2|0.17|0.14|0.12|0.1|0.08[wet]";
    let Some(echo_delay) = echo_delay else {
        return format!(
            "asplit=2[dry][verb];{reverb};[dry]volume@dry=1[d];[d][wet]amix=inputs=2:normalize=0:duration=first"
        );
    };
    let repeats = 1..=ECHO_REPEATS;
    let delays: Vec<String> = repeats
        .clone()
        .map(|n| fmt(echo_delay * 1000.0 * n as f64))
        .collect();
    let decays: Vec<String> = repeats
        .map(|n| fmt(0.5 * 0.6_f64.powi(n as i32 - 1)))
        .collect();
    format!(
        "asplit=3[dry][echo][verb];[echo]volume@echosend=0,aecho=0:1:{}:{}[rep];{reverb};\
         [dry]volume@dry=1[d];[d][rep][wet]amix=inputs=3:normalize=0:duration=first",
        delays.join("|"),
        decays.join("|")
    )
}
