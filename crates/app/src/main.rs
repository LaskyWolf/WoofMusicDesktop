//! WoofMusic Desktop: window, navigation, player and queue (see docs/MILESTONES.md).

mod glass;
mod now_playing;
mod pages;
mod services;
mod theme;
mod title_bar;
mod ui;
mod virtual_list;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use iced::font::Weight;
use iced::widget::operation::{focus, scroll_to};
use iced::widget::{Space, button, column, container, row, scrollable, slider, stack, svg, text};
use iced::{
    Alignment, Background, Border, Color, Element, Length, Padding, Size, Subscription, Task,
    keyboard, window,
};
use trop::audio::player_controller::player_controller::State as PlayState;
use trop::detail::detail_models::{
    AlbumDetailInfo, ArtistDetailInfo, PlaylistDetailInfo, PodcastDetailInfo,
};
use trop::networking::models::home_page::{ExploreSection, HomeSection, MoodItem};
use trop::networking::models::yt_item::{SongItem, YTItem};
use trop::search::search_parser::SearchSection;

use services::{Artwork, Services};

const APP_ID: &str = "com.laskywolf.woofmusic";

/// Rows past this many get no cover download until long lists are virtualized.
const MAX_ROW_COVERS: usize = 300;

/// Previous restarts the song instead once it has played this long, in seconds.
const RESTART_AFTER: f64 = 3.0;

pub fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("WoofMusic")
        .subscription(App::subscription)
        .font(include_bytes!("../assets/InterVariable.ttf").as_slice())
        .default_font(theme::FONT)
        .style(|_, _| iced::theme::Style {
            background_color: theme::BACKGROUND,
            text_color: theme::TEXT,
        })
        .window(window::Settings {
            size: dev_size().unwrap_or(Size::new(1280.0, 800.0)),
            min_size: Some(Size::new(880.0, 560.0)),
            decorations: false,
            #[cfg(target_os = "linux")]
            platform_specific: window::settings::PlatformSpecific {
                application_id: APP_ID.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}

/// A page the app can show. Sidebar tabs start a new stack; the others push.
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    Home,
    Explore,
    Library,
    Search,
    Album(String),
    Artist(String),
    Playlist(String),
    Podcast(String),
    Mood(MoodItem),
}

impl Route {
    const TABS: [(Route, &'static str); 4] = [
        (Route::Home, "Home"),
        (Route::Explore, "Explore"),
        (Route::Library, "Library"),
        (Route::Search, "Search"),
    ];

    /// Cache key of a detail page's data; `None` for the tabs.
    fn key(&self) -> Option<String> {
        match self {
            Route::Album(id) => Some(format!("album:{id}")),
            Route::Artist(id) => Some(format!("artist:{id}")),
            Route::Playlist(id) => Some(format!("playlist:{id}")),
            Route::Podcast(id) => Some(format!("podcast:{id}")),
            Route::Mood(mood) => Some(format!(
                "mood:{}:{}",
                mood.title,
                mood.params.as_deref().unwrap_or_default()
            )),
            Route::Home | Route::Explore | Route::Library | Route::Search => None,
        }
    }
}

/// A page on the navigation stack and how far it was scrolled.
struct Entry {
    route: Route,
    offset: f32,
}

/// The loaded data of a detail page. `songs` is the queue its Play button starts.
#[derive(Debug)]
pub enum Detail {
    Album {
        info: AlbumDetailInfo,
        songs: Arc<Vec<SongItem>>,
    },
    Artist {
        info: ArtistDetailInfo,
        songs: Arc<Vec<SongItem>>,
        shelves: Vec<YTItem>,
    },
    Playlist {
        info: PlaylistDetailInfo,
        songs: Arc<Vec<SongItem>>,
    },
    Podcast {
        info: PodcastDetailInfo,
        songs: Arc<Vec<SongItem>>,
    },
    Mood(Vec<ExploreSection>),
}

impl Detail {
    /// Album tracks come without cover and often without artists: they take the album's.
    fn album(info: AlbumDetailInfo) -> Self {
        let songs = info
            .songs
            .iter()
            .cloned()
            .map(|mut song| {
                song.thumbnail_url = song.thumbnail_url.or_else(|| info.thumbnail_url.clone());
                if song.artists.is_empty() {
                    song.artists = info.artists.clone();
                }
                song
            })
            .collect();
        Self::Album {
            songs: Arc::new(songs),
            info,
        }
    }

    fn artist(info: ArtistDetailInfo) -> Self {
        Self::Artist {
            songs: Arc::new(info.songs.clone()),
            shelves: pages::artist_items(&info),
            info,
        }
    }

    fn playlist(info: PlaylistDetailInfo) -> Self {
        Self::Playlist {
            songs: Arc::new(info.songs.clone()),
            info,
        }
    }

    /// The episodes as songs, one for one, so a row's index plays its episode.
    fn podcast(info: PodcastDetailInfo) -> Self {
        let songs = info.episodes.iter().map(ui::episode_song).collect();
        Self::Podcast {
            songs: Arc::new(songs),
            info,
        }
    }

    fn cover_urls(&self) -> Vec<String> {
        let songs = |songs: &[SongItem]| {
            songs
                .iter()
                .take(MAX_ROW_COVERS)
                .filter_map(|song| song.thumbnail_url.clone())
                .collect::<Vec<_>>()
        };
        match self {
            Detail::Album { info, .. } => info.thumbnail_url.iter().cloned().collect(),
            Detail::Artist { info, shelves, .. } => info
                .thumbnail_url
                .iter()
                .cloned()
                .chain(songs(&info.songs))
                .chain(
                    shelves
                        .iter()
                        .filter_map(|i| i.thumbnail_url().map(str::to_owned)),
                )
                .collect(),
            Detail::Playlist { info, .. } => info
                .thumbnail_url
                .iter()
                .cloned()
                .chain(songs(&info.songs))
                .collect(),
            Detail::Podcast { info, songs: list } => info
                .thumbnail_url
                .iter()
                .cloned()
                .chain(songs(list))
                .collect(),
            Detail::Mood(sections) => explore_covers(sections),
        }
    }
}

struct App {
    services: Arc<Services>,
    maximized: bool,
    /// Never empty: the last entry is the page on screen.
    stack: Vec<Entry>,
    home: Load<Vec<HomeSection>>,
    /// `None` until Explore is first opened.
    explore: Option<Load<Vec<ExploreSection>>>,
    details: HashMap<String, Load<Arc<Detail>>>,
    query: String,
    /// The query the results belong to.
    searched: String,
    results: Option<Load<Vec<SearchSection>>>,
    covers: ui::Covers,
    library: virtual_list::State,
    player: Player,
    /// Now Playing covers the page.
    now_playing: bool,
    queue_open: bool,
    queue_list: virtual_list::State,
    /// The current song's large cover and colours, with its video id.
    artwork: Option<(String, Artwork)>,
}

/// The queue and what the player bar shows.
#[derive(Default)]
struct Player {
    queue: Arc<Vec<SongItem>>,
    index: usize,
    state: PlayState,
    position: f64,
    duration: f64,
    /// The stream is being resolved; the bar shows the song before audio starts.
    starting: bool,
    error: Option<String>,
    /// Counts play requests, so a slow stream for a skipped song is dropped.
    request: u64,
    /// Where the seek slider is while it is dragged; `Tick` leaves it alone.
    seeking: Option<f64>,
}

impl Player {
    fn song(&self) -> Option<&SongItem> {
        self.queue.get(self.index)
    }

    fn has_next(&self) -> bool {
        self.index + 1 < self.queue.len()
    }
}

enum Load<T> {
    Loading,
    Loaded(T),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum Message {
    TitleBar(title_bar::Message),
    Resized,
    Maximized(bool),
    /// Push a page.
    Navigate(Route),
    /// A sidebar tab: start over from that page.
    Tab(Route),
    Back,
    PageScrolled(f32),
    LibraryScrolled(scrollable::Viewport),
    HomeLoaded(Result<Vec<HomeSection>, String>),
    ExploreLoaded(Result<Vec<ExploreSection>, String>),
    DetailLoaded(String, Result<Arc<Detail>, String>),
    SearchInput(String),
    SearchSubmit,
    SearchLoaded(String, Result<Vec<SearchSection>, String>),
    /// Reload the page on screen after a failure.
    Retry,
    Cover(String, Option<iced::widget::image::Handle>),
    /// Play `songs` from `index` on.
    PlayQueue(Arc<Vec<SongItem>>, usize),
    Resolved(u64, Result<String, String>),
    TogglePlay,
    Next,
    Previous,
    /// The seek slider moved; the song jumps there on release (`Seek`).
    Seeking(f64),
    Seek,
    Tick,
    Artwork(String, Option<Artwork>),
    ToggleNowPlaying,
    ToggleQueue,
    QueueScrolled(scrollable::Viewport),
    /// Escape: close the queue, then Now Playing.
    Dismiss,
    Snapshot(window::Screenshot),
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let services = Services::new();
        let mut stack = vec![Entry {
            route: Route::Home,
            offset: 0.0,
        }];
        match dev_route() {
            Some(route) if route.key().is_none() => stack[0].route = route,
            Some(route) => stack.push(Entry { route, offset: 0.0 }),
            None => {}
        }
        let mut app = Self {
            services: services.clone(),
            maximized: false,
            stack,
            home: Load::Loading,
            explore: None,
            details: HashMap::new(),
            query: String::new(),
            searched: String::new(),
            results: None,
            covers: HashMap::new(),
            library: virtual_list::State::default(),
            player: Player::default(),
            // Dev aids: WOOF_NOW_PLAYING=1 and WOOF_QUEUE=1 open them once a song plays.
            now_playing: std::env::var("WOOF_NOW_PLAYING").is_ok(),
            queue_open: std::env::var("WOOF_QUEUE").is_ok(),
            queue_list: virtual_list::State::default(),
            artwork: None,
        };
        // Dev aid: WOOF_SEARCH=query searches at start.
        let search = match std::env::var("WOOF_SEARCH") {
            Ok(query) => {
                app.query = query;
                Task::done(Message::SearchSubmit)
            }
            Err(_) => Task::none(),
        };
        let enter = app.enter();
        // Dev aid: WOOF_SCROLL=px scrolls the page once it has loaded.
        let scroll = match std::env::var("WOOF_SCROLL")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            Some(offset) => Task::future(tokio::time::sleep(Duration::from_secs(4)))
                .then(move |_| scroll_page_to(offset)),
            None => Task::none(),
        };
        (
            app,
            Task::batch([load_home(&services), enter, search, scroll, snapshot_task()]),
        )
    }

    fn route(&self) -> &Route {
        &self.stack.last().expect("the stack is never empty").route
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TitleBar(message) => title_bar::update(message),
            Message::Resized => window::latest()
                .and_then(window::is_maximized)
                .map(Message::Maximized),
            Message::Maximized(maximized) => {
                self.maximized = maximized;
                Task::none()
            }
            Message::Navigate(route) => {
                // A link in Now Playing (artist, album) opens over the page below.
                let closed = self.close_now_playing();
                if *self.route() == route {
                    return closed;
                }
                self.stack.push(Entry { route, offset: 0.0 });
                Task::batch([self.enter(), scroll_page_to(0.0)])
            }
            Message::Tab(route) => {
                self.now_playing = false;
                self.stack = vec![Entry { route, offset: 0.0 }];
                Task::batch([self.enter(), scroll_page_to(0.0)])
            }
            Message::Back => {
                if self.now_playing {
                    return self.close_now_playing();
                }
                if self.stack.len() < 2 {
                    return Task::none();
                }
                self.stack.pop();
                let offset = self.stack.last().map_or(0.0, |entry| entry.offset);
                Task::batch([self.enter(), scroll_page_to(offset)])
            }
            Message::PageScrolled(offset) => {
                if let Some(entry) = self.stack.last_mut() {
                    entry.offset = offset;
                }
                Task::none()
            }
            Message::LibraryScrolled(viewport) => {
                self.library.update(viewport);
                Task::none()
            }
            Message::HomeLoaded(Ok(sections)) => {
                // Dev aid: WOOF_PLAY=1 plays the first Home section with songs.
                let autoplay = std::env::var("WOOF_PLAY").is_ok().then(|| {
                    sections
                        .iter()
                        .map(|section| ui::songs_of(section.items()))
                        .find(|songs| !songs.is_empty())
                });
                let urls = sections
                    .iter()
                    .flat_map(|section| section.items().iter().take(12))
                    .filter_map(|item| item.thumbnail_url().map(str::to_owned))
                    .collect::<Vec<_>>();
                self.home = Load::Loaded(sections);
                let covers = self.fetch_covers(urls);
                match autoplay.flatten() {
                    Some(songs) => Task::batch([covers, self.play(songs, 0)]),
                    None => covers,
                }
            }
            Message::HomeLoaded(Err(error)) => {
                self.home = Load::Failed(error);
                Task::none()
            }
            Message::ExploreLoaded(result) => {
                let covers = match &result {
                    Ok(sections) => self.fetch_covers(explore_covers(sections)),
                    Err(_) => Task::none(),
                };
                self.explore = Some(loaded(result));
                covers
            }
            Message::DetailLoaded(key, result) => {
                let covers = match &result {
                    Ok(detail) => self.fetch_covers(detail.cover_urls()),
                    Err(_) => Task::none(),
                };
                self.details.insert(key, loaded(result));
                covers
            }
            Message::SearchInput(query) => {
                self.query = query;
                Task::none()
            }
            Message::SearchSubmit => {
                let query = self.query.trim().to_owned();
                if query.is_empty() {
                    return Task::none();
                }
                self.searched = query.clone();
                self.results = Some(Load::Loading);
                Task::perform(self.services.clone().search(query.clone()), move |result| {
                    Message::SearchLoaded(query.clone(), result)
                })
            }
            Message::SearchLoaded(query, result) => {
                if query != self.searched {
                    return Task::none();
                }
                let covers = match &result {
                    Ok(sections) => self.fetch_covers(
                        sections
                            .iter()
                            .flat_map(|section| section.items.iter().take(12))
                            .filter_map(|item| item.thumbnail_url().map(str::to_owned))
                            .collect::<Vec<_>>(),
                    ),
                    Err(_) => Task::none(),
                };
                self.results = Some(loaded(result));
                covers
            }
            Message::Retry => {
                match self.route().clone() {
                    Route::Home => self.home = Load::Loading,
                    Route::Explore => self.explore = None,
                    route => {
                        if let Some(key) = route.key() {
                            self.details.remove(&key);
                        }
                    }
                }
                match self.route() {
                    Route::Home => load_home(&self.services),
                    _ => self.enter(),
                }
            }
            Message::Cover(url, handle) => {
                if let Some(handle) = handle {
                    self.covers.insert(url, handle);
                }
                Task::none()
            }
            Message::PlayQueue(songs, index) => self.play(songs, index),
            Message::Resolved(request, result) => {
                if request != self.player.request {
                    return Task::none();
                }
                self.player.starting = false;
                let video_id = self.player.song().map(|song| song.video_id.clone());
                self.player.error = match (result, &self.services.player) {
                    (Err(error), _) => Some(error),
                    (Ok(_), None) => Some("The audio player (libmpv) did not start.".to_owned()),
                    (Ok(url), Some(player)) => player
                        .play(&url, video_id.as_deref())
                        .err()
                        .map(|error| error.to_string()),
                };
                self.read_player();
                Task::none()
            }
            Message::TogglePlay => {
                if let Some(player) = &self.services.player {
                    player.toggle_play_pause();
                }
                self.read_player();
                Task::none()
            }
            Message::Next => self.next(),
            Message::Seeking(position) => {
                self.player.seeking = Some(position);
                Task::none()
            }
            Message::Seek => {
                if let Some(position) = self.player.seeking.take() {
                    if let Some(player) = &self.services.player {
                        player.seek(position);
                    }
                    self.player.position = position;
                }
                Task::none()
            }
            Message::Artwork(video_id, artwork) => {
                if let Some(artwork) = artwork {
                    self.artwork = Some((video_id, artwork));
                }
                Task::none()
            }
            Message::ToggleNowPlaying => {
                if self.now_playing {
                    self.close_now_playing()
                } else {
                    self.now_playing = self.player.song().is_some();
                    Task::none()
                }
            }
            Message::ToggleQueue => {
                self.queue_open = !self.queue_open;
                if !self.queue_open {
                    return Task::none();
                }
                // Open on the playing song, a few rows below the top.
                let y = (self.player.index.saturating_sub(2)) as f32 * ui::TRACK_ROW;
                scroll_to(
                    now_playing::QUEUE_SCROLL,
                    scrollable::AbsoluteOffset { x: 0.0, y },
                )
            }
            Message::QueueScrolled(viewport) => {
                self.queue_list.update(viewport);
                Task::none()
            }
            Message::Dismiss => {
                if self.queue_open {
                    self.queue_open = false;
                    Task::none()
                } else {
                    self.close_now_playing()
                }
            }
            Message::Previous => {
                if self.player.position > RESTART_AFTER || self.player.index == 0 {
                    if let Some(player) = &self.services.player {
                        player.seek(0.0);
                    }
                    self.player.position = 0.0;
                    Task::none()
                } else {
                    self.player.index -= 1;
                    self.start()
                }
            }
            Message::Tick => {
                let was = self.player.state;
                self.read_player();
                // A song that played to its end: on to the next one.
                if was == PlayState::Playing
                    && self.player.state == PlayState::Stopped
                    && !self.player.starting
                {
                    self.next()
                } else {
                    Task::none()
                }
            }
            Message::Snapshot(shot) => {
                let path = std::env::var("WOOF_SNAPSHOT").expect("set when the snapshot runs");
                if let Err(error) = save_png(&path, &shot) {
                    eprintln!("WOOF snapshot failed: {error}");
                }
                window::latest().and_then(window::close)
            }
        }
    }

    /// Closes Now Playing and puts the page below back where it was scrolled:
    /// it left the widget tree, and with it its scroll position.
    fn close_now_playing(&mut self) -> Task<Message> {
        if !self.now_playing {
            return Task::none();
        }
        self.now_playing = false;
        scroll_page_to(self.stack.last().map_or(0.0, |entry| entry.offset))
    }

    /// Starts loading what the page on screen needs, unless it is loaded or loading.
    fn enter(&mut self) -> Task<Message> {
        match self.route().clone() {
            Route::Home | Route::Library => Task::none(),
            Route::Search => focus(pages::SEARCH_INPUT),
            Route::Explore => {
                if self.explore.is_some() {
                    return Task::none();
                }
                self.explore = Some(Load::Loading);
                Task::perform(self.services.clone().load_explore(), Message::ExploreLoaded)
            }
            route => {
                let Some(key) = route.key() else {
                    return Task::none();
                };
                if self.details.contains_key(&key) {
                    return Task::none();
                }
                self.details.insert(key.clone(), Load::Loading);
                Task::perform(self.services.clone().load_detail(route), move |result| {
                    Message::DetailLoaded(key.clone(), result.map(Arc::new))
                })
            }
        }
    }

    fn play(&mut self, songs: Arc<Vec<SongItem>>, index: usize) -> Task<Message> {
        self.player.queue = songs;
        self.player.index = index;
        self.start()
    }

    fn next(&mut self) -> Task<Message> {
        if !self.player.has_next() {
            return Task::none();
        }
        self.player.index += 1;
        self.start()
    }

    /// Resolves the stream of the queue's current song; `Resolved` plays it.
    fn start(&mut self) -> Task<Message> {
        let Some(song) = self.player.song() else {
            return Task::none();
        };
        let video_id = song.video_id.clone();
        let cover = song.thumbnail_url.clone();
        self.player.request += 1;
        let request = self.player.request;
        self.player.starting = true;
        self.player.error = None;
        self.player.position = 0.0;
        self.player.duration = 0.0;
        self.player.seeking = None;
        let artwork = match cover.clone() {
            Some(url) if self.artwork.as_ref().map(|(id, _)| id) != Some(&video_id) => {
                let id = video_id.clone();
                Task::perform(self.services.clone().artwork(url), move |artwork| {
                    Message::Artwork(id.clone(), artwork)
                })
            }
            _ => Task::none(),
        };
        Task::batch([
            Task::perform(self.services.clone().resolve(video_id), move |result| {
                Message::Resolved(request, result)
            }),
            self.fetch_covers(cover),
            artwork,
        ])
    }

    /// Starts downloading the covers not cached or already requested.
    fn fetch_covers(&mut self, urls: impl IntoIterator<Item = String>) -> Task<Message> {
        let mut tasks = Vec::new();
        for url in urls {
            if self.covers.contains_key(&url) || tasks.iter().any(|(u, _)| u == &url) {
                continue;
            }
            let task = Task::perform(
                self.services
                    .clone()
                    .cover(url.clone(), (ui::TILE * 2.0) as i64),
                {
                    let url = url.clone();
                    move |handle| Message::Cover(url.clone(), handle)
                },
            );
            tasks.push((url, task));
        }
        Task::batch(tasks.into_iter().map(|(_, task)| task))
    }

    fn read_player(&mut self) {
        if let Some(player) = &self.services.player {
            self.player.state = player.play_state();
            self.player.position = player.current_time();
            self.player.duration = player.duration();
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            window::resize_events().map(|_| Message::Resized),
            keyboard::listen().filter_map(shortcut),
        ];
        if self.player.song().is_some() {
            subscriptions
                .push(iced::time::every(Duration::from_millis(500)).map(|_| Message::Tick));
        }
        Subscription::batch(subscriptions)
    }

    /// The artwork of the playing song, once it has loaded.
    fn current_artwork(&self) -> Option<&Artwork> {
        let song = self.player.song()?;
        self.artwork
            .as_ref()
            .filter(|(id, _)| *id == song.video_id)
            .map(|(_, artwork)| artwork)
    }

    fn view(&self) -> Element<'_, Message> {
        let now_playing = self.player.song().filter(|_| self.now_playing);
        let (backdrop, page) = match now_playing {
            Some(song) => (
                now_playing::background(self.current_artwork()),
                now_playing::view(&self.covers, self.current_artwork(), song, self.queue_open),
            ),
            None => (ui::wash(), self.page()),
        };
        let mut layers = stack![backdrop, page];
        // Scrolled content fades out under the back and window buttons.
        let scrolled = match self.route() {
            Route::Library => self.library.offset_y,
            _ => self.stack.last().map_or(0.0, |entry| entry.offset),
        } > 0.0;
        if now_playing.is_none() && scrolled {
            layers = layers.push(ui::scroll_edge());
        }
        let mut layers = layers.push(sidebar(&self.stack[0].route));
        if self.queue_open && self.player.song().is_some() {
            layers = layers.push(now_playing::queue(
                &self.covers,
                &self.queue_list,
                &self.player.queue,
                self.player.index,
            ));
        }
        let mut layers = layers
            .push(self.player_bar())
            .push(top_strip(
                self.maximized,
                self.stack.len() > 1 || now_playing.is_some(),
            ))
            .width(Length::Fill)
            .height(Length::Fill);
        if !self.maximized {
            layers = layers.push(title_bar::resize_edges().map(Message::TitleBar));
        }
        layers.into()
    }

    fn page(&self) -> Element<'_, Message> {
        let covers = &self.covers;
        let playing = self.player.song().map(|song| song.video_id.as_str());
        match self.route() {
            Route::Home => pages::home(covers, &self.home, playing),
            Route::Library => pages::library(&self.library),
            Route::Search => pages::search(covers, &self.query, self.results.as_ref(), playing),
            Route::Explore => match &self.explore {
                Some(Load::Loaded(sections)) => pages::explore(covers, sections, playing),
                Some(Load::Failed(error)) => ui::status(Some(error), Message::Retry),
                _ => ui::status(None, Message::Retry),
            },
            route => match route.key().and_then(|key| self.details.get(&key)) {
                Some(Load::Loaded(detail)) => match detail.as_ref() {
                    Detail::Album { info, songs } => pages::album(covers, info, songs, playing),
                    Detail::Artist {
                        info,
                        songs,
                        shelves,
                    } => pages::artist(covers, info, songs, shelves, playing),
                    Detail::Playlist { info, songs } => {
                        pages::playlist(covers, info, songs, playing)
                    }
                    Detail::Podcast { info, songs } => pages::podcast(covers, info, songs, playing),
                    Detail::Mood(sections) => pages::explore(covers, sections, playing),
                },
                Some(Load::Failed(error)) => ui::status(Some(error), Message::Retry),
                _ => ui::status(None, Message::Retry),
            },
        }
    }

    fn player_bar(&self) -> Element<'_, Message> {
        let player = &self.player;
        let now_playing: Element<'_, Message> = match player.song() {
            Some(song) => row![
                ui::cover(
                    &self.covers,
                    song.thumbnail_url.as_deref(),
                    ui::ROW_COVER,
                    8.0
                ),
                column![
                    text(ui::fit(&song.title, ui::SONG_TEXT, 14.0))
                        .size(14)
                        .font(theme::weight(Weight::Medium))
                        .wrapping(text::Wrapping::None),
                    text(match (&player.error, player.starting) {
                        (Some(error), _) => ui::fit(error, ui::SONG_TEXT, 13.0),
                        (None, true) => "Loading…".to_owned(),
                        (None, false) => ui::fit(&ui::artists(&song.artists), ui::SONG_TEXT, 13.0),
                    })
                    .size(13)
                    .color(theme::SECONDARY)
                    .wrapping(text::Wrapping::None),
                ]
                .spacing(2)
                .clip(true),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
            None => text("Pick a song to play").color(theme::SECONDARY).into(),
        };
        let now_playing = button(now_playing)
            .padding(4)
            .on_press_maybe(player.song().map(|_| Message::ToggleNowPlaying))
            .style(|_, status| button::Style {
                background: (status == button::Status::Hovered)
                    .then_some(Background::Color(theme::HOVER)),
                border: Border::default().rounded(12),
                text_color: theme::TEXT,
                ..button::Style::default()
            });
        let has_song = player.song().is_some();
        let playing = player.state == PlayState::Playing;
        let transport = row![
            transport_button(icons::PREVIOUS, 22.0, has_song.then_some(Message::Previous)),
            transport_button(
                if playing { icons::PAUSE } else { icons::PLAY },
                30.0,
                has_song.then_some(Message::TogglePlay),
            ),
            transport_button(
                icons::NEXT,
                22.0,
                player.has_next().then_some(Message::Next)
            ),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let position = player.seeking.unwrap_or(player.position);
        let time = |seconds: f64| {
            text(ui::clock(seconds))
                .size(12)
                .color(theme::SECONDARY)
                .width(36)
        };
        let seek: Element<'_, Message> = if player.duration > 0.0 {
            row![
                time(position),
                slider(0.0..=player.duration, position, Message::Seeking)
                    .on_release(Message::Seek)
                    .step(0.5)
                    .style(seek_style),
                time(player.duration).align_x(Alignment::End),
            ]
            .spacing(8)
            .align_y(Alignment::Center)
            .into()
        } else {
            Space::new().width(Length::Fill).into()
        };
        let queue = transport_button(icons::QUEUE, 20.0, has_song.then_some(Message::ToggleQueue))
            .style(|_, status| button::Style {
                background: (self.queue_open || status == button::Status::Hovered)
                    .then_some(Background::Color(theme::PRESSED)),
                border: Border::default().rounded(22),
                ..button::Style::default()
            });
        let content = row![
            container(now_playing).width(Length::FillPortion(1)),
            // Transport takes its own width, so the seek slider gets the rest
            // of a narrow window; equal sides keep it centred.
            transport,
            row![seek, queue]
                .spacing(14)
                .align_y(Alignment::Center)
                .width(Length::FillPortion(1)),
        ]
        .spacing(16)
        .padding([0, 14])
        .height(Length::Fill)
        .align_y(Alignment::Center);
        container(ui::glass_panel(
            2,
            24.0,
            Length::Fill,
            Length::Fixed(theme::PLAYER_HEIGHT),
            content,
        ))
        .padding(Padding {
            top: 0.0,
            right: theme::INSET,
            bottom: theme::INSET,
            left: theme::INSET + theme::SIDEBAR_WIDTH + theme::INSET,
        })
        .align_bottom(Length::Fill)
        .into()
    }
}

/// Keys that act anywhere, unless a text field took them: Space plays or
/// pauses, Escape closes the queue or Now Playing, Alt+Left goes back.
fn shortcut(event: keyboard::Event) -> Option<Message> {
    use keyboard::key::Named;
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    match key.as_ref() {
        keyboard::Key::Named(Named::Space) => Some(Message::TogglePlay),
        keyboard::Key::Named(Named::Escape) => Some(Message::Dismiss),
        keyboard::Key::Named(Named::ArrowLeft) if modifiers.alt() => Some(Message::Back),
        _ => None,
    }
}

/// Thin white rail, filled part solid; the knob shows on hover and drag.
fn seek_style(_: &iced::Theme, status: slider::Status) -> slider::Style {
    let knob = match status {
        slider::Status::Active => 0.0,
        slider::Status::Hovered | slider::Status::Dragged => 6.0,
    };
    slider::Style {
        rail: slider::Rail {
            backgrounds: (
                Background::Color(theme::TEXT),
                Background::Color(Color {
                    a: 0.2,
                    ..theme::TEXT
                }),
            ),
            width: 4.0,
            border: Border::default().rounded(2),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: knob },
            background: Background::Color(theme::TEXT),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

fn load_home(services: &Arc<Services>) -> Task<Message> {
    Task::perform(services.clone().load_home(), Message::HomeLoaded)
}

fn loaded<T>(result: Result<T, String>) -> Load<T> {
    match result {
        Ok(value) => Load::Loaded(value),
        Err(error) => Load::Failed(error),
    }
}

fn scroll_page_to(offset: f32) -> Task<Message> {
    scroll_to(
        ui::PAGE_SCROLL,
        scrollable::AbsoluteOffset { x: 0.0, y: offset },
    )
}

fn explore_covers(sections: &[ExploreSection]) -> Vec<String> {
    sections
        .iter()
        .flat_map(|section| section.items.iter().take(12))
        .filter_map(|item| item.thumbnail_url().map(str::to_owned))
        .collect()
}

/// Dev aid: WOOF_PAGE opens a page over Home at start: `library`, `explore`,
/// `search`, `album:ID`, `artist:ID`, `playlist:ID` or `podcast:ID`.
fn dev_route() -> Option<Route> {
    let page = std::env::var("WOOF_PAGE").ok()?;
    match page.split_once(':') {
        Some(("album", id)) => Some(Route::Album(id.to_owned())),
        Some(("artist", id)) => Some(Route::Artist(id.to_owned())),
        Some(("playlist", id)) => Some(Route::Playlist(id.to_owned())),
        Some(("podcast", id)) => Some(Route::Podcast(id.to_owned())),
        _ => match page.as_str() {
            "library" => Some(Route::Library),
            "explore" => Some(Route::Explore),
            "search" => Some(Route::Search),
            _ => None,
        },
    }
}

/// Dev aid: WOOF_SIZE=880x560 sets the window size.
fn dev_size() -> Option<Size> {
    let size = std::env::var("WOOF_SIZE").ok()?;
    let (width, height) = size.split_once('x')?;
    Some(Size::new(width.parse().ok()?, height.parse().ok()?))
}

/// Dev aid: WOOF_SNAPSHOT=/path.png saves the window contents after
/// WOOF_SNAPSHOT_DELAY ms (default 1500) and quits.
fn snapshot_task() -> Task<Message> {
    if std::env::var("WOOF_SNAPSHOT").is_err() {
        return Task::none();
    }
    let delay = std::env::var("WOOF_SNAPSHOT_DELAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    Task::future(tokio::time::sleep(Duration::from_millis(delay)))
        .then(|_| window::latest())
        .and_then(window::screenshot)
        .map(Message::Snapshot)
}

/// Round icon button of the player bar; dimmed when there is nothing to do.
fn transport_button<'a>(
    glyph: &'static [u8],
    size: f32,
    message: Option<Message>,
) -> button::Button<'a, Message> {
    // The SVG tint ignores alpha, so a disabled glyph fades with `opacity`.
    let opacity = if message.is_some() { 1.0_f32 } else { 0.3 };
    button(ui::icon(glyph, size).opacity(opacity))
        .padding(6)
        .on_press_maybe(message)
        .style(|_, status| button::Style {
            background: (status == button::Status::Hovered)
                .then_some(Background::Color(theme::HOVER)),
            border: Border::default().rounded(22),
            ..button::Style::default()
        })
}

fn sidebar<'a>(root: &Route) -> Element<'a, Message> {
    let item = |(route, label): (Route, &'static str)| {
        let selected = *root == route;
        button(
            text(label)
                .size(15)
                .font(theme::weight(Weight::Medium))
                .color(if selected {
                    theme::TEXT
                } else {
                    theme::SECONDARY
                }),
        )
        .padding([8, 12])
        .width(Length::Fill)
        .on_press(Message::Tab(route))
        .style(move |_, status| button::Style {
            background: if selected {
                Some(Background::Color(theme::PRESSED))
            } else if status == button::Status::Hovered {
                Some(Background::Color(theme::HOVER))
            } else {
                None
            },
            border: Border::default().rounded(10),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
    };
    let brand = row![
        svg(svg::Handle::from_memory(
            include_bytes!("../assets/woofmusic.svg").as_slice()
        ))
        .width(22)
        .height(22),
        text("WoofMusic").size(15).font(theme::weight(Weight::Bold)),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let content = column![
        title_bar::drag_area(
            container(brand)
                .padding([0, 6])
                .center_y(title_bar::HEIGHT - 8.0)
        )
        .map(Message::TitleBar),
    ]
    .extend(Route::TABS.map(item).map(Element::from))
    .spacing(2)
    .padding(10);
    container(ui::glass_panel(
        1,
        22.0,
        Length::Fixed(theme::SIDEBAR_WIDTH),
        Length::Fill,
        content,
    ))
    .padding(theme::INSET)
    .into()
}

/// The top strip right of the sidebar: the back button, the window drag
/// area and the window buttons.
fn top_strip<'a>(maximized: bool, can_go_back: bool) -> Element<'a, Message> {
    // The back glyph lines up with the page content below it.
    const BACK_PAD: f32 = 6.0;
    let back: Element<'a, Message> = if can_go_back {
        button(ui::icon(icons::BACK, 20.0))
            .padding(BACK_PAD)
            .on_press(Message::Back)
            .style(|_, status| button::Style {
                background: match status {
                    button::Status::Hovered => Some(Background::Color(theme::HOVER)),
                    button::Status::Pressed => Some(Background::Color(theme::PRESSED)),
                    _ => None,
                },
                border: Border::default().rounded(16),
                ..button::Style::default()
            })
            .into()
    } else {
        Space::new().into()
    };
    row![
        Space::new().width(theme::CONTENT_LEFT - BACK_PAD),
        container(back).center_y(title_bar::HEIGHT),
        title_bar::drag_area(Space::new().width(Length::Fill).height(title_bar::HEIGHT))
            .map(Message::TitleBar),
        container(title_bar::buttons(maximized).map(Message::TitleBar)).center_y(title_bar::HEIGHT),
        Space::new().width(10),
    ]
    .height(title_bar::HEIGHT)
    .into()
}

fn save_png(path: &str, shot: &window::Screenshot) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, shot.size.width, shot.size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&shot.rgba)?;
    Ok(())
}

/// Glyphs, filled white; `ui::icon` tints them.
mod icons {
    pub const PLAY: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M7 4.8v14.4a1 1 0 0 0 1.5.86l11.6-7.2a1 1 0 0 0 0-1.72L8.5 3.94A1 1 0 0 0 7 4.8z" fill="#fff"/></svg>"##;
    pub const PAUSE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="6" y="4.5" width="4" height="15" rx="1.2" fill="#fff"/><rect x="14" y="4.5" width="4" height="15" rx="1.2" fill="#fff"/></svg>"##;
    pub const NEXT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M3 6.2v11.6a.8.8 0 0 0 1.2.7l8.6-5.8a.8.8 0 0 0 0-1.4L4.2 5.5a.8.8 0 0 0-1.2.7zM12 6.2v11.6a.8.8 0 0 0 1.2.7l8.6-5.8a.8.8 0 0 0 0-1.4l-8.6-5.8a.8.8 0 0 0-1.2.7z" fill="#fff"/></svg>"##;
    pub const PREVIOUS: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M21 6.2v11.6a.8.8 0 0 1-1.2.7l-8.6-5.8a.8.8 0 0 1 0-1.4l8.6-5.8a.8.8 0 0 1 1.2.7zM12 6.2v11.6a.8.8 0 0 1-1.2.7l-8.6-5.8a.8.8 0 0 1 0-1.4l8.6-5.8a.8.8 0 0 1 1.2.7z" fill="#fff"/></svg>"##;
    pub const QUEUE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><path d="M4 6h11M4 12h11M4 18h7"/></g><path d="M16 14.6v6.8a.6.6 0 0 0 .9.5l5.2-3.4a.6.6 0 0 0 0-1l-5.2-3.4a.6.6 0 0 0-.9.5z" fill="#fff"/></svg>"##;
    pub const BACK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M15 5l-7 7 7 7" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
}
