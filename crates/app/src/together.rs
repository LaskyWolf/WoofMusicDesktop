//! Listen Together (`Trop/ListenTogether/ListenTogether.swift`,
//! `ListenTogetherView.swift`): several people hear the same song at the same
//! moment. Each computer streams the music itself; a metroserver relays only
//! what the host does (song, play, pause, seek, queue). Rooms are compatible
//! with Metrolist's, on the same server.
//!
//! - Host: every half second the player is compared with what the room was
//!   last told, and the difference is sent.
//! - Guest: the room's messages drive the player, and the server's drift
//!   reports pull it back when it runs ahead or behind.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iced::font::Weight;
use iced::widget::{
    Space, button, column, container, mouse_area, opaque, row, scrollable, stack, svg, text,
    text_input,
};
use iced::{
    Alignment, Background, Border, Color, Element, Font, Length, Shadow, Subscription, Task, Vector,
};
use serde_json::Value;
use swiftytube::networking::inner_tube_client::BrowseArgs;
use trop::listen_together::protocol::{self as p, kind};
use trop::listen_together::{self as lt, Event, Outbox};
use trop::networking::models::yt_item::{SongItem, YTArtist};
use trop::settings::settings_store::SettingsStore;

use crate::services::Services;
use crate::{Message, theme, ui};

const FEATURES: [&str; 7] = [
    "drift_report",
    "origin_tag",
    "server_restarting",
    "suggestion_ack",
    "replace_queue",
    "host_sequence",
    "track_revision",
];
/// The most queue entries sent to the room; metroserver caps it too.
const MAX_QUEUE: usize = 100;
/// A position jump bigger than this, between two ticks, is a seek.
const SEEK_THRESHOLD: f64 = 2.0;
/// Guests further than this from the room jump back into place.
const MAX_DRIFT_MS: i64 = 400;
/// metroserver users have only a name (at most 50 bytes), so WoofMusic adds
/// its YouTube channel ID to it: "Name \u{2063}UC…". Other WoofMusic apps
/// show that channel's picture; Metrolist shows the name with the code.
const CHANNEL_TAG: &str = " \u{2063}";
const MAX_NAME_BYTES: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Idle,
    Connecting,
    WaitingForApproval,
    InRoom,
    /// The connection dropped; trying to get back into the same room.
    Reconnecting,
}

#[derive(Debug, Clone)]
pub enum Request {
    Create,
    Join(String),
    Reconnect(String),
}

/// What the player is doing, read for each tick (Swift `NowPlaying`).
#[derive(Default)]
pub struct Playback {
    pub song: Option<SongItem>,
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub loading: bool,
    /// The songs after the current one, in play order.
    pub upcoming: Vec<SongItem>,
    /// The song this app's own crossfade is fading into.
    pub mixing_into: Option<String>,
}

impl Playback {
    fn video_id(&self) -> Option<&str> {
        self.song.as_ref().map(|song| song.video_id.as_str())
    }
}

/// What the room asks of the player; the app does it.
#[derive(Debug, Clone)]
pub enum Effect {
    /// Plays the first song (paused until the room says play), then the rest.
    Load(Vec<SongItem>),
    /// The queue from the current song on, without reloading it.
    SetQueue(Vec<SongItem>),
    Seek(f64),
    Paused(bool),
    EnqueueNext(Box<SongItem>),
}

/// What `update` reads from the app, copied so the room can change meanwhile.
pub struct Cx {
    pub services: Arc<Services>,
    pub server_url: String,
    pub username: String,
    /// The signed-in account's name.
    pub account: Option<String>,
    pub np: Playback,
}

impl Cx {
    /// The name the room sees: the one typed, else the account's.
    fn username(&self) -> String {
        let name = self.username.trim();
        if !name.is_empty() {
            return name.to_owned();
        }
        match &self.account {
            Some(name) if !name.is_empty() && name != "Guest" => name.clone(),
            _ => "WoofMusic listener".to_owned(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Open,
    Close,
    Code(String),
    /// Settings the app saves: the name and the server.
    Name(String),
    Server(String),
    UseDefaultServer,
    Create,
    Join,
    /// The account's channel arrived (or not); the request can start.
    Begin(Option<String>, Request),
    Leave,
    Approve(String),
    Reject(String),
    ApproveSuggestion(String),
    RejectSuggestion(String),
    Kick(String),
    MakeHost(String),
    Suggest(Box<SongItem>),
    CopyCode,
    /// An event of connection number `.0`.
    Event(u64, Event),
    Tick,
    /// Reconnect attempt number `.0` is due.
    Retry(u64),
    /// A member's channel picture.
    Picture(String, Option<String>),
}

pub struct Room {
    pub open: bool,
    pub status: Status,
    code: String,
    room_code: Option<String>,
    my_user_id: Option<String>,
    host_id: Option<String>,
    users: Vec<p::User>,
    /// Host only: people waiting to be let in (ID, name).
    join_requests: Vec<p::Pair>,
    /// Host only: songs guests asked for.
    suggestions: Vec<p::Suggestion>,
    /// The last thing worth telling the user: an error, a refusal, a kick.
    notice: Option<String>,
    copied: bool,

    outbox: Option<Outbox>,
    /// Counts connections, so events of a closed one are dropped.
    connection: u64,
    /// Counts reconnect timers, so a cancelled one does nothing.
    retry: u64,
    pending: Option<Request>,
    session_token: Option<String>,
    reconnect_attempts: u32,
    leaving: bool,

    /// Server clock minus this computer's, in ms, from the pings.
    clock_offset: i64,
    ping_sequence: u64,
    last_ping: Option<Instant>,

    // Host: what the room was last told.
    host_sequence: u64,
    sent_track_id: Option<String>,
    sent_playing: bool,
    sent_queue_ids: Vec<String>,
    last_position: Option<(f64, Instant)>,

    // Guest: the track the room is on, until this computer has loaded it.
    awaiting_buffer: Option<(String, u64)>,
    last_drift_fix: Option<Instant>,
    /// The room's song change, held back while this app's own crossfade
    /// fades into that song: loading it would cut the mix.
    pending_load: Option<(p::Track, Vec<p::Track>, u64)>,

    /// Members' pictures by YouTube channel ID; `None` while asked.
    pictures: HashMap<String, Option<String>>,
    my_channel_id: Option<String>,
}

impl Default for Room {
    fn default() -> Self {
        let mut room = Self {
            open: false,
            status: Status::Idle,
            code: String::new(),
            room_code: None,
            my_user_id: None,
            host_id: None,
            users: Vec::new(),
            join_requests: Vec::new(),
            suggestions: Vec::new(),
            notice: None,
            copied: false,
            outbox: None,
            connection: 0,
            retry: 0,
            pending: None,
            session_token: None,
            reconnect_attempts: 0,
            leaving: false,
            clock_offset: 0,
            ping_sequence: 0,
            last_ping: None,
            host_sequence: 0,
            sent_track_id: None,
            sent_playing: false,
            sent_queue_ids: Vec::new(),
            last_position: None,
            awaiting_buffer: None,
            last_drift_fix: None,
            pending_load: None,
            pictures: HashMap::new(),
            my_channel_id: None,
        };
        room.dev_demo();
        room
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// The name to show, and the YouTube channel a WoofMusic member added to it.
pub fn split(username: &str) -> (&str, Option<&str>) {
    match username.rsplit_once(CHANNEL_TAG) {
        Some((name, id)) if id.len() == 24 && id.starts_with("UC") => (name, Some(id)),
        _ => (username, None),
    }
}

/// `name` with the channel tag, cut so the whole fits metroserver's 50 bytes.
fn tagged(name: &str, channel: Option<&str>) -> String {
    let Some(channel) = channel else {
        return name.to_owned();
    };
    let suffix = format!("{CHANNEL_TAG}{channel}");
    let mut name = name.to_owned();
    while !name.is_empty() && name.len() + suffix.len() > MAX_NAME_BYTES {
        name.pop();
    }
    name + &suffix
}

fn track_for(song: &SongItem, duration: f64) -> p::Track {
    let seconds = if duration > 0.0 {
        duration
    } else {
        song.duration as f64
    };
    p::Track {
        id: song.video_id.clone(),
        title: song.title.clone(),
        artist: ui::artists(&song.artists),
        album: song.album.clone().unwrap_or_default(),
        duration: (seconds * 1000.0) as i64,
        thumbnail: song.thumbnail_url.clone().unwrap_or_default(),
        suggested_by: String::new(),
    }
}

fn song_for(track: &p::Track) -> SongItem {
    SongItem {
        video_id: track.id.clone(),
        title: track.title.clone(),
        artists: if track.artist.is_empty() {
            Vec::new()
        } else {
            vec![YTArtist {
                name: track.artist.clone(),
                id: None,
            }]
        },
        album: (!track.album.is_empty()).then(|| track.album.clone()),
        duration: track.duration / 1000,
        thumbnail_url: (!track.thumbnail.is_empty()).then(|| track.thumbnail.clone()),
        ..SongItem::default()
    }
}

/// Every value stored under `key` anywhere in a JSON tree, outermost first.
fn values<'a>(key: &str, json: &'a Value, found: &mut Vec<&'a Value>) {
    match json {
        Value::Object(map) => {
            if let Some(value) = map.get(key) {
                found.push(value);
            }
            for value in map.values() {
                values(key, value, found);
            }
        }
        Value::Array(list) => list.iter().for_each(|value| values(key, value, found)),
        _ => {}
    }
}

/// This account's channel, from the "Your channel" link of the account menu.
async fn channel_id(services: Arc<Services>) -> Option<String> {
    let menu = services.client.account_menu(None, None, None).await.ok()?;
    let menu = Value::Object(menu.into_iter().collect());
    let mut found = Vec::new();
    values("browseId", &menu, &mut found);
    found
        .into_iter()
        .filter_map(Value::as_str)
        .find(|id| id.len() == 24 && id.starts_with("UC"))
        .map(str::to_owned)
}

/// The channel page's avatar (`foregroundThumbnail`), else its header image.
async fn channel_picture(services: Arc<Services>, channel: String) -> Option<String> {
    let page = services
        .client
        .browse(BrowseArgs {
            browse_id: Some(channel),
            ..BrowseArgs::default()
        })
        .await
        .ok()?;
    let header = page.get("header")?;
    let mut found = Vec::new();
    values("foregroundThumbnail", header, &mut found);
    if found.is_empty() {
        values("thumbnail", header, &mut found);
    }
    let mut thumbnails = Vec::new();
    values("thumbnails", found.first()?, &mut thumbnails);
    thumbnails
        .first()?
        .as_array()?
        .last()?
        .get("url")?
        .as_str()
        .map(str::to_owned)
}

impl Room {
    pub fn is_in_room(&self) -> bool {
        self.room_code.is_some()
    }

    pub fn is_host(&self) -> bool {
        self.is_in_room() && self.host_id.is_some() && self.host_id == self.my_user_id
    }

    /// While a guest, the room drives playback and AutoMix stays off.
    pub fn is_guest(&self) -> bool {
        self.is_in_room() && !self.is_host()
    }

    pub fn room_code(&self) -> Option<&str> {
        self.room_code.as_deref()
    }

    /// A guest's player loads the room's song paused: the room says when to play.
    pub fn holds(&self, video_id: &str) -> bool {
        self.awaiting_buffer
            .as_ref()
            .is_some_and(|(id, _)| id == video_id)
    }

    fn send(&self, kind: &str, payload: &[u8]) {
        if let Some(outbox) = &self.outbox {
            outbox.send(kind, payload);
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if self.outbox.is_none() {
            return Subscription::none();
        }
        iced::time::every(Duration::from_millis(500)).map(|_| Message::Together(Msg::Tick))
    }

    /// `Name`, `Server` and `UseDefaultServer` are the app's: they change settings.
    pub fn update(&mut self, message: Msg, cx: &Cx) -> (Task<Message>, Vec<Effect>) {
        let mut effects = Vec::new();
        let task = match message {
            Msg::Open => {
                self.open = true;
                self.copied = false;
                Task::none()
            }
            Msg::Close => {
                self.open = false;
                Task::none()
            }
            Msg::Code(code) => {
                self.code = code;
                Task::none()
            }
            Msg::Name(_) | Msg::Server(_) | Msg::UseDefaultServer => Task::none(),
            Msg::Create => self.begin(Request::Create, cx),
            Msg::Join => {
                let code = self.code.trim().to_uppercase();
                if code.is_empty() {
                    return (Task::none(), effects);
                }
                self.begin(Request::Join(code), cx)
            }
            Msg::Begin(channel, request) => {
                if channel.is_some() {
                    self.my_channel_id = channel;
                }
                self.start(request, cx)
            }
            Msg::Leave => {
                self.leaving = true;
                if self.is_in_room() {
                    self.send(kind::LEAVE_ROOM, &[]);
                }
                self.shut_down();
                self.reset_room();
                self.status = Status::Idle;
                Task::none()
            }
            Msg::Approve(id) => {
                self.send(kind::APPROVE_JOIN, &p::two_strings(&id, ""));
                self.join_requests.retain(|r| r.0 != id);
                Task::none()
            }
            Msg::Reject(id) => {
                self.send(kind::REJECT_JOIN, &p::two_strings(&id, ""));
                self.join_requests.retain(|r| r.0 != id);
                Task::none()
            }
            Msg::ApproveSuggestion(id) => {
                self.send(kind::APPROVE_SUGGESTION, &p::two_strings(&id, ""));
                self.suggestions.retain(|s| s.id != id);
                Task::none()
            }
            Msg::RejectSuggestion(id) => {
                self.send(kind::REJECT_SUGGESTION, &p::two_strings(&id, ""));
                self.suggestions.retain(|s| s.id != id);
                Task::none()
            }
            Msg::Kick(id) => {
                self.send(kind::KICK_USER, &p::two_strings(&id, ""));
                Task::none()
            }
            Msg::MakeHost(id) => {
                self.send(kind::TRANSFER_HOST, &p::two_strings(&id, ""));
                Task::none()
            }
            Msg::Suggest(song) => {
                self.send(
                    kind::SUGGEST_TRACK,
                    &p::suggest_track(&track_for(&song, 0.0)),
                );
                Task::none()
            }
            Msg::CopyCode => {
                self.copied = true;
                match &self.room_code {
                    Some(code) => iced::clipboard::write(format!(
                        "Listen with me on WoofMusic or Metrolist. Room code: {code}"
                    )),
                    None => Task::none(),
                }
            }
            Msg::Event(connection, _) if connection != self.connection => Task::none(),
            Msg::Event(_, Event::Message(kind, payload)) => {
                match self.handle(&kind, &payload, cx, &mut effects) {
                    Ok(task) => task,
                    Err(error) => {
                        log::warn!("Listen Together: unreadable {kind}: {error}");
                        Task::none()
                    }
                }
            }
            Msg::Event(_, Event::Closed(reason)) => {
                log::info!("Listen Together: connection closed: {reason}");
                self.outbox = None;
                if self.leaving {
                    return (Task::none(), effects);
                }
                match self.session_token.clone() {
                    Some(token) if self.is_in_room() => self.schedule_reconnect(token, None),
                    _ => {
                        self.notice = Some(if self.status == Status::Connecting {
                            "Couldn't reach the Listen Together server.".to_owned()
                        } else {
                            "Disconnected from the room.".to_owned()
                        });
                        self.reset_room();
                        self.status = Status::Idle;
                        Task::none()
                    }
                }
            }
            Msg::Tick => {
                self.tick(&cx.np, &mut effects);
                Task::none()
            }
            Msg::Retry(attempt) => match self.session_token.clone() {
                Some(token) if attempt == self.retry && self.status == Status::Reconnecting => {
                    self.start(Request::Reconnect(token), cx)
                }
                _ => Task::none(),
            },
            Msg::Picture(channel, url) => {
                let fetch = url.clone().map_or(Task::none(), |url| {
                    Task::done(Message::FetchCovers(vec![url]))
                });
                self.pictures.insert(channel, url);
                fetch
            }
        };
        (task, effects)
    }

    /// Asks for the account's channel first, so the name carries it.
    fn begin(&mut self, request: Request, cx: &Cx) -> Task<Message> {
        if self.my_channel_id.is_some() || cx.account.is_none() {
            return Task::done(Message::Together(Msg::Begin(None, request)));
        }
        self.status = Status::Connecting;
        Task::perform(channel_id(cx.services.clone()), move |id| {
            Message::Together(Msg::Begin(id, request.clone()))
        })
    }

    fn start(&mut self, request: Request, cx: &Cx) -> Task<Message> {
        self.leaving = false;
        self.notice = None;
        self.shut_down();
        if matches!(request, Request::Reconnect(_)) {
            self.status = Status::Reconnecting;
        } else {
            self.reset_room();
            self.status = Status::Connecting;
        }
        let address = cx.server_url.trim();
        let address = if address.is_empty() {
            lt::DEFAULT_SERVER_URL
        } else {
            address
        };
        if !(address.starts_with("ws://") || address.starts_with("wss://")) {
            self.notice = Some("The server address must start with wss:// or ws://.".to_owned());
            self.status = Status::Idle;
            return Task::none();
        }
        self.pending = Some(request);
        let version = env!("CARGO_PKG_VERSION");
        let (outbox, events) = lt::connect(
            address.to_owned(),
            format!("WoofMusic/{version} ({})", std::env::consts::OS),
        );
        outbox.send(
            kind::CLIENT_CAPABILITIES,
            &p::client_capabilities(&format!("WoofMusic {version}"), &FEATURES),
        );
        self.outbox = Some(outbox);
        let connection = self.connection;
        Task::run(events, move |event| {
            Message::Together(Msg::Event(connection, event))
        })
    }

    /// Drops the socket; its late events are ignored.
    fn shut_down(&mut self) {
        self.outbox = None;
        self.connection += 1;
        self.retry += 1;
    }

    fn reset_room(&mut self) {
        self.room_code = None;
        self.my_user_id = None;
        self.host_id = None;
        self.users.clear();
        self.join_requests.clear();
        self.suggestions.clear();
        self.session_token = None;
        self.reconnect_attempts = 0;
        self.sent_track_id = None;
        self.sent_queue_ids.clear();
        self.last_position = None;
        self.awaiting_buffer = None;
        self.pending_load = None;
    }

    /// Ends the room here, without telling the server (it already knows).
    fn leave_quietly(&mut self) {
        self.leaving = true;
        self.shut_down();
        self.reset_room();
        self.status = Status::Idle;
    }

    /// Tries again with a growing pause, up to about 15 minutes: how long
    /// the server keeps a place in the room.
    fn schedule_reconnect(&mut self, token: String, after: Option<f64>) -> Task<Message> {
        self.status = Status::Reconnecting;
        self.reconnect_attempts += 1;
        if self.reconnect_attempts > 30 {
            self.notice = Some("Lost the connection to the room.".to_owned());
            self.reset_room();
            self.status = Status::Idle;
            return Task::none();
        }
        self.session_token = Some(token);
        let delay =
            after.unwrap_or_else(|| 2f64.powi(self.reconnect_attempts as i32 - 1).min(30.0));
        self.retry += 1;
        let attempt = self.retry;
        Task::future(tokio::time::sleep(Duration::from_secs_f64(delay)))
            .map(move |()| Message::Together(Msg::Retry(attempt)))
    }

    // MARK: - Messages

    fn handle(
        &mut self,
        kind: &str,
        payload: &[u8],
        cx: &Cx,
        effects: &mut Vec<Effect>,
    ) -> Result<Task<Message>, p::Malformed> {
        match kind {
            kind::SERVER_CAPABILITIES => {
                let name = tagged(&cx.username(), self.my_channel_id.as_deref());
                match self.pending.take() {
                    Some(Request::Create) => self.send(kind::CREATE_ROOM, &p::one_string(&name)),
                    Some(Request::Join(code)) => {
                        self.send(kind::JOIN_ROOM, &p::two_strings(&code, &name));
                        self.status = Status::WaitingForApproval;
                    }
                    Some(Request::Reconnect(token)) => {
                        self.send(kind::RECONNECT, &p::one_string(&token));
                    }
                    None => {}
                }
            }
            kind::ROOM_CREATED | kind::JOIN_APPROVED | kind::RECONNECTED => {
                return Ok(self.enter(p::Joined::decode(kind, payload)?, cx, effects));
            }
            kind::JOIN_REJECTED => {
                let reason = p::Pair::decode(payload)?.0;
                self.notice = Some(if reason.is_empty() {
                    "The host didn't let you in.".to_owned()
                } else {
                    format!("The host didn't let you in: {reason}")
                });
                self.leave_quietly();
            }
            kind::KICKED => {
                let reason = p::Pair::decode(payload)?.0;
                self.notice = Some(if reason.is_empty() {
                    "The host removed you from the room.".to_owned()
                } else {
                    format!("Removed from the room: {reason}")
                });
                self.leave_quietly();
            }
            kind::JOIN_REQUEST => {
                let request = p::Pair::decode(payload)?;
                self.join_requests.retain(|r| r.0 != request.0);
                let picture = self.ask_picture(&request.1, cx);
                self.join_requests.push(request);
                return Ok(picture);
            }
            kind::USER_JOINED => {
                let user = p::Pair::decode(payload)?;
                self.users.retain(|u| u.user_id != user.0);
                self.join_requests.retain(|r| r.0 != user.0);
                let picture = self.ask_picture(&user.1, cx);
                self.users.push(p::User {
                    user_id: user.0,
                    username: user.1,
                    is_host: false,
                    is_connected: true,
                });
                return Ok(picture);
            }
            kind::USER_LEFT => {
                let user = p::Pair::decode(payload)?;
                self.users.retain(|u| u.user_id != user.0);
            }
            kind::USER_DISCONNECTED | kind::USER_RECONNECTED => {
                let user = p::Pair::decode(payload)?;
                if let Some(member) = self.users.iter_mut().find(|u| u.user_id == user.0) {
                    member.is_connected = kind == kind::USER_RECONNECTED;
                }
            }
            kind::HOST_CHANGED => {
                let host = p::Pair::decode(payload)?.0;
                self.set_host(host, &cx.np);
            }
            kind::SYNC_PLAYBACK => {
                let playback = p::Playback::decode(payload)?;
                if self.is_host() {
                    // The host's own actions come back too; only a suggestion
                    // the host approved needs doing here: it plays next.
                    if playback.action == "queue_add"
                        && Some(&playback.origin_user_id) != self.my_user_id.as_ref()
                        && let Some(track) = &playback.track
                    {
                        effects.push(Effect::EnqueueNext(Box::new(song_for(track))));
                    }
                } else {
                    self.guest_apply(&playback, &cx.np, effects);
                }
            }
            kind::SYNC_STATE => {
                let state = p::SyncState::decode(payload)?;
                if self.is_guest() {
                    self.guest_load(state.current_track, state.queue, 0, &cx.np, effects);
                }
            }
            kind::SUGGESTION_RECEIVED => {
                let suggestion = p::Suggestion::decode(kind, payload)?;
                let picture = self.ask_picture(&suggestion.from_username, cx);
                self.suggestions.push(suggestion);
                return Ok(picture);
            }
            kind::SUGGESTION_APPROVED => {
                let track = p::Suggestion::decode(kind, payload)?.track;
                if !self.is_host() {
                    self.notice = Some(format!("The host added {}.", track.title));
                }
            }
            kind::SUGGESTION_REJECTED => {
                if !self.is_host() {
                    self.notice = Some("The host passed on your suggestion.".to_owned());
                }
            }
            kind::SUGGESTION_SUBMITTED => {
                self.notice = Some("Suggestion sent to the host.".to_owned());
            }
            kind::PONG => self.handle_pong(&p::Pong::decode(payload)?, &cx.np, effects),
            kind::SERVER_RESTARTING => {
                let eta = p::restart_eta(payload)? as f64 / 1000.0;
                if let Some(token) = self.session_token.clone() {
                    self.leaving = true;
                    self.shut_down();
                    self.leaving = false;
                    let jitter = 2.0 * noise();
                    return Ok(self.schedule_reconnect(token, Some(eta.max(1.0) + jitter)));
                }
            }
            kind::ERROR => {
                let p::Pair(code, message) = p::Pair::decode(payload)?;
                log::warn!("Listen Together error {code}: {message}");
                // Races the next message repairs on its own.
                if ["stale_action", "stale_track", "sync_rate_limited"].contains(&code.as_str()) {
                    return Ok(Task::none());
                }
                if self.status == Status::Reconnecting {
                    self.notice = Some("The room has closed.".to_owned());
                    self.leave_quietly();
                    return Ok(Task::none());
                }
                self.notice = Some(if message.is_empty() { code } else { message });
                if matches!(self.status, Status::WaitingForApproval | Status::Connecting) {
                    self.leave_quietly();
                }
            }
            _ => {}
        }
        Ok(Task::none())
    }

    fn enter(&mut self, joined: p::Joined, cx: &Cx, effects: &mut Vec<Effect>) -> Task<Message> {
        self.room_code = Some(joined.room_code);
        self.my_user_id = Some(joined.user_id.clone());
        if !joined.session_token.is_empty() {
            self.session_token = Some(joined.session_token);
        }
        self.reconnect_attempts = 0;
        self.status = Status::InRoom;
        match joined.state {
            Some(state) => {
                self.users = state.users;
                self.set_host(state.host_id, &cx.np);
                if self.is_guest() {
                    self.guest_load(state.current_track, state.queue, 0, &cx.np, effects);
                }
            }
            None => {
                // A new room: just this app, as its host.
                self.users = vec![p::User {
                    user_id: joined.user_id.clone(),
                    username: cx.username(),
                    is_host: true,
                    is_connected: true,
                }];
                self.set_host(joined.user_id, &cx.np);
            }
        }
        let names: Vec<String> = self.users.iter().map(|u| u.username.clone()).collect();
        Task::batch(names.iter().map(|name| self.ask_picture(name, cx)))
    }

    fn set_host(&mut self, id: String, np: &Playback) {
        let was_host = self.is_host();
        for user in &mut self.users {
            user.is_host = user.user_id == id;
        }
        self.host_id = Some(id);
        if !self.is_host() || was_host {
            return;
        }
        // Just became host: the room is already on this song, so only what
        // changes from here on is sent. A new room hears the song from scratch.
        let room_has_song = self.users.len() > 1;
        self.sent_track_id = room_has_song
            .then(|| np.video_id().map(str::to_owned))
            .flatten();
        self.sent_playing = room_has_song && np.playing;
        self.sent_queue_ids = if room_has_song {
            np.upcoming
                .iter()
                .take(MAX_QUEUE)
                .map(|s| s.video_id.clone())
                .collect()
        } else {
            Vec::new()
        };
        self.last_position = None;
        self.awaiting_buffer = None;
    }

    /// A member's channel picture, fetched once.
    fn ask_picture(&mut self, username: &str, cx: &Cx) -> Task<Message> {
        let Some(channel) = split(username).1.map(str::to_owned) else {
            return Task::none();
        };
        if self.pictures.contains_key(&channel) {
            return Task::none();
        }
        self.pictures.insert(channel.clone(), None);
        Task::perform(
            channel_picture(cx.services.clone(), channel.clone()),
            move |url| Message::Together(Msg::Picture(channel.clone(), url)),
        )
    }

    fn picture<'a>(
        &'a self,
        user_id: &str,
        username: &str,
        cx_photo: Option<&'a str>,
    ) -> Option<&'a str> {
        if self.my_user_id.as_deref() == Some(user_id) {
            return cx_photo;
        }
        self.pictures.get(split(username).1?)?.as_deref()
    }

    // MARK: - Clock

    fn server_now(&self) -> i64 {
        now_ms() + self.clock_offset
    }

    fn send_ping(&mut self, np: &Playback) {
        self.last_ping = Some(Instant::now());
        self.ping_sequence += 1;
        // A guest playing a loaded song reports where it is, for the drift check.
        let drift = (self.is_guest()
            && self.awaiting_buffer.is_none()
            && self.pending_load.is_none()
            && np.playing
            && !np.loading)
            .then(|| np.video_id())
            .flatten()
            .map(|id| (id, (np.position * 1000.0) as i64, self.server_now()));
        self.send(kind::PING, &p::ping(now_ms(), self.ping_sequence, drift));
    }

    fn handle_pong(&mut self, pong: &p::Pong, np: &Playback, effects: &mut Vec<Effect>) {
        // Assumes the trip there and back take as long: the usual NTP guess.
        let received = now_ms();
        self.clock_offset = ((pong.server_receive_time - pong.client_time)
            + (pong.server_send_time - received))
            / 2;
        let recent = self
            .last_drift_fix
            .is_some_and(|at| at.elapsed() < Duration::from_secs(4));
        if !self.is_guest() || !pong.drift_valid || pong.drift_ms.abs() <= MAX_DRIFT_MS || recent {
            return;
        }
        if !np.playing || np.loading || np.mixing_into.is_some() {
            return;
        }
        self.last_drift_fix = Some(Instant::now());
        effects.push(Effect::Seek(
            (np.position - pong.drift_ms as f64 / 1000.0).max(0.0),
        ));
    }

    // MARK: - Tick

    fn tick(&mut self, np: &Playback, effects: &mut Vec<Effect>) {
        if self.outbox.is_none() {
            return;
        }
        if self
            .last_ping
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(5))
        {
            self.send_ping(np);
        }
        if self.status != Status::InRoom {
            return;
        }
        if self.is_host() {
            self.host_tick(np);
        } else {
            self.guest_tick(np, effects);
        }
    }

    fn action(&mut self, action: &str) -> p::Playback {
        self.host_sequence += 1;
        p::Playback::new(action, self.host_sequence)
    }

    fn host_tick(&mut self, np: &Playback) {
        let Some(song) = &np.song else {
            return;
        };
        let upcoming: Vec<&SongItem> = np.upcoming.iter().take(MAX_QUEUE).collect();
        let queue_ids: Vec<String> = upcoming.iter().map(|s| s.video_id.clone()).collect();
        if self.sent_track_id.as_ref() != Some(&song.video_id) {
            let mut change = self.action("change_track");
            change.track = Some(track_for(song, np.duration));
            change.queue = upcoming.iter().map(|s| track_for(s, 0.0)).collect();
            change.replace_queue = true;
            self.send(kind::PLAYBACK_ACTION, &change.encode());
            self.sent_track_id = Some(song.video_id.clone());
            // change_track leaves the room paused at 0; play follows once loaded.
            self.sent_playing = false;
            self.sent_queue_ids = queue_ids;
            self.last_position = None;
            return;
        }
        if queue_ids != self.sent_queue_ids {
            let mut sync = self.action("sync_queue");
            sync.queue = upcoming.iter().map(|s| track_for(s, 0.0)).collect();
            self.send(kind::PLAYBACK_ACTION, &sync.encode());
            self.sent_queue_ids = queue_ids;
        }
        if np.loading {
            return;
        }
        let position = np.position;
        let video_id = song.video_id.clone();
        if np.playing != self.sent_playing {
            self.send_transport(
                if np.playing { "play" } else { "pause" },
                video_id,
                position,
            );
            self.sent_playing = np.playing;
        } else if let Some((seconds, at)) = self.last_position {
            let expected = seconds
                + if np.playing {
                    at.elapsed().as_secs_f64()
                } else {
                    0.0
                };
            if (position - expected).abs() > SEEK_THRESHOLD {
                self.send_transport("seek", video_id, position);
            }
        }
        self.last_position = Some((position, Instant::now()));
    }

    fn send_transport(&mut self, action: &str, track_id: String, position: f64) {
        let mut playback = self.action(action);
        playback.track_id = track_id;
        playback.position = (position.max(0.0) * 1000.0) as i64;
        playback.captured_at = self.server_now();
        self.send(kind::PLAYBACK_ACTION, &playback.encode());
    }

    // MARK: - Guest

    fn guest_apply(&mut self, playback: &p::Playback, np: &Playback, effects: &mut Vec<Effect>) {
        // Where the host is now: the position plus the time since the server sent it.
        let elapsed = (self.server_now() - playback.server_time).max(0) as f64 / 1000.0;
        let position = playback.position as f64 / 1000.0;
        let ready = self.awaiting_buffer.is_none()
            && self.pending_load.is_none()
            && np.video_id() == Some(playback.track_id.as_str());
        match playback.action.as_str() {
            "change_track" => self.guest_load(
                playback.track.clone(),
                playback.queue.clone(),
                playback.revision,
                np,
                effects,
            ),
            "play" if ready => {
                let target = position + elapsed;
                if (np.position - target).abs() > 0.5 {
                    effects.push(Effect::Seek(target));
                }
                effects.push(Effect::Paused(false));
            }
            "pause" if ready => {
                effects.push(Effect::Paused(true));
                effects.push(Effect::Seek(position));
            }
            "seek" if ready => effects.push(Effect::Seek(if np.playing {
                position + elapsed
            } else {
                position
            })),
            "queue_add" | "queue_remove" | "queue_clear" | "sync_queue" => {
                self.guest_set_queue(playback.queue.clone(), np, effects);
            }
            _ => {}
        }
    }

    /// Puts the room's song on this computer, paused: once it has loaded,
    /// `buffer_ready` asks the server where the room is, and it answers with
    /// a seek and a play.
    fn guest_load(
        &mut self,
        track: Option<p::Track>,
        queue: Vec<p::Track>,
        revision: u64,
        np: &Playback,
        effects: &mut Vec<Effect>,
    ) {
        let Some(track) = track.filter(|t| !t.id.is_empty()) else {
            self.guest_set_queue(queue, np, effects);
            return;
        };
        if np.mixing_into.as_deref() == Some(track.id.as_str()) {
            // Already mixing into the room's song: the tick loads it once the
            // mix ends, which finds it already playing.
            self.pending_load = Some((track, queue, revision));
            return;
        }
        self.pending_load = None;
        let songs: Vec<SongItem> = std::iter::once(&track)
            .chain(&queue)
            .map(song_for)
            .collect();
        self.awaiting_buffer = Some((track.id.clone(), revision));
        effects.push(if np.video_id() == Some(track.id.as_str()) {
            // Already here, e.g. this app reached the song on its own.
            Effect::SetQueue(songs)
        } else {
            Effect::Load(songs)
        });
    }

    fn guest_set_queue(&mut self, queue: Vec<p::Track>, np: &Playback, effects: &mut Vec<Effect>) {
        if let Some(pending) = &mut self.pending_load {
            pending.1 = queue;
            return;
        }
        let Some(current) = &np.song else {
            return;
        };
        let songs = std::iter::once(current.clone())
            .chain(queue.iter().map(song_for))
            .collect();
        effects.push(Effect::SetQueue(songs));
    }

    fn guest_tick(&mut self, np: &Playback, effects: &mut Vec<Effect>) {
        if let Some((track, queue, revision)) = self.pending_load.clone()
            && np.mixing_into.as_deref() != Some(track.id.as_str())
        {
            self.guest_load(Some(track), queue, revision, np, effects);
        }
        let Some((id, revision)) = self.awaiting_buffer.clone() else {
            return;
        };
        if np.video_id() == Some(id.as_str()) && !np.loading && np.duration > 0.0 {
            self.awaiting_buffer = None;
            self.send(kind::BUFFER_READY, &p::buffer_ready(&id, revision));
        }
    }

    /// Dev aid: WOOF_TOGETHER=1 opens the panel; =room fills it with a
    /// pretend room (no server) to look at.
    fn dev_demo(&mut self) {
        let Ok(mode) = std::env::var("WOOF_TOGETHER") else {
            return;
        };
        self.open = true;
        if mode != "room" {
            return;
        }
        let user = |id: &str, name: &str, host: bool, connected: bool| p::User {
            user_id: id.into(),
            username: name.into(),
            is_host: host,
            is_connected: connected,
        };
        self.status = Status::InRoom;
        self.room_code = Some("WOOF42".into());
        self.my_user_id = Some("me".into());
        self.host_id = Some("me".into());
        self.users = vec![
            user("me", "You", true, true),
            user("a", "Marta", false, true),
            user("b", "Kenji (Metrolist)", false, false),
        ];
        self.join_requests = vec![p::Pair("c".into(), "Sam".into())];
        self.suggestions = vec![p::Suggestion {
            id: "s".into(),
            from_username: "Marta".into(),
            track: p::Track {
                title: "Espresso".into(),
                artist: "Sabrina Carpenter".into(),
                ..p::Track::default()
            },
        }];
    }

    // MARK: - View

    /// The panel over the whole window, while open.
    pub fn overlay<'a>(
        &'a self,
        prefs: &'a SettingsStore,
        account: Option<(&'a str, Option<&'a str>)>,
        covers: &'a ui::Covers,
    ) -> Option<Element<'a, Message>> {
        if !self.open {
            return None;
        }
        let mut body = column![].spacing(18);
        if let Some(notice) = &self.notice {
            body = body.push(
                row![
                    ui::icon(INFO, 16.0).style(|_, _| svg::Style {
                        color: Some(theme::SECONDARY)
                    }),
                    text(notice).size(13).color(theme::SECONDARY),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            );
        }
        body = match self.status {
            Status::Idle => self.start_sections(body, prefs, account),
            Status::Connecting | Status::WaitingForApproval => body.push(group(
                None,
                vec![
                    text(if self.status == Status::Connecting {
                        "Connecting…"
                    } else {
                        "Waiting for the host to let you in…"
                    })
                    .size(14)
                    .color(theme::SECONDARY)
                    .into(),
                    ui::pill_button("Cancel", Message::Together(Msg::Leave)),
                ],
                None,
            )),
            Status::InRoom | Status::Reconnecting => {
                self.room_sections(body, account.and_then(|a| a.1), covers)
            }
        };
        let header = row![
            text("Listen Together")
                .size(18)
                .font(theme::weight(Weight::Bold))
                .width(Length::Fill),
            ui::pill_button("Done", Message::Together(Msg::Close)),
        ]
        .align_y(Alignment::Center);
        let panel = container(
            column![
                header,
                scrollable(body.padding(iced::Padding::ZERO.right(12))).height(Length::Shrink),
            ]
            .spacing(16),
        )
        .width(480)
        .max_height(760)
        .padding(22)
        .style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgba8(0x1C, 0x1C, 0x1E, 0.98))),
            border: Border::default().rounded(22).width(1).color(theme::HOVER),
            shadow: Shadow {
                color: Color {
                    a: 0.5,
                    ..Color::BLACK
                },
                offset: Vector::new(0.0, 16.0),
                blur_radius: 40.0,
            },
            ..container::Style::default()
        });
        let scrim = mouse_area(
            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_| container::Style {
                    background: Some(Background::Color(Color {
                        a: 0.6,
                        ..Color::BLACK
                    })),
                    ..container::Style::default()
                }),
        )
        .on_press(Message::Together(Msg::Close));
        Some(opaque(stack![
            scrim,
            container(opaque(panel)).center(Length::Fill)
        ]))
    }

    fn start_sections<'a>(
        &'a self,
        body: iced::widget::Column<'a, Message>,
        prefs: &'a SettingsStore,
        account: Option<(&'a str, Option<&'a str>)>,
    ) -> iced::widget::Column<'a, Message> {
        let placeholder = account.map_or("Your name", |a| a.0);
        let joinable = !self.code.trim().is_empty();
        let mut server = vec![
            text_input("wss://…", &prefs.listen_together_server_url)
                .on_input(|s| Message::Together(Msg::Server(s)))
                .padding([8, 12])
                .size(13)
                .font(Font::MONOSPACE)
                .style(ui::field_style)
                .into(),
        ];
        if prefs.listen_together_server_url != lt::DEFAULT_SERVER_URL {
            server.push(ui::pill_button(
                "Use Metrolist's Server",
                Message::Together(Msg::UseDefaultServer),
            ));
        }
        body.push(group(
            Some("Name"),
            vec![
                text_input(placeholder, &prefs.listen_together_username)
                    .on_input(|s| Message::Together(Msg::Name(s)))
                    .padding([8, 12])
                    .size(14)
                    .style(ui::field_style)
                    .into(),
            ],
            Some("Everyone in the room sees it."),
        ))
        .push(group(
            None,
            vec![ui::accent_button(
                "Start a Room",
                Some(PEOPLE),
                Some(Message::Together(Msg::Create)),
            )],
            Some("You control the music. Share the room code; you let each person in."),
        ))
        .push(group(
            Some("Join"),
            vec![
                row![
                    text_input("Room code", &self.code)
                        .on_input(|s| Message::Together(Msg::Code(s.to_uppercase())))
                        .on_submit(Message::Together(Msg::Join))
                        .padding([8, 12])
                        .size(15)
                        .font(Font::MONOSPACE)
                        .style(ui::field_style),
                    button(text("Join Room").size(14).font(theme::weight(Weight::Medium)))
                        .padding([8, 16])
                        .on_press_maybe(joinable.then_some(Message::Together(Msg::Join)))
                        .style(pill),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
                .into(),
            ],
            Some("The host's music plays here, in step with theirs."),
        ))
        .push(group(
            Some("Server"),
            server,
            Some("Rooms are shared with Metrolist on the same server, so Android friends can join too."),
        ))
    }

    fn room_sections<'a>(
        &'a self,
        mut body: iced::widget::Column<'a, Message>,
        my_photo: Option<&'a str>,
        covers: &'a ui::Covers,
    ) -> iced::widget::Column<'a, Message> {
        let mut head = column![
            text(if self.is_host() {
                "You're the host"
            } else {
                "Listening along"
            })
            .size(13)
            .color(theme::SECONDARY),
            text(self.room_code.as_deref().unwrap_or_default())
                .size(34)
                .font(Font {
                    weight: Weight::Bold,
                    ..Font::MONOSPACE
                }),
        ]
        .spacing(4);
        if self.status == Status::Reconnecting {
            head = head.push(text("Reconnecting…").size(12).color(ORANGE));
        }
        body = body.push(group(
            None,
            vec![
                head.into(),
                ui::pill_button(
                    if self.copied {
                        "Copied"
                    } else {
                        "Copy Room Code"
                    },
                    Message::Together(Msg::CopyCode),
                ),
            ],
            self.is_guest()
                .then_some("The host controls the music. Suggest songs from any song's menu."),
        ));
        if self.is_host() && !self.join_requests.is_empty() {
            body = body.push(group(
                Some("Wants to Join"),
                self.join_requests
                    .iter()
                    .map(|p::Pair(id, username)| {
                        let name = split(username).0;
                        member_row(
                            member_picture(name, self.picture(id, username, None), false, covers),
                            text(name).size(14).into(),
                            row![
                                small_button(
                                    "Decline",
                                    Message::Together(Msg::Reject(id.clone())),
                                    false
                                ),
                                small_button(
                                    "Let In",
                                    Message::Together(Msg::Approve(id.clone())),
                                    true
                                ),
                            ]
                            .spacing(6)
                            .into(),
                        )
                    })
                    .collect(),
                None,
            ));
        }
        if self.is_host() && !self.suggestions.is_empty() {
            body = body.push(group(
                Some("Suggestions"),
                self.suggestions
                    .iter()
                    .map(|s| {
                        let from = split(&s.from_username).0;
                        member_row(
                            Space::new().width(0).into(),
                            column![
                                text(ui::fit(&s.track.title, 230.0, 14.0)).size(14),
                                text(ui::fit(
                                    &format!("{} · from {from}", s.track.artist),
                                    230.0,
                                    12.0
                                ))
                                .size(12)
                                .color(theme::SECONDARY),
                            ]
                            .spacing(2)
                            .into(),
                            row![
                                small_button(
                                    "Skip",
                                    Message::Together(Msg::RejectSuggestion(s.id.clone())),
                                    false
                                ),
                                small_button(
                                    "Play Next",
                                    Message::Together(Msg::ApproveSuggestion(s.id.clone())),
                                    true
                                ),
                            ]
                            .spacing(6)
                            .into(),
                        )
                    })
                    .collect(),
                None,
            ));
        }
        let me = self.my_user_id.as_deref();
        body.push(group(
            Some("In the Room"),
            self.users
                .iter()
                .map(|user| {
                    let name = split(&user.username).0;
                    let mine = me == Some(user.user_id.as_str());
                    let label = if mine {
                        format!("{name} (you)")
                    } else {
                        name.to_owned()
                    };
                    let mut trailing = row![].spacing(6).align_y(Alignment::Center);
                    if !user.is_connected {
                        trailing = trailing.push(text("Away").size(12).color(theme::SECONDARY));
                    }
                    if self.is_host() && !mine {
                        trailing = trailing
                            .push(small_button(
                                "Make Host",
                                Message::Together(Msg::MakeHost(user.user_id.clone())),
                                false,
                            ))
                            .push(small_button(
                                "Remove",
                                Message::Together(Msg::Kick(user.user_id.clone())),
                                false,
                            ));
                    }
                    member_row(
                        member_picture(
                            name,
                            self.picture(&user.user_id, &user.username, my_photo),
                            user.is_host,
                            covers,
                        ),
                        text(ui::fit(&label, 200.0, 14.0)).size(14).into(),
                        trailing.into(),
                    )
                })
                .collect(),
            None,
        ))
        .push(
            button(
                text("Leave Room")
                    .size(14)
                    .font(theme::weight(Weight::Medium))
                    .color(theme::CLOSE),
            )
            .padding([8, 16])
            .on_press(Message::Together(Msg::Leave))
            .style(pill),
        )
    }
}

/// Swift's `.random(in: 0...1)` for the restart jitter.
fn noise() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos()) as f64
        / 1e9
}

const ORANGE: Color = Color::from_rgb(1.0, 0.62, 0.04);

fn pill(_: &iced::Theme, status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered => theme::PRESSED,
            _ => theme::HOVER,
        })),
        border: Border::default().rounded(18),
        text_color: if status == button::Status::Disabled {
            theme::SECONDARY
        } else {
            theme::TEXT
        },
        ..button::Style::default()
    }
}

fn small_button<'a>(label: &'a str, message: Message, strong: bool) -> Element<'a, Message> {
    button(text(label).size(12).font(theme::weight(Weight::Semibold)))
        .padding([5, 11])
        .on_press(message)
        .style(move |_, status| button::Style {
            background: Some(Background::Color(match (strong, status) {
                (true, button::Status::Hovered) => theme::accent_hover(),
                (true, _) => theme::accent(),
                (false, button::Status::Hovered) => theme::PRESSED,
                (false, _) => theme::HOVER,
            })),
            border: Border::default().rounded(14),
            text_color: if strong {
                theme::BACKGROUND
            } else {
                theme::TEXT
            },
            ..button::Style::default()
        })
        .into()
}

/// A section: an optional title, rows on a rounded card, an optional footnote.
fn group<'a>(
    title: Option<&'a str>,
    rows: Vec<Element<'a, Message>>,
    footer: Option<&'a str>,
) -> Element<'a, Message> {
    let mut section = column![].spacing(6);
    if let Some(title) = title {
        section = section.push(
            text(title.to_uppercase())
                .size(11)
                .font(theme::weight(Weight::Semibold))
                .color(theme::SECONDARY),
        );
    }
    section = section.push(
        container(column(rows).spacing(12))
            .width(Length::Fill)
            .padding(14)
            .style(|_| container::Style {
                background: Some(Background::Color(theme::HOVER)),
                border: Border::default().rounded(14),
                ..container::Style::default()
            }),
    );
    if let Some(footer) = footer {
        section = section.push(text(footer).size(12).color(theme::SECONDARY));
    }
    section.into()
}

fn member_row<'a>(
    picture: Element<'a, Message>,
    label: Element<'a, Message>,
    trailing: Element<'a, Message>,
) -> Element<'a, Message> {
    row![picture, container(label).width(Length::Fill), trailing]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
}

/// A member's YouTube picture, or their initial on a colour of their own,
/// with a crown for the host.
fn member_picture<'a>(
    name: &str,
    url: Option<&'a str>,
    host: bool,
    covers: &'a ui::Covers,
) -> Element<'a, Message> {
    const SIDE: f32 = 30.0;
    const COLORS: [Color; 7] = [
        Color::from_rgb(1.0, 0.58, 0.0),
        Color::from_rgb(1.0, 0.18, 0.33),
        Color::from_rgb(0.69, 0.32, 0.87),
        Color::from_rgb(0.35, 0.34, 0.84),
        Color::from_rgb(0.19, 0.69, 0.78),
        Color::from_rgb(0.2, 0.78, 0.35),
        Color::from_rgb(0.64, 0.52, 0.37),
    ];
    let picture: Element<'a, Message> = match url.filter(|url| covers.contains_key(*url)) {
        Some(url) => ui::cover(covers, Some(url), SIDE, SIDE / 2.0),
        None => {
            // The same colour for a name on every launch.
            let sum = name.chars().fold(0u32, |sum, c| sum.wrapping_add(c as u32));
            let color = COLORS[sum as usize % COLORS.len()];
            let initial = name
                .split(' ')
                .next()
                .and_then(|word| word.chars().next())
                .map_or("?".to_owned(), |c| c.to_uppercase().collect());
            container(
                text(initial)
                    .size(13)
                    .font(theme::weight(Weight::Semibold))
                    .color(Color::WHITE),
            )
            .center(SIDE)
            .style(move |_| container::Style {
                background: Some(Background::Color(color)),
                border: Border::default().rounded(SIDE / 2.0),
                ..container::Style::default()
            })
            .into()
        }
    };
    // The crown's room, kept on every picture so the names line up.
    let room = iced::Padding {
        right: 6.0,
        bottom: 5.0,
        ..iced::Padding::ZERO
    };
    if !host {
        return container(picture).padding(room).into();
    }
    let crown = container(ui::icon(CROWN, 11.0).style(|_, _| svg::Style {
        color: Some(Color::from_rgb(1.0, 0.8, 0.0)),
    }))
    .center(17)
    .style(|_| container::Style {
        background: Some(Background::Color(Color::from_rgb8(0x2C, 0x2C, 0x2E))),
        border: Border::default().rounded(8.5),
        ..container::Style::default()
    });
    stack![
        container(picture).padding(room),
        container(crown)
            .align_right(SIDE + 6.0)
            .align_bottom(SIDE + 5.0),
    ]
    .into()
}

/// `person.2.wave.2`.
pub const PEOPLE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><circle cx="9" cy="8" r="3"/><path d="M3.5 19c.6-3.2 2.8-5 5.5-5s4.9 1.8 5.5 5"/><path d="M16 5.5a3 3 0 0 1 0 5M18 14.5c1.5.8 2.4 2.4 2.7 4.5"/></g></svg>"##;
/// `crown.fill`.
const CROWN: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M3 7l4.5 4L12 4l4.5 7L21 7l-2 12H5z" fill="#fff"/></svg>"##;
/// `info.circle`.
const INFO: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5v.5"/></g></svg>"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_carries_its_channel_within_fifty_bytes() {
        let channel = "UCabcdefghijklmnopqrstuv";
        let long = "A very long display name that does not fit at all";
        let name = tagged(long, Some(channel));
        assert!(name.len() <= MAX_NAME_BYTES);
        assert_eq!(split(&name).1, Some(channel));
        assert!(long.starts_with(split(&name).0));
        assert_eq!(split("Marta"), ("Marta", None));
        assert_eq!(tagged("Marta", None), "Marta");
    }

    fn playback(id: &str, playing: bool, position: f64) -> Playback {
        Playback {
            song: Some(SongItem {
                video_id: id.into(),
                ..SongItem::default()
            }),
            playing,
            position,
            duration: 200.0,
            ..Playback::default()
        }
    }

    #[test]
    fn a_guest_loads_the_room_song_then_follows_play() {
        let mut room = Room {
            room_code: Some("AB".into()),
            my_user_id: Some("me".into()),
            host_id: Some("host".into()),
            ..Room::default()
        };
        let mut effects = Vec::new();
        let track = p::Track {
            id: "song".into(),
            ..p::Track::default()
        };
        let mut change = p::Playback::new("change_track", 1);
        change.track = Some(track);
        room.guest_apply(&change, &playback("other", true, 3.0), &mut effects);
        assert!(matches!(&effects[..], [Effect::Load(songs)] if songs[0].video_id == "song"));
        assert!(room.holds("song"));

        // Play before the song has loaded is ignored; buffer_ready asks again.
        let mut play = p::Playback::new("play", 2);
        play.track_id = "song".into();
        effects.clear();
        room.guest_apply(&play, &playback("song", false, 0.0), &mut effects);
        assert!(effects.is_empty());

        room.status = Status::InRoom;
        room.guest_tick(&playback("song", false, 0.0), &mut effects);
        assert!(!room.holds("song"));
        play.position = 40_000;
        play.server_time = room.server_now();
        room.guest_apply(&play, &playback("song", false, 0.0), &mut effects);
        assert!(
            matches!(&effects[..], [Effect::Seek(t), Effect::Paused(false)] if (*t - 40.0).abs() < 0.5)
        );
    }
}
