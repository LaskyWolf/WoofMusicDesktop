//! The Search page (Swift `SearchView`, `SearchResultsView` and
//! `SearchViewModel`): recent searches, suggestions while typing, filter
//! chips, sorting, the artist hero, Library matches and the incognito switch.
//! Recent searches, the sort and incognito are settings, saved through
//! `settings::Message::Change`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use iced::font::Weight;
use iced::widget::{Space, button, column, container, pick_list, row, text, text_input};
use iced::{Alignment, Background, Border, Element, Length, Task};
use trop::search::search_parser::SearchSection;
use trop::search::search_view_model::{self as model, Hero, SearchSort};
use trop::settings::settings_store::SettingsStore;

use crate::services::Services;
use crate::ui::{self, Covers};
use crate::{Load, Message, Route, settings, theme};

pub const INPUT: &str = "search";

/// What a search found.
#[derive(Debug, Clone)]
pub struct Results {
    pub sections: Vec<SearchSection>,
    pub hero: Option<Hero>,
    /// The library's matches, for the Library chip.
    pub library: Vec<SearchSection>,
}

impl Results {
    fn cover_urls(&self) -> Vec<String> {
        let hero = self
            .hero
            .iter()
            .filter_map(|h| h.artist.thumbnail_url.clone());
        let items = self.sections.iter().chain(&self.library).flat_map(|s| {
            s.items
                .iter()
                .filter_map(|item| item.thumbnail_url().map(str::to_owned))
        });
        hero.chain(items).collect()
    }
}

/// The chip that is on.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Filter {
    #[default]
    All,
    Library,
    Section(String),
}

#[derive(Default)]
pub struct State {
    /// The field's text.
    pub query: String,
    /// The query the results belong to.
    submitted: String,
    /// `None`: recent searches or suggestions.
    results: Option<Load<Results>>,
    filter: Filter,
    /// Completions for the text being typed. The last ones stay until the
    /// next arrive, so the list doesn't flash at each letter.
    suggestions: Vec<String>,
    /// Counts keystrokes: a `Suggest` with an older count was typed over.
    typed: u64,
    /// Latest play per song, for Recently Played.
    recency: HashMap<String, SystemTime>,
    /// The results in display order, sorted.
    shown: Vec<SearchSection>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Input(String),
    Submit,
    /// A recent search or a suggestion was clicked.
    Pick(String),
    /// Typing paused after keystroke `n`.
    Suggest(u64),
    Suggestions(String, Vec<String>),
    Loaded(String, Result<Results, String>),
    Filter(Filter),
    /// A section title: drills into that section, or back out.
    Section(String),
    Sort(SearchSort),
    Recency(HashMap<String, SystemTime>),
    Delete(String),
    Clear,
    Incognito,
}

fn wrap(msg: Msg) -> Message {
    Message::Search(msg)
}

/// Saves the settings with one change, at once: a change sent as a message
/// could be overwritten by another sent from the same old settings.
fn change(
    settings: &mut settings::State,
    services: &Arc<Services>,
    edit: impl FnOnce(&mut SettingsStore),
) -> Task<Message> {
    let mut prefs = settings.prefs.clone();
    edit(&mut prefs);
    settings.change(prefs, services)
}

impl State {
    pub fn update(
        &mut self,
        msg: Msg,
        services: &Arc<Services>,
        settings: &mut settings::State,
    ) -> Task<Message> {
        let prefs = &settings.prefs;
        match msg {
            Msg::Input(query) => {
                self.query = query;
                // While fetching, the field belongs to the submission.
                if matches!(self.results, Some(Load::Loading)) {
                    return Task::none();
                }
                // Any edit returns to recent searches: results only come from Enter.
                self.results = None;
                self.filter = Filter::default();
                self.typed += 1;
                // Each suggestion request would send YouTube the words as they are typed.
                if self.query.trim().is_empty() || prefs.is_incognito {
                    self.suggestions.clear();
                    return Task::none();
                }
                let typed = self.typed;
                Task::perform(tokio::time::sleep(Duration::from_millis(250)), move |_| {
                    wrap(Msg::Suggest(typed))
                })
            }
            Msg::Suggest(typed) => {
                if typed != self.typed {
                    return Task::none();
                }
                let query = self.query.trim().to_owned();
                Task::perform(services.clone().suggestions(query.clone()), move |found| {
                    wrap(Msg::Suggestions(query.clone(), found))
                })
            }
            Msg::Suggestions(query, found) => {
                if query == self.query.trim() {
                    self.suggestions = found;
                }
                Task::none()
            }
            Msg::Pick(query) => {
                self.query = query;
                self.submit(services, settings)
            }
            Msg::Submit => self.submit(services, settings),
            Msg::Loaded(query, result) => {
                if query != self.submitted {
                    return Task::none();
                }
                let mut tasks = Vec::new();
                if let Ok(results) = &result {
                    tasks.push(Task::done(Message::FetchCovers(results.cover_urls())));
                    // Dev aid: WOOF_PLAY=1 with WOOF_SEARCH plays the first songs found.
                    if crate::dev_autoplay(true)
                        && std::env::var("WOOF_SEARCH").is_ok_and(|q| q == query)
                        && let Some(songs) = results
                            .sections
                            .iter()
                            .map(|s| ui::songs_of(&s.items))
                            .find(|songs| !songs.is_empty())
                    {
                        tasks.push(Task::done(Message::PlayQueue(songs, 0)));
                    }
                }
                self.results = Some(crate::loaded(result));
                self.sort(prefs.search_sort);
                Task::batch(tasks)
            }
            Msg::Filter(filter) => {
                // The Library chip turns off again on a second click.
                self.filter = if filter == Filter::Library && self.filter == filter {
                    Filter::All
                } else {
                    filter
                };
                Task::none()
            }
            Msg::Section(title) => {
                let filter = Filter::Section(title);
                self.filter = if self.filter == filter {
                    Filter::All
                } else {
                    filter
                };
                Task::none()
            }
            Msg::Sort(sort) => {
                self.sort(sort);
                Task::batch([
                    change(settings, services, |p| p.search_sort = sort),
                    self.load_recency(services, sort),
                ])
            }
            Msg::Recency(map) => {
                self.recency = map;
                self.sort(prefs.search_sort);
                Task::none()
            }
            Msg::Delete(query) => change(settings, services, |p| {
                p.search_history.retain(|q| *q != query)
            }),
            Msg::Clear => change(settings, services, |p| p.search_history.clear()),
            Msg::Incognito => {
                self.suggestions.clear();
                change(settings, services, |p| p.is_incognito = !p.is_incognito)
            }
        }
    }

    /// Orders and sorts the results for display.
    fn sort(&mut self, sort: SearchSort) {
        self.shown = match &self.results {
            Some(Load::Loaded(results)) => {
                model::display_sections(&results.sections, sort, &self.recency)
            }
            _ => Vec::new(),
        };
    }

    fn submit(
        &mut self,
        services: &Arc<Services>,
        settings: &mut settings::State,
    ) -> Task<Message> {
        let sort = settings.prefs.search_sort;
        let query = self.query.trim().to_owned();
        if query.is_empty() {
            return Task::none();
        }
        // A pasted YouTube, YouTube Music, Shazam, Spotify or Apple Music link opens instead.
        if crate::is_music_link(&query) {
            return Task::done(Message::OpenLink(query));
        }
        self.submitted = query.clone();
        self.results = Some(Load::Loading);
        self.filter = Filter::default();
        let history = if settings.prefs.track_search_history && !settings.prefs.is_incognito {
            change(settings, services, |p| {
                model::remember(&mut p.search_history, &query)
            })
        } else {
            Task::none()
        };
        let search = Task::perform(services.clone().search(query.clone()), move |result| {
            wrap(Msg::Loaded(query.clone(), result))
        });
        Task::batch([search, history, self.load_recency(services, sort)])
    }

    /// Refetches play recency for Recently Played; the last map stays in use meanwhile.
    fn load_recency(&self, services: &Arc<Services>, sort: SearchSort) -> Task<Message> {
        if sort != SearchSort::Recent {
            return Task::none();
        }
        Task::perform(services.clone().recency(), |map| wrap(Msg::Recency(map)))
    }

    pub fn view<'a>(
        &'a self,
        covers: &'a Covers,
        prefs: &'a SettingsStore,
        playing: Option<&str>,
    ) -> Element<'a, Message> {
        let field = text_input("Songs, artists, albums", &self.query)
            .id(INPUT)
            .on_input(|query| wrap(Msg::Input(query)))
            .on_submit(wrap(Msg::Submit))
            .size(16)
            .padding([12, 16])
            .style(ui::field_style);
        let header = row![field, incognito_button(prefs.is_incognito),]
            .spacing(8)
            .align_y(Alignment::Center);
        let mut page = vec![container(header).max_width(700).into()];
        match &self.results {
            None => page.push(self.idle(prefs)),
            Some(Load::Loading) => page.push(note("Searching…")),
            Some(Load::Failed(error)) => page.push(
                column![
                    text("Search failed")
                        .size(18)
                        .font(theme::weight(Weight::Bold)),
                    text(error.as_str()).size(14).color(theme::SECONDARY),
                    ui::pill_button("Try again", wrap(Msg::Pick(self.submitted.clone()))),
                ]
                .spacing(10)
                .into(),
            ),
            Some(Load::Loaded(results))
                if results.sections.is_empty() && results.hero.is_none() =>
            {
                page.push(empty(
                    "No Results",
                    Some(format!("No results found for \"{}\"", self.submitted)),
                ))
            }
            Some(Load::Loaded(results)) => {
                page.extend(self.results_view(covers, results, prefs, playing))
            }
        }
        ui::scroll_page(page)
    }

    /// Recent searches, or while typing the matching ones and YouTube's suggestions.
    fn idle<'a>(&'a self, prefs: &'a SettingsStore) -> Element<'a, Message> {
        let mut list = column![].spacing(2).max_width(640);
        if prefs.is_incognito {
            list = list.push(
                row![
                    ui::icon(icons::GLASSES, 18.0).style(|_, _| iced::widget::svg::Style {
                        color: Some(theme::SECONDARY),
                    }),
                    text("Incognito is on. Searches and plays aren't saved on this computer, and songs stay out of your YouTube Music history. YouTube still receives your searches.")
                        .size(13)
                        .color(theme::SECONDARY),
                ]
                .spacing(10)
                .padding(iced::Padding::ZERO.bottom(12)),
            );
        }
        let history = &prefs.search_history;
        let typed = self.query.trim();
        if !typed.is_empty() {
            let matching = model::matching_history(history, typed);
            let suggestions = model::new_suggestions(&self.suggestions, &matching);
            for query in matching {
                list = list.push(query_row(query, icons::HISTORY, true));
            }
            for query in suggestions {
                list = list.push(query_row(query, icons::SEARCH, false));
            }
        } else if history.is_empty() {
            list = list.push(empty(
                "No Recent Searches",
                (!prefs.is_incognito).then(|| "Your recent searches will appear here.".to_owned()),
            ));
        } else {
            list = list.push(
                row![
                    text("Recently searched")
                        .size(17)
                        .font(theme::weight(Weight::Bold)),
                    Space::new().width(Length::Fill),
                    button(text("Clear").size(14).color(theme::accent()))
                        .padding([4, 8])
                        .on_press(wrap(Msg::Clear))
                        .style(|_, status| button::Style {
                            background: (status == button::Status::Hovered)
                                .then_some(Background::Color(theme::HOVER)),
                            border: Border::default().rounded(10),
                            ..button::Style::default()
                        }),
                ]
                .align_y(Alignment::Center)
                .padding(iced::Padding::ZERO.bottom(6)),
            );
            for query in history {
                list = list.push(query_row(query, icons::HISTORY, true));
            }
        }
        list.into()
    }

    fn results_view<'a>(
        &'a self,
        covers: &'a Covers,
        results: &'a Results,
        prefs: &'a SettingsStore,
        playing: Option<&str>,
    ) -> Vec<Element<'a, Message>> {
        let chip = |label: &'a str, filter: Filter| {
            settings::chip(label, self.filter == filter, wrap(Msg::Filter(filter)))
        };
        let chips = std::iter::once(chip("All Results", Filter::All)).chain(
            model::available_filters(&results.sections)
                .into_iter()
                .map(|title| match title {
                    "Library" => chip(title, Filter::Library),
                    title => chip(title, Filter::Section(title.to_owned())),
                }),
        );
        let sort = pick_list(SearchSort::ALL, Some(prefs.search_sort), |sort| {
            wrap(Msg::Sort(sort))
        })
        .text_size(14)
        .padding([7, 14])
        .style(settings::pick_style)
        .menu_style(settings::menu_style);
        let bar = row![
            iced::widget::Row::with_children(chips)
                .spacing(8)
                .wrap()
                .vertical_spacing(8),
            Space::new().width(Length::Fill),
            sort,
        ]
        .spacing(16)
        .align_y(Alignment::Center);
        let mut page = vec![bar.into()];
        if let Some(hero) = &results.hero
            && self.filter != Filter::Library
        {
            page.push(hero_card(covers, hero));
        }
        let sections: Vec<&SearchSection> = match &self.filter {
            Filter::Library => results.library.iter().collect(),
            Filter::All => self.shown.iter().collect(),
            Filter::Section(title) => self.shown.iter().filter(|s| s.title == *title).collect(),
        };
        if sections.is_empty() && self.filter == Filter::Library {
            page.push(note("Nothing in your library matches this search."));
        }
        for section in sections {
            page.push(self.section(covers, section, playing));
        }
        page
    }

    /// A section with its title, which drills into it (not in Library).
    fn section<'a>(
        &self,
        covers: &'a Covers,
        section: &'a SearchSection,
        playing: Option<&str>,
    ) -> Element<'a, Message> {
        let open = self.filter == Filter::Section(section.title.clone());
        let mut heading = row![ui::section_title(&section.title)]
            .spacing(4)
            .align_y(Alignment::Center);
        if self.filter != Filter::Library {
            let glyph = if open {
                icons::CHEVRON_LEFT
            } else {
                icons::CHEVRON_RIGHT
            };
            heading = heading.push(
                ui::icon(glyph, 18.0).style(|_, _| iced::widget::svg::Style {
                    color: Some(theme::SECONDARY),
                }),
            );
        }
        let title = button(heading)
            .padding([2, 6])
            .on_press_maybe(
                (self.filter != Filter::Library).then(|| wrap(Msg::Section(section.title.clone()))),
            )
            .style(|_, status| button::Style {
                background: (status == button::Status::Hovered)
                    .then_some(Background::Color(theme::HOVER)),
                border: Border::default().rounded(10),
                text_color: theme::TEXT,
                ..button::Style::default()
            });
        column![
            title,
            ui::section_items(covers, &section.items, usize::MAX, playing)
        ]
        .spacing(10)
        .into()
    }
}

/// Swift `TabHeaderView`'s glasses: teal while incognito is on.
fn incognito_button<'a>(on: bool) -> Element<'a, Message> {
    let glyph = ui::icon(icons::GLASSES, 22.0).style(move |_, _| iced::widget::svg::Style {
        color: Some(if on {
            theme::accent()
        } else {
            theme::SECONDARY
        }),
    });
    crate::downloads::hint(
        button(glyph)
            .padding(10)
            .on_press(wrap(Msg::Incognito))
            .style(|_, status| button::Style {
                background: (status == button::Status::Hovered)
                    .then_some(Background::Color(theme::HOVER)),
                border: Border::default().rounded(22),
                ..button::Style::default()
            }),
        if on {
            "Turn Off Incognito"
        } else {
            "Turn On Incognito"
        }
        .to_owned(),
    )
}

/// A recent search (with its delete button) or a suggestion: a click searches it.
fn query_row<'a>(query: &'a str, glyph: &'static [u8], earlier: bool) -> Element<'a, Message> {
    let color = if earlier {
        theme::accent()
    } else {
        theme::SECONDARY
    };
    let content = row![
        ui::icon(glyph, 18.0).style(move |_, _| iced::widget::svg::Style { color: Some(color) }),
        text(query).size(15),
    ]
    .spacing(14)
    .align_y(Alignment::Center)
    .width(Length::Fill);
    let mut line = row![ui::clickable(
        container(content).padding([4, 6]),
        Some(wrap(Msg::Pick(query.to_owned()))),
        6.0
    )]
    .align_y(Alignment::Center);
    if earlier {
        line = line.push(crate::downloads::hint(
            crate::transport_button(
                crate::title_bar::icons::CLOSE,
                14.0,
                Some(wrap(Msg::Delete(query.to_owned()))),
            ),
            "Delete".to_owned(),
        ));
    }
    line.into()
}

/// Swift's artist-first hero: round artwork, the kind, name and audience lines.
fn hero_card<'a>(covers: &'a Covers, hero: &'a Hero) -> Element<'a, Message> {
    const ART: f32 = 84.0;
    let artist = &hero.artist;
    let mut info = column![
        // ponytail: Swift asks MusicBrainz for the kind ("Band", "Singer");
        // not ported, so every hero says "Artist".
        text("Artist")
            .size(12)
            .font(theme::weight(Weight::Semibold))
            .color(theme::SECONDARY),
        text(artist.name.as_str())
            .size(22)
            .font(theme::weight(Weight::Bold)),
    ]
    .spacing(3);
    if let Some(detail) = &hero.detail {
        info = info.push(text(detail.as_str()).size(14).color(theme::SECONDARY));
    }
    let content = row![
        ui::cover(covers, artist.thumbnail_url.as_deref(), ART, ART / 2.0),
        info.width(Length::Fill),
        ui::icon(icons::CHEVRON_RIGHT, 16.0).style(|_, _| iced::widget::svg::Style {
            color: Some(theme::SECONDARY),
        }),
    ]
    .spacing(16)
    .align_y(Alignment::Center);
    container(
        button(container(content).padding([12, 16]))
            .padding(0)
            .on_press(Message::Navigate(Route::Artist(artist.browse_id.clone())))
            .style(|_, status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered => theme::PRESSED,
                    _ => theme::HOVER,
                })),
                border: Border::default().rounded(16),
                text_color: theme::TEXT,
                ..button::Style::default()
            }),
    )
    .max_width(640)
    .into()
}

fn note(line: &str) -> Element<'_, Message> {
    text(line).size(15).color(theme::SECONDARY).into()
}

/// Swift `ContentUnavailableView`: a title and an optional line under it.
fn empty<'a>(title: &'a str, line: Option<String>) -> Element<'a, Message> {
    let mut content = column![text(title).size(18).font(theme::weight(Weight::Bold))].spacing(6);
    if let Some(line) = line {
        content = content.push(text(line).size(14).color(theme::SECONDARY));
    }
    content.into()
}

/// Glyphs, white; `ui::icon` tints them.
mod icons {
    /// `magnifyingglass`.
    pub const SEARCH: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round"><circle cx="10.5" cy="10.5" r="6"/><path d="M15 15l5 5"/></g></svg>"##;
    /// A clock with an arrow running back round it: an earlier search.
    pub const HISTORY: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4.6 13a7.5 7.5 0 1 0 1.9-6.3L4 9.2"/><path d="M4 4.5v4.7h4.7"/><path d="M12 8v4.3l3 1.8"/></g></svg>"##;
    /// `eyeglasses`: incognito.
    pub const GLASSES: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="6.5" cy="14.5" r="3.5"/><circle cx="17.5" cy="14.5" r="3.5"/><path d="M10 14.2c1.3-.9 2.7-.9 4 0M3 14l1.6-6.2A1.8 1.8 0 0 1 6.4 6.5H7M21 14l-1.6-6.2a1.8 1.8 0 0 0-1.8-1.3H17"/></g></svg>"##;
    pub const CHEVRON_RIGHT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M9.5 6l6 6-6 6" fill="none" stroke="#fff" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
    pub const CHEVRON_LEFT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M14.5 6l-6 6 6 6" fill="none" stroke="#fff" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
}
