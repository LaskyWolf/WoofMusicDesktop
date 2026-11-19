//! The lyrics panel's extras: the "⋯" options menu (Swift `LyricsOptionsMenu`)
//! with its Edit Lyrics and Search Lyrics sheets, the karaoke greeting (`KaraokeCountdownOverlay`) and the lyric share card
//! (`LyricShareComposer`, `ShareCardStudio`), saved as a PNG.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::advanced::widget::Id;
use iced::advanced::widget::operation::{Operation, Outcome};
use iced::font::Weight;
use iced::widget::{
    Space, button, column, container, image, mouse_area, opaque, row, scrollable, slider, stack,
    text, text_editor, text_input, toggler,
};
use iced::{
    Alignment, Background, Border, Color, Element, Length, Padding, Rectangle, Shadow,
    Subscription, Task, Vector, window,
};
use trop::audio::lyrics::lyrics_manager::LyricSearchResult;
use trop::audio::lyrics::lyrics_view;
use trop::audio::lyrics::providers::lyrics_provider::LyricsParsing;
use trop::networking::models::yt_item::SongItem;
use trop::settings::settings_store::SettingsStore;
use trop::utilities::ui_image_color_extraction::{FALLBACK_COLORS, Rgb};

use crate::services::{Artwork, Lyrics, Services};
use crate::{theme, ui};

type Msg = crate::Message;

/// Swift `LyricShareComposer.maxLines`.
const MAX_LINES: usize = 4;
/// The card on screen; the saved PNG is this size times the display scale.
const MENU_WIDTH: f32 = 300.0;
const PICK_SCROLL: &str = "lyric-share-lines";

#[derive(Default)]
pub struct State {
    menu_open: bool,
    /// The karaoke switch, saved by the lyrics service.
    pub karaoke: bool,
    greeting: Option<Greeting>,
    composer: Option<Composer>,
    sheet: Option<Sheet>,
    /// "Copy Lyrics" was pressed for this song: its row shows "Copied".
    copied: Option<String>,
    /// The lyrics service's refresh token the shown lyrics belong to.
    refresh_token: u64,
    /// Dev aid WOOF_SHARE_CARD=pick or style: opens the share card once lyrics show.
    dev_share: Option<String>,
    /// Dev aid WOOF_LYRICS_SHEET=edit or search: opens that sheet once lyrics show.
    dev_sheet: Option<String>,
}

/// The karaoke start or stop greeting at the top of the lyrics.
struct Greeting {
    started: Instant,
    /// Dev aid: the animation held at this many seconds.
    frozen: Option<f32>,
    /// Karaoke after the change: on barks, off yawns.
    on: bool,
    /// Every third start brings the wolf.
    wolf: bool,
}

impl Greeting {
    fn time(&self) -> f32 {
        self.frozen
            .unwrap_or_else(|| self.started.elapsed().as_secs_f32())
    }

    /// About 2 s for the barks, 3 s for the goodbye (Swift `countdown` / `goodbye`).
    fn length(&self) -> f32 {
        if self.on { 2.1 } else { 3.0 }
    }
}

/// Edit Lyrics or Search Lyrics, over the whole window.
enum Sheet {
    /// Swift `EditLyricsSheet`: the lyrics as text with their timestamps.
    Edit {
        video_id: String,
        title: String,
        text: text_editor::Content,
    },
    Search(Search),
}

/// Swift `SearchLyricsSheet`.
struct Search {
    video_id: String,
    /// Used when the query names no artist.
    artist: String,
    duration: f64,
    query: String,
    searching: bool,
    /// `None` until a search ends.
    results: Option<Vec<LyricSearchResult>>,
    expanded: Option<u64>,
}

const SEARCH_INPUT: &str = "lyrics-search";

/// Swift `SearchLyricsSheet.search`: "Adele - Hello" searches Hello by
/// Adele; a title alone keeps the song's artist. `None` for an empty title.
pub fn split_query(raw: &str, artist: &str) -> Option<(String, String)> {
    let raw = raw.trim();
    if let Some((left, right)) = raw.split_once(" - ") {
        let (left, right) = (left.trim(), right.trim());
        if !left.is_empty() && !right.is_empty() {
            return Some((right.to_owned(), left.to_owned()));
        }
    }
    (!raw.is_empty()).then(|| (raw.to_owned(), artist.to_owned()))
}

struct Composer {
    title: String,
    artist: String,
    cover: Option<image::Handle>,
    colors: [Rgb; 2],
    lines: Vec<String>,
    starts: Vec<Option<f64>>,
    video_id: String,
    /// Picked line indices, in lyric order.
    picked: Vec<usize>,
    /// A fifth line was tapped: the counter turns red.
    full: bool,
}

#[derive(Debug, Clone)]
pub enum Message {
    ToggleMenu,
    CloseMenu,
    Refetch,
    Copy,
    EditLyrics,
    SearchLyrics,
    Edit(text_editor::Action),
    SaveEdit,
    Query(String),
    Search,
    SearchOnline,
    /// Results of a search for this query.
    Found(String, Vec<LyricSearchResult>),
    Expand(u64),
    Choose(u64),
    CloseSheet,
    Karaoke(bool),
    /// Redraws the greeting; it ends on its own.
    Frame,
    /// Reloads the lyrics when word-timed ones arrived in the background.
    CheckRefresh,
    ShareCard,
    Pick(usize),
    /// Opens the studio with the picked lines.
    Next,
    CloseComposer,
}

/// What `update` reads from the app.
pub struct Context<'a> {
    pub services: &'a Arc<Services>,
    pub song: Option<&'a SongItem>,
    pub lyrics: Option<&'a Lyrics>,
    pub artwork: Option<&'a Artwork>,
    pub time: f64,
    pub prefs: &'a SettingsStore,
}

impl State {
    pub fn new(services: &Services) -> Self {
        // Dev aids: WOOF_LYRICS_MENU=1 opens the options; WOOF_GREETING=1.2
        // (or -1.2 for the goodbye, w1.2 for the wolf) holds the greeting at 1.2 s.
        let greeting = std::env::var("WOOF_GREETING").ok().and_then(|value| {
            let wolf = value.starts_with('w');
            let value = value.trim_start_matches('w');
            let at: f32 = value.parse().ok()?;
            Some(Greeting {
                started: Instant::now(),
                frozen: Some(at.abs()),
                on: !value.starts_with('-'),
                wolf,
            })
        });
        Self {
            karaoke: services.karaoke_enabled(),
            refresh_token: services.lyrics_refresh_token(),
            menu_open: std::env::var("WOOF_LYRICS_MENU").is_ok(),
            greeting,
            dev_share: std::env::var("WOOF_SHARE_CARD").ok(),
            dev_sheet: std::env::var("WOOF_LYRICS_SHEET").ok(),
            ..Self::default()
        }
    }

    pub fn update(&mut self, message: Message, cx: Context<'_>) -> Task<Msg> {
        match message {
            Message::ToggleMenu => {
                self.menu_open = !self.menu_open;
                Task::none()
            }
            Message::CloseMenu => {
                self.menu_open = false;
                Task::none()
            }
            Message::Refetch => {
                self.menu_open = false;
                let Some(song) = cx.song.cloned() else {
                    return Task::none();
                };
                Task::future(cx.services.clone().refetch_lyrics(song))
                    .map(|()| crate::Message::RetryLyrics)
            }
            Message::Copy => {
                let Some(song) = cx.song else {
                    return Task::none();
                };
                match cx.services.lyrics_text(&song.video_id) {
                    Some(lyrics) => {
                        self.copied = Some(song.video_id.clone());
                        iced::clipboard::write(lyrics)
                    }
                    None => Task::none(),
                }
            }
            Message::EditLyrics => {
                self.menu_open = false;
                let Some(song) = cx.song else {
                    return Task::none();
                };
                let text = cx.services.editable_lyrics(&song.video_id);
                self.sheet = Some(Sheet::Edit {
                    video_id: song.video_id.clone(),
                    title: song.title.clone(),
                    text: text_editor::Content::with_text(&text),
                });
                Task::none()
            }
            Message::SearchLyrics => {
                self.menu_open = false;
                let Some(song) = cx.song else {
                    return Task::none();
                };
                self.sheet = Some(Sheet::Search(Search {
                    video_id: song.video_id.clone(),
                    artist: ui::artists(&song.artists),
                    duration: song.duration as f64,
                    query: song.title.clone(),
                    searching: false,
                    results: None,
                    expanded: None,
                }));
                iced::widget::operation::focus(SEARCH_INPUT)
            }
            Message::Edit(action) => {
                if let Some(Sheet::Edit { text, .. }) = &mut self.sheet {
                    text.perform(action);
                }
                Task::none()
            }
            Message::SaveEdit => {
                let Some(Sheet::Edit { video_id, text, .. }) = &self.sheet else {
                    return Task::none();
                };
                let text = text.text();
                if text.trim().is_empty() {
                    return Task::none();
                }
                cx.services.save_lyrics(video_id, &text, None);
                self.sheet = None;
                Task::done(crate::Message::RetryLyrics)
            }
            Message::Query(query) => {
                if let Some(Sheet::Search(search)) = &mut self.sheet {
                    search.query = query;
                }
                Task::none()
            }
            Message::Search => {
                let Some(Sheet::Search(search)) = &mut self.sheet else {
                    return Task::none();
                };
                let Some((title, artist)) = split_query(&search.query, &search.artist) else {
                    return Task::none();
                };
                search.searching = true;
                search.results = None;
                search.expanded = None;
                let query = search.query.clone();
                Task::perform(
                    cx.services
                        .clone()
                        .search_lyrics(title, artist, search.duration),
                    move |results| crate::Message::Lyrics(Message::Found(query.clone(), results)),
                )
            }
            Message::Found(query, results) => {
                if let Some(Sheet::Search(search)) = &mut self.sheet
                    && search.searching
                    && search.query == query
                {
                    search.searching = false;
                    search.results = Some(results);
                }
                Task::none()
            }
            Message::SearchOnline => {
                if let Some(Sheet::Search(search)) = &self.sheet {
                    let q = format!("{} lyrics", search.query.trim());
                    if let Ok(url) = reqwest::Url::parse_with_params(
                        "https://www.google.com/search",
                        [("q", q.trim())],
                    ) {
                        crate::comments::open_link(url.as_str());
                    }
                }
                Task::none()
            }
            Message::Expand(id) => {
                if let Some(Sheet::Search(search)) = &mut self.sheet {
                    search.expanded = (search.expanded != Some(id)).then_some(id);
                }
                Task::none()
            }
            Message::Choose(id) => {
                let Some(Sheet::Search(search)) = &self.sheet else {
                    return Task::none();
                };
                let Some(result) = search.results.iter().flatten().find(|r| r.id == id) else {
                    return Task::none();
                };
                let raw = LyricsParsing::serialize_lines(&result.lines);
                cx.services
                    .save_lyrics(&search.video_id, &raw, Some(&result.provider_name));
                self.sheet = None;
                Task::done(crate::Message::RetryLyrics)
            }
            Message::CloseSheet => {
                self.sheet = None;
                Task::none()
            }
            Message::Karaoke(on) => {
                self.karaoke = on;
                cx.services.set_karaoke(on, cx.song);
                let mut prefs = cx.prefs.clone();
                if on {
                    prefs.karaoke_greeting_count += 1;
                }
                self.greeting = Some(Greeting {
                    started: Instant::now(),
                    frozen: None,
                    on,
                    wolf: prefs.karaoke_greeting_count.is_multiple_of(3),
                });
                if on {
                    Task::done(crate::Message::Settings(crate::settings::Message::Change(
                        prefs,
                    )))
                } else {
                    Task::none()
                }
            }
            Message::Frame => {
                if self
                    .greeting
                    .as_ref()
                    .is_some_and(|g| g.frozen.is_none() && g.time() > g.length())
                {
                    self.greeting = None;
                }
                Task::none()
            }
            Message::CheckRefresh if cx.lyrics.is_some() && self.dev_sheet.is_some() => {
                match self.dev_sheet.take().as_deref() {
                    Some("search") => {
                        let open = self.update(Message::SearchLyrics, cx);
                        Task::batch([open, Task::done(crate::Message::Lyrics(Message::Search))])
                    }
                    _ => self.update(Message::EditLyrics, cx),
                }
            }
            Message::CheckRefresh if cx.lyrics.is_some() && self.dev_share.is_some() => {
                let step = self.dev_share.take();
                let open = self.update(Message::ShareCard, cx);
                if let Some(composer) = &mut self.composer
                    && step.as_deref() == Some("style")
                {
                    if composer.picked.is_empty() {
                        composer.picked = (0..composer.lines.len().min(2)).collect();
                    }
                    return Task::batch([open, Task::done(crate::Message::Lyrics(Message::Next))]);
                }
                open
            }
            Message::CheckRefresh => {
                let token = cx.services.lyrics_refresh_token();
                if token == self.refresh_token {
                    return Task::none();
                }
                self.refresh_token = token;
                Task::done(crate::Message::RetryLyrics)
            }
            Message::ShareCard => {
                self.menu_open = false;
                let (Some(song), Some(lyrics)) = (cx.song, cx.lyrics) else {
                    return Task::none();
                };
                let lines: Vec<_> = lyrics
                    .lines
                    .iter()
                    .filter(|line| !lyrics_view::is_instrumental_marker(line))
                    .collect();
                // The line playing now starts ticked (Swift `loadLinesIfNeeded`).
                let t = cx.time + cx.prefs.lyrics_offset_seconds;
                let current = lines
                    .iter()
                    .rposition(|line| line.start_time.is_some_and(|start| start <= t));
                self.composer = Some(Composer {
                    title: song.title.clone(),
                    artist: ui::artists(&song.artists),
                    cover: cx.artwork.map(|artwork| artwork.image.clone()),
                    colors: cx.artwork.map_or(FALLBACK_COLORS, |artwork| artwork.colors),
                    lines: lines.iter().map(|line| line.text.clone()).collect(),
                    starts: lines.iter().map(|line| line.start_time).collect(),
                    video_id: song.video_id.clone(),
                    picked: current.into_iter().collect(),
                    full: false,
                });
                // The ticked line in view, a little below the top (a one-line row is 40 px).
                let offset = current.map_or(0.0, |index| (index as f32 - 3.0).max(0.0) * 40.0);
                iced::widget::operation::scroll_to(
                    PICK_SCROLL,
                    scrollable::AbsoluteOffset { x: 0.0, y: offset },
                )
            }
            Message::Pick(index) => {
                if let Some(composer) = &mut self.composer {
                    if let Some(at) = composer.picked.iter().position(|&i| i == index) {
                        composer.picked.remove(at);
                        composer.full = false;
                    } else if composer.picked.len() < MAX_LINES {
                        composer.picked.push(index);
                        composer.picked.sort_unstable();
                    } else {
                        composer.full = true;
                    }
                }
                Task::none()
            }
            Message::Next => {
                let Some(composer) = &self.composer else {
                    return Task::none();
                };
                if composer.picked.is_empty() {
                    return Task::none();
                }
                let content = crate::share_card::Content {
                    lines: composer
                        .picked
                        .iter()
                        .map(|&i| composer.lines[i].clone())
                        .collect(),
                    title: composer.title.clone(),
                    artist: composer.artist.clone(),
                    cover: composer.cover.clone(),
                    colors: composer.colors,
                    start_time: composer.picked.first().and_then(|&i| composer.starts[i]),
                    video_id: Some(composer.video_id.clone()),
                };
                Task::done(crate::Message::OpenStudio(content, true))
            }
            Message::CloseComposer => {
                self.composer = None;
                Task::none()
            }
        }
    }

    /// `showing`: the lyrics are on screen.
    pub fn subscription(&self, showing: bool) -> Subscription<Msg> {
        let mut list = Vec::new();
        if self.greeting.is_some() {
            list.push(
                iced::time::every(Duration::from_millis(33))
                    .map(|_| crate::Message::Lyrics(Message::Frame)),
            );
        }
        if showing && (self.karaoke || self.dev_share.is_some() || self.dev_sheet.is_some()) {
            list.push(
                iced::time::every(Duration::from_secs(1))
                    .map(|_| crate::Message::Lyrics(Message::CheckRefresh)),
            );
        }
        Subscription::batch(list)
    }

    /// True while a sheet is open: Escape closes it.
    pub fn close_sheet(&mut self) -> bool {
        self.sheet.take().is_some()
    }

    /// The share card composer or a lyrics sheet over the whole window, while one is open.
    pub fn overlay(&self) -> Option<Element<'_, Msg>> {
        let (body, close) = match (&self.composer, &self.sheet) {
            (Some(composer), _) => (picker(composer), Message::CloseComposer),
            (None, Some(Sheet::Edit { title, text, .. })) => {
                (edit_sheet(title, text), Message::CloseSheet)
            }
            (None, Some(Sheet::Search(search))) => (search_sheet(search), Message::CloseSheet),
            (None, None) => return None,
        };
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
        .on_press(crate::Message::Lyrics(close));
        Some(opaque(stack![
            scrim,
            container(opaque(body)).center(Length::Fill)
        ]))
    }

    /// The lyrics header (the "⋯" button), the greeting, and the menu when it is open,
    /// over `lines`.
    pub fn around<'a>(
        &'a self,
        lines: Element<'a, Msg>,
        lyrics: Option<&'a Lyrics>,
        video_id: &'a str,
        prefs: &'a SettingsStore,
    ) -> Element<'a, Msg> {
        let more = button(ui::icon(MORE, 22.0))
            .padding(8)
            .on_press(crate::Message::Lyrics(Message::ToggleMenu))
            .style(move |_, status| button::Style {
                background: (self.menu_open || status == button::Status::Hovered)
                    .then_some(Background::Color(theme::PRESSED)),
                border: Border::default().rounded(20),
                ..button::Style::default()
            });
        let mut layers = stack![column![row![Space::new().width(Length::Fill), more], lines]];
        if let Some(greeting) = &self.greeting {
            layers = layers.push(
                container(greeting_view(greeting))
                    .width(Length::Fill)
                    .align_x(Alignment::Center),
            );
        }
        if self.menu_open {
            let catcher = mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
                .on_press(crate::Message::Lyrics(Message::CloseMenu));
            layers = layers.push(catcher).push(
                container(opaque(self.menu(lyrics, video_id, prefs)))
                    .padding([48, 0])
                    .align_right(Length::Fill),
            );
        }
        layers.into()
    }

    fn menu<'a>(
        &'a self,
        lyrics: Option<&'a Lyrics>,
        video_id: &str,
        prefs: &'a SettingsStore,
    ) -> Element<'a, Msg> {
        let has_lines = lyrics.is_some_and(|lyrics| !lyrics.lines.is_empty());
        let action = |label: &'a str, message: Option<Message>| -> Element<'a, Msg> {
            let enabled = message.is_some();
            button(text(label).size(14).font(theme::weight(Weight::Medium)))
                .width(Length::Fill)
                .padding([9, 14])
                .on_press_maybe(message.map(crate::Message::Lyrics))
                .style(move |_, status| button::Style {
                    background: (status == button::Status::Hovered)
                        .then_some(Background::Color(theme::HOVER)),
                    border: Border::default().rounded(10),
                    text_color: if enabled {
                        theme::TEXT
                    } else {
                        Color {
                            a: 0.4,
                            ..theme::TEXT
                        }
                    },
                    ..button::Style::default()
                })
                .into()
        };
        let change = |edit: fn(&mut SettingsStore, bool)| {
            move |on| {
                let mut prefs = prefs.clone();
                edit(&mut prefs, on);
                crate::Message::Settings(crate::settings::Message::Change(prefs))
            }
        };
        let offset = prefs.lyrics_offset_seconds;
        let set_offset = move |value: f64| {
            crate::Message::Settings(crate::settings::Message::Change(SettingsStore {
                lyrics_offset_seconds: ((value * 100.0).round() / 100.0).clamp(-3.0, 3.0),
                ..prefs.clone()
            }))
        };
        let step = |glyph, delta: f64| {
            crate::transport_button(glyph, 16.0, Some(set_offset(offset + delta)))
        };
        let mut offset_title = row![
            text("Lyric Offset")
                .size(14)
                .font(theme::weight(Weight::Medium))
                .width(Length::Fill),
            text(prefs.offset_description())
                .size(13)
                .color(theme::SECONDARY),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        if offset != 0.0 {
            offset_title = offset_title.push(
                button(text("Reset").size(13).color(theme::accent()))
                    .padding([2, 6])
                    .on_press(set_offset(0.0))
                    .style(|_, _| button::Style::default()),
            );
        }
        let offset = column![
            offset_title,
            row![
                step(MINUS, -0.05),
                slider(-3.0..=3.0, offset.clamp(-3.0, 3.0), set_offset)
                    .step(0.1)
                    .style(crate::seek_style),
                step(PLUS, 0.05),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        ]
        .spacing(6)
        .padding([8, 14]);
        let karaoke_note = (self.karaoke
            && lyrics.is_some_and(|lyrics| !lyrics.lines.iter().any(|l| l.words.is_some())))
        .then(|| {
            text("No word-timed lyrics found")
                .size(12)
                .color(theme::SECONDARY)
        });
        let copied = self.copied.as_deref() == Some(video_id);
        let list = column![
            action("Edit Lyrics…", Some(Message::EditLyrics)),
            action("Search Lyrics…", Some(Message::SearchLyrics)),
            action("Refetch Lyrics", Some(Message::Refetch)),
            action(
                if copied { "Copied" } else { "Copy Lyrics" },
                has_lines.then_some(Message::Copy),
            ),
            action("Share Card…", has_lines.then_some(Message::ShareCard)),
            divider(),
            offset,
            switch_row(
                "Show Interval Indicator",
                None,
                prefs.show_interval_indicator,
                change(|p, on| p.show_interval_indicator = on),
            ),
            switch_row(
                "Romanize Current Track",
                None,
                prefs.romanize_current_track,
                change(|p, on| p.romanize_current_track = on),
            ),
            switch_row("Karaoke", karaoke_note, self.karaoke, |on| {
                crate::Message::Lyrics(Message::Karaoke(on))
            }),
        ]
        .spacing(2)
        .padding(6);
        // A solid panel: the lyrics under it would show through glass.
        container(list)
            .width(MENU_WIDTH)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgba8(0x1C, 0x1C, 0x1E, 0.97))),
                border: Border::default().rounded(18).width(1).color(theme::HOVER),
                shadow: Shadow {
                    color: Color {
                        a: 0.4,
                        ..Color::BLACK
                    },
                    offset: Vector::new(0.0, 12.0),
                    blur_radius: 32.0,
                },
                ..container::Style::default()
            })
            .into()
    }
}

/// A sheet's panel: solid, rounded, with a title row.
fn sheet_panel<'a>(
    title: &str,
    trailing: Element<'a, Msg>,
    body: Element<'a, Msg>,
) -> Element<'a, Msg> {
    let header = row![
        text(ui::fit(title, 380.0, 18.0))
            .size(18)
            .font(theme::weight(Weight::Bold))
            .wrapping(text::Wrapping::None)
            .width(Length::Fill),
        trailing,
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    container(column![header, body].spacing(16))
        .width(560)
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
        })
        .into()
}

/// A quiet text button of a sheet's header.
fn sheet_button<'a>(label: &'a str, message: Option<Message>, strong: bool) -> Element<'a, Msg> {
    let enabled = message.is_some();
    button(text(label).size(14).font(theme::weight(if strong {
        Weight::Bold
    } else {
        Weight::Medium
    })))
    .padding([7, 14])
    .on_press_maybe(message.map(crate::Message::Lyrics))
    .style(move |_, status| button::Style {
        background: Some(Background::Color(match (strong, status) {
            (true, _) if enabled => theme::accent(),
            (_, button::Status::Hovered) => theme::PRESSED,
            _ => theme::HOVER,
        })),
        border: Border::default().rounded(16),
        text_color: match (strong && enabled, enabled) {
            (true, _) => theme::BACKGROUND,
            (_, true) => theme::TEXT,
            _ => Color {
                a: 0.4,
                ..theme::TEXT
            },
        },
        ..button::Style::default()
    })
    .into()
}

/// Swift `EditLyricsSheet`: the lyrics as editable text, timestamps included.
fn edit_sheet<'a>(title: &'a str, content: &'a text_editor::Content) -> Element<'a, Msg> {
    let can_save = !content.text().trim().is_empty();
    let buttons = row![
        sheet_button("Cancel", Some(Message::CloseSheet), false),
        sheet_button("Save", can_save.then_some(Message::SaveEdit), true),
    ]
    .spacing(8);
    let editor = text_editor(content)
        .on_action(|action| crate::Message::Lyrics(Message::Edit(action)))
        .size(14)
        .height(440)
        .padding(12)
        .style(|_, status| {
            let focused = matches!(status, text_editor::Status::Focused { .. });
            text_editor::Style {
                // Solid: the editor draws in its own layer, over which glass would show the page.
                background: Background::Color(Color::from_rgb8(0x2A, 0x2A, 0x2D)),
                border: Border::default().rounded(12).width(1).color(if focused {
                    theme::accent()
                } else {
                    Color::TRANSPARENT
                }),
                placeholder: theme::SECONDARY,
                value: theme::TEXT,
                selection: Color {
                    a: 0.35,
                    ..theme::accent()
                },
            }
        });
    sheet_panel(title, buttons.into(), editor.into())
}

/// Swift `SearchLyricsSheet`: a query, then every provider's match.
fn search_sheet(search: &Search) -> Element<'_, Msg> {
    let field = text_input("Song title — e.g. Adele - Hello", &search.query)
        .id(SEARCH_INPUT)
        .on_input(|query| crate::Message::Lyrics(Message::Query(query)))
        .on_submit(crate::Message::Lyrics(Message::Search))
        .size(14)
        .padding([10, 14])
        .style(ui::field_style);
    let buttons = row![
        sheet_button("Cancel", Some(Message::CloseSheet), false),
        Space::new().width(Length::Fill),
        sheet_button("Search Online", Some(Message::SearchOnline), false),
        sheet_button(
            "Search",
            (!search.searching).then_some(Message::Search),
            true
        ),
    ]
    .spacing(8);
    let mut body = column![field, buttons].spacing(12);
    let note = |label: &'static str| {
        container(text(label).size(14).color(theme::SECONDARY))
            .center_x(Length::Fill)
            .padding([24, 0])
    };
    match &search.results {
        _ if search.searching => body = body.push(note("Searching…")),
        Some(results) if results.is_empty() => body = body.push(note("Lyrics not found")),
        Some(results) => {
            let rows = results
                .iter()
                .map(|result| result_row(result, search.expanded));
            body = body.push(
                scrollable(column(rows).spacing(4).padding(Padding::ZERO.right(16)))
                    .direction(theme::scrollbar())
                    .style(theme::scroll_style)
                    .height(Length::Shrink),
            );
        }
        None => {}
    }
    sheet_panel(
        "Search Lyrics",
        Space::new().into(),
        container(body).max_height(520.0).into(),
    )
}

/// A result: two lines of preview (all of them when expanded), the provider
/// and a mark when the lines are timed. A click keeps these lyrics.
fn result_row(result: &LyricSearchResult, expanded: Option<u64>) -> Element<'_, Msg> {
    let open = expanded == Some(result.id);
    let preview = if open {
        result
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        // Swift's `lineLimit(2)`: each preview line kept to one row.
        result
            .preview_text()
            .lines()
            .map(|line| ui::fit(line, 440.0, 14.0))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let mut caption = row![
        text(result.provider_name.clone())
            .size(12)
            .font(theme::weight(Weight::Medium))
            .color(theme::SECONDARY)
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    if result.is_synced() {
        caption = caption.push(text("Synced").size(12).color(theme::accent()));
    }
    let chevron = button(ui::icon(
        if open {
            crate::comments::icons::CHEVRON_UP
        } else {
            crate::comments::icons::CHEVRON_DOWN
        },
        16.0,
    ))
    .padding(6)
    .on_press(crate::Message::Lyrics(Message::Expand(result.id)))
    .style(|_, status| button::Style {
        background: (status == button::Status::Hovered)
            .then_some(Background::Color(theme::PRESSED)),
        border: Border::default().rounded(14),
        ..button::Style::default()
    });
    let content = row![
        column![text(preview).size(14), caption]
            .spacing(6)
            .width(Length::Fill),
        chevron,
    ]
    .spacing(8);
    ui::clickable(
        content,
        Some(crate::Message::Lyrics(Message::Choose(result.id))),
        10.0,
    )
}

fn divider<'a>() -> Element<'a, Msg> {
    let line = container(Space::new().width(Length::Fill).height(1)).style(|_| container::Style {
        background: Some(Background::Color(theme::PRESSED)),
        ..container::Style::default()
    });
    container(line).padding([6, 8]).into()
}

fn switch_row<'a>(
    label: &'a str,
    note: Option<text::Text<'a>>,
    on: bool,
    message: impl Fn(bool) -> Msg + 'a,
) -> Element<'a, Msg> {
    let title = column![text(label).size(14).font(theme::weight(Weight::Medium))].push(note);
    row![
        title.width(Length::Fill),
        toggler(on)
            .on_toggle(message)
            .size(20)
            .style(|theme, status| {
                let on = matches!(
                    status,
                    toggler::Status::Active { is_toggled: true }
                        | toggler::Status::Hovered { is_toggled: true }
                );
                toggler::Style {
                    background: Background::Color(if on {
                        theme::accent()
                    } else {
                        theme::PRESSED
                    }),
                    foreground: Background::Color(if on { theme::BACKGROUND } else { theme::TEXT }),
                    background_border_width: 0.0,
                    foreground_border_width: 0.0,
                    ..toggler::default(theme, status)
                }
            }),
    ]
    .spacing(12)
    .padding([8, 14])
    .align_y(Alignment::Center)
    .into()
}

/// Swift `KaraokeCountdownOverlay` as a function of time: the dog (or wolf)
/// pops up, cocks its head, barks three times and drops away; on stop it
/// rises, yawns and a 💤 drifts up.
fn greeting_view<'a>(greeting: &Greeting) -> Element<'a, Msg> {
    let t = greeting.time();
    let ease = |x: f32| 1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3);
    let (lift, bubbles): (f32, Vec<(&str, f32)>) = if greeting.on {
        let lift = if t < 1.8 {
            ease(t / 0.4)
        } else {
            1.0 - ease((t - 1.8) / 0.3)
        };
        let words = if greeting.wolf {
            ["a-", "woo", "AWOOO!"]
        } else {
            ["woof!", "woof!", "WOOF!"]
        };
        let starts = if greeting.wolf {
            [0.9, 1.2, 1.5]
        } else {
            [0.7, 1.0, 1.3]
        };
        // Each bark pops in, floats up and fades over 0.6 s.
        let bubbles = words
            .into_iter()
            .zip(starts)
            .map(|(word, start)| (word, (t - start) / 0.6))
            .collect();
        (lift, bubbles)
    } else {
        let lift = if t < 2.4 {
            ease(t / 0.5)
        } else {
            1.0 - ease((t - 2.4) / 0.6)
        };
        let sleep = if greeting.wolf { "🌙" } else { "💤" };
        (lift, vec![(sleep, (t - 0.8) / 1.8)])
    };
    let mascot = text(if greeting.wolf { "🐺" } else { "🐶" }).size(12.0 + 40.0 * lift);
    let bubble_row = row(bubbles.into_iter().map(|(word, progress)| {
        let shown = (0.0..1.0).contains(&progress);
        let alpha = if progress < 0.6 {
            1.0
        } else {
            1.0 - (progress - 0.6) / 0.4
        };
        let rise = 26.0 * ease(progress);
        let label = text(word)
            .size(if word.chars().any(char::is_uppercase) {
                17
            } else {
                13
            })
            .font(theme::weight(Weight::Black))
            .color(Color {
                a: alpha,
                ..Color::BLACK
            });
        let bubble = container(label)
            .padding([4, 10])
            .style(move |_| container::Style {
                background: Some(Background::Color(Color {
                    a: alpha,
                    ..Color::WHITE
                })),
                border: Border::default().rounded(14),
                ..container::Style::default()
            });
        // Keeps its slot while hidden, so the others do not jump.
        let slot: Element<'a, Msg> = if shown {
            bubble.into()
        } else {
            Space::new().into()
        };
        container(slot)
            .height(Length::Fixed(56.0))
            .padding(iced::Padding::ZERO.bottom(rise))
            .align_bottom(Length::Fixed(56.0))
            .width(Length::Fixed(74.0))
            .align_x(Alignment::Center)
            .into()
    }));
    column![
        bubble_row,
        container(mascot)
            .height(Length::Fixed(56.0))
            .align_bottom(Length::Fixed(56.0)),
    ]
    .align_x(Alignment::Center)
    .into()
}

/// Step 1 (Swift `pickStep`): the lines with a tick each, up to four.
fn picker(composer: &Composer) -> Element<'_, Msg> {
    let counter = text(format!("{}/{MAX_LINES}", composer.picked.len()))
        .size(15)
        .font(theme::weight(Weight::Bold))
        .color(if composer.full {
            theme::CLOSE
        } else {
            theme::SECONDARY
        });
    let header = row![
        ui::pill_button("Cancel", crate::Message::Lyrics(Message::CloseComposer)),
        column![
            text("Share Card")
                .size(18)
                .font(theme::weight(Weight::Bold)),
            counter,
        ]
        .align_x(Alignment::Center)
        .width(Length::Fill),
        ui::accent_button(
            "Next",
            None,
            (!composer.picked.is_empty()).then_some(crate::Message::Lyrics(Message::Next)),
        ),
    ]
    .align_y(Alignment::Center)
    .padding([16, 18]);
    let lines = column(composer.lines.iter().enumerate().map(|(index, line)| {
        let on = composer.picked.contains(&index);
        let mark: Element<'_, Msg> = if on {
            container(ui::icon(TICK, 16.0).style(|_, _| iced::widget::svg::Style {
                color: Some(theme::BACKGROUND),
            }))
            .center(22)
            .style(|_| container::Style {
                background: Some(Background::Color(theme::accent())),
                border: Border::default().rounded(11),
                ..container::Style::default()
            })
            .into()
        } else {
            ui::icon(CIRCLE, 22.0)
                .style(|_, _| iced::widget::svg::Style {
                    color: Some(Color {
                        a: 0.35,
                        ..theme::TEXT
                    }),
                })
                .into()
        };
        button(
            row![
                mark,
                text(line.as_str())
                    .size(15)
                    .font(theme::weight(if on {
                        Weight::Semibold
                    } else {
                        Weight::Normal
                    }))
                    .color(if on { theme::TEXT } else { theme::SECONDARY }),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .width(Length::Fill)
        .padding([9, 14])
        .on_press(crate::Message::Lyrics(Message::Pick(index)))
        .style(|_, status| button::Style {
            background: (status == button::Status::Hovered)
                .then_some(Background::Color(theme::HOVER)),
            border: Border::default().rounded(10),
            ..button::Style::default()
        })
        .into()
    }))
    .padding([0, 8]);
    let list = scrollable(lines)
        .id(PICK_SCROLL)
        .direction(theme::scrollbar())
        .style(theme::scroll_style)
        .height(Length::Fill);
    ui::glass_panel(
        11,
        24.0,
        Length::Fixed(560.0),
        Length::Fixed(620.0),
        column![header, list].padding(iced::Padding::ZERO.bottom(10)),
    )
}

/// Finds the bounds of the container with `target` as its id.
pub(crate) fn find_bounds(target: Id) -> impl Operation<Rectangle> {
    struct Find {
        target: Id,
        found: Option<Rectangle>,
    }
    impl Operation<Rectangle> for Find {
        fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
            if id == Some(&self.target) {
                self.found = Some(bounds);
            }
        }
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Rectangle>)) {
            operate(self);
        }
        fn finish(&self) -> Outcome<Rectangle> {
            self.found.map_or(Outcome::None, Outcome::Some)
        }
    }
    Find {
        target,
        found: None,
    }
}

/// Cuts the card out of the window screenshot and writes it as a PNG in the
/// Pictures folder; a taken name gets " 2", " 3"…
pub(crate) fn save_card(
    shot: &window::Screenshot,
    bounds: Option<Rectangle>,
    name: &str,
) -> Result<PathBuf, String> {
    let bounds = bounds.ok_or("the card was not on screen")?;
    let scale = shot.scale_factor;
    let region = Rectangle {
        x: (bounds.x * scale).round() as u32,
        y: (bounds.y * scale).round() as u32,
        width: (bounds.width * scale).round() as u32,
        height: (bounds.height * scale).round() as u32,
    };
    let card = shot.crop(region).map_err(|error| format!("{error:?}"))?;
    let folder = dirs::picture_dir()
        .or_else(dirs::home_dir)
        .ok_or("there is no Pictures folder")?
        .join("WoofMusic");
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let clean: String = name
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) { '-' } else { c })
        .collect();
    let path = (1..)
        .map(|n| match n {
            1 => folder.join(format!("{clean}.png")),
            n => folder.join(format!("{clean} {n}.png")),
        })
        .find(|path| !path.exists())
        .expect("some name is free");
    crate::save_png(&path.to_string_lossy(), &card).map_err(|error| error.to_string())?;
    Ok(path)
}

const MORE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="#fff"><circle cx="5.5" cy="12" r="1.8"/><circle cx="12" cy="12" r="1.8"/><circle cx="18.5" cy="12" r="1.8"/></g></svg>"##;
const MINUS: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 12h12" stroke="#fff" stroke-width="2.2" stroke-linecap="round"/></svg>"##;
const PLUS: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 12h12M12 6v12" stroke="#fff" stroke-width="2.2" stroke-linecap="round"/></svg>"##;
const CIRCLE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="9" fill="none" stroke="#fff" stroke-width="1.8"/></svg>"##;
const TICK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 7.5" fill="none" stroke="#fff" stroke-width="2.8" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;

#[cfg(test)]
mod tests {
    use super::split_query;

    #[test]
    fn a_query_with_a_dash_names_the_artist() {
        let q = |raw| split_query(raw, "Song Artist");
        assert_eq!(q("Adele - Hello"), Some(("Hello".into(), "Adele".into())));
        assert_eq!(q("  Hello "), Some(("Hello".into(), "Song Artist".into())));
        assert_eq!(q("- Hello"), Some(("- Hello".into(), "Song Artist".into())));
        assert_eq!(q("   "), None);
    }
}
