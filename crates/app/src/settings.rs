//! The Settings page: the integrations of Swift `IntegrationsSettingsView`
//! (Discord, Last.fm, ListenBrainz, Odesli), and the player bar's "Copy song link".

use iced::font::Weight;
use iced::widget::{Space, button, column, container, row, slider, text, text_input, toggler};
use iced::{Alignment, Background, Border, Color, Element, Length, Task};
use trop::audio::crossfade::TransitionSettings;
use trop::odesli::odesli_defaults::OdesliDefaults;

use crate::integrations::{self, Action, Status};
use crate::{theme, ui};

#[derive(Default)]
pub struct State {
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

impl State {
    pub fn share_links(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|status| status.share_links)
    }

    pub fn update(&mut self, message: Message) -> Task<crate::Message> {
        match message {
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

    /// `transitions` is `None` without libmpv: no Playback group then.
    pub fn view<'a>(
        &'a self,
        transitions: Option<&'a TransitionSettings>,
        skip_non_music: bool,
    ) -> Element<'a, crate::Message> {
        let Some(status) = &self.status else {
            return ui::status(
                self.error.as_deref(),
                crate::Message::Settings(Message::Load),
            );
        };
        let mut page = column![
            text("Settings").size(32).font(theme::weight(Weight::Bold)),
            Space::new().height(20),
        ]
        .max_width(720);
        if let Some(transitions) = transitions {
            page = page
                .push(ui::section_title("Playback"))
                .push(Space::new().height(14))
                .push(playback(transitions, skip_non_music))
                .push(Space::new().height(28));
        }
        let mut page = page.extend([column![
            ui::section_title("Integrations"),
            Space::new().height(4),
            text("Share what you play with other services.")
                .size(14)
                .color(theme::SECONDARY),
            Space::new().height(14),
            group(vec![
                self.discord(status),
                self.last_fm(status),
                self.listen_brainz(status),
                odesli(status),
            ]),
        ]
        .into()]);
        if let Some(error) = &self.error {
            page = page.push(Space::new().height(10)).push(error_text(error));
        }
        ui::scroll_page(vec![page.into()])
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
fn playback(settings: &TransitionSettings, skip_non_music: bool) -> Element<'_, crate::Message> {
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
    title: &'a str,
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
                background: Background::Color(if on { theme::ACCENT } else { theme::PRESSED }),
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
