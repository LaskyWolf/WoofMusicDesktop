//! The song menu (Swift `SongActionsMenu`): right-click a song, or the
//! player bar's More button, to get Play Next, Add to Queue, Like, Add to
//! Playlist, Start Radio (with its options), Go To and Share. The playing song's menu also
//! has the Sleep Timer and Playback Speed.

use std::time::Instant;

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{self, Tree, Widget};
use iced::advanced::{Clipboard, Shell, mouse, overlay, renderer};
use iced::font::Weight;
use iced::widget::{
    Space, button, column, container, mouse_area, row, scrollable, svg, text, text_input,
};
use iced::{
    Alignment, Background, Border, Element, Event, Length, Padding, Point, Rectangle, Size, Vector,
};
use trop::audio::playback_speed::PlaybackSpeed;
use trop::audio::sleep_timer::{self, SleepTimer};
use trop::database::entities::playlist_entity::PlaylistEntity;
use trop::networking::models::yt_item::{SongItem, YTArtist};
use trop::services::artist_kind_service;
use trop::services::personalization_service::{RadioOptions, RadioStyle};

use crate::services::SongDetails;
use crate::{Message, Route, theme, ui};

const WIDTH: f32 = 232.0;
/// Song Details lists long values: its panel is wider.
const DETAILS_WIDTH: f32 = 380.0;
pub(crate) const ITEM: f32 = 36.0;
pub(crate) const PAD: f32 = 6.0;
/// The glass shader's id for the menu panel (1–3 are the sidebar, player bar and queue).
pub(crate) const GLASS: u64 = 4;

/// An open menu.
pub struct Menu {
    pub song: SongItem,
    /// Window position of the click; the menu opens there.
    pub at: Point,
    /// The song's place in the queue, when the menu came from the queue panel.
    pub queue_index: Option<usize>,
    pub page: Page,
}

/// What the menu panel lists.
#[derive(Clone, Debug)]
pub enum Page {
    Actions,
    /// After Add to Playlist: the playlists, `None` while they load.
    Playlists(Option<Vec<PlaylistEntity>>),
    /// New Playlist from Add to Playlist: the name typed so far.
    NewPlaylist(String),
    Sleep,
    Speed,
    /// Swift `RadioOptionsSheet`: the options Start Radio uses.
    Radio(RadioOptions),
    /// Swift `ArtistPickerSheet`: the song's artists, to open one.
    Artists,
    /// Swift `SongDetailsView`: `None` while the extras load.
    Details(Option<Box<SongDetails>>),
    /// Remove Download asks before deleting the file.
    ConfirmRemove,
}

/// What the menu needs from the app to label its items.
pub struct Context<'a> {
    pub liked: bool,
    /// The song is saved for offline: Remove Download is offered.
    pub downloaded: bool,
    /// Like and Add to Playlist need an account.
    pub account: bool,
    /// The menu's song is the one playing: Sleep Timer and Speed apply to it.
    pub playing: bool,
    pub sleep: &'a SleepTimer,
    /// The Listen Together room's code, while in one.
    pub room: Option<&'a str>,
    /// In a room, not its host: songs can be suggested.
    pub guest: bool,
    pub speed: f64,
    pub window: Size,
}

#[derive(Clone, Debug)]
pub enum Action {
    PlayNext,
    AddToQueue,
    Like(bool),
    AddToPlaylist,
    AddTo(String, String),
    NewPlaylist,
    PlaylistName(String),
    /// Creates the playlist with the typed name and adds the song.
    CreatePlaylist,
    /// Opens the radio options.
    StartRadio,
    RadioStyle(RadioStyle),
    /// Tracks: 5...50 in steps of 5.
    RadioLimit(usize),
    RadioExplicit(bool),
    /// Starts the radio with the options on the page.
    RadioStart,
    GoTo(Route),
    CopyLink,
    /// Asks the Listen Together host to play the song.
    Suggest,
    /// Opens the Listen Together panel.
    ListenTogether,
    /// Opens the share card studio for the song.
    ShareCard,
    Download,
    /// Deletes the saved file, after the confirmation page.
    RemoveDownload,
    /// Opens an artist; one without an id is looked up by name.
    OpenArtist(YTArtist),
    /// Loads the artists' kinds for the picker.
    ShowArtists,
    ShowDetails,
    RemoveFromQueue(usize),
    Open(Page),
    /// Starts a countdown of this many minutes, or turns the timer off.
    Sleep(Option<u64>),
    SleepAtSongEnd,
    Speed(f64),
}

/// The speeds the menu offers (`PlaybackSpeed.presets` plus both ends).
const SPEEDS: [f64; 8] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0];

/// The menu over the whole window: a click outside closes it.
pub fn view<'a>(menu: &'a Menu, context: Context<'_>) -> Element<'a, Message> {
    let window = context.window;
    let items: Vec<Element<'a, Message>> = match &menu.page {
        Page::Actions => actions(menu, &context),
        Page::Sleep => sleep_items(context.sleep),
        Page::Speed => SPEEDS
            .iter()
            .map(|&rate| {
                checked(
                    &PlaybackSpeed::label(rate),
                    Action::Speed(rate),
                    (context.speed - rate).abs() < 0.001,
                )
            })
            .collect(),
        Page::Radio(options) => radio_items(options),
        Page::Playlists(list) => {
            let mut items = vec![item(icons::PLUS, "New Playlist…", Action::NewPlaylist)];
            match list {
                None => items.push(note("Loading playlists…")),
                Some(list) => items.extend(list.iter().map(|playlist| {
                    item(
                        icons::PLAYLIST,
                        &playlist.name,
                        Action::AddTo(playlist.id.clone(), playlist.name.clone()),
                    )
                })),
            }
            items
        }
        Page::NewPlaylist(name) => new_playlist_items(name),
        Page::Artists => artist_items(&menu.song),
        Page::Details(details) => detail_items(&menu.song, details.as_deref()),
        Page::ConfirmRemove => vec![
            note("Remove this download?"),
            item(icons::REMOVE, "Remove Download", Action::RemoveDownload),
            item(icons::BACK, "Cancel", Action::Open(Page::Actions)),
        ],
    };
    let width = match menu.page {
        Page::Details(_) => DETAILS_WIDTH,
        _ => WIDTH,
    };
    let rows = items.len() as f32;
    let height = (rows * ITEM + 2.0 * PAD).min(window.height - 2.0 * theme::INSET);
    let list = scrollable(column(items).padding(PAD)).direction(scrollable::Direction::Vertical(
        scrollable::Scrollbar::hidden(),
    ));
    let panel = ui::glass_panel(
        GLASS,
        14.0,
        Length::Fixed(width),
        Length::Fixed(height),
        list,
    );
    // Below and right of the cursor, flipped where the window ends.
    let x = menu
        .at
        .x
        .min(window.width - width - theme::INSET)
        .max(theme::INSET);
    let y = if menu.at.y + height > window.height - theme::INSET {
        (menu.at.y - height).max(theme::INSET)
    } else {
        menu.at.y
    };
    let backdrop = mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
        .on_press(Message::CloseMenu)
        .on_right_press(Message::CloseMenu);
    iced::widget::stack![
        backdrop,
        container(panel).padding(Padding {
            top: y,
            left: x,
            ..Padding::ZERO
        }),
    ]
    .into()
}

fn actions<'a>(menu: &Menu, context: &Context<'_>) -> Vec<Element<'a, Message>> {
    let song = &menu.song;
    let mut items = Vec::new();
    // Swift `AddedByMenuItem`; the avatar is not drawn.
    if let Some(adder) = &song.added_by {
        let line = if adder.is_owner {
            format!("Added by {} · Owner", adder.name)
        } else {
            format!("Added by {}", adder.name)
        };
        items.push(item_note(icons::ARTIST, &line));
    }
    items.extend([
        item(icons::PLAY_NEXT, "Play Next", Action::PlayNext),
        item(icons::ADD_QUEUE, "Add to Queue", Action::AddToQueue),
    ]);
    if context.guest {
        items.push(item(
            crate::together::PEOPLE,
            "Suggest to Room",
            Action::Suggest,
        ));
    }
    if context.account {
        items.push(if context.liked {
            item(icons::HEART_FILL, "Unlike", Action::Like(false))
        } else {
            item(icons::HEART, "Like", Action::Like(true))
        });
        items.push(item(
            icons::PLAYLIST,
            "Add to Playlist…",
            Action::AddToPlaylist,
        ));
    }
    items.push(item(icons::RADIO, "Start Radio…", Action::StartRadio));
    if let Some(id) = song.album_id.clone() {
        items.push(item(
            icons::ALBUM,
            "Go to Album",
            Action::GoTo(Route::Album(id)),
        ));
    }
    let named: Vec<&YTArtist> = song
        .artists
        .iter()
        .filter(|artist| !artist.name.is_empty())
        .collect();
    // Swift `handleViewArtist`: several artists ask which; one opens, by name when it has no id.
    match named.as_slice() {
        [] => {}
        [artist] => items.push(item(
            icons::ARTIST,
            "Go to Artist",
            match &artist.id {
                Some(id) => Action::GoTo(Route::Artist(id.clone())),
                None => Action::OpenArtist((*artist).clone()),
            },
        )),
        _ => items.push(item(icons::ARTIST, "Go to Artist…", Action::ShowArtists)),
    }
    items.push(item(
        icons::SIMILAR,
        "Similar Songs…",
        Action::GoTo(Route::Similar(song.video_id.clone())),
    ));
    items.push(item(icons::INFO, "Song Details…", Action::ShowDetails));
    if context.downloaded {
        items.push(item(
            icons::REMOVE,
            "Remove Download",
            Action::Open(Page::ConfirmRemove),
        ));
    } else {
        items.push(item(icons::DOWNLOAD, "Download", Action::Download));
    }
    items.push(item(icons::LINK, "Copy Song Link", Action::CopyLink));
    items.push(item(icons::SHARE, "Share Card…", Action::ShareCard));
    if let Some(index) = menu.queue_index {
        items.push(item(
            icons::REMOVE,
            "Remove from Queue",
            Action::RemoveFromQueue(index),
        ));
    }
    if context.playing {
        let sleep = match context.sleep.summary(Instant::now()) {
            Some(summary) => format!("Sleep Timer · {summary}"),
            None => "Sleep Timer…".to_owned(),
        };
        items.push(item(icons::MOON, &sleep, Action::Open(Page::Sleep)));
        let together = context.room.map_or("Listen Together…".to_owned(), |code| {
            format!("Listen Together · {code}")
        });
        items.push(item(
            crate::together::PEOPLE,
            &together,
            Action::ListenTogether,
        ));
        let speed = format!("Speed · {}", PlaybackSpeed::label(context.speed));
        items.push(item(icons::SPEED, &speed, Action::Open(Page::Speed)));
    }
    items
}

/// The artists with their kind from MusicBrainz, once it answered.
fn artist_items<'a>(song: &SongItem) -> Vec<Element<'a, Message>> {
    song.artists
        .iter()
        .filter(|artist| !artist.name.is_empty())
        .map(|artist| {
            let label = match artist_kind_service::cached_kind(&artist.name) {
                Some(kind) => format!("{} · {kind}", artist.name),
                None => artist.name.clone(),
            };
            let action = match &artist.id {
                Some(id) => Action::GoTo(Route::Artist(id.clone())),
                None => Action::OpenArtist(artist.clone()),
            };
            item(icons::ARTIST, &label, action)
        })
        .collect()
}

/// A line of text with an icon that does nothing when pressed.
fn item_note<'a>(icon: &'static [u8], label: &str) -> Element<'a, Message> {
    container(
        row![
            ui::icon(icon, 18.0).style(|_, _| svg::Style {
                color: Some(theme::SECONDARY)
            }),
            text(ui::fit(label, WIDTH - 60.0, 14.0))
                .size(14)
                .color(theme::SECONDARY)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .center_y(ITEM)
    .padding([0, 10])
    .into()
}

/// Swift `SongDetailsView`: Track, Stream and Stats.
fn detail_items<'a>(song: &SongItem, details: Option<&SongDetails>) -> Vec<Element<'a, Message>> {
    use swiftytube::formatting::count_format::CountFormatInt;
    use swiftytube::formatting::duration_format::DurationFormatInt;

    let duration = if song.duration > 0 {
        song.duration
    } else {
        details.and_then(|d| d.duration).unwrap_or(0)
    };
    let mut rows = vec![heading("Track")];
    rows.push(detail("Title", &song.title));
    rows.push(detail("Artists", &song.artist_names_display()));
    if let Some(album) = &song.album {
        rows.push(detail("Album", album));
    }
    rows.push(detail("Duration", &duration.formatted_duration()));
    rows.push(detail("Video ID", &song.video_id));
    rows.push(heading("Stream"));
    match details {
        None => rows.push(detail("Status", "Loading…")),
        Some(details) => {
            if let Some(stream) = &details.stream {
                rows.push(detail("Client", &stream.client_name));
                rows.push(detail("ITAG", &stream.itag.to_string()));
                rows.push(detail(
                    "Bitrate",
                    &format!("{} kbps", stream.bitrate / 1000),
                ));
                rows.push(detail(
                    "Audio quality",
                    &stream.audio_quality.replace("AUDIO_QUALITY_", ""),
                ));
                if let Some(loudness) = stream.loudness_db {
                    rows.push(detail("Loudness", &format!("{loudness:.1} dB")));
                }
                rows.push(detail(
                    "Expires in",
                    &format!("{}s", stream.expires_in_seconds),
                ));
            } else if let Some(format) = &details.format {
                rows.push(detail("Source", "Cached format"));
                rows.push(detail("ITAG", &format.itag.to_string()));
                rows.push(detail(
                    "Bitrate",
                    &format!("{} kbps", format.bitrate / 1000),
                ));
                if format.content_length > 0 {
                    rows.push(detail(
                        "Size",
                        &ui::format_bytes(format.content_length as u64),
                    ));
                }
            } else {
                rows.push(detail("Status", "Not resolved yet"));
            }
        }
    }
    rows.push(heading("Stats"));
    rows.push(detail(
        "Views",
        &match details {
            None => "Loading…".to_owned(),
            Some(details) => details.views.map_or("Unavailable".to_owned(), |views| {
                views.formatted_views(" views")
            }),
        },
    ));
    rows
}

fn heading<'a>(label: &'a str) -> Element<'a, Message> {
    container(
        text(label)
            .size(12)
            .font(theme::weight(Weight::Bold))
            .color(theme::SECONDARY),
    )
    .center_y(ITEM)
    .padding([0, 10])
    .into()
}

/// A label and its value, the value cut to fit the row.
fn detail<'a>(label: &'a str, value: &str) -> Element<'a, Message> {
    let value = if value.is_empty() { "—" } else { value };
    container(
        row![
            text(label).size(14).color(theme::SECONDARY),
            Space::new().width(Length::Fill),
            text(ui::fit(value, DETAILS_WIDTH - 140.0, 14.0))
                .size(14)
                .wrapping(text::Wrapping::None),
        ]
        .align_y(Alignment::Center),
    )
    .center_y(ITEM)
    .padding([0, 10])
    .into()
}

/// Swift `SleepTimerSheet`: End of Song, the countdowns, and Off while one runs.
fn sleep_items<'a>(timer: &SleepTimer) -> Vec<Element<'a, Message>> {
    let mut items = vec![checked(
        "End of Song",
        Action::SleepAtSongEnd,
        timer.stops_at_song_end,
    )];
    items.extend(sleep_timer::PRESETS.iter().map(|&minutes| {
        let label = match minutes {
            60 => "1 hour".to_owned(),
            90 => "1 hour 30 min".to_owned(),
            m => format!("{m} minutes"),
        };
        checked(
            &label,
            Action::Sleep(Some(minutes)),
            timer.minutes == Some(minutes),
        )
    }));
    if timer.is_active() {
        items.push(item(icons::REMOVE, "Turn Off Timer", Action::Sleep(None)));
    }
    items
}

/// Swift's New Playlist prompt: a name field, then Create.
fn new_playlist_items<'a>(name: &str) -> Vec<Element<'a, Message>> {
    let field = text_input("Playlist name", name)
        .id(NEW_PLAYLIST_INPUT)
        .on_input(|name| Message::MenuAction(Action::PlaylistName(name)))
        .on_submit(Message::MenuAction(Action::CreatePlaylist))
        .size(14)
        .padding([6, 10])
        .style(ui::field_style);
    vec![
        container(field).center_y(ITEM).padding([0, 4]).into(),
        // Swift names the song; the menu is too narrow for a title.
        note("Adds this song once created."),
        item(icons::PLUS, "Create", Action::CreatePlaylist),
    ]
}

/// The New Playlist name field, focused when it opens.
pub const NEW_PLAYLIST_INPUT: &str = "menu-new-playlist";

/// Swift `RadioOptionsSheet`: Style, Tracks and Include explicit, then Start.
fn radio_items<'a>(options: &RadioOptions) -> Vec<Element<'a, Message>> {
    let mut items: Vec<Element<'a, Message>> = RadioStyle::ALL
        .iter()
        .map(|&style| {
            checked(
                style.display_name(),
                Action::RadioStyle(style),
                options.style == style,
            )
        })
        .collect();
    items.push(note(options.style.description()));
    let step = |icon, limit: usize| {
        let enabled = (5..=50).contains(&limit);
        button(ui::icon(icon, 16.0).style(move |_, _| svg::Style {
            color: Some(if enabled {
                theme::TEXT
            } else {
                theme::SECONDARY
            }),
        }))
        .padding(6)
        .on_press_maybe(enabled.then_some(Message::MenuAction(Action::RadioLimit(limit))))
        .style(|_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then_some(Background::Color(theme::PRESSED)),
            border: Border::default().rounded(8),
            ..button::Style::default()
        })
    };
    let tracks = row![
        text("Tracks")
            .size(14)
            .font(theme::weight(Weight::Medium))
            .width(Length::Fill),
        step(icons::MINUS, options.limit.saturating_sub(5)),
        text(options.limit.to_string())
            .size(14)
            .color(theme::SECONDARY)
            .width(28)
            .align_x(Alignment::Center),
        step(icons::PLUS, options.limit + 5),
    ]
    .align_y(Alignment::Center);
    items.push(
        container(tracks)
            .center_y(ITEM)
            .padding(Padding {
                left: 10.0,
                right: 4.0,
                ..Padding::ZERO
            })
            .into(),
    );
    items.push(checked(
        "Include explicit",
        Action::RadioExplicit(!options.allow_explicit),
        options.allow_explicit,
    ));
    items.push(item(icons::RADIO, "Start Radio", Action::RadioStart));
    items
}

/// A choice with a check mark when it is the current one.
fn checked<'a>(label: &str, action: Action, on: bool) -> Element<'a, Message> {
    let mark: Element<'a, Message> = if on {
        ui::icon(icons::CHECK, 18.0)
            .style(|_, _| svg::Style {
                color: Some(theme::accent()),
            })
            .into()
    } else {
        Space::new().width(18).into()
    };
    let content = row![
        text(label.to_owned())
            .size(14)
            .font(theme::weight(Weight::Medium))
            .width(Length::Fill),
        mark,
    ]
    .align_y(Alignment::Center);
    button(container(content).center_y(ITEM).padding([0, 10]))
        .width(Length::Fill)
        .padding(0)
        .on_press(Message::MenuAction(action))
        .style(|_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then_some(Background::Color(theme::PRESSED)),
            border: Border::default().rounded(8),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
        .into()
}

fn item<'a>(icon: &'static [u8], label: &str, action: Action) -> Element<'a, Message> {
    row_button(icon, label, Message::MenuAction(action))
}

/// A menu row that sends `message`.
pub(crate) fn row_button<'a>(
    icon: &'static [u8],
    label: &str,
    message: Message,
) -> Element<'a, Message> {
    let content = row![
        ui::icon(icon, 18.0).style(|_, _| svg::Style {
            color: Some(theme::SECONDARY)
        }),
        text(ui::fit(label, WIDTH - 60.0, 14.0))
            .size(14)
            .font(theme::weight(Weight::Medium))
            .wrapping(text::Wrapping::None),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    button(container(content).center_y(ITEM).padding([0, 10]))
        .width(Length::Fill)
        .padding(0)
        .on_press(message)
        .style(|_, status| button::Style {
            background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                .then_some(Background::Color(theme::PRESSED)),
            border: Border::default().rounded(8),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
        .into()
}

pub(crate) fn note<'a>(label: &'a str) -> Element<'a, Message> {
    container(text(label).size(14).color(theme::SECONDARY))
        .center_y(ITEM)
        .padding([0, 10])
        .into()
}

/// Right-click on a song opens its menu: `Message::SongMenu`.
pub fn on_right_click<'a>(
    content: impl Into<Element<'a, Message>>,
    song: &SongItem,
    queue_index: Option<usize>,
) -> Element<'a, Message> {
    mouse_area(content)
        .on_right_press(Message::SongMenu(Box::new(song.clone()), queue_index))
        .into()
}

/// Wraps the whole window: on a right-click it sends where the cursor is in
/// the window (`Message::RightClickAt`), before the song under it sends
/// `SongMenu`. Inside a scrollable the cursor is in content coordinates, so
/// only the root knows the window position.
pub fn root<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    Element::new(Root {
        content: content.into(),
    })
}

struct Root<'a> {
    content: Element<'a, Message>,
}

impl Widget<Message, iced::Theme, iced::Renderer> for Root<'_> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) = event
            && let Some(at) = cursor.position()
        {
            shell.publish(Message::RightClickAt(at));
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

/// Outline icons in the app's 2 px stroke; `ui::icon` tints them.
pub mod icons {
    pub const PLUS: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round"/></svg>"##;
    pub const MINUS: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M5 12h14" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round"/></svg>"##;
    pub const CHECK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M5 12.5l4.5 4.5L19 7.5" fill="none" stroke="#fff" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
    pub const MOON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M19.5 14.5A8 8 0 0 1 9.5 4.5a8 8 0 1 0 10 10z" fill="none" stroke="#fff" stroke-width="2" stroke-linejoin="round"/></svg>"##;
    pub const SPEED: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><path d="M4.5 17a8.5 8.5 0 1 1 15 0"/><path d="M12 13l4-4"/></g></svg>"##;
    pub const PLAY_NEXT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6h10M4 11h10M4 16h6"/><path d="M15 14l4 3-4 3z" fill="#fff"/></g></svg>"##;
    pub const ADD_QUEUE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><path d="M4 6h12M4 11h12M4 16h7M17 14v6M14 17h6"/></g></svg>"##;
    pub const HEART: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 19.5s-7-4.3-8.6-8.5C2.2 7.9 4.1 5 7.2 5c2 0 3.4 1 4.8 2.8C13.4 6 14.8 5 16.8 5c3.1 0 5 2.9 3.8 6-1.6 4.2-8.6 8.5-8.6 8.5z" fill="none" stroke="#fff" stroke-width="2" stroke-linejoin="round"/></svg>"##;
    pub const HEART_FILL: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 19.5s-7-4.3-8.6-8.5C2.2 7.9 4.1 5 7.2 5c2 0 3.4 1 4.8 2.8C13.4 6 14.8 5 16.8 5c3.1 0 5 2.9 3.8 6-1.6 4.2-8.6 8.5-8.6 8.5z" fill="#fff" stroke="#fff" stroke-width="2" stroke-linejoin="round"/></svg>"##;
    pub const PLAYLIST: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><path d="M4 6h11M4 11h11M4 16h6"/><circle cx="16.5" cy="17" r="2.5"/><path d="M19 17V9l2-1"/></g></svg>"##;
    pub const RADIO: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="2"/><path d="M8.5 8.5a5 5 0 0 0 0 7M15.5 8.5a5 5 0 0 1 0 7M5.6 5.6a9 9 0 0 0 0 12.8M18.4 5.6a9 9 0 0 1 0 12.8"/></g></svg>"##;
    pub const ALBUM: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2"><circle cx="12" cy="12" r="8.5"/><circle cx="12" cy="12" r="2.5"/></g></svg>"##;
    pub const ARTIST: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="8" r="3.5"/><path d="M5 20c.8-3.6 3.6-5.5 7-5.5s6.2 1.9 7 5.5"/></g></svg>"##;
    pub const DOWNLOAD: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 4v11M7 10.5l5 5 5-5M5 20h14"/></g></svg>"##;
    pub const LINK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/></g></svg>"##;
    /// `square.and.arrow.up`.
    pub const SHARE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v12M8 7l4-4 4 4M8 11H6v9h12v-9h-2"/></g></svg>"##;
    pub const SIMILAR: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 3.5l1.9 5.1 5.1 1.9-5.1 1.9L12 17.5l-1.9-5.1L5 10.5l5.1-1.9zM19 16.5l.8 2 2 .8-2 .8-.8 2-.8-2-2-.8 2-.8z" fill="none" stroke="#fff" stroke-width="1.8" stroke-linejoin="round"/></svg>"##;
    pub const INFO: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="8.5"/><path d="M12 11v5.5M12 7.8v.2"/></g></svg>"##;
    pub const BACK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M14.5 5.5L8 12l6.5 6.5" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
    pub const REMOVE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="8.5"/><path d="M8 12h8"/></g></svg>"##;
    pub const MORE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="#fff"><circle cx="5.5" cy="12" r="1.8"/><circle cx="12" cy="12" r="1.8"/><circle cx="18.5" cy="12" r="1.8"/></g></svg>"##;
    pub const SHUFFLE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 7h3.5c2 0 3.2 1 4.3 2.7l2.4 4.6c1.1 1.7 2.3 2.7 4.3 2.7H21M3 17h3.5c1.4 0 2.4-.5 3.2-1.4M13.3 8.4C14.1 7.5 15.1 7 16.5 7H21M18 4l3 3-3 3M18 14l3 3-3 3"/></g></svg>"##;
    pub const REPEAT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 11V9.5A2.5 2.5 0 0 1 6.5 7H19M16 4l3 3-3 3M20 13v1.5a2.5 2.5 0 0 1-2.5 2.5H5M8 20l-3-3 3-3"/></g></svg>"##;
    pub const VOLUME: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4 9.5v5a1 1 0 0 0 1 1h3l4.3 3.6a.6.6 0 0 0 1-.5V5.4a.6.6 0 0 0-1-.5L8 8.5H5a1 1 0 0 0-1 1z" fill="#fff"/><path d="M16.5 8.5a5 5 0 0 1 0 7M19 6a8.5 8.5 0 0 1 0 12" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"/></svg>"##;
    pub const MUTED: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M4 9.5v5a1 1 0 0 0 1 1h3l4.3 3.6a.6.6 0 0 0 1-.5V5.4a.6.6 0 0 0-1-.5L8 8.5H5a1 1 0 0 0-1 1z" fill="#fff"/><path d="M16.5 9.5l5 5M21.5 9.5l-5 5" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"/></svg>"##;
}
