// Trop/Audio/PlayerController/PlayerController.swift, the parts R3 needs:
// one mpv deck that loads, plays, pauses, seeks and stops a stream.
//
// Left for later milestones:
// - Second deck, `promoteSpare`, crossfade/AutoMix filters (`applyPlaybackSettings`,
//   `filterChain`, `effectsGraph`): R6. `spare` goes next to `active`.
// - NowPlaying, PlaybackStateService, PlaybackSpeed, lock screen and remote
//   commands (MPRIS/SMTC): R4/R6.
// - AVAudioSession, interruptions and route changes: no desktop equivalent.
// - `pendingURL` parking: the deck is created synchronously in `new`, so mpv
//   is always ready before `play`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use libmpv2::events::Event;
use libmpv2::{Mpv, mpv_end_file_reason};

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
}

/// One mpv instance. mpv handles are thread-safe, and the event thread holds a clone.
#[derive(Clone)]
pub struct MpvDeck {
    handle: Arc<Mpv>,
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
    active: MpvDeck,
    shared: Arc<Mutex<Shared>>,
    is_running: Arc<AtomicBool>,
    event_loop_thread: Option<JoinHandle<()>>,
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|e| e.into_inner())
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
        }));
        let is_running = Arc::new(AtomicBool::new(true));
        let event_loop_thread =
            Self::start_event_loop(active.clone(), shared.clone(), is_running.clone());
        Ok(Self {
            active,
            shared,
            is_running,
            event_loop_thread: Some(event_loop_thread),
        })
    }

    pub fn active_deck(&self) -> &MpvDeck {
        &self.active
    }

    pub fn current_time(&self) -> f64 {
        self.active.double_property("time-pos")
    }

    pub fn duration(&self) -> f64 {
        self.active.double_property("duration")
    }

    pub fn play_state(&self) -> State {
        lock(&self.shared).play_state
    }

    pub fn current_video_id(&self) -> Option<String> {
        lock(&self.shared).current_video_id.clone()
    }

    /// File path or stream URL of the playing song.
    pub fn active_path(&self) -> Option<String> {
        self.active.handle.get_property("path").ok()
    }

    // MARK: - Playback

    pub fn play(&self, url: &str, video_id: Option<&str>) -> Result<(), libmpv2::Error> {
        let desired_playing = {
            let mut shared = lock(&self.shared);
            // ponytail: Swift keeps a paused player paused here when the user
            // turned on Persistent Play/Pause (R6 settings); always plays for now.
            shared.desired_playing = true;
            shared.pending_video_id = video_id.map(str::to_string);
            shared.desired_playing
        };
        log::info!("TRANSITION play videoId={}", video_id.unwrap_or("nil"));
        let mpv = &self.active.handle;
        mpv.set_property("pause", !desired_playing)?;
        // A cancelled crossfade can leave the deck faded or sped up (R6).
        mpv.set_property("volume", 100.0)?;
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
        if let Err(error) = self.active.handle.set_property("time-pos", time)
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
        let _ = self.active.command("stop", &[]);
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
        let _ = self.active.handle.set_property("pause", paused);
        shared.play_state = if paused {
            State::Paused
        } else {
            State::Playing
        };
    }

    pub fn toggle_play_pause(&self) {
        self.set_paused(self.play_state() == State::Playing);
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
        shared: Arc<Mutex<Shared>>,
        is_running: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        std::thread::Builder::new()
            .name("PlayerController.eventLoop".into())
            .spawn(move || Self::event_loop(&deck, &shared, &is_running))
            .expect("spawn mpv event thread")
    }

    fn event_loop(deck: &MpvDeck, shared: &Mutex<Shared>, is_running: &AtomicBool) {
        while is_running.load(Ordering::Acquire) {
            // Blocks until an event; Drop sends `quit`, which ends with Shutdown.
            match deck.handle.wait_event(-1.0) {
                Some(Ok(Event::FileLoaded)) => Self::handle_file_loaded(deck, shared),
                Some(Ok(Event::StartFile)) => log::debug!("TRANSITION START_FILE"),
                Some(Ok(Event::EndFile(reason))) => Self::handle_end_file(shared, reason),
                // libmpv2 reports an END_FILE with an error as `Err` (we send no async commands).
                Some(Err(error)) => {
                    log::debug!("TRANSITION END_FILE error={error}");
                    Self::handle_end_file(shared, mpv_end_file_reason::Error);
                }
                Some(Ok(Event::Shutdown)) => break,
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
            shared.play_state = State::Stopped;
        } else if is_eof {
            // R4: NowPlaying advances the queue here.
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
        let _ = self.active.command("quit", &[]);
        if let Some(thread) = self.event_loop_thread.take() {
            let _ = thread.join();
        }
    }
}
