//! OS media controls: media keys and the desktop's "now playing" widget
//! (MPRIS on Linux, SMTC on Windows, Now Playing on macOS), via `playwire`.

use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use iced::futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use iced::futures::{StreamExt, stream};
use iced::{Subscription, Task, window};
use playwire::{Capabilities, Event, MediaControls, PlaybackState, PlayerConfig, Track};

use super::{Message, PlayState, Player};

/// The controls, once attached to the window; `None` if they failed to start.
static CONTROLS: Mutex<Option<MediaControls>> = Mutex::new(None);
/// Whether the player was playing, and where, at the last `publish`.
static NOW: Mutex<(bool, f64)> = Mutex::new((false, 0.0));
/// OS events, from playwire's thread to `events`, which takes the receiver.
type Channel = (
    UnboundedSender<Event>,
    Mutex<Option<UnboundedReceiver<Event>>>,
);
static CHANNEL: LazyLock<Channel> = LazyLock::new(|| {
    let (sender, receiver) = unbounded();
    (sender, Mutex::new(Some(receiver)))
});

/// Registers the controls. They need the window: SMTC attaches to its HWND.
pub fn attach() -> Task<Message> {
    window::latest()
        .and_then(|id| window::run(id, start))
        .discard()
}

fn start(window: &dyn window::Window) {
    let config = PlayerConfig::new("WoofMusic").desktop_entry(super::APP_ID);
    #[cfg(windows)]
    let config = {
        use window::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        match window.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(handle)) => config.hwnd(handle.hwnd.get() as u64),
            _ => config,
        }
    };
    #[cfg(not(windows))]
    let _ = window;
    let sender = CHANNEL.0.clone();
    match MediaControls::new(config, move |event| {
        let _ = sender.unbounded_send(event);
    }) {
        Ok(controls) => *CONTROLS.lock().unwrap() = Some(controls),
        Err(error) => eprintln!("Media controls are off: {error}"),
    }
}

/// Media keys and the OS widget, as player messages.
pub fn events() -> Subscription<Message> {
    Subscription::run(|| {
        let receiver = CHANNEL.1.lock().unwrap().take();
        stream::iter(receiver)
            .flatten()
            .flat_map(|event| stream::iter(messages(event, *NOW.lock().unwrap())))
    })
}

fn messages(event: Event, (playing, position): (bool, f64)) -> Vec<Message> {
    let seek = |to: f64| vec![Message::Seeking(to.max(0.0)), Message::Seek];
    match event {
        Event::PlayPause => vec![Message::TogglePlay],
        Event::Play if !playing => vec![Message::TogglePlay],
        Event::Pause | Event::Stop if playing => vec![Message::TogglePlay],
        Event::Next => vec![Message::Next],
        Event::Previous => vec![Message::Previous],
        Event::SeekTo(to) => seek(to.as_secs_f64()),
        Event::SeekBy(by) => seek(position + by),
        _ => Vec::new(),
    }
}

/// Tells the OS what plays. Cheap to call often: playwire only sends changes.
pub fn publish(player: &Player) {
    let playing = player.state == PlayState::Playing;
    *NOW.lock().unwrap() = (playing, player.position);
    let mut controls = CONTROLS.lock().unwrap();
    let Some(controls) = controls.as_mut() else {
        return;
    };
    let song = player.song();
    let duration = match player.duration {
        d if d > 0.0 => d,
        _ => song.map_or(0.0, |song| song.duration as f64),
    };
    let state = PlaybackState {
        track: song.map(|song| Track {
            id: song.video_id.clone(),
            title: song.title.clone(),
            artists: song.artists.iter().map(|a| a.name.clone()).collect(),
            album: song.album.clone().unwrap_or_default(),
            artwork_url: song.thumbnail_url.clone().unwrap_or_default(),
            url: format!("https://music.youtube.com/watch?v={}", song.video_id),
        }),
        playing,
        position: Duration::from_secs_f64(player.position.max(0.0)),
        duration: (duration > 0.0).then(|| Duration::from_secs_f64(duration)),
        volume: 1.0,
        capabilities: Capabilities {
            can_go_next: player.has_next(),
            can_go_previous: song.is_some(),
            can_seek: duration > 0.0,
        },
        ..PlaybackState::default()
    };
    // ponytail: errors are transient (a D-Bus hiccup) and the next Tick retries, so they are dropped.
    let _ = controls.set_state(&state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_and_pause_only_toggle_when_needed() {
        assert!(messages(Event::Play, (true, 0.0)).is_empty());
        assert!(matches!(
            messages(Event::Play, (false, 0.0))[..],
            [Message::TogglePlay]
        ));
        assert!(matches!(
            messages(Event::Pause, (true, 0.0))[..],
            [Message::TogglePlay]
        ));
        assert!(matches!(
            messages(Event::SeekBy(-10.0), (true, 4.0))[..],
            [Message::Seeking(0.0), Message::Seek]
        ));
    }
}
