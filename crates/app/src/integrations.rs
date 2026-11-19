//! Discord presence, Last.fm and ListenBrainz scrobbling, and the Odesli
//! setting, run on their own thread.
//!
//! The thread owns every integration and handles one command at a time, so
//! their keyring reads, Discord's blocking socket and the scrobble requests
//! never hold up the window or iced's executor. The app sends what plays with
//! `tick`, and Settings asks for changes with `request`.

use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use iced::futures::StreamExt;
use iced::futures::channel::{mpsc, oneshot};
use swiftytube::networking::http_client::Session;
use trop::discord::discord_activity::DiscordActivity;
use trop::discord::discord_activity_builder::DiscordActivityBuilder;
use trop::discord::discord_defaults::{DiscordDefaults, DiscordSettings};
use trop::discord::discord_presence::ActivityType;
use trop::last_fm::last_fm_defaults::LastFMSettings;
use trop::last_fm::last_fm_integration::{LastFMIntegration, PlayingSong};
use trop::last_fm::last_fm_service::LastFMService;
use trop::last_fm::last_fm_token_store::LastFMTokenStore;
use trop::listen_brainz::listen_brainz::ListenBrainz;
use trop::listen_brainz::listen_brainz_defaults::ListenBrainzSettings;
use trop::networking::models::yt_item::SongItem;
use trop::odesli::odesli::Odesli;
use trop::odesli::odesli_defaults::OdesliSettings;

use super::{PlayState, Player};

/// How long Settings waits for the Last.fm page to be approved in the browser.
// ponytail: no Cancel button; a sign-in left unapproved ends after this. Add
// one (abort the polling task) if users ask for it.
const LAST_FM_SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Swift `DiscordIntegration`: a missing Discord is looked for again this often.
const DISCORD_RETRY: Duration = Duration::from_secs(30);

/// What Settings shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub discord: bool,
    /// The Discord app answered; false while it isn't running.
    // ponytail: read when Settings asks, not pushed when Discord comes or goes;
    // push it from `Discord::sync` if the page should update live.
    pub discord_connected: bool,
    /// The build has Last.fm API keys.
    pub last_fm_available: bool,
    /// The Last.fm user name, empty if unknown; `None` while signed out.
    pub last_fm_user: Option<String>,
    pub scrobbling: bool,
    pub listen_brainz_user: Option<String>,
    pub share_links: bool,
    /// An Odesli API key is saved, so Spotify and Apple Music links open.
    pub odesli_connected: bool,
    /// All the Last.fm and Discord options, for the Settings groups.
    pub last_fm_options: LastFMSettings,
    pub discord_options: DiscordSettings,
}

/// A change Settings asks for. Each answers with the new `Status`.
#[derive(Clone, Debug)]
pub enum Action {
    Status,
    SetDiscord(bool),
    SetScrobbling(bool),
    SetShareLinks(bool),
    /// Saves the Last.fm options (the sign-in stays as it is).
    SetLastFmOptions(Box<LastFMSettings>),
    SetDiscordOptions(Box<DiscordSettings>),
    /// Opens the Last.fm approval page and waits until it is approved.
    LastFmSignIn,
    LastFmSignOut,
    ListenBrainzConnect(String),
    ListenBrainzDisconnect,
    OdesliConnect(String),
    OdesliDisconnect,
}

type Reply = oneshot::Sender<Result<Status, String>>;

enum Command {
    /// What plays, and whether incognito is on.
    Tick(Option<Box<Now>>, bool),
    Request(Action, Reply),
    LastFmSignedIn(Result<(), String>, Reply),
    /// Swift `LikeStore.toggle`: a like loves the track on Last.fm.
    Love {
        artist: String,
        track: String,
        love: bool,
    },
}

/// The player at one tick.
#[derive(Clone)]
struct Now {
    song: SongItem,
    playing: bool,
    position: f64,
    duration: f64,
    at: SystemTime,
}

/// The thread starts on first use.
static COMMANDS: LazyLock<mpsc::UnboundedSender<Command>> = LazyLock::new(|| {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("integrations".into())
        .spawn(move || run(receiver));
    if let Err(error) = spawned {
        eprintln!("Integrations are off: {error}");
    }
    sender
});

/// Reports what plays. Cheap: the thread does the work. While `incognito`
/// nothing is scrobbled and Discord shows nothing (Swift `isIncognito`).
pub fn tick(player: &Player, incognito: bool) {
    let now = player.song().map(|song| {
        Box::new(Now {
            song: song.clone(),
            // While the next stream resolves, mpv still reports the previous song.
            playing: player.state == PlayState::Playing && !player.starting,
            position: player.position,
            duration: match player.duration {
                d if d > 0.0 => d,
                _ => song.duration as f64,
            },
            at: SystemTime::now(),
        })
    });
    let _ = COMMANDS.unbounded_send(Command::Tick(now, incognito));
}

/// Loves or un-loves `song` on Last.fm, when it is connected.
pub fn love(song: &SongItem, love: bool) {
    let artist = song
        .artists
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if artist.is_empty() || song.title.is_empty() {
        return;
    }
    let track = song.title.clone();
    let _ = COMMANDS.unbounded_send(Command::Love {
        artist,
        track,
        love,
    });
}

pub async fn request(action: Action) -> Result<Status, String> {
    let (reply, answer) = oneshot::channel();
    COMMANDS
        .unbounded_send(Command::Request(action, reply))
        .map_err(|_| "Integrations are not running.".to_owned())?;
    answer
        .await
        .map_err(|_| "Integrations stopped.".to_owned())?
}

fn run(mut commands: mpsc::UnboundedReceiver<Command>) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return eprintln!("Integrations are off: {error}"),
    };
    runtime.block_on(async move {
        let mut worker = Worker::new();
        while let Some(command) = commands.next().await {
            worker.handle(command).await;
        }
    });
}

fn defaults_path(path: Option<PathBuf>, name: &str) -> PathBuf {
    path.unwrap_or_else(|| std::env::temp_dir().join(name))
}

struct Worker {
    last_fm: LastFMIntegration,
    last_fm_path: PathBuf,
    listen_brainz: ListenBrainz,
    discord: Discord,
    discord_path: PathBuf,
    odesli: OdesliSettings,
    odesli_path: PathBuf,
    /// The last tick, so a settings change applies to Discord at once.
    now: Option<Box<Now>>,
}

impl Worker {
    fn new() -> Self {
        let last_fm_path = defaults_path(LastFMSettings::default_path(), "woofmusic-last_fm.json");
        let discord_path = defaults_path(DiscordSettings::default_path(), "woofmusic-discord.json");
        let odesli_path = defaults_path(OdesliSettings::default_path(), "woofmusic-odesli.json");
        Self {
            last_fm: LastFMIntegration::new(Session::shared(), last_fm_path.clone()),
            last_fm_path,
            listen_brainz: ListenBrainz::new(
                Session::shared(),
                defaults_path(
                    ListenBrainzSettings::default_path(),
                    "woofmusic-listen_brainz.json",
                ),
            ),
            discord: Discord::new(DiscordSettings::load(&discord_path)),
            discord_path,
            odesli: OdesliSettings::load(&odesli_path),
            odesli_path,
            now: None,
        }
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::Tick(now, incognito) => {
                self.last_fm.incognito = incognito;
                self.listen_brainz.incognito = incognito;
                let at = now.as_ref().map_or_else(SystemTime::now, |now| now.at);
                let playing = now.as_deref().map(Now::playing_song);
                self.last_fm.sync_state(playing.as_ref(), at).await;
                self.listen_brainz.tick(playing.as_ref(), at).await;
                // Swift `syncDiscordState`: incognito clears the presence.
                self.now = now.filter(|_| !incognito);
                self.discord.sync(self.now.as_deref());
            }
            Command::Request(Action::LastFmSignIn, reply) => {
                match self.start_last_fm_sign_in().await {
                    Ok(started) => wait_for_last_fm(started, reply),
                    Err(error) => {
                        let _ = reply.send(Err(error));
                    }
                }
            }
            Command::Request(action, reply) => {
                let result = self.apply(action).await;
                // The receiver is gone if Settings stopped waiting: nothing to do.
                let _ = reply.send(result.map(|()| self.status()));
            }
            Command::Love {
                artist,
                track,
                love,
            } => {
                self.last_fm.handle_love(&artist, &track, love).await;
            }
            Command::LastFmSignedIn(result, reply) => {
                let result = result.and_then(|()| {
                    // A fresh integration: the old one cached "signed out".
                    self.last_fm =
                        LastFMIntegration::new(Session::shared(), self.last_fm_path.clone());
                    // Connecting means wanting scrobbles.
                    self.last_fm.set_enabled(true).map_err(save_error)
                });
                let _ = reply.send(result.map(|()| self.status()));
            }
        }
    }

    async fn apply(&mut self, action: Action) -> Result<(), String> {
        match action {
            // Sign-in answers later, from `wait_for_last_fm`.
            Action::Status | Action::LastFmSignIn => Ok(()),
            Action::SetDiscord(enabled) => {
                self.discord.settings.enabled = enabled;
                // Turned on: look for Discord now, not after the retry delay.
                self.discord.last_attempt = None;
                self.discord.sync(self.now.as_deref());
                self.discord
                    .settings
                    .save(&self.discord_path)
                    .map_err(save_error)
            }
            Action::SetDiscordOptions(options) => {
                self.discord.settings = *options;
                // The presence is sent again with the new templates and buttons.
                self.discord.shown = None;
                self.discord.last_attempt = None;
                self.discord.sync(self.now.as_deref());
                self.discord
                    .settings
                    .save(&self.discord_path)
                    .map_err(save_error)
            }
            Action::SetLastFmOptions(options) => {
                // The sign-in and the scrobbling switch belong to the integration.
                let mut settings = self.last_fm.settings();
                settings.use_now_playing = options.use_now_playing;
                settings.use_send_likes = options.use_send_likes;
                settings.scrobble_delay_percent = options.scrobble_delay_percent;
                settings.scrobble_min_duration = options.scrobble_min_duration;
                settings.scrobble_delay_seconds = options.scrobble_delay_seconds;
                settings.save(&self.last_fm_path).map_err(save_error)
            }
            Action::SetScrobbling(enabled) => self.last_fm.set_enabled(enabled).map_err(save_error),
            Action::SetShareLinks(enabled) => {
                self.odesli.share_links = enabled;
                self.odesli.save(&self.odesli_path).map_err(save_error)
            }
            Action::LastFmSignOut => {
                self.last_fm.logout();
                Ok(())
            }
            Action::ListenBrainzConnect(token) => self
                .listen_brainz
                .connect(&token)
                .await
                .map_err(|error| error.to_string()),
            Action::ListenBrainzDisconnect => {
                self.listen_brainz.disconnect();
                Ok(())
            }
            Action::OdesliConnect(key) => Odesli::new(Session::shared())
                .connect(&key)
                .await
                .map_err(|error| error.to_string()),
            Action::OdesliDisconnect => {
                Odesli::new(Session::shared()).disconnect();
                Ok(())
            }
        }
    }

    /// Asks Last.fm for a token and opens its approval page in the browser.
    async fn start_last_fm_sign_in(&self) -> Result<(LastFMService, String), String> {
        let service = LastFMService::new(
            Session::shared(),
            LastFMTokenStore::new(self.last_fm_path.clone()),
        );
        if !service.is_configured() {
            return Err("Last.fm isn't available in this build.".into());
        }
        let (token, url) = service
            .start_sign_in()
            .await
            .map_err(|error| format!("Couldn't reach Last.fm: {error}"))?;
        open_in_browser(&url).map_err(|error| format!("Couldn't open your browser: {error}"))?;
        Ok((service, token))
    }

    fn status(&self) -> Status {
        let logged_in = self.last_fm.is_logged_in();
        Status {
            discord: self.discord.settings.enabled,
            discord_connected: self.discord.client.is_some(),
            last_fm_available: self.last_fm.service.is_configured(),
            last_fm_user: logged_in.then(|| {
                self.last_fm
                    .service
                    .token_store
                    .retrieve_username()
                    .unwrap_or_default()
            }),
            scrobbling: self.last_fm.is_enabled(),
            listen_brainz_user: self.listen_brainz.user_name().map(str::to_owned),
            share_links: self.odesli.share_links,
            odesli_connected: Odesli::new(Session::shared()).is_connected(),
            last_fm_options: self.last_fm.settings(),
            discord_options: self.discord.settings.clone(),
        }
    }
}

/// Polls Last.fm until the page is approved, beside the ticks, then hands the
/// result back to the worker.
fn wait_for_last_fm((service, token): (LastFMService, String), reply: Reply) {
    tokio::spawn(async move {
        let result =
            match tokio::time::timeout(LAST_FM_SIGN_IN_TIMEOUT, service.complete_sign_in(&token))
                .await
            {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(error)) => Err(format!("Couldn't save the Last.fm session: {error}")),
                Err(_) => Err("Last.fm wasn't approved in time. Connect again to retry.".into()),
            };
        let _ = COMMANDS.unbounded_send(Command::LastFmSignedIn(result, reply));
    });
}

fn save_error(error: std::io::Error) -> String {
    format!("Couldn't save the setting: {error}")
}

pub fn open_in_browser(url: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    std::process::Command::new(program)
        .arg(url)
        .spawn()
        .map(drop)
}

impl Now {
    fn playing_song(&self) -> PlayingSong {
        PlayingSong {
            video_id: self.song.video_id.clone(),
            title: self.song.title.clone(),
            artist: crate::ui::artists(&self.song.artists),
            album: self.song.album.clone(),
            duration: self.duration,
            is_playing: self.playing,
        }
    }
}

/// Swift `DiscordIntegration.syncDiscordState` over Discord's local IPC
/// socket instead of the Social SDK: no sign-in, the Discord app must run.
struct Discord {
    settings: DiscordSettings,
    client: Option<DiscordIpcClient>,
    last_attempt: Option<Instant>,
    /// Discord's absence was logged; it is logged once per run.
    logged: bool,
    /// Swift `isShowingSong`: the song, whether it plays, and its start in ms.
    shown: Option<(String, bool, i64)>,
}

impl Discord {
    fn new(settings: DiscordSettings) -> Self {
        Self {
            settings,
            client: None,
            last_attempt: None,
            logged: false,
            shown: None,
        }
    }

    fn sync(&mut self, now: Option<&Now>) {
        if !self.settings.enabled {
            if let Some(mut client) = self.client.take() {
                let _ = client.close();
            }
            self.shown = None;
            return;
        }
        let Some(now) = now else {
            return self.clear();
        };
        if self.client.is_none() && !self.connect() {
            return;
        }
        let now_ms = millis(now.at);
        let position = match now.duration {
            d if d > 0.0 => now.position.clamp(0.0, d),
            _ => now.position.max(0.0),
        };
        let start_ms = if now.playing {
            now_ms - (position * 1000.0) as i64
        } else {
            0
        };
        // A playing song's start moves only on a seek: within 3 s, Discord is right.
        if let Some((id, playing, start)) = &self.shown
            && *id == now.song.video_id
            && *playing == now.playing
            && (start_ms - start).abs() < 3_000
        {
            return;
        }
        let end_ms = (now.playing && now.duration > 0.0)
            .then(|| now_ms + ((now.duration - position) * 1000.0) as i64);
        let artists = crate::ui::artists(&now.song.artists);
        let artist = match artists.as_str() {
            "" => DiscordDefaults::UNKNOWN_ARTIST,
            name => name,
        };
        let built = DiscordActivityBuilder::build(
            &now.song,
            artist,
            now.song.album.as_deref(),
            None,
            start_ms,
            end_ms,
            &self.settings,
        );
        let Some(client) = self.client.as_mut() else {
            return;
        };
        // Reading the answer keeps Discord's replies from piling up in the socket.
        match client
            .set_activity(ipc_activity(&built))
            .and_then(|()| client.recv().map(drop))
        {
            Ok(()) => self.shown = Some((now.song.video_id.clone(), now.playing, start_ms)),
            Err(_) => {
                // Discord quit: look for it again later.
                self.client = None;
                self.shown = None;
            }
        }
    }

    /// True once the Discord app answers; tried every `DISCORD_RETRY`.
    fn connect(&mut self) -> bool {
        if self
            .last_attempt
            .is_some_and(|at| at.elapsed() < DISCORD_RETRY)
        {
            return false;
        }
        self.last_attempt = Some(Instant::now());
        let mut client = DiscordIpcClient::new(DiscordDefaults::APP_ID);
        match client.connect() {
            Ok(()) => {
                self.client = Some(client);
                true
            }
            Err(error) => {
                if !self.logged {
                    self.logged = true;
                    eprintln!("Discord presence waits for the Discord app: {error}");
                }
                false
            }
        }
    }

    fn clear(&mut self) {
        if self.shown.take().is_some()
            && let Some(client) = self.client.as_mut()
            && client
                .clear_activity()
                .and_then(|()| client.recv().map(drop))
                .is_err()
        {
            self.client = None;
        }
    }
}

fn ipc_activity(built: &DiscordActivity) -> activity::Activity<'_> {
    let kind = match built.activity_type {
        ActivityType::Playing => activity::ActivityType::Playing,
        ActivityType::Watching => activity::ActivityType::Watching,
        ActivityType::Competing => activity::ActivityType::Competing,
        // IPC has no Streaming or Custom activity.
        _ => activity::ActivityType::Listening,
    };
    let mut timestamps = activity::Timestamps::new();
    if built.start_timestamp > 0 {
        timestamps = timestamps.start(built.start_timestamp);
    }
    if let Some(end) = built.end_timestamp {
        timestamps = timestamps.end(end);
    }
    let mut assets = activity::Assets::new();
    if let Some(image) = &built.large_image {
        assets = assets.large_image(image.as_str());
    }
    if let Some(text) = &built.large_text {
        assets = assets.large_text(text.as_str());
    }
    if let Some(text) = &built.small_text {
        assets = assets.small_text(text.as_str());
    }
    let buttons = [
        (&built.button1_label, &built.button1_url),
        (&built.button2_label, &built.button2_url),
    ]
    .into_iter()
    .filter_map(|(label, url)| Some((label.as_deref()?, url.as_deref()?)))
    .filter(|(label, url)| !label.is_empty() && !url.is_empty())
    .map(|(label, url)| activity::Button::new(label, url))
    .collect();
    let mut ipc = activity::Activity::new()
        .activity_type(kind)
        .state(built.state.as_str())
        .timestamps(timestamps)
        .assets(assets)
        .buttons(buttons);
    if let Some(name) = &built.name {
        ipc = ipc.name(name.as_str());
    }
    if let Some(details) = &built.details {
        ipc = ipc.details(details.as_str());
    }
    ipc
}

fn millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
