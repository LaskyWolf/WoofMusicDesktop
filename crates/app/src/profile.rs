//! `Trop/Profile/`: the Profile page (account, Woof Recap, badges) and the
//! Recap story, slides that advance every few seconds. `trop::profile`
//! computes the stats and badges.

use std::path::PathBuf;
use std::sync::Arc;

use iced::advanced::widget::Id;
use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{
    Space, button, column, container, image, mouse_area, opaque, progress_bar, row, stack, svg,
    text,
};
use iced::{
    Alignment, Background, Border, Color, Element, Gradient, Length, Point, Radians, Rectangle,
    Shadow, Subscription, Task, Vector, window,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use trop::history::history_view_model::HistoryViewModel;
use trop::profile::recap_service::{self as recap, BadgeKind, Interval, RecapBadge, RecapStats};
use trop::settings::settings_store::SettingsStore;
use trop::utilities::ui_image_color_extraction::Rgb;

use crate::services::{Artwork, Services};
use crate::{Message, Session, settings, theme, ui};
use swiftytube::auth::profile_storage::Registry;

const SLIDE_SECONDS: f32 = 4.0;
const TICK_MS: u64 = 50;
const CARD_WIDTH: f32 = 402.0;
const CARD_HEIGHT: f32 = 740.0;
const SLIDE_WIDTH: f32 = CARD_WIDTH - 44.0;
const SHARE_ID: &str = "recap-share-card";
/// Swift `RecapPalette`.
const GOLD: Color = Color::from_rgb(1.0, 0.81, 0.29);
const TEAL: Color = Color::from_rgb(0.37, 0.7, 0.66);
const DIM: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.75);
/// Swift `RecapStoryView.fallbackColors`.
const FALLBACK: [Rgb; 2] = [[0.24, 0.16, 0.48], [0.08, 0.32, 0.3]];

#[derive(Default)]
pub struct Profile {
    /// Last month, while its card is on Home.
    home_card: Option<Interval>,
    /// Home checked for the card in this run.
    home_checked: bool,
    badges: Vec<RecapBadge>,
    last_month: Option<Interval>,
    selected: Option<BadgeKind>,
    story: Option<Story>,
}

struct Story {
    interval: Interval,
    month: String,
    /// `None` while it loads.
    stats: Option<Option<RecapStats>>,
    artwork: Option<Artwork>,
    slide: usize,
    /// 0...1 through the slide on screen.
    progress: f32,
    paused: bool,
    cursor_x: f32,
    /// The share card is on screen for the screenshot that saves it.
    capturing: bool,
    saved: Option<Result<PathBuf, String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slide {
    Intro,
    Minutes,
    Artist,
    Song,
    Badges,
    Summary,
}

#[derive(Debug, Clone)]
pub enum Msg {
    HomeCard(Option<Interval>),
    Loaded(Vec<RecapBadge>, Option<Interval>),
    Select(Option<BadgeKind>),
    Open(Interval),
    StatsLoaded(Interval, Option<RecapStats>),
    ArtworkLoaded(Interval, Option<Artwork>),
    Tick,
    Cursor(Point),
    Press,
    Next,
    Previous,
    Pause,
    Replay,
    Close,
    Save,
    Shot(window::Screenshot, Option<Rectangle>),
    Saved(Result<PathBuf, String>),
}

/// What the page needs from the app.
pub struct Cx {
    pub services: Arc<Services>,
    pub signed_in: bool,
    /// A podcast speed other than 1× was picked (Time Bender).
    pub time_bent: bool,
}

impl Profile {
    /// Profile opened, or a recap closed: badges, and whether last month has a recap.
    pub fn enter(&self, cx: &Cx) -> Task<Message> {
        let db = cx.services.db.clone();
        let time_bent = cx.time_bent;
        let last = recap::last_month(Timestamp::now(), &TimeZone::system());
        let found = has_history(cx, last);
        Task::perform(
            async move {
                let badges = tokio::task::spawn_blocking(move || {
                    recap::lifetime_badges(&db, time_bent, &TimeZone::system())
                })
                .await
                .unwrap_or_default();
                (badges, found.await.then_some(last))
            },
            |(badges, last)| Message::Profile(Msg::Loaded(badges, last)),
        )
    }

    /// Swift `RecapHomeCard`: in the first week of a month, Home shows last
    /// month's recap when it has history and ✕ did not hide it.
    pub fn enter_home(&mut self, cx: &Cx, prefs: &SettingsStore) -> Task<Message> {
        let tz = TimeZone::system();
        let now = Timestamp::now();
        let last = recap::last_month(now, &tz);
        let in_window = now.to_zoned(tz.clone()).day() <= HOME_CARD_DAYS
            || std::env::var("WOOF_RECAP_CARD").is_ok();
        if self.home_checked
            || !in_window
            || !prefs.track_play_history
            || prefs.recap_dismissed_month == month_key(last, &tz)
        {
            return Task::none();
        }
        self.home_checked = true;
        Task::perform(has_history(cx, last), move |found| {
            Message::Profile(Msg::HomeCard(found.then_some(last)))
        })
    }

    /// The card, unless ✕ hid it or play history is off.
    pub fn home_card(&self, prefs: &SettingsStore) -> Option<Element<'_, Message>> {
        let interval = self.home_card?;
        let tz = TimeZone::system();
        let key = month_key(interval, &tz);
        if !prefs.track_play_history || prefs.recap_dismissed_month == key {
            return None;
        }
        let month = recap::month_name(interval, &tz);
        let mut hidden = prefs.clone();
        hidden.recap_dismissed_month = key;
        let body = row![
            column![
                text("NEW · WOOF RECAP")
                    .size(11)
                    .font(theme::weight(Weight::Bold))
                    .color(GOLD),
                text(format!("Your {month} in Woof is ready ✨"))
                    .size(20)
                    .font(theme::weight(Weight::Bold)),
                text("Minutes, top song and badges")
                    .size(13)
                    .color(Color::from_rgba(1.0, 1.0, 1.0, 0.7)),
            ]
            .spacing(2)
            .width(Length::Fill),
            container(
                svg(svg::Handle::from_memory(PLAY))
                    .width(14)
                    .height(14)
                    .style(|_, _| svg::Style {
                        color: Some(Color::BLACK)
                    }),
            )
            .center(40)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::WHITE)),
                border: Border::default().rounded(20),
                ..container::Style::default()
            }),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        // ponytail: a still gold rim; Swift turns a rainbow gradient around the card.
        let glow = Linear::new(Radians(4.0))
            .add_stop(0.0, Color { a: 0.25, ..GOLD })
            .add_stop(0.6, Color::from_rgb(0.08, 0.06, 0.15));
        let card = button(body)
            .padding(iced::Padding {
                top: 16.0,
                left: 16.0,
                bottom: 16.0,
                right: 36.0,
            })
            .width(Length::Fill)
            .on_press(Message::Profile(Msg::Open(interval)))
            .style(move |_, status| button::Style {
                background: Some(Background::Gradient(Gradient::Linear(glow))),
                border: Border::default()
                    .rounded(22)
                    .width(if status == button::Status::Hovered {
                        2
                    } else {
                        1
                    })
                    .color(Color { a: 0.7, ..GOLD }),
                shadow: Shadow {
                    color: Color { a: 0.3, ..TEAL },
                    offset: Vector::ZERO,
                    blur_radius: 18.0,
                },
                text_color: Color::WHITE,
                ..button::Style::default()
            });
        let close = button(container(ui::icon(CLOSE, 10.0)).center(28))
            .padding(0)
            .on_press(Message::Settings(settings::Message::Change(hidden)))
            .style(|_, status| button::Style {
                background: (status == button::Status::Hovered)
                    .then_some(Background::Color(theme::PRESSED)),
                border: Border::default().rounded(14),
                ..button::Style::default()
            });
        Some(
            stack![
                card,
                container(close)
                    .padding(4)
                    .width(Length::Fill)
                    .align_x(Alignment::End)
            ]
            .into(),
        )
    }

    pub fn update(&mut self, message: Msg, cx: &Cx) -> Task<Message> {
        match message {
            Msg::HomeCard(interval) => {
                self.home_card = interval;
                Task::none()
            }
            Msg::Loaded(badges, last) => {
                self.badges = badges;
                self.last_month = last;
                Task::none()
            }
            Msg::Select(kind) => {
                self.selected = kind;
                Task::none()
            }
            Msg::Open(interval) => {
                self.story = Some(Story {
                    interval,
                    month: recap::month_name(interval, &TimeZone::system()),
                    stats: None,
                    artwork: None,
                    slide: 0,
                    progress: 0.0,
                    paused: false,
                    cursor_x: CARD_WIDTH,
                    capturing: false,
                    saved: None,
                });
                let db = cx.services.db.clone();
                let client = cx.services.client.clone();
                let (signed_in, time_bent) = (cx.signed_in, cx.time_bent);
                Task::perform(
                    async move {
                        let tz = TimeZone::system();
                        let remote = if signed_in {
                            HistoryViewModel::read_remote(client)
                                .await
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                        let plays = recap::remote_plays(&remote, interval, Timestamp::now(), &tz);
                        tokio::task::spawn_blocking(move || {
                            recap::compute(&db, &plays, interval, time_bent, &tz)
                                .inspect_err(|error| log::error!("Recap failed: {error}"))
                                .ok()
                        })
                        .await
                        .ok()
                        .flatten()
                    },
                    move |stats| Message::Profile(Msg::StatsLoaded(interval, stats)),
                )
            }
            Msg::StatsLoaded(interval, stats) => {
                let Some(story) = self.story.as_mut().filter(|s| s.interval == interval) else {
                    return Task::none();
                };
                let cover = stats.as_ref().and_then(|stats| {
                    stats
                        .top_song
                        .as_ref()
                        .and_then(|s| s.thumbnail_url.clone())
                        .or_else(|| stats.top_artists.first()?.thumbnail_url.clone())
                });
                let artist = stats
                    .as_ref()
                    .and_then(|s| s.top_artists.first()?.thumbnail_url.clone());
                story.stats = Some(stats);
                let artwork = match cover {
                    Some(url) => Task::perform(cx.services.clone().artwork(url), move |art| {
                        Message::Profile(Msg::ArtworkLoaded(interval, art))
                    }),
                    None => Task::none(),
                };
                Task::batch([
                    artwork,
                    Task::done(Message::FetchCovers(artist.into_iter().collect())),
                ])
            }
            Msg::ArtworkLoaded(interval, artwork) => {
                if let Some(story) = self.story.as_mut().filter(|s| s.interval == interval) {
                    story.artwork = artwork;
                }
                Task::none()
            }
            Msg::Tick => {
                if let Some(story) = &mut self.story {
                    story.progress += TICK_MS as f32 / 1000.0 / SLIDE_SECONDS;
                    if story.progress >= 1.0 {
                        story.step(1);
                    }
                }
                Task::none()
            }
            Msg::Cursor(point) => {
                if let Some(story) = &mut self.story {
                    story.cursor_x = point.x;
                }
                Task::none()
            }
            // Swift: a tap on the left third goes back, anywhere else forward.
            Msg::Press => {
                if let Some(story) = &mut self.story {
                    let back = story.cursor_x < CARD_WIDTH / 3.0;
                    story.step(if back { -1 } else { 1 });
                }
                Task::none()
            }
            Msg::Next | Msg::Previous => {
                if let Some(story) = &mut self.story {
                    story.step(if matches!(message, Msg::Next) { 1 } else { -1 });
                }
                Task::none()
            }
            Msg::Pause => {
                if let Some(story) = &mut self.story {
                    story.paused = !story.paused;
                }
                Task::none()
            }
            Msg::Replay => {
                if let Some(story) = &mut self.story {
                    story.slide = 0;
                    story.progress = 0.0;
                    story.saved = None;
                }
                Task::none()
            }
            Msg::Close => {
                self.selected = None;
                if self.story.take().is_some() {
                    // Badges may have changed while the recap was open.
                    return self.enter(cx);
                }
                Task::none()
            }
            Msg::Save => {
                let Some(story) = &mut self.story else {
                    return Task::none();
                };
                story.capturing = true;
                story.saved = None;
                window::latest().and_then(window::screenshot).then(|shot| {
                    iced::advanced::widget::operate(crate::lyrics::find_bounds(Id::new(SHARE_ID)))
                        .map(Some)
                        .chain(Task::done(None))
                        .collect()
                        .map(move |found| {
                            Message::Profile(Msg::Shot(
                                shot.clone(),
                                found.into_iter().flatten().next(),
                            ))
                        })
                })
            }
            Msg::Shot(shot, bounds) => {
                let Some(story) = &mut self.story else {
                    return Task::none();
                };
                story.capturing = false;
                let name = format!("Woof Recap {}", story.month);
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            crate::lyrics::save_card(&shot, bounds, &name)
                        })
                        .await
                        .map_err(|error| error.to_string())?
                    },
                    |result| Message::Profile(Msg::Saved(result)),
                )
            }
            Msg::Saved(result) => {
                if let Some(story) = &mut self.story {
                    story.saved = Some(result);
                }
                Task::none()
            }
        }
    }

    /// Escape closes the badge or the story.
    pub fn is_open(&self) -> bool {
        self.story.is_some() || self.selected.is_some()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let running = self.story.as_ref().is_some_and(|story| {
            !story.paused
                && !story.capturing
                && story.slides().len() > story.slide + 1
                && story.stats.is_some()
        });
        if running {
            iced::time::every(std::time::Duration::from_millis(TICK_MS))
                .map(|_| Message::Profile(Msg::Tick))
        } else {
            Subscription::none()
        }
    }

    /// The Profile page.
    pub fn view<'a>(
        &'a self,
        covers: &'a ui::Covers,
        session: &'a Session,
        track_history: bool,
        profiles: &'a Registry,
        changing: bool,
    ) -> Element<'a, Message> {
        let (name, photo, signed_in) = match session {
            Session::SignedIn(Some(Ok(info))) => {
                (info.name.as_str(), info.thumbnail_url.as_deref(), true)
            }
            Session::SignedIn(_) => ("Signed in", None, true),
            _ => ("Not Signed In", None, false),
        };
        let header = column![
            ui::cover(covers, photo, 96.0, 48.0),
            text(name).size(22).font(theme::weight(Weight::Bold)),
            text(if signed_in {
                "YouTube Music"
            } else {
                "Recap and badges use this computer's history"
            })
            .size(14)
            .color(theme::SECONDARY),
        ]
        .spacing(10)
        .align_x(Alignment::Center)
        .width(Length::Fill);

        let account: Element<'a, Message> = if signed_in {
            settings::group(vec![
                row_button(
                    text("Change Profile Picture").size(15).into(),
                    Some(Message::ChangePicture),
                ),
                row_button(
                    text("Sign Out").size(15).color(theme::CLOSE).into(),
                    Some(Message::SignOut),
                ),
            ])
        } else {
            column![
                settings::group(vec![row_button(
                    text("Sign In to YouTube Music")
                        .size(15)
                        .color(theme::accent())
                        .into(),
                    Some(Message::SignIn),
                )]),
                container(settings::caption("Sign in to sync your library.")).padding([8, 18]),
            ]
            .into()
        };

        let tz = TimeZone::system();
        let now = Timestamp::now();
        let open = |interval| track_history.then_some(Message::Profile(Msg::Open(interval)));
        let accounts = accounts(covers, profiles, signed_in, changing);
        let mut recaps = vec![recap_row(
            SPARKLES,
            "This Month's Recap".to_owned(),
            open(recap::month_to_date(now, &tz)),
        )];
        if let Some(last) = self.last_month {
            recaps.push(recap_row(
                CALENDAR,
                format!("{} Recap", recap::month_name(last, &tz)),
                open(last),
            ));
        }
        let mut recap_section = column![
            ui::section_title("Woof Recap"),
            Space::new().height(12),
            settings::group(recaps)
        ];
        if !track_history {
            recap_section = recap_section.push(
                container(settings::caption(
                    "Turn on Track Play History in Settings to build your recap.",
                ))
                .padding([8, 18]),
            );
        }

        let tiles = self.badges.iter().map(|badge| {
            button(badge_tile(badge, false))
                .padding(0)
                .on_press(Message::Profile(Msg::Select(Some(badge.kind))))
                .style(|_, _| button::Style {
                    text_color: theme::TEXT,
                    ..button::Style::default()
                })
                .into()
        });
        let badges = column![
            ui::section_title("Badges"),
            Space::new().height(16),
            iced::widget::Row::with_children(tiles)
                .spacing(8)
                .wrap()
                .vertical_spacing(20),
            Space::new().height(12),
            settings::caption("Badges come from what this account played on this computer."),
        ];

        let page = column![
            text("Profile").size(32).font(theme::weight(Weight::Bold)),
            Space::new().height(24),
            header,
            Space::new().height(28),
            account,
            accounts,
            Space::new().height(32),
            recap_section,
            Space::new().height(32),
            badges,
        ]
        .max_width(720);
        ui::scroll_page(vec![page.into()])
    }

    /// The badge sheet or the story, over everything.
    pub fn overlay<'a>(&'a self, covers: &'a ui::Covers) -> Option<Element<'a, Message>> {
        let (body, dim) = match (&self.story, self.selected) {
            (Some(story), _) => (story.view(covers), 0.85),
            (None, Some(kind)) => {
                let badge = self.badges.iter().find(|b| b.kind == kind)?;
                (badge_detail(badge), 0.6)
            }
            (None, None) => return None,
        };
        let scrim = mouse_area(
            container(Space::new())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(Background::Color(Color {
                        a: dim,
                        ..Color::BLACK
                    })),
                    ..container::Style::default()
                }),
        )
        .on_press(Message::Profile(Msg::Close));
        Some(opaque(stack![
            scrim,
            container(opaque(body)).center(Length::Fill).padding(20)
        ]))
    }
}

/// Days at the start of a month that the Home card shows.
const HOME_CARD_DAYS: i8 = 7;

/// "2026-9": Swift's key for the recap month.
fn month_key(interval: Interval, tz: &TimeZone) -> String {
    let date = interval.start.to_zoned(tz.clone()).date();
    format!("{}-{}", date.year(), date.month())
}

/// Swift `RecapService.hasHistory`: anything played in `interval`, by this
/// app or in the account's YouTube Music history.
fn has_history(cx: &Cx, interval: Interval) -> impl Future<Output = bool> + use<> {
    let db = cx.services.db.clone();
    let client = cx.services.client.clone();
    let signed_in = cx.signed_in;
    async move {
        let local = tokio::task::spawn_blocking(move || recap::has_local_history(&db, interval))
            .await
            .unwrap_or(false);
        local
            || (signed_in
                && HistoryViewModel::read_remote(client)
                    .await
                    .is_ok_and(|sections| {
                        let tz = TimeZone::system();
                        !recap::remote_plays(&sections, interval, Timestamp::now(), &tz).is_empty()
                    }))
    }
}

/// Dev aid: WOOF_RECAP=this|last opens that month's recap once the session is
/// back; WOOF_RECAP_SLIDE=n pauses it on slide n.
pub fn dev_open() -> Task<Message> {
    let tz = TimeZone::system();
    let now = Timestamp::now();
    let interval = match std::env::var("WOOF_RECAP").as_deref() {
        Ok("this") => recap::month_to_date(now, &tz),
        Ok("last") => recap::last_month(now, &tz),
        _ => return Task::none(),
    };
    let slide: usize = std::env::var("WOOF_RECAP_SLIDE")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);
    let then: Vec<Message> = (0..slide).map(|_| Message::Profile(Msg::Next)).collect();
    Task::future(tokio::time::sleep(std::time::Duration::from_secs(2)))
        .then(move |_| {
            Task::done(Message::Profile(Msg::Open(interval)))
                .chain(Task::done(Message::Profile(Msg::Pause)))
        })
        .chain(
            Task::future(tokio::time::sleep(std::time::Duration::from_secs(8)))
                .then(move |_| Task::batch(then.clone().into_iter().map(Task::done))),
        )
}

impl Story {
    fn slides(&self) -> Vec<Slide> {
        let Some(Some(stats)) = &self.stats else {
            return Vec::new();
        };
        if stats.is_empty() {
            return Vec::new();
        }
        let mut list = vec![Slide::Intro, Slide::Minutes];
        if !stats.top_artists.is_empty() {
            list.push(Slide::Artist);
        }
        if stats.top_song.is_some() {
            list.push(Slide::Song);
        }
        // The shelf only gets a slide when the month unlocked something.
        if !stats.new_badges.is_empty() {
            list.push(Slide::Badges);
        }
        list.push(Slide::Summary);
        list
    }

    fn step(&mut self, by: i32) {
        let last = self.slides().len().saturating_sub(1);
        let next = (self.slide as i32 + by).clamp(0, last as i32) as usize;
        if next != self.slide || by < 0 {
            self.progress = 0.0;
        } else {
            self.progress = 1.0;
        }
        self.slide = next;
    }

    fn colors(&self) -> [Rgb; 2] {
        self.artwork.as_ref().map_or(FALLBACK, |art| art.colors)
    }

    fn view<'a>(&'a self, covers: &'a ui::Covers) -> Element<'a, Message> {
        let slides = self.slides();
        let colors = self.colors();
        let color = colors[self.slide % 2];
        let tint = Color::from_rgb(color[0] as f32, color[1] as f32, color[2] as f32);
        // ponytail: a linear wash from the slide's corner; Swift draws a radial gradient whose centre moves per slide.
        let angles = [3.6, 0.6, 3.4, 3.8, std::f32::consts::PI, 0.0];
        let wash = Linear::new(Radians(angles[self.slide % angles.len()]))
            .add_stop(0.0, Color { a: 0.9, ..tint })
            .add_stop(0.75, Color::TRANSPARENT);
        let background = container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Gradient(Gradient::Linear(wash))),
                ..container::Style::default()
            });

        // Keeps white text readable on light covers.
        let shade = container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.25))),
                ..container::Style::default()
            });
        let mut layers = stack![background, shade];
        let stats = match &self.stats {
            Some(Some(stats)) if !stats.is_empty() => Some(stats),
            _ => None,
        };
        match (stats, slides.get(self.slide)) {
            (Some(stats), Some(_)) if self.capturing => {
                layers = layers.push(
                    container(share_card(
                        stats,
                        &self.month,
                        self.artwork.as_ref(),
                        colors,
                    ))
                    .center(Length::Fill),
                );
            }
            (Some(stats), Some(&slide)) => {
                let content = match slide {
                    Slide::Intro => intro(&self.month),
                    // Paused, the count-up would stop part way: show the total.
                    Slide::Minutes => minutes(stats, if self.paused { 1.0 } else { self.progress }),
                    Slide::Artist => artist(stats, covers),
                    Slide::Song => song(stats, self.artwork.as_ref()),
                    Slide::Badges => badges_slide(stats, &self.month),
                    Slide::Summary => summary(stats),
                };
                layers = layers
                    .push(
                        container(content)
                            .padding(iced::Padding {
                                top: 84.0,
                                left: 22.0,
                                right: 22.0,
                                bottom: 30.0,
                            })
                            .width(Length::Fill)
                            .height(Length::Fill),
                    )
                    .push(
                        mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
                            .on_move(|point| Message::Profile(Msg::Cursor(point)))
                            .on_press(Message::Profile(Msg::Press)),
                    );
                if slide == Slide::Summary {
                    layers = layers.push(
                        container(self.summary_buttons())
                            .padding([30, 22])
                            .height(Length::Fill)
                            .align_y(Alignment::End),
                    );
                }
            }
            _ if self.stats.is_none() => {
                layers = layers.push(
                    container(text("Sniffing out your month…").size(15).color(DIM))
                        .center(Length::Fill),
                );
            }
            _ => {
                layers = layers.push(
                    container(
                        column![
                            text("🐾").size(56),
                            text("Nothing to sniff yet")
                                .size(22)
                                .font(theme::weight(Weight::Bold)),
                            text("Play some music and come back.").size(15).color(DIM),
                        ]
                        .spacing(12)
                        .align_x(Alignment::Center),
                    )
                    .center(Length::Fill),
                );
            }
        }
        if !self.capturing {
            layers = layers.push(self.chrome(slides.len()));
        }
        container(layers)
            .width(CARD_WIDTH)
            .height(Length::Fill)
            .max_height(CARD_HEIGHT)
            .clip(true)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgb(0.05, 0.06, 0.09))),
                border: Border::default().rounded(24),
                text_color: Some(Color::WHITE),
                ..container::Style::default()
            })
            .into()
    }

    /// The slide bars, the title and the close button.
    fn chrome(&self, count: usize) -> Element<'_, Message> {
        let bars = row((0..count).map(|i| {
            let fill = match i.cmp(&self.slide) {
                std::cmp::Ordering::Less => 1.0,
                std::cmp::Ordering::Equal => self.progress.min(1.0),
                std::cmp::Ordering::Greater => 0.0,
            };
            progress_bar(0.0..=1.0, fill)
                .girth(3)
                .style(|_| progress_bar::Style {
                    background: Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.3)),
                    bar: Background::Color(Color::WHITE),
                    border: Border::default().rounded(2),
                })
                .into()
        }))
        .spacing(4);
        let pause_label = if self.paused { "Resume" } else { "Pause" };
        let mut top = row![
            text("🐾 Woof Recap")
                .size(15)
                .font(theme::weight(Weight::Bold))
                .width(Length::Fill),
        ]
        .align_y(Alignment::Center)
        .spacing(4);
        if count > 1 {
            top = top.push(chrome_button(
                if self.paused { PLAY } else { PAUSE },
                pause_label,
                Message::Profile(Msg::Pause),
            ));
        }
        top = top.push(chrome_button(CLOSE, "Close", Message::Profile(Msg::Close)));
        column![bars, top]
            .spacing(10)
            .padding([14, 16])
            .width(Length::Fill)
            .into()
    }

    fn summary_buttons(&self) -> Element<'_, Message> {
        let replay = button(
            container(
                row![
                    ui::icon(REPLAY, 16.0),
                    text("Replay").size(16).font(theme::weight(Weight::Bold))
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .center_x(Length::Fill),
        )
        .padding([14, 0])
        .width(Length::Fill)
        .on_press(Message::Profile(Msg::Replay))
        .style(|_, status| button::Style {
            background: Some(Background::Color(Color::from_rgba(
                1.0,
                1.0,
                1.0,
                if status == button::Status::Hovered {
                    0.24
                } else {
                    0.16
                },
            ))),
            border: Border::default().rounded(26),
            text_color: Color::WHITE,
            ..button::Style::default()
        });
        let save = button(
            text("Save Image")
                .size(16)
                .font(theme::weight(Weight::Bold))
                .width(Length::Fill)
                .align_x(Alignment::Center),
        )
        .padding([14, 0])
        .width(Length::Fill)
        .on_press(Message::Profile(Msg::Save))
        .style(|_, status| button::Style {
            background: Some(Background::Color(if status == button::Status::Hovered {
                Color::from_rgb(0.88, 0.88, 0.88)
            } else {
                Color::WHITE
            })),
            border: Border::default().rounded(26),
            text_color: Color::BLACK,
            ..button::Style::default()
        });
        let mut buttons = column![row![replay, save].spacing(8)].spacing(10);
        if let Some(saved) = &self.saved {
            let line = match saved {
                Ok(path) => format!("Saved to {}", path.display()),
                Err(error) => format!("Not saved: {error}"),
            };
            buttons = buttons.push(text(line).size(12).color(DIM));
        }
        buttons.into()
    }
}

// MARK: - Slides

fn kicker(line: String) -> Element<'static, Message> {
    text(line.to_uppercase())
        .size(12)
        .font(theme::weight(Weight::Bold))
        .color(GOLD)
        .into()
}

fn headline(line: String, size: f32) -> Element<'static, Message> {
    text(line)
        .size(size)
        .font(theme::weight(Weight::ExtraBold))
        .line_height(1.1)
        .into()
}

fn sub(line: String) -> Element<'static, Message> {
    text(line).size(15).color(DIM).into()
}

fn intro(month: &str) -> Element<'_, Message> {
    let sun = Linear::new(Radians(2.4))
        .add_stop(0.0, Color::from_rgb(1.0, 0.96, 0.85))
        .add_stop(0.5, Color::from_rgb(1.0, 0.83, 0.43))
        .add_stop(1.0, Color::from_rgb(0.88, 0.64, 0.23));
    let badge = container(text("🐾").size(44))
        .center(120)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(sun))),
            border: Border::default().rounded(60),
            shadow: Shadow {
                color: Color::from_rgba(1.0, 0.83, 0.43, 0.5),
                offset: Vector::ZERO,
                blur_radius: 30.0,
            },
            ..container::Style::default()
        });
    column![
        container(badge).center_x(Length::Fill).padding([30, 0]),
        kicker(format!("Woof Recap · {month}")),
        headline(format!("Your {month}\nin Woof"), 34.0),
        sub("The pack has been busy. Let's sniff out what you played.".into()),
    ]
    .spacing(8)
    .into()
}

/// "3 days and 8 hours", "12 hours", "45 minutes".
fn wagging(minutes: i64) -> String {
    let days = minutes / 1440;
    let hours = minutes % 1440 / 60;
    match (days, hours) {
        (0, 0) if minutes == 1 => "1 minute".into(),
        (0, 0) => format!("{minutes} minutes"),
        (0, 1) => "1 hour".into(),
        (0, h) => format!("{h} hours"),
        (d, 0) => format!("{d} days"),
        (d, h) => format!("{d} days and {h} hours"),
    }
}

fn duration(minutes: i64) -> String {
    if minutes >= 60 {
        format!("{} h", minutes / 60)
    } else {
        format!("{minutes} min")
    }
}

/// "12,345".
fn grouped(value: i64) -> String {
    let digits = value.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if value < 0 { format!("-{out}") } else { out }
}

fn minutes(stats: &RecapStats, progress: f32) -> Element<'_, Message> {
    // Swift counts up over about 1.2 s after 0.4 s, easing out.
    let t = ((progress * SLIDE_SECONDS - 0.4) / 1.2).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t).powi(3);
    let counted = (stats.total_minutes as f32 * eased) as i64;
    let mut slide = column![
        kicker("Minutes listened".into()),
        Space::new().height(32),
        text(grouped(counted))
            .size(64)
            .font(theme::weight(Weight::ExtraBold)),
        sub(format!(
            "That's {} of tail-wagging.",
            wagging(stats.total_minutes)
        )),
    ]
    .spacing(8);
    let weeks = &stats.weekly_minutes;
    if weeks.len() > 1 {
        let top = weeks.iter().copied().max().unwrap_or(1).max(1);
        let loudest = weeks
            .iter()
            .enumerate()
            .max_by_key(|(i, m)| (**m, std::cmp::Reverse(*i)))
            .map(|(i, _)| i);
        let bars = row(weeks.iter().enumerate().map(|(i, &m)| {
            let height = (90.0 * m as f32 / top as f32).max(4.0);
            let fill = if Some(i) == loudest { GOLD } else { TEAL };
            container(Space::new())
                .width(Length::Fill)
                .height(height)
                .style(move |_| container::Style {
                    background: Some(Background::Color(fill)),
                    border: Border::default().rounded(iced::border::Radius {
                        top_left: 6.0,
                        top_right: 6.0,
                        bottom_right: 2.0,
                        bottom_left: 2.0,
                    }),
                    ..container::Style::default()
                })
                .into()
        }))
        .spacing(8)
        .height(90)
        .align_y(Alignment::End);
        slide = slide.push(Space::new().height(16)).push(bars);
        if let Some(loudest) = loudest {
            slide = slide.push(
                text(format!(
                    "Week {} was your loudest: {}.",
                    loudest + 1,
                    duration(weeks[loudest])
                ))
                .size(12)
                .color(DIM),
            );
        }
    }
    slide.into()
}

fn artist<'a>(stats: &'a RecapStats, covers: &'a ui::Covers) -> Element<'a, Message> {
    let Some(top) = stats.top_artists.first() else {
        return Space::new().into();
    };
    let rest = column(stats.top_artists.iter().enumerate().skip(1).map(|(i, a)| {
        text(ui::fit(
            &format!("{}   {}", i + 1, a.name),
            SLIDE_WIDTH,
            15.0,
        ))
        .size(15)
        .color(DIM)
        .into()
    }))
    .spacing(4)
    .width(Length::Fill);
    column![
        container(kicker("Top artist".into())).width(Length::Fill),
        container(ui::cover(covers, top.thumbnail_url.as_deref(), 180.0, 90.0))
            .padding(10)
            .style(|_| container::Style {
                border: Border::default().rounded(100),
                shadow: Shadow {
                    color: Color { a: 0.4, ..GOLD },
                    offset: Vector::ZERO,
                    blur_radius: 25.0,
                },
                ..container::Style::default()
            }),
        text(top.name.as_str())
            .size(30)
            .font(theme::weight(Weight::ExtraBold))
            .align_x(Alignment::Center),
        sub(format!(
            "{} min · {} plays",
            grouped(top.minutes),
            grouped(top.plays)
        )),
        Space::new().height(10),
        rest,
    ]
    .spacing(8)
    .align_x(Alignment::Center)
    .into()
}

fn artwork_image(artwork: Option<&Artwork>, side: f32, radius: f32) -> Element<'_, Message> {
    match artwork {
        Some(art) => image(art.image.clone())
            .width(side)
            .height(side)
            .content_fit(iced::ContentFit::Cover)
            .border_radius(radius)
            .into(),
        None => {
            let [a, b] = FALLBACK;
            let wash = Linear::new(Radians(2.4))
                .add_stop(0.0, Color::from_rgb(a[0] as f32, a[1] as f32, a[2] as f32))
                .add_stop(1.0, Color::from_rgb(b[0] as f32, b[1] as f32, b[2] as f32));
            container(ui::icon(NOTE, side * 0.3))
                .center(side)
                .style(move |_| container::Style {
                    background: Some(Background::Gradient(Gradient::Linear(wash))),
                    border: Border::default().rounded(radius),
                    ..container::Style::default()
                })
                .into()
        }
    }
}

fn song<'a>(stats: &'a RecapStats, artwork: Option<&'a Artwork>) -> Element<'a, Message> {
    let mut slide = column![container(kicker("Top song".into())).width(Length::Fill)]
        .spacing(8)
        .align_x(Alignment::Center)
        .height(Length::Fill);
    if let Some(song) = &stats.top_song {
        let times = if song.plays == 1 { "time" } else { "times" };
        let line = match &song.artist {
            Some(artist) => format!("{artist} · played {} {times}", song.plays),
            None => format!("played {} {times}", song.plays),
        };
        slide = slide
            .push(
                container(artwork_image(artwork, 150.0, 14.0))
                    .padding([4, 0])
                    .style(|_| container::Style {
                        shadow: Shadow {
                            color: Color::from_rgba(0.0, 0.0, 0.0, 0.4),
                            offset: Vector::new(0.0, 12.0),
                            blur_radius: 15.0,
                        },
                        ..container::Style::default()
                    }),
            )
            .push(
                text(song.title.as_str())
                    .size(28)
                    .font(theme::weight(Weight::ExtraBold))
                    .align_x(Alignment::Center),
            )
            .push(text(line).size(15).color(DIM).align_x(Alignment::Center));
    }
    slide = slide.push(Space::new().height(Length::Fill));
    if let Some(badge) = stats.new_badges.first() {
        slide = slide.push(unlocked_card(badge));
    }
    slide.into()
}

/// "Badge unlocked" card.
fn unlocked_card(badge: &RecapBadge) -> Element<'_, Message> {
    let accent = theme::accent();
    container(
        row![
            gem(badge.kind, true, 52.0),
            column![
                text(format!("Badge unlocked: {}", badge.kind.name()))
                    .size(16)
                    .font(theme::weight(Weight::Bold)),
                text(badge.kind.summary()).size(12).color(DIM),
            ]
            .spacing(2)
            .width(Length::Fill),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding(14)
    .width(Length::Fill)
    .style(move |_| container::Style {
        background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.1))),
        border: Border::default()
            .rounded(20)
            .width(1)
            .color(Color { a: 0.5, ..accent }),
        shadow: Shadow {
            color: Color { a: 0.25, ..accent },
            offset: Vector::ZERO,
            blur_radius: 20.0,
        },
        ..container::Style::default()
    })
    .into()
}

fn badges_slide<'a>(stats: &'a RecapStats, month: &str) -> Element<'a, Message> {
    let count = stats.new_badges.len();
    let title = if count == 1 {
        format!("1 badge unlocked in {month}")
    } else {
        format!("{count} badges unlocked in {month}")
    };
    let tiles = stats.new_badges.iter().map(|badge| badge_tile(badge, true));
    column![
        kicker("New on your shelf".into()),
        headline(title, 26.0),
        text(format!(
            "{} of {} collected",
            stats.unlocked_count(),
            stats.badges.len()
        ))
        .size(15)
        .color(theme::accent()),
        Space::new().height(12),
        iced::widget::Row::with_children(tiles)
            .spacing(6)
            .wrap()
            .vertical_spacing(14),
    ]
    .spacing(8)
    .into()
}

fn summary(stats: &RecapStats) -> Element<'_, Message> {
    let line = |label: &'static str, value: String| -> Element<'_, Message> {
        row![
            text(label).size(15).width(Length::Fill),
            text(ui::fit(&value, SLIDE_WIDTH * 0.6, 15.0))
                .size(15)
                .font(theme::weight(Weight::Bold)),
        ]
        .padding([6, 0])
        .into()
    };
    let mut rows = column![line("Minutes", grouped(stats.total_minutes))];
    if let Some(artist) = stats.top_artists.first() {
        rows = rows.push(line("Top artist", artist.name.clone()));
    }
    if let Some(song) = &stats.top_song {
        rows = rows.push(line("Top song", song.title.clone()));
    }
    let unlocked = stats.unlocked_count();
    let mut gems = row(stats
        .badges
        .iter()
        .filter(|b| b.is_unlocked)
        .take(4)
        .map(|b| gem(b.kind, true, 26.0)))
    .spacing(4)
    .align_y(Alignment::Center);
    if unlocked == 0 {
        gems = gems.push(text("None yet").size(15).font(theme::weight(Weight::Bold)));
    } else if unlocked > 4 {
        gems = gems.push(
            text(format!("+{}", unlocked - 4))
                .size(15)
                .font(theme::weight(Weight::Bold)),
        );
    }
    rows = rows.push(
        row![text("Badges").size(15).width(Length::Fill), gems]
            .align_y(Alignment::Center)
            .padding([6, 0]),
    );
    column![
        kicker("That's a wrap".into()),
        headline("Show off\nyour pack".into(), 34.0),
        Space::new().height(8),
        container(rows)
            .padding(16)
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgba(1.0, 1.0, 1.0, 0.08))),
                border: Border::default()
                    .rounded(22)
                    .width(1)
                    .color(Color::from_rgba(1.0, 1.0, 1.0, 0.16)),
                ..container::Style::default()
            }),
    ]
    .spacing(8)
    .into()
}

/// The 9:16 image Save Image writes (Swift `RecapShareCard`).
fn share_card<'a>(
    stats: &'a RecapStats,
    month: &str,
    artwork: Option<&'a Artwork>,
    colors: [Rgb; 2],
) -> Element<'a, Message> {
    let label = |line: &'static str| {
        text(line)
            .size(11)
            .font(theme::weight(Weight::ExtraBold))
            .color(GOLD)
    };
    let mut card = column![
        text(format!("🐾 WOOF RECAP · {}", month.to_uppercase()))
            .size(13)
            .font(theme::weight(Weight::ExtraBold))
            .color(GOLD),
        Space::new().height(28),
        text(grouped(stats.total_minutes))
            .size(72)
            .font(theme::weight(Weight::ExtraBold)),
        text("minutes listened")
            .size(17)
            .font(theme::weight(Weight::Semibold))
            .color(DIM),
        Space::new().height(Length::Fill),
    ];
    if let Some(song) = &stats.top_song {
        card = card.push(
            row![
                artwork_image(artwork, 96.0, 12.0),
                column![
                    label("TOP SONG"),
                    text(ui::fit(&song.title, 180.0, 20.0))
                        .size(20)
                        .font(theme::weight(Weight::Bold)),
                    text(format!("{} plays", song.plays))
                        .size(14)
                        .font(theme::weight(Weight::Semibold))
                        .color(DIM),
                ]
                .spacing(4),
            ]
            .spacing(14)
            .align_y(Alignment::Center),
        );
    }
    if let Some(artist) = stats.top_artists.first() {
        card = card.push(Space::new().height(22)).push(
            column![
                label("TOP ARTIST"),
                text(ui::fit(&artist.name, 296.0, 24.0))
                    .size(24)
                    .font(theme::weight(Weight::Bold)),
            ]
            .spacing(4),
        );
    }
    card = card.push(Space::new().height(Length::Fill));
    if stats.unlocked_count() > 0 {
        card = card.push(
            iced::widget::Row::with_children(
                stats
                    .badges
                    .iter()
                    .filter(|b| b.is_unlocked)
                    .map(|b| gem(b.kind, true, 38.0)),
            )
            .spacing(6)
            .wrap()
            .vertical_spacing(6),
        );
    }
    card = card.push(Space::new().height(18)).push(
        text("WOOF MUSIC")
            .size(12)
            .font(theme::weight(Weight::ExtraBold))
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.7)),
    );
    let rgb = |c: Rgb| Color::from_rgb(c[0] as f32, c[1] as f32, c[2] as f32);
    let wash = Linear::new(Radians(2.6))
        .add_stop(0.0, rgb(colors[0]))
        .add_stop(1.0, rgb(colors[1]));
    container(card)
        .id(SHARE_ID)
        .padding(32)
        .width(360)
        .height(640)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(wash))),
            text_color: Some(Color::WHITE),
            ..container::Style::default()
        })
        .into()
}

// MARK: - Badges

/// Swift `BadgeGem`: a pointy-top hexagon in the badge's colours under its
/// emoji; grey and faded while locked.
fn gem(kind: BadgeKind, unlocked: bool, size: f32) -> Element<'static, Message> {
    let (light, dark) = if unlocked {
        kind.colors()
    } else {
        (0x8E8E93, 0x48484A)
    };
    let hexagon = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 112"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#{light:06X}"/><stop offset="1" stop-color="#{dark:06X}"/></linearGradient></defs><path d="M50 0L100 28V84L50 112L0 84V28Z" fill="url(#g)"/></svg>"##
    );
    let height = size * 1.12;
    let mut layers = stack![
        svg(svg::Handle::from_memory(hexagon.into_bytes()))
            .width(size)
            .height(height),
        container(text(kind.emoji()).size(size * 0.46)).center(Length::Fill),
    ];
    if !unlocked {
        // Fades the emoji with the gem: the page shows through, like Swift's 0.35 opacity.
        layers = layers.push(
            svg(svg::Handle::from_memory(
                br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 112"><path d="M50 0L100 28V84L50 112L0 84V28Z" fill="#000" fill-opacity="0.65"/></svg>"##,
            ))
            .width(size)
            .height(height),
        );
    }
    container(layers).width(size).height(height).into()
}

/// Gem, name with a lock when locked, and the rule or the progress toward it.
fn badge_tile(badge: &RecapBadge, is_new: bool) -> Element<'_, Message> {
    const WIDTH: f32 = 112.0;
    let mut art = stack![container(gem(badge.kind, badge.is_unlocked, 52.0)).center_x(WIDTH)];
    if is_new {
        art = art.push(
            container(
                container(
                    text("NEW")
                        .size(9)
                        .font(theme::weight(Weight::ExtraBold))
                        .color(Color::WHITE),
                )
                .padding([2, 5])
                .style(|_| container::Style {
                    background: Some(Background::Color(theme::accent())),
                    border: Border::default().rounded(8),
                    ..container::Style::default()
                }),
            )
            .padding(iced::Padding {
                left: WIDTH / 2.0 + 14.0,
                ..iced::Padding::ZERO
            }),
        );
    }
    let mut name = row![
        text(ui::fit(badge.kind.name(), WIDTH - 12.0, 12.0))
            .size(12)
            .font(theme::weight(Weight::Semibold)),
    ]
    .spacing(3)
    .align_y(Alignment::Center);
    if !badge.is_unlocked {
        name = name.push(ui::icon(LOCK, 9.0));
    }
    let detail = if badge.is_unlocked {
        badge.kind.summary()
    } else {
        badge.progress_text.clone()
    };
    column![
        art,
        name,
        text(detail)
            .size(11)
            .color(theme::SECONDARY)
            .align_x(Alignment::Center)
            .width(WIDTH),
    ]
    .spacing(5)
    .width(WIDTH)
    .align_x(Alignment::Center)
    .into()
}

/// Swift `BadgeDetailView`.
fn badge_detail(badge: &RecapBadge) -> Element<'_, Message> {
    let accent = theme::accent();
    let status: Element<'_, Message> = if badge.is_unlocked {
        row![
            stack![
                svg(svg::Handle::from_memory(SEAL))
                    .width(18)
                    .height(18)
                    .style(move |_, _| svg::Style {
                        color: Some(accent)
                    }),
                svg(svg::Handle::from_memory(SEAL_CHECK))
                    .width(18)
                    .height(18),
            ],
            text("Unlocked")
                .size(16)
                .font(theme::weight(Weight::Bold))
                .color(accent),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    } else {
        column![
            text(badge.progress_text.as_str())
                .size(14)
                .color(theme::SECONDARY),
            progress_bar(0.0..=1.0, badge.progress as f32)
                .girth(6)
                .style(move |_| progress_bar::Style {
                    background: Background::Color(theme::PRESSED),
                    bar: Background::Color(accent),
                    border: Border::default().rounded(3),
                }),
        ]
        .spacing(8)
        .width(260)
        .into()
    };
    container(
        column![
            gem(badge.kind, badge.is_unlocked, 96.0),
            text(badge.kind.name())
                .size(22)
                .font(theme::weight(Weight::Bold)),
            text(badge.kind.summary())
                .size(15)
                .color(theme::SECONDARY)
                .align_x(Alignment::Center),
            Space::new().height(4),
            status,
        ]
        .spacing(12)
        .align_x(Alignment::Center),
    )
    .padding([32, 24])
    .width(380)
    .style(|_| container::Style {
        background: Some(Background::Color(Color::from_rgb8(0x1C, 0x1C, 0x1E))),
        border: Border::default().rounded(20),
        text_color: Some(theme::TEXT),
        ..container::Style::default()
    })
    .into()
}

// MARK: - Rows

/// A whole-row button inside a `settings::group`.
/// Swift `ProfileView` accounts section: switch to a saved account, add another.
fn accounts<'a>(
    covers: &'a ui::Covers,
    profiles: &'a Registry,
    signed_in: bool,
    changing: bool,
) -> Element<'a, Message> {
    if !signed_in && profiles.profiles.is_empty() {
        return Space::new().into();
    }
    let current = profiles.current.as_deref();
    let mut rows: Vec<Element<'a, Message>> = profiles
        .profiles
        .iter()
        .map(|profile| {
            let is_current = Some(profile.id.as_str()) == current;
            let mark: Element<'a, Message> = if is_current {
                ui::icon(crate::song_menu::icons::CHECK, 16.0).into()
            } else {
                Space::new().into()
            };
            row_button(
                row![
                    ui::cover(covers, profile.thumbnail_url.as_deref(), 32.0, 16.0),
                    text(profile.name.as_str()).size(15).width(Length::Fill),
                    mark,
                ]
                .spacing(12)
                .align_y(Alignment::Center)
                .into(),
                (!is_current && !changing).then(|| Message::SwitchProfile(profile.id.clone())),
            )
        })
        .collect();
    // Another account's session can't be saved over this one until this one is saved.
    let can_add =
        !changing && current.is_some_and(|id| profiles.profiles.iter().any(|p| p.id == id));
    rows.push(row_button(
        row![
            ui::icon(crate::song_menu::icons::PLUS, 16.0).opacity(if can_add {
                1.0_f32
            } else {
                0.4
            }),
            text("Add Account").size(15).color(if can_add {
                theme::accent()
            } else {
                theme::SECONDARY
            }),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into(),
        can_add.then_some(Message::AddAccount),
    ));
    column![
        Space::new().height(32),
        ui::section_title("Accounts"),
        Space::new().height(12),
        settings::group(rows),
        container(settings::caption(if changing {
            "Accounts can't change while songs are downloading."
        } else {
            "Each account keeps its own library and downloads on this computer."
        }))
        .padding([8, 18]),
    ]
    .into()
}

fn row_button<'a>(label: Element<'a, Message>, message: Option<Message>) -> Element<'a, Message> {
    button(label)
        .padding([14, 18])
        .width(Length::Fill)
        .on_press_maybe(message)
        .style(|_, status| button::Style {
            background: (status == button::Status::Hovered)
                .then_some(Background::Color(theme::HOVER)),
            border: Border::default().rounded(16),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
        .into()
}

fn recap_row<'a>(
    icon: &'static [u8],
    title: String,
    open: Option<Message>,
) -> Element<'a, Message> {
    let enabled = open.is_some();
    let fade = if enabled { 1.0 } else { 0.4 };
    let accent = theme::accent();
    row_button(
        row![
            svg(svg::Handle::from_memory(icon))
                .width(18)
                .height(18)
                .opacity(fade)
                .style(move |_, _| svg::Style {
                    color: Some(accent)
                }),
            text(title)
                .size(15)
                .color(Color {
                    a: fade,
                    ..theme::TEXT
                })
                .width(Length::Fill),
            ui::icon(PLAY, 12.0).opacity(0.5 * fade),
        ]
        .spacing(12)
        .align_y(Alignment::Center)
        .into(),
        open,
    )
}

fn chrome_button<'a>(icon: &'static [u8], _label: &str, message: Message) -> Element<'a, Message> {
    button(container(ui::icon(icon, 14.0)).center(28))
        .padding(0)
        .on_press(message)
        .style(|_, status| button::Style {
            background: Some(Background::Color(Color::from_rgba(
                1.0,
                1.0,
                1.0,
                if status == button::Status::Hovered {
                    0.24
                } else {
                    0.12
                },
            ))),
            border: Border::default().rounded(14),
            ..button::Style::default()
        })
        .into()
}

/// `sparkles`.
const SPARKLES: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="#fff"><path d="M10 3.5l1.6 4.6a3 3 0 0 0 1.8 1.8L18 11.5l-4.6 1.6a3 3 0 0 0-1.8 1.8L10 19.5l-1.6-4.6a3 3 0 0 0-1.8-1.8L2 11.5l4.6-1.6a3 3 0 0 0 1.8-1.8z"/><path d="M18.5 2l.7 1.9a1.3 1.3 0 0 0 .8.8l1.9.7-1.9.7a1.3 1.3 0 0 0-.8.8l-.7 1.9-.7-1.9a1.3 1.3 0 0 0-.8-.8L15 5.4l1.9-.7a1.3 1.3 0 0 0 .8-.8z"/></g></svg>"##;
/// `calendar`.
const CALENDAR: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><rect x="3.5" y="5" width="17" height="15.5" rx="3"/><path d="M3.5 10h17M8 3v4M16 3v4"/></g></svg>"##;
const PLAY: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M7 4.8v14.4a1 1 0 0 0 1.5.86l11.6-7.2a1 1 0 0 0 0-1.72L8.5 3.94A1 1 0 0 0 7 4.8z" fill="#fff"/></svg>"##;
const PAUSE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="6" y="4.5" width="4" height="15" rx="1.2" fill="#fff"/><rect x="14" y="4.5" width="4" height="15" rx="1.2" fill="#fff"/></svg>"##;
const CLOSE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 6l12 12M18 6L6 18" stroke="#fff" stroke-width="2.6" stroke-linecap="round"/></svg>"##;
/// `arrow.counterclockwise`.
const REPLAY: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3"/><path d="M4.5 3.5v4.2h4.2"/></g></svg>"##;
/// `lock.fill`.
const LOCK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M7.5 10V7.5a4.5 4.5 0 0 1 9 0V10" fill="none" stroke="#fff" stroke-width="2.4"/><rect x="4.5" y="10" width="15" height="11" rx="2.5" fill="#fff"/></svg>"##;
/// `checkmark.seal.fill`.
const SEAL: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 1.8l2.6 1.9 3.2-.1 1 3 2.6 1.9-1 3 1 3-2.6 1.9-1 3-3.2-.1L12 22.2l-2.6-1.9-3.2.1-1-3-2.6-1.9 1-3-1-3 2.6-1.9 1-3 3.2.1z" fill="#fff"/></svg>"##;
const SEAL_CHECK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M7.8 12.2l2.8 2.8 5.6-5.8" fill="none" stroke="#1C1C1E" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
/// `music.note`.
const NOTE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M9 18V5.5l11-2V16" fill="none" stroke="#fff" stroke-width="2" stroke-linejoin="round"/><circle cx="6.5" cy="18" r="2.5" fill="#fff"/><circle cx="17.5" cy="16" r="2.5" fill="#fff"/></svg>"##;

#[cfg(test)]
mod tests {
    use super::{grouped, wagging};

    #[test]
    fn minutes_read_as_days_hours_or_minutes() {
        assert_eq!(wagging(45), "45 minutes");
        assert_eq!(wagging(60), "1 hour");
        assert_eq!(wagging(4800), "3 days and 8 hours");
        assert_eq!(grouped(12345), "12,345");
        assert_eq!(grouped(999), "999");
    }
}
