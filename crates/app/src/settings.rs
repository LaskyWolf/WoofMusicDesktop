//! The Settings page, one tab per Swift settings screen: Appearance, Player,
//! Content, Lyrics (`LyricsSettingsView`, `LyricsProviderOrderView`),
//! Integrations (`IntegrationsSettingsView`: Discord, Last.fm, ListenBrainz,
//! Odesli) and Diagnostics (`DiagnosticsView`); plus the player bar's "Copy song link".

use std::sync::Arc;

use iced::font::Weight;
use iced::widget::{
    Space, button, column, container, pick_list, row, slider, text, text_input, toggler,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Task};
use trop::audio::crossfade::TransitionSettings;
use trop::audio::lyrics::lyrics_manager::LyricsProviderRegistry;
use trop::odesli::odesli_defaults::OdesliDefaults;
use trop::settings::settings_store::{
    AUDIO_QUALITIES, COUNTRIES, DEFAULT_TABS, LibraryLayout, LyricsAlignment,
    PlayerBackgroundStyle, SettingsStore,
};

use crate::integrations::{self, Action, Status};
use crate::services::Services;
use crate::{Route, Session, theme, ui};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Appearance,
    Player,
    Content,
    Lyrics,
    Integrations,
    Diagnostics,
}

impl Tab {
    const ALL: [(Tab, &'static str); 6] = [
        (Tab::Appearance, "Appearance"),
        (Tab::Player, "Player"),
        (Tab::Content, "Content"),
        (Tab::Lyrics, "Lyrics"),
        (Tab::Integrations, "Integrations"),
        (Tab::Diagnostics, "Diagnostics"),
    ];
}

pub struct State {
    /// The saved settings; every change goes through `Message::Change`.
    pub prefs: SettingsStore,
    tab: Tab,
    /// Diagnostics' Copy Report was pressed: its button shows a check.
    report_copied: bool,
    /// `None` until the integrations thread first answers.
    status: Option<Status>,
    /// Why the status could not load, or the last toggle did not save.
    error: Option<String>,
    last_fm_waiting: bool,
    last_fm_error: Option<String>,
    token: String,
    listen_brainz_connecting: bool,
    listen_brainz_error: Option<String>,
    /// The song whose link was copied: its button shows a check.
    copied: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tab(Tab),
    /// A setting changed: saved, and handed to the services that read it.
    Change(SettingsStore),
    CopyReport(String),
    Load,
    Loaded(Result<Status, String>),
    /// A switch: `SetDiscord`, `SetScrobbling` or `SetShareLinks`.
    Set(Action),
    Saved(Result<Status, String>),
    LastFmConnect,
    LastFmDone(Result<Status, String>),
    LastFmDisconnect,
    TokenInput(String),
    ListenBrainzConnect,
    ListenBrainzDone(Result<Status, String>),
    ListenBrainzDisconnect,
    /// Copies the song.link page of this video.
    CopyLink(String),
}

/// Reads the integrations' status, at start and when Settings opens.
pub fn load() -> Task<crate::Message> {
    ask(Action::Status, Message::Loaded)
}

fn ask(action: Action, done: fn(Result<Status, String>) -> Message) -> Task<crate::Message> {
    Task::perform(integrations::request(action), move |result| {
        crate::Message::Settings(done(result))
    })
}

/// The country's English name, for the region menu.
fn country_name(code: &str) -> &'static str {
    match code {
        "US" => "United States",
        "GB" => "United Kingdom",
        "CA" => "Canada",
        "AU" => "Australia",
        "FR" => "France",
        "DE" => "Germany",
        "ES" => "Spain",
        "IT" => "Italy",
        "PT" => "Portugal",
        "JP" => "Japan",
        "KR" => "South Korea",
        "MX" => "Mexico",
        "BR" => "Brazil",
        "IN" => "India",
        _ => "Other",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Country(&'static str);

impl std::fmt::Display for Country {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(country_name(self.0))
    }
}

/// The settings file, `None` when there is no data folder.
fn prefs_path() -> Option<std::path::PathBuf> {
    SettingsStore::default_path()
}

impl State {
    /// The saved settings, handed to `services`. Dev aid: WOOF_SETTINGS=lyrics
    /// (or appearance, player, content, integrations, diagnostics) opens that tab.
    pub fn new(services: &Services) -> Self {
        let prefs =
            prefs_path().map_or_else(SettingsStore::default, |path| SettingsStore::load(&path));
        services.apply_settings(&prefs);
        let tab = std::env::var("WOOF_SETTINGS")
            .ok()
            .and_then(|name| {
                Tab::ALL
                    .into_iter()
                    .find(|(_, label)| label.eq_ignore_ascii_case(&name))
            })
            .map_or(Tab::default(), |(tab, _)| tab);
        Self {
            prefs,
            tab,
            report_copied: false,
            status: None,
            error: None,
            last_fm_waiting: false,
            last_fm_error: None,
            token: String::new(),
            listen_brainz_connecting: false,
            listen_brainz_error: None,
            copied: None,
        }
    }

    /// The sidebar page the app opens on (Swift `defaultTab`).
    pub fn start_route(&self) -> Route {
        match self.prefs.default_tab {
            1 => Route::Library,
            2 => Route::Explore,
            3 => Route::Search,
            _ => Route::Home,
        }
    }

    /// Saves `prefs` off the UI thread and hands them to the services; Home
    /// reloads when what it shows changed.
    fn change(&mut self, prefs: SettingsStore, services: &Arc<Services>) -> Task<crate::Message> {
        let home_changed = prefs.hide_explicit != self.prefs.hide_explicit
            || prefs.show_quick_picks != self.prefs.show_quick_picks
            || prefs.content_country != self.prefs.content_country;
        self.prefs = prefs.clone();
        services.apply_settings(&prefs);
        let save = match prefs_path() {
            Some(path) => Task::future(tokio::task::spawn_blocking(move || {
                if let Err(error) = prefs.save(&path) {
                    eprintln!("Settings were not saved: {error}");
                }
            }))
            .discard(),
            None => Task::none(),
        };
        if home_changed {
            Task::batch([save, crate::load_home(services)])
        } else {
            save
        }
    }

    pub fn share_links(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| status.share_links)
    }

    pub fn update(&mut self, message: Message, services: &Arc<Services>) -> Task<crate::Message> {
        match message {
            Message::Tab(tab) => {
                self.tab = tab;
                self.report_copied = false;
                Task::none()
            }
            Message::Change(prefs) => self.change(prefs, services),
            Message::CopyReport(report) => {
                self.report_copied = true;
                iced::clipboard::write(report)
            }
            Message::Load => load(),
            Message::Loaded(result) => {
                match result {
                    Ok(status) => {
                        self.status = Some(status);
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
                Task::none()
            }
            Message::Set(action) => {
                // The switch moves at once; the answer confirms it.
                if let Some(status) = &mut self.status {
                    match action {
                        Action::SetDiscord(on) => status.discord = on,
                        Action::SetScrobbling(on) => status.scrobbling = on,
                        Action::SetShareLinks(on) => status.share_links = on,
                        _ => {}
                    }
                }
                ask(action, Message::Saved)
            }
            Message::Saved(Ok(status)) => {
                self.status = Some(status);
                self.error = None;
                Task::none()
            }
            Message::Saved(Err(error)) => {
                self.error = Some(error);
                // Puts the switches back where the saved settings are.
                load()
            }
            Message::LastFmConnect => {
                if self.last_fm_waiting {
                    return Task::none();
                }
                self.last_fm_waiting = true;
                self.last_fm_error = None;
                ask(Action::LastFmSignIn, Message::LastFmDone)
            }
            Message::LastFmDone(result) => {
                self.last_fm_waiting = false;
                match result {
                    Ok(status) => self.status = Some(status),
                    Err(error) => self.last_fm_error = Some(error),
                }
                Task::none()
            }
            Message::LastFmDisconnect => ask(Action::LastFmSignOut, Message::Loaded),
            Message::TokenInput(token) => {
                self.token = token;
                Task::none()
            }
            Message::ListenBrainzConnect => {
                let token = self.token.trim().to_owned();
                if token.is_empty() || self.listen_brainz_connecting {
                    return Task::none();
                }
                self.listen_brainz_connecting = true;
                self.listen_brainz_error = None;
                ask(
                    Action::ListenBrainzConnect(token),
                    Message::ListenBrainzDone,
                )
            }
            Message::ListenBrainzDone(result) => {
                self.listen_brainz_connecting = false;
                match result {
                    Ok(status) => {
                        self.status = Some(status);
                        self.token.clear();
                    }
                    Err(error) => self.listen_brainz_error = Some(error),
                }
                Task::none()
            }
            Message::ListenBrainzDisconnect => ask(Action::ListenBrainzDisconnect, Message::Loaded),
            Message::CopyLink(video_id) => {
                let url = OdesliDefaults::song_page_url(&video_id);
                self.copied = Some(video_id);
                iced::clipboard::write(url)
            }
        }
    }

    /// `transitions` is `None` without libmpv: no transitions group then.
    pub fn view<'a>(
        &'a self,
        transitions: Option<&'a TransitionSettings>,
        skip_non_music: bool,
        services: &Services,
        session: &Session,
    ) -> Element<'a, crate::Message> {
        let tabs = row(Tab::ALL.map(|(tab, label)| {
            chip(
                label,
                self.tab == tab,
                crate::Message::Settings(Message::Tab(tab)),
            )
        }))
        .spacing(8)
        .wrap();
        let body = match self.tab {
            Tab::Appearance => self.appearance(),
            Tab::Player => self.player(transitions, skip_non_music),
            Tab::Content => self.content(),
            Tab::Lyrics => self.lyrics(),
            Tab::Integrations => self.integrations(),
            Tab::Diagnostics => self.diagnostics(services, session),
        };
        let page = column![
            text("Settings").size(32).font(theme::weight(Weight::Bold)),
            Space::new().height(16),
            tabs,
            Space::new().height(28),
            body,
        ]
        .max_width(720);
        ui::scroll_page(vec![page.into()])
    }

    /// A switch that saves `change` with its new state.
    fn toggle<'a>(
        &'a self,
        on: bool,
        change: fn(&mut SettingsStore, bool),
    ) -> Element<'a, crate::Message> {
        toggle(on, move |on| self.set(|p| change(p, on)))
    }

    /// A copy of the settings with one change, as the message that saves it.
    fn set(&self, change: impl FnOnce(&mut SettingsStore)) -> crate::Message {
        let mut prefs = self.prefs.clone();
        change(&mut prefs);
        crate::Message::Settings(Message::Change(prefs))
    }

    fn appearance(&self) -> Element<'_, crate::Message> {
        let prefs = &self.prefs;
        let swatches = theme::ACCENTS.iter().map(|&(name, color)| {
            // An unknown saved name shows the default as chosen, as it is applied.
            let chosen = theme::accent() == color;
            let mark: Element<'_, crate::Message> = if chosen {
                ui::icon(CHECK, 16.0).into()
            } else {
                Space::new().into()
            };
            button(container(mark).center(34))
                .padding(0)
                .on_press(self.set(|p| p.accent_name = name.to_owned()))
                .style(move |_, status| button::Style {
                    background: Some(Background::Color(color)),
                    border: Border::default().rounded(17).width(2).color(
                        if status == button::Status::Hovered {
                            theme::TEXT
                        } else {
                            Color::TRANSPARENT
                        },
                    ),
                    ..button::Style::default()
                })
                .into()
        });
        let accents = container(
            iced::widget::Row::with_children(swatches)
                .spacing(14)
                .wrap()
                .vertical_spacing(14),
        )
        .padding([16, 18]);
        sections(vec![
            (
                "Accent Color",
                "Applies to buttons, highlights, and selected items.",
                group(vec![accents.into()]),
            ),
            (
                "Player",
                "Album Art spreads the cover's colours behind Now Playing. Solid uses one colour from the cover.",
                group(vec![plain(
                    "Player Background",
                    segmented(PlayerBackgroundStyle::ALL.map(|style| {
                        (
                            style.display_name(),
                            prefs.player_background_style == style,
                            self.set(|p| p.player_background_style = style),
                        )
                    })),
                )]),
            ),
            (
                "Library",
                "",
                group(vec![plain(
                    "Library Layout",
                    segmented(LibraryLayout::ALL.map(|layout| {
                        (
                            layout.display_name(),
                            prefs.library_layout == layout,
                            self.set(|p| p.library_layout = layout),
                        )
                    })),
                )]),
            ),
            (
                "Lyrics",
                "",
                group(vec![plain(
                    "Lyrics Alignment",
                    segmented(LyricsAlignment::ALL.map(|alignment| {
                        (
                            alignment.display_name(),
                            prefs.lyrics_alignment == alignment,
                            self.set(|p| p.lyrics_alignment = alignment),
                        )
                    })),
                )]),
            ),
            (
                "General",
                "",
                group(vec![setting(
                    "Start Page",
                    caption("The page WoofMusic opens on."),
                    Some(segmented(DEFAULT_TABS.iter().enumerate().map(
                        |(index, label)| {
                            (
                                *label,
                                prefs.default_tab == index,
                                self.set(|p| p.default_tab = index),
                            )
                        },
                    ))),
                )]),
            ),
        ])
    }

    fn player<'a>(
        &'a self,
        transitions: Option<&'a TransitionSettings>,
        skip_non_music: bool,
    ) -> Element<'a, crate::Message> {
        let audio = group(vec![
            setting(
                "Audio Quality",
                caption("Applies from the next song. Downloads keep their own quality."),
                Some(segmented(AUDIO_QUALITIES.map(|(quality, label)| {
                    (
                        label,
                        self.prefs.audio_quality == quality,
                        self.set(|p| p.audio_quality = quality),
                    )
                }))),
            ),
            setting(
                "Equalizer",
                caption("Adjust the sound with presets or a custom curve."),
                Some(ui::pill_button(
                    "Open",
                    crate::Message::Navigate(crate::Route::Equalizer),
                )),
            ),
            setting(
                "Skip non-music parts",
                caption(
                    "Jumps over the talking intros and skits of music videos, marked by SponsorBlock users.",
                ),
                Some(toggle(skip_non_music, crate::Message::SkipNonMusic)),
            ),
            plain(
                "Audio Normalization",
                self.toggle(self.prefs.audio_normalization, |p, on| {
                    p.audio_normalization = on
                }),
            ),
            plain(
                "Gapless Playback",
                self.toggle(self.prefs.gapless_playback, |p, on| p.gapless_playback = on),
            ),
        ]);
        let queue = group(vec![
            plain(
                "Autoplay Similar",
                self.toggle(self.prefs.autoplay_similar, |p, on| p.autoplay_similar = on),
            ),
            plain(
                "Persist Queue",
                self.toggle(self.prefs.persist_queue, |p, on| p.persist_queue = on),
            ),
        ]);
        let play_pause = group(vec![plain(
            "Persistent Play/Pause",
            self.toggle(self.prefs.persistent_play_pause, |p, on| {
                p.persistent_play_pause = on
            }),
        )]);
        let mut list = vec![
            ("Audio", "", audio),
            (
                "Queue",
                "Persist Queue restores your queue, shuffle, and repeat state when the app relaunches.",
                queue,
            ),
            (
                "Playback",
                "On: if the player is paused, a new song you pick stays paused. Off: the new song starts playing.",
                play_pause,
            ),
        ];
        if let Some(transitions) = transitions {
            list.push((
                "Transitions",
                "AutoMix and Smart Crossfade start from the next song.",
                playback(transitions),
            ));
        }
        sections(list)
    }

    fn content(&self) -> Element<'_, crate::Message> {
        let prefs = &self.prefs;
        let toggle_row = |title: &'static str, on: bool, change: fn(&mut SettingsStore, bool)| {
            let prefs = prefs.clone();
            plain(
                title,
                toggle(on, move |on| {
                    let mut prefs = prefs.clone();
                    change(&mut prefs, on);
                    crate::Message::Settings(Message::Change(prefs))
                }),
            )
        };
        let countries: Vec<Country> = COUNTRIES.iter().map(|code| Country(code)).collect();
        let selected = COUNTRIES
            .iter()
            .find(|code| **code == prefs.content_country)
            .map(|code| Country(code));
        let region = {
            let prefs = prefs.clone();
            pick_list(countries, selected, move |Country(code)| {
                crate::Message::Settings(Message::Change(SettingsStore {
                    content_country: code.to_owned(),
                    ..prefs.clone()
                }))
            })
            .text_size(14)
            .padding([8, 14])
            .style(pick_style)
            .menu_style(menu_style)
        };
        sections(vec![
            (
                "Home",
                "",
                group(vec![
                    toggle_row("Hide Explicit Content", prefs.hide_explicit, |p, on| {
                        p.hide_explicit = on
                    }),
                    toggle_row("Show Quick Picks", prefs.show_quick_picks, |p, on| {
                        p.show_quick_picks = on
                    }),
                ]),
            ),
            (
                "Region",
                "The country YouTube Music answers for: charts, Explore and Home.",
                group(vec![plain("Country", region.into())]),
            ),
            (
                "Privacy",
                "When off, the songs you play are not recorded here: no play counts, no My Top 100.",
                group(vec![toggle_row(
                    "Track Play History",
                    prefs.track_play_history,
                    |p, on| p.track_play_history = on,
                )]),
            ),
            (
                "Library Sync",
                "Choose which library sections sync from your YouTube Music account.",
                group(vec![
                    toggle_row("Sync Artists", prefs.sync_artists, |p, on| {
                        p.sync_artists = on
                    }),
                    toggle_row("Sync Playlists", prefs.sync_playlists, |p, on| {
                        p.sync_playlists = on
                    }),
                    toggle_row("Sync Albums", prefs.sync_albums, |p, on| p.sync_albums = on),
                    toggle_row("Sync Podcasts", prefs.sync_podcasts, |p, on| {
                        p.sync_podcasts = on
                    }),
                    toggle_row("Sync Liked Songs", prefs.sync_songs, |p, on| {
                        p.sync_songs = on
                    }),
                ]),
            ),
        ])
    }

    /// Swift `LyricsSettingsView` and `LyricsProviderOrderView` in one list:
    /// each source with its switch and arrows that move it in the order.
    fn lyrics(&self) -> Element<'_, crate::Message> {
        let lyrics = self.prefs.lyrics_settings();
        let providers = LyricsProviderRegistry::all();
        let last = lyrics.provider_order.len().saturating_sub(1);
        let rows = lyrics
            .provider_order
            .iter()
            .enumerate()
            .filter_map(|(index, id)| {
                let provider = providers.iter().find(|p| p.id() == id)?;
                let id = provider.id();
                let on = !lyrics.disabled_providers.contains(id);
                let mover = |glyph, step: isize, enabled: bool| {
                    arrow_button(
                        glyph,
                        enabled.then(|| self.set(|p| p.move_lyrics_provider(id, step))),
                    )
                };
                let prefs = self.prefs.clone();
                let control = row![
                    mover(UP, -1, index > 0),
                    mover(DOWN, 1, index < last),
                    Space::new().width(12),
                    toggle(on, move |on| {
                        let mut prefs = prefs.clone();
                        prefs.set_lyrics_provider_enabled(id, on);
                        crate::Message::Settings(Message::Change(prefs))
                    }),
                ]
                .spacing(2)
                .align_y(Alignment::Center);
                Some(plain(
                    format!("{}. {}", index + 1, provider.name()),
                    control.into(),
                ))
            })
            .collect();
        sections(vec![(
            "Sources",
            "Lyrics are searched in this order; turned-off sources are skipped.",
            group(rows),
        )])
    }

    fn integrations(&self) -> Element<'_, crate::Message> {
        let Some(status) = &self.status else {
            return ui::status(
                self.error.as_deref(),
                crate::Message::Settings(Message::Load),
            );
        };
        let mut page = column![
            caption("Share what you play with other services."),
            Space::new().height(14),
            group(vec![
                self.discord(status),
                self.last_fm(status),
                self.listen_brainz(status),
                odesli(status),
            ]),
        ];
        if let Some(error) = &self.error {
            page = page.push(Space::new().height(10)).push(error_text(error));
        }
        page.into()
    }

    /// Swift `DiagnosticsView` without the iPhone parts (signing, widgets):
    /// the build, the system, the account and the settings file, and a
    /// button that copies them for a bug report.
    fn diagnostics(&self, services: &Services, session: &Session) -> Element<'_, crate::Message> {
        let report = diagnostics(&self.prefs, services, session);
        let mut text_report = Vec::new();
        let mut list = Vec::new();
        for (title, rows) in report {
            text_report.push(format!("## {title}"));
            let mut cells = Vec::new();
            for (label, value) in rows {
                text_report.push(format!("{label}: {value}"));
                cells.push(
                    row![
                        text(label)
                            .size(14)
                            .font(theme::weight(Weight::Medium))
                            .width(Length::Fixed(180.0)),
                        text(value).size(14).color(theme::SECONDARY),
                    ]
                    .spacing(16)
                    .padding([12, 18])
                    .into(),
                );
            }
            list.push((title, "", group(cells)));
        }
        let copy = ui::accent_button(
            if self.report_copied {
                "Copied"
            } else {
                "Copy Report"
            },
            None,
            Some(crate::Message::Settings(Message::CopyReport(
                text_report.join("\n"),
            ))),
        );
        column![
            caption("Paste the report into a bug report so the problem can be found."),
            Space::new().height(12),
            copy,
            Space::new().height(28),
            sections(list),
        ]
        .into()
    }

    fn discord<'a>(&self, status: &Status) -> Element<'a, crate::Message> {
        let detail = match (status.discord, status.discord_connected) {
            (true, true) => "Showing the song you're playing on your Discord profile.",
            _ => "Show the song you're playing on your Discord profile while the Discord app runs.",
        };
        setting(
            "Discord",
            caption(detail),
            Some(switch(status.discord, Action::SetDiscord)),
        )
    }

    fn last_fm<'a>(&'a self, status: &'a Status) -> Element<'a, crate::Message> {
        if !status.last_fm_available {
            return setting(
                "Last.fm",
                caption("Last.fm isn't available in this build."),
                Some(quiet_button("Connect", None)),
            );
        }
        let Some(user) = &status.last_fm_user else {
            let mut detail = column![caption(if self.last_fm_waiting {
                "Approve WoofMusic on the Last.fm page in your browser."
            } else {
                "Scrobble the songs you play to your Last.fm profile."
            })]
            .spacing(4);
            if let Some(error) = &self.last_fm_error {
                detail = detail.push(error_text(error));
            }
            return setting(
                "Last.fm",
                detail.into(),
                Some(ui::accent_button(
                    if self.last_fm_waiting {
                        "Waiting…"
                    } else {
                        "Connect"
                    },
                    None,
                    (!self.last_fm_waiting)
                        .then_some(crate::Message::Settings(Message::LastFmConnect)),
                )),
            );
        };
        let name = if user.is_empty() { "Connected" } else { user };
        column![
            setting(
                "Last.fm",
                caption(format!("Connected as {name}")),
                Some(quiet_button("Disconnect", Some(Message::LastFmDisconnect))),
            ),
            divider(),
            setting(
                "Scrobble to Last.fm",
                caption("A song counts once half of it, or 3 minutes, has played."),
                Some(switch(status.scrobbling, Action::SetScrobbling)),
            ),
        ]
        .into()
    }

    fn listen_brainz<'a>(&'a self, status: &'a Status) -> Element<'a, crate::Message> {
        if let Some(user) = &status.listen_brainz_user {
            return setting(
                "ListenBrainz",
                caption(format!(
                    "Connected as {user}. Songs count once half of them, or 4 minutes, has played."
                )),
                Some(quiet_button(
                    "Disconnect",
                    Some(Message::ListenBrainzDisconnect),
                )),
            );
        }
        let connecting = self.listen_brainz_connecting;
        let can_connect = !connecting && !self.token.trim().is_empty();
        let mut field = text_input("User token", &self.token)
            .secure(true)
            .size(14)
            .padding([9, 14])
            .style(ui::field_style);
        if !connecting {
            field = field
                .on_input(|token| crate::Message::Settings(Message::TokenInput(token)))
                .on_submit(crate::Message::Settings(Message::ListenBrainzConnect));
        }
        let mut detail = column![
            caption("Copy your user token from listenbrainz.org/settings."),
            row![
                field,
                ui::accent_button(
                    if connecting {
                        "Connecting…"
                    } else {
                        "Connect"
                    },
                    None,
                    can_connect.then_some(crate::Message::Settings(Message::ListenBrainzConnect)),
                ),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        ]
        .spacing(10);
        if let Some(error) = &self.listen_brainz_error {
            detail = detail.push(error_text(error));
        }
        setting("ListenBrainz", detail.into(), None)
    }

    /// The player bar's "Copy song link" button; `None` while Odesli is off.
    pub fn copy_link_button<'a>(
        &self,
        video_id: Option<&str>,
    ) -> Option<Element<'a, crate::Message>> {
        if !self.share_links() {
            return None;
        }
        let copied = video_id.is_some() && self.copied.as_deref() == video_id;
        let message = video_id.map(|id| crate::Message::Settings(Message::CopyLink(id.to_owned())));
        Some(
            crate::transport_button(if copied { CHECK } else { LINK }, 20.0, message)
                .style(|_, status| button::Style {
                    background: (status == button::Status::Hovered)
                        .then_some(Background::Color(theme::PRESSED)),
                    border: Border::default().rounded(22),
                    ..button::Style::default()
                })
                .into(),
        )
    }
}

fn odesli<'a>(status: &Status) -> Element<'a, crate::Message> {
    setting(
        "Odesli song links",
        caption(
            "Adds Copy song link to the player bar. The song.link page lists the song on every service.",
        ),
        Some(switch(status.share_links, Action::SetShareLinks)),
    )
}

/// Swift `PlayerSettingsView`'s transitions: AutoMix, Smart Crossfade and its length.
fn playback(settings: &TransitionSettings) -> Element<'_, crate::Message> {
    let set = move |change: fn(&mut TransitionSettings, bool)| {
        let settings = settings.clone();
        move |on| {
            let mut settings = settings.clone();
            change(&mut settings, on);
            crate::Message::Transitions(settings)
        }
    };
    let mut rows = vec![
        setting(
            "AutoMix",
            caption(
                "Blends songs like a DJ: matches their tempo, keeps the beats together and \
                 swaps the bass on the bar. Songs without a steady beat get a Smart Crossfade.",
            ),
            Some(toggle(
                settings.auto_mix_enabled,
                set(|s, on| s.auto_mix_enabled = on),
            )),
        ),
        setting(
            "Smart Crossfade",
            caption("Fades into the next song where the current one winds down."),
            Some(toggle(
                settings.crossfade_enabled,
                set(|s, on| s.crossfade_enabled = on),
            )),
        ),
    ];
    if settings.crossfade_enabled {
        let seconds = settings.crossfade_seconds;
        let length = {
            let settings = settings.clone();
            move |seconds| {
                crate::Message::Transitions(TransitionSettings {
                    crossfade_seconds: seconds,
                    ..settings.clone()
                })
            }
        };
        rows.push(
            row![
                text(format!("Fade {} s", seconds.round() as i64))
                    .size(14)
                    .width(Length::Fixed(72.0)),
                slider(1.0..=12.0, seconds, length)
                    .step(1.0)
                    .style(crate::seek_style),
            ]
            .spacing(16)
            .padding([14, 18])
            .align_y(Alignment::Center)
            .into(),
        );
    }
    group(rows)
}

/// Rows on one rounded surface, a hairline between each.
pub(crate) fn group(rows: Vec<Element<'_, crate::Message>>) -> Element<'_, crate::Message> {
    let mut list = column![];
    for (i, row) in rows.into_iter().enumerate() {
        if i > 0 {
            list = list.push(divider());
        }
        list = list.push(row);
    }
    container(list)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::PLACEHOLDER)),
            border: Border::default().rounded(16),
            ..container::Style::default()
        })
        .into()
}

fn divider<'a>() -> Element<'a, crate::Message> {
    container(Space::new().width(Length::Fill).height(1))
        .padding([0, 18])
        .style(|_| container::Style {
            background: Some(Background::Color(theme::HOVER)),
            ..container::Style::default()
        })
        .into()
}

/// A setting: its name and details on the left, its control on the right.
pub(crate) fn setting<'a>(
    title: impl text::IntoFragment<'a>,
    detail: Element<'a, crate::Message>,
    control: Option<Element<'a, crate::Message>>,
) -> Element<'a, crate::Message> {
    row![
        column![
            text(title).size(15).font(theme::weight(Weight::Medium)),
            detail,
        ]
        .spacing(4)
        .width(Length::Fill),
    ]
    .push(control)
    .spacing(24)
    .padding([16, 18])
    .align_y(Alignment::Center)
    .into()
}

pub(crate) fn caption<'a>(line: impl text::IntoFragment<'a>) -> Element<'a, crate::Message> {
    text(line).size(13).color(theme::SECONDARY).into()
}

fn error_text<'a>(error: &'a str) -> Element<'a, crate::Message> {
    text(error).size(13).color(theme::CLOSE).into()
}

fn switch<'a>(on: bool, action: fn(bool) -> Action) -> Element<'a, crate::Message> {
    toggle(on, move |on| {
        crate::Message::Settings(Message::Set(action(on)))
    })
}

pub(crate) fn toggle<'a>(
    on: bool,
    message: impl Fn(bool) -> crate::Message + 'a,
) -> Element<'a, crate::Message> {
    toggler(on)
        .on_toggle(message)
        .size(22)
        .style(|theme, status| {
            let on = matches!(
                status,
                toggler::Status::Active { is_toggled: true }
                    | toggler::Status::Hovered { is_toggled: true }
            );
            toggler::Style {
                background: Background::Color(if on { theme::accent() } else { theme::PRESSED }),
                foreground: Background::Color(if on { theme::BACKGROUND } else { theme::TEXT }),
                background_border_width: 0.0,
                foreground_border_width: 0.0,
                ..toggler::default(theme, status)
            }
        })
        .into()
}

/// The light pill of `ui::pill_button`, dimmed while it does nothing.
fn quiet_button<'a>(label: &'a str, message: Option<Message>) -> Element<'a, crate::Message> {
    button(text(label).size(14).font(theme::weight(Weight::Medium)))
        .padding([8, 16])
        .on_press_maybe(message.map(crate::Message::Settings))
        .style(|_, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered => theme::PRESSED,
                _ => theme::HOVER,
            })),
            border: Border::default().rounded(18),
            text_color: match status {
                button::Status::Disabled => Color {
                    a: 0.4,
                    ..theme::TEXT
                },
                _ => theme::TEXT,
            },
            ..button::Style::default()
        })
        .into()
}

/// A chain link.
const LINK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 14a4.5 4.5 0 0 0 6.4 0l3.2-3.2a4.5 4.5 0 0 0-6.4-6.4L11.8 5.8"/><path d="M14 10a4.5 4.5 0 0 0-6.4 0l-3.2 3.2a4.5 4.5 0 0 0 6.4 6.4l1.4-1.4"/></g></svg>"##;
const CHECK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 7.5" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;

/// A setting with only a name and its control.
fn plain<'a>(
    title: impl text::IntoFragment<'a>,
    control: Element<'a, crate::Message>,
) -> Element<'a, crate::Message> {
    row![
        text(title)
            .size(15)
            .font(theme::weight(Weight::Medium))
            .width(Length::Fill),
        control,
    ]
    .spacing(24)
    .padding([14, 18])
    .align_y(Alignment::Center)
    .into()
}

/// Titled groups, each with an optional line under its title.
fn sections<'a>(
    list: Vec<(&'a str, &'a str, Element<'a, crate::Message>)>,
) -> Element<'a, crate::Message> {
    let mut page = column![];
    for (index, (title, detail, body)) in list.into_iter().enumerate() {
        if index > 0 {
            page = page.push(Space::new().height(32));
        }
        page = page.push(ui::section_title(title));
        if !detail.is_empty() {
            page = page.push(Space::new().height(4)).push(caption(detail));
        }
        page = page.push(Space::new().height(12)).push(body);
    }
    page.into()
}

/// A pill like the Library filters: white when selected.
fn chip<'a>(
    label: &'a str,
    selected: bool,
    message: crate::Message,
) -> Element<'a, crate::Message> {
    button(text(label).size(14).font(theme::weight(Weight::Medium)))
        .padding([7, 16])
        .on_press(message)
        .style(move |_, status| button::Style {
            background: Some(Background::Color(match (selected, status) {
                (true, _) => theme::TEXT,
                (false, button::Status::Hovered) => theme::PRESSED,
                (false, _) => theme::HOVER,
            })),
            border: Border::default().rounded(18),
            text_color: if selected {
                theme::BACKGROUND
            } else {
                theme::TEXT
            },
            ..button::Style::default()
        })
        .into()
}

/// One choice of a few, as touching segments on one track.
fn segmented<'a>(
    options: impl IntoIterator<Item = (&'a str, bool, crate::Message)>,
) -> Element<'a, crate::Message> {
    let segments = row(options.into_iter().map(|(label, selected, message)| {
        button(text(label).size(13).font(theme::weight(Weight::Medium)))
            .padding([6, 12])
            .on_press(message)
            .style(move |_, status| button::Style {
                background: match (selected, status) {
                    (true, _) => Some(Background::Color(theme::TEXT)),
                    (false, button::Status::Hovered) => Some(Background::Color(theme::HOVER)),
                    _ => None,
                },
                border: Border::default().rounded(14),
                text_color: if selected {
                    theme::BACKGROUND
                } else {
                    theme::TEXT
                },
                ..button::Style::default()
            })
            .into()
    }))
    .spacing(2);
    container(segments)
        .padding(3)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::HOVER)),
            border: Border::default().rounded(17),
            ..container::Style::default()
        })
        .into()
}

/// Moves a lyrics source up or down; dimmed at the end of the list.
fn arrow_button<'a>(
    glyph: &'static [u8],
    message: Option<crate::Message>,
) -> Element<'a, crate::Message> {
    crate::transport_button(glyph, 18.0, message).into()
}

fn pick_style(_: &iced::Theme, status: pick_list::Status) -> pick_list::Style {
    pick_list::Style {
        text_color: theme::TEXT,
        placeholder_color: theme::SECONDARY,
        handle_color: theme::SECONDARY,
        background: Background::Color(match status {
            pick_list::Status::Hovered | pick_list::Status::Opened { .. } => theme::PRESSED,
            pick_list::Status::Active => theme::HOVER,
        }),
        border: Border::default().rounded(16),
    }
}

fn menu_style(_: &iced::Theme) -> iced::overlay::menu::Style {
    iced::overlay::menu::Style {
        background: Background::Color(Color::from_rgb8(0x1C, 0x1C, 0x1E)),
        border: Border::default().rounded(12),
        text_color: theme::TEXT,
        selected_text_color: theme::BACKGROUND,
        selected_background: Background::Color(theme::TEXT),
        shadow: iced::Shadow::default(),
    }
}

/// The Diagnostics groups: title, then label and value rows.
fn diagnostics(
    prefs: &SettingsStore,
    services: &Services,
    session: &Session,
) -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    let lyrics = prefs.lyrics_settings();
    let providers = LyricsProviderRegistry::all();
    let sources: Vec<&str> = lyrics
        .provider_order
        .iter()
        .filter(|id| !lyrics.disabled_providers.contains(*id))
        .filter_map(|id| providers.iter().find(|p| p.id() == id).map(|p| p.name()))
        .collect();
    let (signed_in, name) = match session {
        Session::SignedIn(Some(Ok(info))) => ("Yes", info.name.clone()),
        Session::SignedIn(Some(Err(error))) => ("Yes", format!("Not loaded: {error}")),
        Session::SignedIn(None) => ("Yes", "Loading".to_owned()),
        Session::Guest | Session::SigningIn => ("No", "–".to_owned()),
    };
    let on_off = |on: bool| if on { "On" } else { "Off" }.to_owned();
    vec![
        (
            "App",
            vec![
                ("Version", env!("CARGO_PKG_VERSION").to_owned()),
                (
                    "System",
                    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
                ),
                (
                    "Language",
                    std::env::var("LANG").unwrap_or_else(|_| "Unknown".to_owned()),
                ),
                (
                    "Settings File",
                    prefs_path().map_or("None (no data folder)".to_owned(), |path| {
                        path.display().to_string()
                    }),
                ),
            ],
        ),
        (
            "Playback",
            vec![
                (
                    "libmpv",
                    if services.player.is_some() {
                        "Running"
                    } else {
                        "Did not start"
                    }
                    .to_owned(),
                ),
                (
                    "Audio Quality",
                    AUDIO_QUALITIES
                        .iter()
                        .find(|(quality, _)| *quality == prefs.audio_quality)
                        .map_or("Auto", |(_, label)| label)
                        .to_owned(),
                ),
            ],
        ),
        (
            "Account",
            vec![
                ("Signed In", signed_in.to_owned()),
                ("Name", name),
                ("Play History", on_off(prefs.track_play_history)),
            ],
        ),
        (
            "Lyrics",
            vec![
                (
                    "Sources",
                    if sources.is_empty() {
                        "None".to_owned()
                    } else {
                        sources.join(", ")
                    },
                ),
                ("Karaoke", on_off(services.karaoke_enabled())),
                ("Offset", prefs.offset_description()),
            ],
        ),
    ]
}

const UP: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 15l6-6 6 6" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
const DOWN: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 9l6 6 6-6" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
