//! WoofMusic Desktop: window, pages and player bar (see docs/MILESTONES.md).

mod glass;
mod services;
mod theme;
mod title_bar;
mod virtual_list;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{Space, button, column, container, image, row, scrollable, stack, svg, text};
use iced::{
    Alignment, Background, Border, Color, ContentFit, Element, Gradient, Length, Padding, Radians,
    Size, Subscription, Task, window,
};
use trop::audio::player_controller::player_controller::State as PlayState;
use trop::networking::models::home_page::HomeSection;
use trop::networking::models::yt_item::{SongItem, YTItem};

use services::Services;

const APP_ID: &str = "com.laskywolf.woofmusic";

/// Cover sizes, logical px. Covers are fetched at twice the size for HiDPI.
const TILE: f32 = 168.0;
const ROW_COVER: f32 = 48.0;
/// Width of the text next to a row cover.
const SONG_TEXT: f32 = 236.0;

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
            size: Size::new(1280.0, 800.0),
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

struct App {
    services: Arc<Services>,
    maximized: bool,
    page: Page,
    home: Load<Vec<HomeSection>>,
    covers: HashMap<String, image::Handle>,
    library: virtual_list::State,
    player: Player,
}

/// What the player bar shows.
#[derive(Default)]
struct Player {
    song: Option<SongItem>,
    state: PlayState,
    position: f64,
    duration: f64,
    /// The song is being resolved; the bar shows it before audio starts.
    starting: bool,
    error: Option<String>,
}

enum Load<T> {
    Loading,
    Loaded(T),
    Failed(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Home,
    Explore,
    Library,
    Search,
}

impl Page {
    const ALL: [Page; 4] = [Page::Home, Page::Explore, Page::Library, Page::Search];

    fn label(self) -> &'static str {
        match self {
            Page::Home => "Home",
            Page::Explore => "Explore",
            Page::Library => "Library",
            Page::Search => "Search",
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    TitleBar(title_bar::Message),
    Resized,
    Maximized(bool),
    Open(Page),
    LibraryScrolled(scrollable::Viewport),
    HomeLoaded(Result<Vec<HomeSection>, String>),
    RetryHome,
    Cover(String, Option<image::Handle>),
    Play(Box<SongItem>),
    Played(Result<(), String>),
    TogglePlay,
    Tick,
    Snapshot(window::Screenshot),
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let services = Services::new();
        let page = match std::env::var("WOOF_PAGE").as_deref() {
            Ok("library") => Page::Library,
            _ => Page::Home,
        };
        let app = Self {
            services: services.clone(),
            maximized: false,
            page,
            home: Load::Loading,
            covers: HashMap::new(),
            library: virtual_list::State::default(),
            player: Player::default(),
        };
        (app, Task::batch([load_home(&services), snapshot_task()]))
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
            Message::Open(page) => {
                self.page = page;
                Task::none()
            }
            Message::LibraryScrolled(viewport) => {
                self.library.update(viewport);
                Task::none()
            }
            Message::HomeLoaded(Ok(sections)) => {
                // Dev aid: WOOF_PLAY=1 plays the first song of Home.
                let autoplay = std::env::var("WOOF_PLAY").is_ok().then(|| {
                    sections
                        .iter()
                        .flat_map(|s| s.items())
                        .find_map(|item| match item {
                            YTItem::Song(song) => Some(song.clone()),
                            _ => None,
                        })
                });
                let urls = sections
                    .iter()
                    .flat_map(|section| section.items().iter().take(section_limit(section)))
                    .filter_map(|item| item.thumbnail_url().map(str::to_owned))
                    .collect::<Vec<_>>();
                self.home = Load::Loaded(sections);
                let covers = self.fetch_covers(urls);
                match autoplay.flatten() {
                    Some(song) => Task::batch([covers, Task::done(Message::Play(Box::new(song)))]),
                    None => covers,
                }
            }
            Message::HomeLoaded(Err(error)) => {
                self.home = Load::Failed(error);
                Task::none()
            }
            Message::RetryHome => {
                self.home = Load::Loading;
                load_home(&self.services)
            }
            Message::Cover(url, handle) => {
                if let Some(handle) = handle {
                    self.covers.insert(url, handle);
                }
                Task::none()
            }
            Message::Play(song) => {
                let cover = song.thumbnail_url.clone();
                let video_id = song.video_id.clone();
                self.player.song = Some(*song);
                self.player.starting = true;
                self.player.error = None;
                self.player.position = 0.0;
                self.player.duration = 0.0;
                Task::batch([
                    Task::perform(self.services.clone().play(video_id), Message::Played),
                    self.fetch_covers(cover),
                ])
            }
            Message::Played(result) => {
                self.player.starting = false;
                self.player.error = result.err();
                Task::none()
            }
            Message::TogglePlay => {
                if let Some(player) = &self.services.player {
                    player.toggle_play_pause();
                }
                self.read_player();
                Task::none()
            }
            Message::Tick => {
                self.read_player();
                Task::none()
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
                    .cover(url.clone(), (TILE * 2.0) as i64),
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
        let mut subscriptions = vec![window::resize_events().map(|_| Message::Resized)];
        if self.player.song.is_some() {
            subscriptions
                .push(iced::time::every(Duration::from_millis(500)).map(|_| Message::Tick));
        }
        Subscription::batch(subscriptions)
    }

    fn view(&self) -> Element<'_, Message> {
        let mut layers = stack![
            wash(),
            match self.page {
                Page::Library => library(&self.library),
                _ => self.home_page(),
            },
            sidebar(self.page),
            self.player_bar(),
            top_strip(self.maximized),
        ]
        .width(Length::Fill)
        .height(Length::Fill);
        if !self.maximized {
            layers = layers.push(title_bar::resize_edges().map(Message::TitleBar));
        }
        layers.into()
    }

    fn home_page(&self) -> Element<'_, Message> {
        let sections = match &self.home {
            Load::Loading => return centered(text("Loading Home…").color(theme::SECONDARY)),
            Load::Failed(error) => {
                return centered(
                    column![
                        text("Home could not load")
                            .size(18)
                            .font(theme::weight(Weight::Bold)),
                        text(error.as_str()).size(13).color(theme::SECONDARY),
                        pill_button("Try again", Message::RetryHome),
                    ]
                    .spacing(10)
                    .align_x(Alignment::Center),
                );
            }
            Load::Loaded(sections) => sections,
        };
        let sections = sections
            .iter()
            .filter(|section| !section.items().is_empty())
            .map(|section| {
                let items = section.items().iter().take(section_limit(section));
                let body: Element<'_, Message> = if section.is_songs_only() {
                    iced::widget::Row::with_children(items.map(|item| self.song_cell(item)))
                        .spacing(12)
                        .wrap()
                        .vertical_spacing(4)
                        .into()
                } else {
                    iced::widget::Row::with_children(items.map(|item| self.tile(item)))
                        .spacing(20)
                        .wrap()
                        .vertical_spacing(24)
                        .into()
                };
                column![
                    text(section.display_title())
                        .size(22)
                        .font(theme::weight(Weight::Bold)),
                    body,
                ]
                .spacing(14)
                .into()
            })
            .collect::<Vec<Element<'_, Message>>>();
        scrollable(column(sections).spacing(40).padding(CONTENT_PADDING))
            .direction(theme::scrollbar())
            .style(theme::scroll_style)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Album, playlist, artist… tile: cover with title and subtitle below.
    fn tile<'a>(&'a self, item: &'a YTItem) -> Element<'a, Message> {
        let round = matches!(item, YTItem::Artist(_));
        let content = column![
            self.cover(
                item.thumbnail_url(),
                TILE,
                if round { TILE / 2.0 } else { 10.0 }
            ),
            text(fit(item.title(), TILE, 14.0))
                .size(14)
                .font(theme::weight(Weight::Medium))
                .wrapping(text::Wrapping::None),
            text(fit(&subtitle(item), TILE, 13.0))
                .size(13)
                .color(theme::SECONDARY)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(4)
        .width(TILE)
        .clip(true);
        match item {
            YTItem::Song(song) => row_button(content, Message::Play(Box::new(song.clone())), 6.0),
            _ => content.into(),
        }
    }

    /// Song in a songs-only section (Quick picks): a compact row.
    fn song_cell<'a>(&'a self, item: &'a YTItem) -> Element<'a, Message> {
        let content = row![
            self.cover(item.thumbnail_url(), ROW_COVER, 6.0),
            column![
                text(fit(item.title(), SONG_TEXT, 14.0))
                    .size(14)
                    .font(theme::weight(Weight::Medium))
                    .wrapping(text::Wrapping::None),
                text(fit(&subtitle(item), SONG_TEXT, 13.0))
                    .size(13)
                    .color(theme::SECONDARY)
                    .wrapping(text::Wrapping::None),
            ]
            .spacing(2)
            .clip(true),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .width(300);
        match item {
            YTItem::Song(song) => row_button(content, Message::Play(Box::new(song.clone())), 8.0),
            _ => content.into(),
        }
    }

    /// The cover once downloaded, a dark square until then.
    fn cover<'a>(&'a self, url: Option<&str>, size: f32, radius: f32) -> Element<'a, Message> {
        match url.and_then(|url| self.covers.get(url)) {
            Some(handle) => image(handle.clone())
                .width(size)
                .height(size)
                .content_fit(ContentFit::Cover)
                .border_radius(radius)
                .into(),
            None => container(Space::new())
                .width(size)
                .height(size)
                .style(move |_| container::Style {
                    background: Some(Background::Color(theme::PLACEHOLDER)),
                    border: Border::default().rounded(radius),
                    ..container::Style::default()
                })
                .into(),
        }
    }

    fn player_bar(&self) -> Element<'_, Message> {
        let player = &self.player;
        let now_playing: Element<'_, Message> = match &player.song {
            Some(song) => row![
                self.cover(song.thumbnail_url.as_deref(), ROW_COVER, 8.0),
                column![
                    text(fit(&song.title, SONG_TEXT, 14.0))
                        .size(14)
                        .font(theme::weight(Weight::Medium))
                        .wrapping(text::Wrapping::None),
                    text(match (&player.error, player.starting) {
                        (Some(error), _) => error.clone(),
                        (None, true) => "Loading…".to_owned(),
                        (None, false) => fit(&artists(&song.artists), SONG_TEXT, 13.0),
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
        let playing = player.state == PlayState::Playing;
        let transport = row![
            icon(icons::PREVIOUS, 22.0),
            button(icon(if playing { icons::PAUSE } else { icons::PLAY }, 30.0))
                .padding(4)
                .on_press_maybe(player.song.as_ref().map(|_| Message::TogglePlay))
                .style(|_, status| button::Style {
                    background: (status == button::Status::Hovered)
                        .then_some(Background::Color(theme::HOVER)),
                    border: Border::default().rounded(20),
                    ..button::Style::default()
                }),
            icon(icons::NEXT, 22.0),
        ]
        .spacing(20)
        .align_y(Alignment::Center);
        let time = (player.duration > 0.0).then(|| {
            text(format!(
                "{} / {}",
                clock(player.position),
                clock(player.duration)
            ))
            .size(13)
            .color(theme::SECONDARY)
        });
        let content = row![
            container(now_playing).width(Length::FillPortion(1)),
            container(transport).center_x(Length::FillPortion(1)),
            container(time).align_right(Length::FillPortion(1)),
        ]
        .padding([0, 18])
        .height(Length::Fill)
        .align_y(Alignment::Center);
        container(glass_panel(
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

fn load_home(services: &Arc<Services>) -> Task<Message> {
    Task::perform(services.clone().load_home(), Message::HomeLoaded)
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

/// Items shown per section: three rows of song cells, or two rows of tiles on a wide window.
fn section_limit(section: &HomeSection) -> usize {
    if section.is_songs_only() { 12 } else { 10 }
}

fn subtitle(item: &YTItem) -> String {
    match item {
        YTItem::Song(song) => artists(&song.artists),
        YTItem::Album(album) => artists(&album.artists),
        YTItem::Artist(_) => "Artist".to_owned(),
        YTItem::Playlist(_) => "Playlist".to_owned(),
        YTItem::Podcast(_) => "Podcast".to_owned(),
        YTItem::Episode(_) => "Episode".to_owned(),
    }
}

fn artists(artists: &[trop::networking::models::yt_item::YTArtist]) -> String {
    artists
        .iter()
        .map(|artist| artist.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Shortens `text` with an ellipsis so it fits `width` at font `size`.
// ponytail: estimates Inter's average glyph width (0.55 em) instead of
// measuring; wide glyphs (W, CJK) can still clip. Measure with the renderer's
// paragraph if that shows.
fn fit(text: &str, width: f32, size: f32) -> String {
    let max = (width / (size * 0.55)).floor() as usize;
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn centered<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(CONTENT_PADDING)
        .center(Length::Fill)
        .into()
}

fn icon<'a>(bytes: &'static [u8], size: f32) -> svg::Svg<'a> {
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(|_, _| svg::Style {
            color: Some(theme::TEXT),
        })
}

/// A clickable area with the hover and press wash.
fn row_button<'a>(
    content: impl Into<Element<'a, Message>>,
    message: Message,
    pad: f32,
) -> Element<'a, Message> {
    button(content)
        .padding(pad)
        .on_press(message)
        .style(|_, status| button::Style {
            background: match status {
                button::Status::Hovered => Some(Background::Color(theme::HOVER)),
                button::Status::Pressed => Some(Background::Color(theme::PRESSED)),
                _ => None,
            },
            border: Border::default().rounded(12),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
        .into()
}

fn pill_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(14).font(theme::weight(Weight::Medium)))
        .padding([8, 16])
        .on_press(message)
        .style(|_, status| button::Style {
            background: Some(Background::Color(if status == button::Status::Hovered {
                theme::PRESSED
            } else {
                theme::HOVER
            })),
            border: Border::default().rounded(18),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
        .into()
}

/// A faint wash behind everything.
fn wash<'a>() -> Element<'a, Message> {
    let (from, to) = theme::PALETTES[4];
    let gradient = Linear::new(Radians(2.4))
        .add_stop(0.0, Color { a: 0.30, ..from })
        .add_stop(0.55, Color { a: 0.08, ..to })
        .add_stop(1.0, Color::TRANSPARENT);
    container(Space::new())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(gradient))),
            ..container::Style::default()
        })
        .into()
}

/// Space around the scrolling content: below the title strip, left of the
/// sidebar, above the player bar.
const CONTENT_PADDING: Padding = Padding {
    top: title_bar::HEIGHT + 8.0,
    right: theme::CONTENT_RIGHT,
    bottom: theme::PLAYER_HEIGHT + 2.0 * theme::INSET + 32.0,
    left: theme::CONTENT_LEFT,
};

/// Library demo: 5,000 song rows in a virtualized list (real library in R5).
fn library(state: &virtual_list::State) -> Element<'_, Message> {
    const ROW_HEIGHT: f32 = 56.0;
    virtual_list::view(
        state,
        5000,
        ROW_HEIGHT,
        CONTENT_PADDING,
        |i| demo_song_row(i, ROW_HEIGHT),
        Message::LibraryScrolled,
    )
}

fn demo_song_row<'a>(index: usize, height: f32) -> Element<'a, Message> {
    let seconds = 150 + (index * 37) % 180;
    let (from, to) = theme::PALETTES[index % theme::PALETTES.len()];
    let gradient = Linear::new(Radians(0.8))
        .add_stop(0.0, from)
        .add_stop(1.0, to);
    let swatch = container(Space::new())
        .width(40)
        .height(40)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(gradient))),
            border: Border::default().rounded(6),
            ..container::Style::default()
        });
    let content = row![
        swatch,
        column![
            text(format!("Song {}", index + 1))
                .size(14)
                .font(theme::weight(Weight::Medium)),
            text("Artist").size(13).color(theme::SECONDARY),
        ]
        .spacing(2)
        .width(Length::Fill),
        text(clock(seconds as f64)).size(13).color(theme::SECONDARY),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .height(height);
    row_button(content, Message::Open(Page::Library), 0.0)
}

/// A glass panel: the glass under `content`, both filling `width` × `height`.
fn glass_panel<'a>(
    id: u64,
    radius: f32,
    width: Length,
    height: Length,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    stack![glass::glass(id, radius, theme::GLASS_TINT), content.into()]
        .width(width)
        .height(height)
        .into()
}

fn sidebar<'a>(current: Page) -> Element<'a, Message> {
    let item = |page: Page| {
        let selected = page == current;
        button(
            text(page.label())
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
        .on_press(Message::Open(page))
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
    .extend(Page::ALL.map(item).map(Element::from))
    .spacing(2)
    .padding(10);
    container(glass_panel(
        1,
        22.0,
        Length::Fixed(theme::SIDEBAR_WIDTH),
        Length::Fill,
        content,
    ))
    .padding(theme::INSET)
    .into()
}

/// The top strip right of the sidebar: moves the window, holds the window buttons.
fn top_strip<'a>(maximized: bool) -> Element<'a, Message> {
    Element::from(
        row![
            Space::new().width(theme::INSET + theme::SIDEBAR_WIDTH),
            title_bar::drag_area(Space::new().width(Length::Fill).height(title_bar::HEIGHT)),
            container(title_bar::buttons(maximized)).center_y(title_bar::HEIGHT),
            Space::new().width(10),
        ]
        .height(title_bar::HEIGHT),
    )
    .map(Message::TitleBar)
}

fn save_png(path: &str, shot: &window::Screenshot) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, shot.size.width, shot.size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&shot.rgba)?;
    Ok(())
}

/// Transport glyphs, filled, coloured by the bar.
mod icons {
    pub const PLAY: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M7 4.8v14.4a1 1 0 0 0 1.5.86l11.6-7.2a1 1 0 0 0 0-1.72L8.5 3.94A1 1 0 0 0 7 4.8z" fill="#fff"/></svg>"##;
    pub const PAUSE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="6" y="4.5" width="4" height="15" rx="1.2" fill="#fff"/><rect x="14" y="4.5" width="4" height="15" rx="1.2" fill="#fff"/></svg>"##;
    pub const NEXT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M3 6.2v11.6a.8.8 0 0 0 1.2.7l8.6-5.8a.8.8 0 0 0 0-1.4L4.2 5.5a.8.8 0 0 0-1.2.7zM12 6.2v11.6a.8.8 0 0 0 1.2.7l8.6-5.8a.8.8 0 0 0 0-1.4l-8.6-5.8a.8.8 0 0 0-1.2.7z" fill="#fff"/></svg>"##;
    pub const PREVIOUS: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M21 6.2v11.6a.8.8 0 0 1-1.2.7l-8.6-5.8a.8.8 0 0 1 0-1.4l8.6-5.8a.8.8 0 0 1 1.2.7zM12 6.2v11.6a.8.8 0 0 1-1.2.7l-8.6-5.8a.8.8 0 0 1 0-1.4l8.6-5.8a.8.8 0 0 1 1.2.7z" fill="#fff"/></svg>"##;
}
