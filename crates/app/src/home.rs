//! Home (Swift `HomeScreenView`, `KeepListeningStrip`, `MixesRow`,
//! `RadioDialTile`, `ChipsRowView` and `ShimmerLoadingView`): mood chips,
//! Keep Listening, Your Mixes, Quick Picks with the pinned songs first,
//! YouTube's shelves and the account's own sections.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{
    Row, Space, Stack, button, column, container, image, row, scrollable, stack, svg, text,
};
use iced::{
    Alignment, Background, Border, Color, ContentFit, Element, Gradient, Length, Padding, Radians,
    Subscription, Task,
};
use trop::audio::player_controller::player_controller::State as PlayState;
use trop::home::mixes::{Mix, MixKind};
use trop::networking::models::home_page::{HomePageChip, HomeSection};
use trop::networking::models::yt_item::{PodcastItem, SongItem, YTItem};
use trop::services::personalization_service::RadioOptions;
use trop::settings::settings_store::QuickPicksSize;

use crate::services::{HomeFeed, QUICK_PICKS_COUNT, Services};
use crate::ui::{self, Covers};
use crate::{App, Detail, Load, Message, Route, theme};

/// Home's state in the app.
pub struct Home {
    pub feed: Load<HomeFeed>,
    /// Songs pinned to Quick Picks, newest pin first.
    pub pins: Vec<SongItem>,
    /// Keep Listening is spread out.
    pub spread: bool,
    /// Episode video id → its show's browse id, for the podcast tiles.
    pub episode_shows: HashMap<String, String>,
}

impl Default for Home {
    fn default() -> Self {
        Self {
            feed: Load::Loading,
            pins: Vec::new(),
            spread: false,
            episode_shows: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// The library's sections and saved page, the account's sections, a
    /// chip's feed or the next page.
    /// Numbered by the load that asked; an answer from an older load is dropped.
    Updated(u64, HomeFeed),
    Chip(HomePageChip),
    Scrolled(scrollable::Viewport),
    ToggleKeepListening,
    PlayMix(usize),
    /// The radio that follows a mix's songs, after the mix's first song.
    MixRadio(String, Result<(Vec<SongItem>, usize), String>),
    /// A podcast tile's play button: the show's episodes.
    PlayShow(String),
    ShowLoaded(Result<Arc<Vec<SongItem>>, String>),
    /// Redraws the placeholders' pulse.
    Pulse,
}

/// Swift `HomeViewModel.load`: the library's sections and the saved page
/// first, then YouTube's page; the account's sections alongside.
pub fn load(services: &Arc<Services>) -> Task<Message> {
    let load = services.start_home_load();
    let updated = move |feed| Message::Home(Msg::Updated(load, feed));
    Task::batch([
        Task::perform(services.clone().home_start(), updated).chain(Task::perform(
            services.clone().home_fetch(),
            move |result| Message::HomeLoaded(load, result),
        )),
        Task::perform(services.clone().home_phase2(), updated),
    ])
}

impl App {
    pub(crate) fn update_home(&mut self, msg: Msg) -> Task<Message> {
        match msg {
            Msg::Updated(load, _) if load != self.services.home_generation() => Task::none(),
            Msg::Updated(_, feed) => {
                // Until YouTube's page or the saved one is in, the placeholders stay.
                if !feed.has_page && !matches!(self.home.feed, Load::Loaded(_)) {
                    return Task::none();
                }
                self.show_home(feed)
            }
            Msg::Chip(chip) => {
                if let Load::Loaded(feed) = &mut self.home.feed {
                    let closing = feed.chip.as_deref() == Some(chip.title.as_str());
                    feed.chip = (!closing).then(|| chip.title.clone());
                    feed.chip_loading = !closing;
                }
                let load = self.services.home_generation();
                Task::perform(self.services.clone().home_chip(Some(chip)), move |feed| {
                    Message::Home(Msg::Updated(load, feed))
                })
            }
            Msg::Scrolled(viewport) => {
                if let Some(entry) = self.stack.last_mut() {
                    entry.offset = viewport.absolute_offset().y;
                }
                let Load::Loaded(feed) = &mut self.home.feed else {
                    return Task::none();
                };
                // Swift starts the next page when one of the last sections shows.
                if !feed.has_more
                    || feed.loading_more
                    || feed.chip_loading
                    || viewport.relative_offset().y < 0.8
                {
                    return Task::none();
                }
                feed.loading_more = true;
                let load = self.services.home_generation();
                Task::perform(self.services.clone().home_more(), move |feed| {
                    Message::Home(Msg::Updated(load, feed))
                })
            }
            Msg::ToggleKeepListening => {
                self.home.spread = !self.home.spread;
                Task::none()
            }
            Msg::PlayMix(index) => {
                let Load::Loaded(feed) = &self.home.feed else {
                    return Task::none();
                };
                let Some(mix) = feed.mixes.get(index) else {
                    return Task::none();
                };
                let songs = Arc::new(mix.songs.clone());
                let seed = songs
                    .iter()
                    .find(|song| song.is_available())
                    .map(|song| song.video_id.clone());
                let play = self.play(songs, 0);
                let Some(seed) = seed else {
                    return play;
                };
                let radio = Task::perform(
                    self.services
                        .clone()
                        .radio(seed.clone(), RadioOptions::default()),
                    move |result| Message::Home(Msg::MixRadio(seed, result)),
                );
                Task::batch([play, radio])
            }
            Msg::MixRadio(seed, Ok((songs, _))) => {
                let queue = &self.player.queue.songs;
                // The queue was replaced while the radio loaded: leave it alone.
                if !queue.iter().any(|song| song.video_id == seed) {
                    return Task::none();
                }
                let mut known: HashSet<String> = queue.iter().map(|s| s.video_id.clone()).collect();
                let more: Vec<SongItem> = songs
                    .into_iter()
                    .filter(|song| known.insert(song.video_id.clone()))
                    .collect();
                self.player.queue.append(&more);
                Task::none()
            }
            Msg::MixRadio(_, Err(error)) => {
                log::warn!("The mix's radio did not load: {error}");
                Task::none()
            }
            Msg::PlayShow(browse_id) => Task::perform(
                self.services.clone().load_detail(Route::Podcast(browse_id)),
                |result| {
                    Message::Home(Msg::ShowLoaded(result.map(|detail| match detail {
                        Detail::Podcast { songs, .. } => songs,
                        _ => Arc::default(),
                    })))
                },
            ),
            Msg::ShowLoaded(Ok(songs)) if !songs.is_empty() => {
                // Loading the show stored its episodes: the tile can light up.
                self.home.episode_shows = self.services.episode_shows();
                self.play(songs, 0)
            }
            Msg::ShowLoaded(_) => self.notify("The show's episodes did not load."),
            Msg::Pulse => Task::none(),
        }
    }

    /// Shows `feed` and loads its covers and the pinned songs'.
    pub(crate) fn show_home(&mut self, feed: HomeFeed) -> Task<Message> {
        self.home.pins = self.services.pins();
        self.home.episode_shows = self.services.episode_shows();
        let urls: Vec<String> = feed
            .sections
            .iter()
            .flat_map(|section| section.items().iter().take(QUICK_PICKS_COUNT))
            .filter_map(|item| item.thumbnail_url().map(str::to_owned))
            .chain(
                self.home
                    .pins
                    .iter()
                    .filter_map(|song| song.thumbnail_url.clone()),
            )
            .collect();
        self.home.feed = Load::Loaded(feed);
        self.fetch_covers(urls)
    }

    /// Swift `PinState.toggle`: pins or unpins the song in Quick Picks.
    pub(crate) fn pin(&mut self, song: SongItem, pinned: bool) -> Task<Message> {
        if let Err(error) = self.services.set_pinned(&song, pinned) {
            log::warn!("Pinning failed: {error}");
            return self.notify("The pin did not change.");
        }
        self.home.pins = self.services.pins();
        let covers = self.fetch_covers(song.thumbnail_url);
        let notice = self.notify(if pinned {
            "Pinned to Quick Picks"
        } else {
            "Unpinned from Quick Picks"
        });
        Task::batch([covers, notice])
    }

    /// The placeholders pulse while something on Home loads.
    pub(crate) fn home_subscription(&self) -> Subscription<Message> {
        let busy = match &self.home.feed {
            Load::Loading => true,
            Load::Loaded(feed) => feed.chip_loading || feed.loading_more,
            Load::Failed(_) => false,
        };
        if busy && matches!(self.route(), Route::Home) {
            iced::time::every(Duration::from_millis(50)).map(|_| Message::Home(Msg::Pulse))
        } else {
            Subscription::none()
        }
    }
}

// MARK: - Page

pub(crate) fn view(app: &App) -> Element<'_, Message> {
    let feed = match &app.home.feed {
        Load::Loaded(feed) => feed,
        Load::Loading => {
            let prefs = &app.settings.prefs;
            return page(placeholders(
                true,
                prefs.show_quick_picks.then_some(prefs.quick_picks_size),
            ));
        }
        Load::Failed(error) => return ui::status(Some(error), Message::Retry),
    };
    let covers = &app.covers;
    let playing = app.player.song().map(|song| song.video_id.as_str());
    let mut sections: Vec<Element<'_, Message>> = Vec::new();
    if feed.chip_loading {
        // YouTube localises chip titles; "Podcast" matches English and Italian.
        let podcasts = feed
            .chip
            .as_deref()
            .is_some_and(|chip| chip.to_lowercase().contains("podcast"));
        sections.extend(chip_placeholders(podcasts));
    } else {
        // Swift shows the Recap card in Keep Listening's place.
        let recap = app.profile.home_card(&app.settings.prefs);
        let has_recap = recap.is_some();
        sections.extend(recap);
        for section in &feed.sections {
            match section {
                HomeSection::QuickPicks(items) => {
                    let size = app.settings.prefs.quick_picks_size;
                    sections.extend(quick_picks(covers, items, &app.home.pins, playing, size));
                }
                HomeSection::KeepListening(items) => {
                    if !has_recap {
                        sections.push(keep_listening(covers, items, app.home.spread));
                    }
                    if !feed.mixes.is_empty() {
                        sections.push(mixes(&feed.mixes));
                    }
                }
                section
                    if section
                        .items()
                        .iter()
                        .any(|item| matches!(item, YTItem::Podcast(_))) =>
                {
                    sections.push(podcasts(app, section));
                }
                section => {
                    let limit = if section.is_songs_only() { 12 } else { 10 };
                    sections.push(ui::section(
                        covers,
                        section.display_title(),
                        section.items(),
                        limit,
                        playing,
                    ));
                }
            }
        }
        if feed.loading_more {
            sections.push(cards_placeholder(3));
        }
    }
    if !feed.chips.is_empty() {
        let chips = Row::with_children(feed.chips.iter().map(|chip| {
            ui::chip(
                &chip.title,
                feed.chip.as_deref() == Some(chip.title.as_str()),
                Message::Home(Msg::Chip(chip.clone())),
            )
        }))
        .spacing(8)
        .wrap()
        .vertical_spacing(8);
        // Close above the first section, like Swift's chips under the title.
        let mut top = column![chips].spacing(20);
        if !sections.is_empty() {
            top = top.push(sections.remove(0));
        }
        sections.insert(0, top.into());
    }
    page(sections)
}

/// `ui::scroll_page`, reporting the viewport so the next page loads near the end.
fn page(sections: Vec<Element<'_, Message>>) -> Element<'_, Message> {
    scrollable(column(sections).spacing(40).padding(ui::CONTENT_PADDING))
        .id(ui::PAGE_SCROLL)
        .on_scroll(|viewport| Message::Home(Msg::Scrolled(viewport)))
        .direction(theme::scrollbar())
        .style(theme::scroll_style)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// MARK: - Quick Picks

const PICK: f32 = 136.0;

/// Pinned songs, then the section's songs that aren't pinned, in pages of the Quick Picks size
/// that wrap side by side.
fn quick_picks<'a>(
    covers: &'a Covers,
    items: &'a [YTItem],
    pins: &'a [SongItem],
    playing: Option<&str>,
    size: QuickPicksSize,
) -> Option<Element<'a, Message>> {
    let pinned: HashSet<&str> = pins.iter().map(|song| song.video_id.as_str()).collect();
    let picks = items.iter().filter_map(|item| match item {
        YTItem::Song(song) if !pinned.contains(song.video_id.as_str()) => Some(song),
        _ => None,
    });
    let cards: Vec<(&SongItem, bool)> = pins
        .iter()
        .map(|song| (song, true))
        .chain(picks.map(|song| (song, false)))
        .take(QUICK_PICKS_COUNT)
        .collect();
    if cards.is_empty() {
        return None;
    }
    let queue: Arc<Vec<SongItem>> =
        Arc::new(cards.iter().map(|(song, _)| (*song).clone()).collect());
    let pages = cards
        .chunks(size.per_page())
        .enumerate()
        .map(|(page, cards)| {
            column(
                cards
                    .chunks(size.columns())
                    .enumerate()
                    .map(|(line, cards)| {
                        row(cards.iter().enumerate().map(|(column, (song, is_pinned))| {
                            let index = page * size.per_page() + line * size.columns() + column;
                            let is_playing = playing == Some(song.video_id.as_str());
                            quick_pick(covers, song, *is_pinned, &queue, index, is_playing)
                        }))
                        .spacing(8)
                        .into()
                    }),
            )
            .spacing(8)
            .into()
        });
    Some(
        column![
            ui::section_title("Quick Picks"),
            Row::with_children(pages)
                .spacing(24)
                .wrap()
                .vertical_spacing(8),
        ]
        .spacing(14)
        .into(),
    )
}

/// Square cover with the title over a dark fade, and a pin when pinned.
fn quick_pick<'a>(
    covers: &'a Covers,
    song: &'a SongItem,
    is_pinned: bool,
    queue: &Arc<Vec<SongItem>>,
    index: usize,
    playing: bool,
) -> Element<'a, Message> {
    let fade = Linear::new(Radians(std::f32::consts::PI))
        .add_stop(0.0, Color::TRANSPARENT)
        .add_stop(1.0, Color::from_rgba(0.0, 0.0, 0.0, 0.7));
    let caption = container(
        text(ui::fit(&song.title, PICK - 16.0, 13.0))
            .size(13)
            .font(theme::weight(Weight::Semibold))
            .color(if playing {
                theme::accent()
            } else {
                theme::TEXT
            })
            .wrapping(text::Wrapping::None),
    )
    .padding(Padding {
        top: 28.0,
        bottom: 6.0,
        left: 8.0,
        right: 8.0,
    })
    .width(PICK)
    .style(move |_| container::Style {
        background: Some(Background::Gradient(Gradient::Linear(fade))),
        border: Border::default().rounded(iced::border::bottom(8)),
        ..container::Style::default()
    });
    let mut card = stack![
        art(
            covers,
            song.thumbnail_url.as_deref(),
            PICK,
            8.0,
            0.0,
            song.is_available()
        ),
        container(caption).align_bottom(PICK),
    ];
    if is_pinned {
        card = card.push(
            container(ui::icon(icons::PIN, 12.0).style(|_, _| svg::Style {
                color: Some(theme::accent()),
            }))
            .padding(6)
            .align_right(PICK),
        );
    }
    crate::song_menu::on_right_click(
        ui::clickable(card, Some(Message::PlayQueue(queue.clone(), index)), 0.0),
        song,
        None,
    )
}

/// A cover image tilted by `tilt` radians and faded when unavailable; the
/// placeholder square until it loads.
fn art<'a>(
    covers: &'a Covers,
    url: Option<&str>,
    size: f32,
    radius: f32,
    tilt: f32,
    available: bool,
) -> Element<'a, Message> {
    match url.and_then(|url| covers.get(url)) {
        Some(handle) => image(handle.clone())
            .width(size)
            .height(size)
            .content_fit(ContentFit::Cover)
            .border_radius(radius)
            .rotation(Radians(tilt))
            .opacity(if available { 1.0_f32 } else { 0.4 })
            .into(),
        None => block(size, size, radius, theme::PLACEHOLDER),
    }
}

// MARK: - Keep Listening

const KEEP_COVER: f32 = 60.0;
const STACKED_STEP: f32 = 24.0;
const SPREAD_STEP: f32 = 66.0;
const STACKED_LIMIT: usize = 8;
const TILT_DEGREES: f32 = 5.0;

/// Keep Listening as one card: the title on top, covers fanned in a stack
/// below it, spread out on click.
fn keep_listening<'a>(
    covers: &'a Covers,
    items: &'a [YTItem],
    spread: bool,
) -> Element<'a, Message> {
    let toggle = Message::Home(Msg::ToggleKeepListening);
    let trailing: Element<'a, Message> = if spread {
        button(ui::icon(crate::icons::BACK, 16.0).style(|_, _| svg::Style {
            color: Some(theme::SECONDARY),
        }))
        .padding(6)
        .on_press(toggle.clone())
        .style(|_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then_some(Background::Color(theme::PRESSED)),
            border: Border::default().rounded(8),
            ..button::Style::default()
        })
        .into()
    } else {
        text("Click to spread")
            .size(13)
            .color(theme::SECONDARY)
            .into()
    };
    let header = row![
        text("Keep Listening")
            .size(20)
            .font(theme::weight(Weight::Bold)),
        Space::new().width(Length::Fill),
        trailing,
    ]
    .align_y(Alignment::Center)
    .height(28);

    let songs = ui::songs_of(items);
    let covers_row: Element<'a, Message> = if spread {
        let row = Row::with_children(items.iter().map(|item| {
            let cover = keep_cover(covers, item, 0.0);
            let open = ui::clickable(cover, ui::open(item, &songs), 0.0);
            match item {
                YTItem::Song(song) => crate::song_menu::on_right_click(open, song, None),
                _ => open,
            }
        }))
        .spacing(SPREAD_STEP - KEEP_COVER);
        scrollable(container(row).padding([4, 0]))
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::new().width(4).scroller_width(4),
            ))
            .style(theme::scroll_style)
            .into()
    } else {
        // The first cover on top: pushed last.
        let mut fan = Stack::new();
        for (index, item) in items.iter().take(STACKED_LIMIT).enumerate().rev() {
            let tilt = if index % 2 == 0 {
                -TILT_DEGREES
            } else {
                TILT_DEGREES
            };
            fan = fan.push(
                container(keep_cover(covers, item, tilt.to_radians()))
                    .padding(Padding::ZERO.left(index as f32 * STACKED_STEP)),
            );
        }
        container(fan).padding([4, 0]).into()
    };
    let card = container(column![header, covers_row].spacing(8))
        .padding(12)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::HOVER)),
            border: Border::default().rounded(18),
            ..container::Style::default()
        });
    if spread {
        card.into()
    } else {
        // Folded, the whole card spreads it.
        button(card)
            .padding(0)
            .on_press(toggle)
            .style(|_, status| button::Style {
                border: Border::default().rounded(18).width(1).color(
                    if status == button::Status::Hovered {
                        Color::from_rgba(1.0, 1.0, 1.0, 0.2)
                    } else {
                        Color::TRANSPARENT
                    },
                ),
                text_color: theme::TEXT,
                ..button::Style::default()
            })
            .into()
    }
}

/// Artists are round, everything else a rounded square.
fn keep_cover<'a>(covers: &'a Covers, item: &YTItem, tilt: f32) -> Element<'a, Message> {
    let radius = if matches!(item, YTItem::Artist(_)) {
        KEEP_COVER / 2.0
    } else {
        10.0
    };
    art(covers, item.thumbnail_url(), KEEP_COVER, radius, tilt, true)
}

// MARK: - Your Mixes

/// Swift `MixesRow`: a coloured card per mix; a click plays it.
fn mixes(mixes: &[Mix]) -> Element<'_, Message> {
    column![
        ui::section_title("Your Mixes"),
        Row::with_children(
            mixes
                .iter()
                .enumerate()
                .map(|(index, mix)| mix_card(index, mix))
        )
        .spacing(12)
        .wrap()
        .vertical_spacing(12),
    ]
    .spacing(14)
    .into()
}

/// Swift `MoodCard`: the mix's colour, its symbol large and faint at the
/// right, the play symbol in the corner.
fn mix_card(index: usize, mix: &Mix) -> Element<'_, Message> {
    const WIDTH: f32 = 168.0;
    const HEIGHT: f32 = 104.0;
    let [r, g, b] = mix.color;
    let tint = Color::from_rgb(r, g, b);
    let darker = Color::from_rgb(r * 0.7, g * 0.7, b * 0.7);
    let fill = Linear::new(Radians(2.4))
        .add_stop(0.0, tint)
        .add_stop(1.0, darker);
    let symbol = match mix.kind {
        MixKind::Flow => icons::WAVEFORM,
        MixKind::OnRepeat => crate::song_menu::icons::REPEAT,
        MixKind::Rewind => icons::REWIND,
        MixKind::Artist => crate::song_menu::icons::ARTIST,
    };
    let mark = container(
        ui::icon(symbol, 64.0)
            .opacity(0.3_f32)
            .rotation(Radians((-18.0_f32).to_radians())),
    )
    .align_right(Length::Fill)
    .align_bottom(Length::Fill);
    let play =
        container(ui::icon(crate::icons::PLAY, 18.0).opacity(0.9_f32)).align_right(Length::Fill);
    let content = column![
        Space::new().height(Length::Fill),
        text(ui::fit(&mix.title, WIDTH - 48.0, 15.0))
            .size(15)
            .font(theme::weight(Weight::Bold))
            .wrapping(text::Wrapping::None),
        text(&mix.subtitle)
            .size(12)
            .font(theme::weight(Weight::Semibold))
            .color(Color::from_rgba(1.0, 1.0, 1.0, 0.85)),
    ]
    .spacing(1);
    let card = container(stack![mark, play, content])
        .padding(12)
        .width(WIDTH)
        .height(HEIGHT)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(fill))),
            border: Border::default().rounded(14),
            ..container::Style::default()
        });
    button(card)
        .padding(0)
        .on_press(Message::Home(Msg::PlayMix(index)))
        .style(|_, status| button::Style {
            border: Border::default().rounded(14).width(2).color(
                if status == button::Status::Hovered {
                    Color::from_rgba(1.0, 1.0, 1.0, 0.35)
                } else {
                    Color::TRANSPARENT
                },
            ),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
        .into()
}

// MARK: - Podcast shelves

const DIAL_WIDTH: f32 = 150.0;
const DIAL_HEIGHT: f32 = 248.0;
const DIAL_ART: f32 = 110.0;
const DIAL_INSET: f32 = 12.0;
const DIAL_PLAY: f32 = 44.0;
/// #FFCF4A.
const NEEDLE: Color = Color::from_rgb(1.0, 0.81, 0.29);

/// A shelf with podcasts: radio-dial tiles for the shows, tiles for the rest.
fn podcasts<'a>(app: &'a App, section: &'a HomeSection) -> Element<'a, Message> {
    let songs = ui::songs_of(section.items());
    let cards = section.items().iter().take(10).map(|item| match item {
        YTItem::Podcast(podcast) => dial(app, podcast),
        item => ui::tile(&app.covers, item, ui::subtitle(item), &songs, false),
    });
    column![
        ui::section_title(section.display_title()),
        Row::with_children(cards)
            .spacing(20)
            .wrap()
            .vertical_spacing(24),
    ]
    .spacing(14)
    .into()
}

/// Swift `RadioDialTile`: a show's card styled like a radio dial, with an
/// ON AIR light while one of its episodes plays and a tuning scale whose
/// needle marks the listening progress.
fn dial<'a>(app: &'a App, podcast: &'a PodcastItem) -> Element<'a, Message> {
    let show = podcast.browse_id.as_str();
    let current = app.player.song().is_some_and(|song| {
        app.home
            .episode_shows
            .get(&song.video_id)
            .map(String::as_str)
            == Some(show)
    });
    let playing = current && app.player.state == PlayState::Playing;
    // The playing tile follows the player; the others show the saved position.
    let progress = if current {
        (app.player.position / app.player.duration.max(1.0)) as f32
    } else {
        app.home
            .episode_shows
            .iter()
            .filter(|(_, owner)| owner.as_str() == show)
            .filter_map(|(id, _)| app.episodes.position(id))
            .max_by(|a, b| a.updated_at.total_cmp(&b.updated_at))
            .map_or(0.0, |saved| {
                (saved.position / saved.duration.max(1.0)) as f32
            })
    };

    let mut cover = stack![ui::cover(
        &app.covers,
        podcast.thumbnail_url.as_deref(),
        DIAL_ART,
        10.0
    )];
    if playing {
        cover = cover.push(container(on_air()).padding(6));
    }
    let white = |alpha| Color::from_rgba(1.0, 1.0, 1.0, alpha);
    let text_width = DIAL_WIDTH - 2.0 * DIAL_INSET;
    let content = column![
        cover,
        Space::new().height(8),
        text(ui::fit(&podcast.title, text_width, 14.0))
            .size(14)
            .font(theme::weight(Weight::Semibold))
            .color(white(1.0))
            .wrapping(text::Wrapping::None),
        text(ui::fit(
            podcast.author.as_deref().unwrap_or("Podcast"),
            text_width,
            12.0
        ))
        .size(12)
        .color(white(0.75))
        .wrapping(text::Wrapping::None),
        Space::new().height(Length::Fill),
        tuning_scale(progress.clamp(0.0, 1.0)),
    ];
    let [from, to] = backdrop(show);
    let fill = Linear::new(Radians(2.4))
        .add_stop(0.0, from)
        .add_stop(1.0, to);
    let card = button(
        container(content)
            .padding(DIAL_INSET)
            .width(DIAL_WIDTH)
            .height(DIAL_HEIGHT)
            .style(move |_| container::Style {
                background: Some(Background::Gradient(Gradient::Linear(fill))),
                border: Border::default().rounded(18),
                ..container::Style::default()
            }),
    )
    .padding(0)
    .on_press(Message::Navigate(Route::Podcast(podcast.browse_id.clone())))
    .style(|_, status| button::Style {
        border: Border::default().rounded(18).width(2).color(
            if status == button::Status::Hovered {
                Color::from_rgba(1.0, 1.0, 1.0, 0.35)
            } else {
                Color::TRANSPARENT
            },
        ),
        text_color: theme::TEXT,
        ..button::Style::default()
    });
    // Over the card rather than inside its button, so its clicks never open the page.
    let play = button(
        container(
            ui::icon(
                if playing {
                    crate::icons::PAUSE
                } else {
                    crate::icons::PLAY
                },
                18.0,
            )
            .style(|_, _| svg::Style {
                color: Some(theme::BACKGROUND),
            }),
        )
        .center(DIAL_PLAY),
    )
    .padding(0)
    .on_press(if current {
        Message::TogglePlay
    } else {
        Message::Home(Msg::PlayShow(podcast.browse_id.clone()))
    })
    .style(|_, status| button::Style {
        background: Some(Background::Color(if status == button::Status::Hovered {
            theme::accent_hover()
        } else {
            theme::accent()
        })),
        border: Border::default().rounded(DIAL_PLAY / 2.0),
        ..button::Style::default()
    });
    stack![
        card,
        container(play)
            .padding(DIAL_INSET)
            .align_right(DIAL_WIDTH)
            .align_bottom(DIAL_HEIGHT),
    ]
    .into()
}

/// Red capsule that blinks slowly: one blink every 1.6 s.
fn on_air<'a>() -> Element<'a, Message> {
    let phase = (START.elapsed().as_secs_f32() % 1.6) / 0.8;
    let alpha = 1.0 - 0.65 * if phase < 1.0 { phase } else { 2.0 - phase };
    container(
        text("ON AIR")
            .size(9)
            .font(theme::weight(Weight::Black))
            .color(Color::WHITE),
    )
    .padding([3, 6])
    .style(move |_| container::Style {
        background: Some(Background::Color(Color::from_rgba(1.0, 0.23, 0.19, alpha))),
        border: Border::default().rounded(8),
        ..container::Style::default()
    })
    .into()
}

/// A tick every 7 px, every fifth one full height, with a glowing needle at
/// `progress`; room is left at the right for the play button.
fn tuning_scale<'a>(progress: f32) -> Element<'a, Message> {
    const HEIGHT: f32 = 22.0;
    let width = DIAL_WIDTH - 2.0 * DIAL_INSET - DIAL_PLAY - 10.0;
    let ticks = Row::with_children((0..=(width / 7.0) as usize).map(|i| {
        block(
            1.0,
            if i % 5 == 0 { HEIGHT } else { HEIGHT / 2.0 },
            0.0,
            Color::from_rgba(1.0, 1.0, 1.0, 0.4),
        )
    }))
    .spacing(6)
    .height(HEIGHT)
    .align_y(Alignment::End);
    let needle = container(block(2.0, HEIGHT, 1.0, NEEDLE))
        .padding(Padding::ZERO.left((width - 2.0) * progress));
    container(stack![ticks, needle])
        .width(width)
        .height(DIAL_PLAY)
        .align_y(Alignment::Center)
        .clip(true)
        .into()
}

/// The card's two colours: Swift takes them from the artwork; here a palette
/// picked from the show id, so a show keeps its colours.
// ponytail: palette instead of artwork colours; extract them from the cover
// (`ui_image_color_extraction`) if the tiles look too alike.
fn backdrop(show: &str) -> [Color; 2] {
    let index = show.bytes().map(usize::from).sum::<usize>() % theme::PALETTES.len();
    let (first, last) = theme::PALETTES[index];
    let mix = |c: Color, black: f32| {
        Color::from_rgb(
            c.r * (1.0 - black),
            c.g * (1.0 - black),
            c.b * (1.0 - black),
        )
    };
    [mix(first, 0.1), mix(last, 0.45)]
}

// MARK: - Placeholders (Swift `ShimmerLoadingView`)

static START: LazyLock<Instant> = LazyLock::new(Instant::now);

/// The placeholder grey, pulsing once every 1.6 s.
fn pulse() -> Color {
    let phase = START.elapsed().as_secs_f32() / 1.6 * std::f32::consts::TAU;
    Color::from_rgba(1.0, 1.0, 1.0, 0.07 + 0.04 * phase.sin())
}

fn block<'a>(width: f32, height: f32, radius: f32, color: Color) -> Element<'a, Message> {
    container(Space::new())
        .width(width)
        .height(height)
        .style(move |_| container::Style {
            background: Some(Background::Color(color)),
            border: Border::default().rounded(radius),
            ..container::Style::default()
        })
        .into()
}

fn shimmer<'a>(width: f32, height: f32, radius: f32) -> Element<'a, Message> {
    block(width, height, radius, pulse())
}

/// Home before the first page: chips, Keep Listening, Quick Picks, then card
/// and song shelves, in the order `HomeViewModel` sorts sections.
fn placeholders<'a>(chips: bool, quick_picks: Option<QuickPicksSize>) -> Vec<Element<'a, Message>> {
    let mut sections = Vec::new();
    if chips {
        sections.push(
            Row::with_children((0..6).map(|_| shimmer(80.0, 34.0, 17.0)))
                .spacing(8)
                .into(),
        );
    }
    // Keep Listening folded: the title, then the tilted covers' places.
    let mut fan = Stack::new();
    for index in (0..6).rev() {
        fan = fan.push(
            container(shimmer(KEEP_COVER, KEEP_COVER, 10.0))
                .padding(Padding::ZERO.left(index as f32 * STACKED_STEP)),
        );
    }
    sections.push(
        container(column![shimmer(140.0, 22.0, 6.0), container(fan).padding([4, 0])].spacing(10))
            .padding(12)
            .width(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(theme::HOVER)),
                border: Border::default().rounded(18),
                ..container::Style::default()
            })
            .into(),
    );
    if let Some(size) = quick_picks {
        let grid = column((0..size.rows()).map(|_| {
            row((0..size.columns()).map(|_| shimmer(PICK, PICK, 8.0)))
                .spacing(8)
                .into()
        }))
        .spacing(8);
        sections.push(titled(grid.into()));
    }
    sections.push(cards_placeholder(5));
    sections.push(titled(
        Row::with_children((0..8).map(|_| {
            row![
                shimmer(ui::ROW_COVER, ui::ROW_COVER, 6.0),
                column![shimmer(170.0, 14.0, 4.0), shimmer(110.0, 12.0, 4.0)].spacing(6),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .width(300)
            .into()
        }))
        .spacing(12)
        .wrap()
        .vertical_spacing(12)
        .into(),
    ));
    sections.push(cards_placeholder(5));
    sections
}

/// A mood chip's feed: no local sections; the Podcasts chip has dial tiles.
fn chip_placeholders<'a>(podcasts: bool) -> Vec<Element<'a, Message>> {
    if !podcasts {
        return placeholders(false, None).split_off(1);
    }
    (0..3)
        .map(|_| {
            titled(
                row((0..3).map(|_| shimmer(DIAL_WIDTH, DIAL_HEIGHT, 18.0)))
                    .spacing(20)
                    .into(),
            )
        })
        .collect()
}

/// `count` card places: a square cover with two lines under it.
fn cards_placeholder<'a>(count: usize) -> Element<'a, Message> {
    titled(
        row((0..count).map(|_| {
            column![
                shimmer(ui::TILE, ui::TILE, 10.0),
                shimmer(130.0, 14.0, 4.0),
                shimmer(90.0, 12.0, 4.0),
            ]
            .spacing(6)
            .into()
        }))
        .spacing(20)
        .into(),
    )
}

fn titled(content: Element<'_, Message>) -> Element<'_, Message> {
    column![shimmer(160.0, 22.0, 6.0), content]
        .spacing(14)
        .into()
}

/// Filled symbols in the app's style; `ui::icon` tints them.
pub mod icons {
    /// `pin.fill`.
    pub const PIN: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M8.5 3h7a1 1 0 0 1 .8 1.6L15 6.5v3.8l2.6 2.6a1 1 0 0 1 .4.8V15a1 1 0 0 1-1 1h-4v5l-1 1.5L11 21v-5H7a1 1 0 0 1-1-1v-1.3a1 1 0 0 1 .3-.7L9 10.3V6.5L7.7 4.6A1 1 0 0 1 8.5 3z" fill="#fff"/></svg>"##;
    /// `pin.slash`.
    pub const PIN_SLASH: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M8.5 3h7a1 1 0 0 1 .8 1.6L15 6.5v3.8l2.6 2.6a1 1 0 0 1 .4.8V15a1 1 0 0 1-1 1h-4v5l-1 1.5L11 21v-5H7a1 1 0 0 1-1-1v-1.3a1 1 0 0 1 .3-.7L9 10.3V6.5L7.7 4.6A1 1 0 0 1 8.5 3z" fill="none" stroke="#fff" stroke-width="1.8" stroke-linejoin="round"/><path d="M4 4l16 16" stroke="#fff" stroke-width="2" stroke-linecap="round"/></svg>"##;
    /// `waveform`.
    pub const WAVEFORM: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M3 10.5v3M7 7v10M11 3.5v17M15 6.5v11M19 9v6M22 11v2" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round"/></svg>"##;
    /// `backward.end.fill`.
    pub const REWIND: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 5v14" stroke="#fff" stroke-width="2.4" stroke-linecap="round"/><path d="M19 5.8v12.4a.8.8 0 0 1-1.2.7l-9.3-6.2a.8.8 0 0 1 0-1.4l9.3-6.2a.8.8 0 0 1 1.2.7z" fill="#fff"/></svg>"##;
}
