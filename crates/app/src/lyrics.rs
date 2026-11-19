//! The lyrics panel's extras: the "⋯" options menu (Swift `LyricsOptionsMenu`),
//! the karaoke greeting (`KaraokeCountdownOverlay`) and the lyric share card
//! (`LyricShareComposer`, `ShareCardStudio`), saved as a PNG.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::advanced::widget::Id;
use iced::advanced::widget::operation::{Operation, Outcome};
use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{
    Space, button, column, container, image, mouse_area, opaque, row, scrollable, slider, stack,
    text, toggler,
};
use iced::{
    Alignment, Background, Border, Color, Element, Gradient, Length, Radians, Rectangle, Shadow,
    Subscription, Task, Vector, window,
};
use trop::audio::lyrics::lyrics_view;
use trop::networking::models::yt_item::SongItem;
use trop::settings::settings_store::SettingsStore;
use trop::utilities::ui_image_color_extraction::{FALLBACK_COLORS, Rgb};

use crate::services::{Artwork, Lyrics, Services};
use crate::{theme, ui};

type Msg = crate::Message;

/// Swift `LyricShareComposer.maxLines`.
const MAX_LINES: usize = 4;
/// The card on screen; the saved PNG is this size times the display scale.
const CARD_WIDTH: f32 = 324.0;
const CARD_HEIGHT: f32 = CARD_WIDTH * 16.0 / 9.0;
const CARD_ID: &str = "lyric-share-card";
const MENU_WIDTH: f32 = 300.0;
const PICK_SCROLL: &str = "lyric-share-lines";

#[derive(Default)]
pub struct State {
    menu_open: bool,
    /// The karaoke switch, saved by the lyrics service.
    pub karaoke: bool,
    greeting: Option<Greeting>,
    composer: Option<Composer>,
    /// "Copy Lyrics" was pressed for this song: its row shows "Copied".
    copied: Option<String>,
    /// The lyrics service's refresh token the shown lyrics belong to.
    refresh_token: u64,
    /// Dev aid WOOF_SHARE_CARD=pick, style or save: opens the share card once lyrics show.
    dev_share: Option<String>,
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

struct Composer {
    title: String,
    artist: String,
    cover: Option<image::Handle>,
    colors: [Rgb; 2],
    lines: Vec<String>,
    /// Picked line indices, in lyric order.
    picked: Vec<usize>,
    /// Second step: the card preview.
    styling: bool,
    /// Which of the three colour orders the card uses.
    palette: usize,
    /// A fifth line was tapped: the counter turns red.
    full: bool,
    saving: bool,
    saved: Option<Result<PathBuf, String>>,
}

#[derive(Debug, Clone)]
pub enum Message {
    ToggleMenu,
    CloseMenu,
    Refetch,
    Copy,
    Karaoke(bool),
    /// Redraws the greeting; it ends on its own.
    Frame,
    /// Reloads the lyrics when word-timed ones arrived in the background.
    CheckRefresh,
    ShareCard,
    Pick(usize),
    Next,
    Back,
    Palette(usize),
    CloseComposer,
    Save,
    Shot(window::Screenshot, Option<Rectangle>),
    Saved(Result<PathBuf, String>),
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
            Message::CheckRefresh if cx.lyrics.is_some() && self.dev_share.is_some() => {
                let step = self.dev_share.take();
                let open = self.update(Message::ShareCard, cx);
                if let Some(composer) = &mut self.composer
                    && matches!(step.as_deref(), Some("style" | "save"))
                {
                    if composer.picked.is_empty() {
                        composer.picked = (0..composer.lines.len().min(2)).collect();
                    }
                    composer.styling = true;
                }
                if step.as_deref() == Some("save") {
                    let save = Task::future(tokio::time::sleep(Duration::from_secs(1)))
                        .map(|()| crate::Message::Lyrics(Message::Save));
                    return Task::batch([open, save]);
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
                    picked: current.into_iter().collect(),
                    styling: false,
                    palette: 0,
                    full: false,
                    saving: false,
                    saved: None,
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
                if let Some(composer) = &mut self.composer {
                    composer.styling = !composer.picked.is_empty();
                }
                Task::none()
            }
            Message::Back => {
                if let Some(composer) = &mut self.composer {
                    composer.styling = false;
                    composer.saved = None;
                }
                Task::none()
            }
            Message::Palette(palette) => {
                if let Some(composer) = &mut self.composer {
                    composer.palette = palette;
                    composer.saved = None;
                }
                Task::none()
            }
            Message::CloseComposer => {
                self.composer = None;
                Task::none()
            }
            Message::Save => {
                let Some(composer) = &mut self.composer else {
                    return Task::none();
                };
                composer.saving = true;
                composer.saved = None;
                // What is on screen is what is saved: a screenshot cut to the card.
                window::latest()
                    .and_then(window::screenshot)
                    .then(|shot| {
                        iced::advanced::widget::operate(find_bounds(Id::new(CARD_ID)))
                            .map(Some)
                            .chain(Task::done(None))
                            .collect()
                            .map(move |found| {
                                Message::Shot(shot.clone(), found.into_iter().flatten().next())
                            })
                    })
                    .map(crate::Message::Lyrics)
            }
            Message::Shot(shot, bounds) => {
                let Some(composer) = &self.composer else {
                    return Task::none();
                };
                let name = format!("{} – {}", composer.title, composer.artist);
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || save_card(&shot, bounds, &name))
                            .await
                            .map_err(|error| error.to_string())?
                    },
                    |result| crate::Message::Lyrics(Message::Saved(result)),
                )
            }
            Message::Saved(result) => {
                if let Some(composer) = &mut self.composer {
                    composer.saving = false;
                    composer.saved = Some(result);
                }
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
        if showing && (self.karaoke || self.dev_share.is_some()) {
            list.push(
                iced::time::every(Duration::from_secs(1))
                    .map(|_| crate::Message::Lyrics(Message::CheckRefresh)),
            );
        }
        Subscription::batch(list)
    }

    /// The share card composer over the whole window, while it is open.
    pub fn overlay(&self) -> Option<Element<'_, Msg>> {
        let composer = self.composer.as_ref()?;
        let body = if composer.styling {
            studio(composer)
        } else {
            picker(composer)
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
        .on_press(crate::Message::Lyrics(Message::CloseComposer));
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

/// Step 2 (Swift `ShareCardStudio`): the card, its colours, and Save Image.
fn studio(composer: &Composer) -> Element<'_, Msg> {
    let swatch = |index: usize| {
        let [a, b] = palette(composer.colors, index);
        let selected = composer.palette == index;
        button(Space::new().width(36).height(36))
            .padding(0)
            .on_press(crate::Message::Lyrics(Message::Palette(index)))
            .style(move |_, _| button::Style {
                background: Some(Background::Gradient(Gradient::Linear(
                    Linear::new(Radians(2.4)).add_stop(0.0, a).add_stop(1.0, b),
                ))),
                border: Border::default()
                    .rounded(18)
                    .width(if selected { 3 } else { 1 })
                    .color(if selected {
                        theme::TEXT
                    } else {
                        theme::PRESSED
                    }),
                ..button::Style::default()
            })
    };
    let status: Element<'_, Msg> = match &composer.saved {
        Some(Ok(path)) => text(format!("Saved to {}", path.display()))
            .size(13)
            .color(theme::SECONDARY)
            .into(),
        Some(Err(error)) => text(format!("The image was not saved: {error}"))
            .size(13)
            .color(theme::CLOSE)
            .into(),
        None => Space::new().into(),
    };
    let side = column![
        text("Style").size(22).font(theme::weight(Weight::Bold)),
        text(ui::fit(&composer.title, 230.0, 13.0))
            .size(13)
            .color(theme::SECONDARY),
        Space::new().height(12),
        text("Colours").size(14).font(theme::weight(Weight::Medium)),
        row![swatch(0), swatch(1), swatch(2)].spacing(10),
        Space::new().height(Length::Fill),
        status,
        ui::accent_button(
            if composer.saving {
                "Saving…"
            } else {
                "Save Image"
            },
            None,
            (!composer.saving).then_some(crate::Message::Lyrics(Message::Save)),
        ),
        row![
            ui::pill_button("Back", crate::Message::Lyrics(Message::Back)),
            ui::pill_button("Done", crate::Message::Lyrics(Message::CloseComposer)),
        ]
        .spacing(8),
    ]
    .spacing(8)
    .width(Length::Fixed(230.0))
    .height(Length::Fixed(CARD_HEIGHT));
    let content = row![card(composer), side].spacing(28).padding(24);
    // The glass needs a fixed size: the card, the side column and the gaps.
    ui::glass_panel(
        11,
        24.0,
        Length::Fixed(CARD_WIDTH + 230.0 + 28.0 + 48.0),
        Length::Fixed(CARD_HEIGHT + 48.0),
        content,
    )
}

/// The card's three colours, rotated so each palette leads with a different one
/// (Swift `ShareCardContent.palette`).
fn palette(colors: [Rgb; 2], index: usize) -> [Color; 2] {
    let rgb = |[r, g, b]: Rgb| Color::from_rgb(r as f32, g as f32, b as f32);
    let (a, b) = (rgb(colors[0]), rgb(colors[1]));
    let deep = Color::from_rgb(a.r * 0.35, a.g * 0.35, a.b * 0.35);
    match index % 3 {
        0 => [a, b],
        1 => [b, deep],
        _ => [deep, a],
    }
}

/// The 9:16 card: the cover's colours with a glow, the cover as a taped
/// photo with the title under it, the picked lines on paper strips, and the
/// WoofMusic credit (Swift `ShareCard` with the Cover Glow, Polaroid and Paper looks).
fn card(composer: &Composer) -> Element<'_, Msg> {
    let [top, bottom] = palette(composer.colors, composer.palette);
    let base = Linear::new(Radians(std::f32::consts::PI))
        .add_stop(0.0, top)
        .add_stop(1.0, bottom);
    let glow = Linear::new(Radians(2.2))
        .add_stop(
            0.0,
            Color {
                a: 0.5,
                ..Color::WHITE
            },
        )
        .add_stop(0.45, Color::TRANSPARENT);
    let photo_side = 150.0;
    let photo_image: Element<'_, Msg> = match &composer.cover {
        Some(cover) => image(cover.clone())
            .width(photo_side)
            .height(photo_side)
            .content_fit(iced::ContentFit::Cover)
            .into(),
        None => container(ui::icon(crate::icons::LYRICS, 48.0))
            .center(photo_side)
            .style(move |_| container::Style {
                background: Some(Background::Color(bottom)),
                ..container::Style::default()
            })
            .into(),
    };
    let photo = container(
        column![
            photo_image,
            text(ui::fit(&composer.title, photo_side, 13.0))
                .size(13)
                .font(theme::weight(Weight::Bold))
                .color(Color::from_rgb8(0x22, 0x22, 0x22)),
            text(ui::fit(&composer.artist, photo_side, 11.0))
                .size(11)
                .color(Color::from_rgb8(0x55, 0x55, 0x55)),
        ]
        .spacing(4),
    )
    .padding(iced::Padding::new(8.0).bottom(12.0))
    .style(|_| container::Style {
        background: Some(Background::Color(Color::from_rgb8(0xFA, 0xF8, 0xF2))),
        border: Border::default().rounded(3),
        shadow: Shadow {
            color: Color {
                a: 0.35,
                ..Color::BLACK
            },
            offset: Vector::new(0.0, 8.0),
            blur_radius: 18.0,
        },
        ..container::Style::default()
    });
    let lines = column(composer.picked.iter().map(|&index| {
        container(
            text(composer.lines[index].as_str())
                .size(17)
                .font(theme::weight(Weight::Bold))
                .color(Color::from_rgb8(0x1A, 0x1A, 0x1A)),
        )
        .padding([6, 10])
        .style(|_| container::Style {
            background: Some(Background::Color(Color::from_rgb8(0xFA, 0xF8, 0xF2))),
            border: Border::default().rounded(2),
            ..container::Style::default()
        })
        .into()
    }))
    .spacing(6);
    let credit = row![
        text("WoofMusic")
            .size(12)
            .font(theme::weight(Weight::Bold))
            .color(Color {
                a: 0.85,
                ..Color::WHITE
            }),
    ];
    let layout = column![
        container(photo).center_x(Length::Fill),
        Space::new().height(Length::Fill),
        lines,
        Space::new().height(18),
        credit,
    ]
    .padding(22);
    let fill = |gradient: Linear| {
        container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Gradient(Gradient::Linear(gradient))),
                ..container::Style::default()
            })
    };
    container(stack![fill(base), fill(glow), layout])
        .id(CARD_ID)
        .width(CARD_WIDTH)
        .height(CARD_HEIGHT)
        .clip(true)
        .into()
}

/// Finds the bounds of the container with `target` as its id.
fn find_bounds(target: Id) -> impl Operation<Rectangle> {
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
fn save_card(
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
