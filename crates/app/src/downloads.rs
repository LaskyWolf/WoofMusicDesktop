//! Downloads for offline play: the player bar button, the Downloads page and
//! their messages. `trop::services::download_manager` does the saving.

use std::collections::HashSet;
use std::sync::Arc;

use iced::font::Weight;
use iced::widget::{button, column, container, progress_bar, row, svg, text, tooltip};
use iced::{Alignment, Background, Border, Color, Element, Length, Task};
use trop::database::entities::downloaded_track_entity::DownloadedTrackEntity;
use trop::networking::models::yt_item::{SongItem, YTArtist};
use trop::services::download_manager::{ActiveDownload, DownloadState};

use crate::services::Services;
use crate::ui::{self, Covers};
use crate::{App, Message, Route, theme};

/// What the app knows about downloads between two reads of the manager.
#[derive(Default)]
pub struct Downloads {
    /// Running, queued and failed downloads, read again on every `Tick`.
    pub active: Vec<ActiveDownload>,
    /// Ids of the saved songs.
    pub saved: HashSet<String>,
}

impl Downloads {
    /// A download is moving: the page and the player bar need `Tick`s.
    pub fn running(&self) -> bool {
        self.active
            .iter()
            .any(|download| matches!(download.state, DownloadState::Downloading(_)))
    }

    fn state(&self, video_id: &str) -> Option<&DownloadState> {
        self.active
            .iter()
            .find(|download| download.song.video_id == video_id)
            .map(|download| &download.state)
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    Start(Box<SongItem>),
    Finished(String, Result<bool, String>),
    /// Stops a download, or clears a failed one.
    Cancel(String),
    Delete(String),
    Deleted(String, Result<(), String>),
    /// The saved ids, read at start.
    Saved(Vec<String>),
    Tick,
}

/// Reads which songs are saved, for the player bar.
pub fn load(services: &Arc<Services>) -> Task<Message> {
    Task::perform(services.clone().downloaded_ids(), |ids| {
        Message::Download(Msg::Saved(ids))
    })
}

/// A saved song as a queue entry. The row keeps the artists as one name.
pub fn song(track: &DownloadedTrackEntity) -> SongItem {
    SongItem {
        video_id: track.id.clone(),
        title: track.title.clone(),
        artists: (!track.artist.is_empty())
            .then(|| YTArtist {
                name: track.artist.clone(),
                id: None,
            })
            .into_iter()
            .collect(),
        duration: track.duration,
        thumbnail_url: track.thumbnail_url.clone(),
        ..SongItem::default()
    }
}

impl App {
    pub(crate) fn update_download(&mut self, message: Msg) -> Task<Message> {
        let downloads = &self.services.downloads;
        match message {
            Msg::Start(song) => {
                let song = *song;
                let id = song.video_id.clone();
                // Shown at once; `Tick` reads the real state from here on.
                self.downloads.active.retain(|d| d.song.video_id != id);
                self.downloads.active.push(ActiveDownload {
                    song: song.clone(),
                    state: DownloadState::Downloading(0.0),
                });
                Task::batch([
                    self.fetch_covers(song.thumbnail_url.clone()),
                    Task::perform(self.services.clone().download(song), move |result| {
                        Message::Download(Msg::Finished(id.clone(), result))
                    }),
                ])
            }
            Msg::Finished(id, result) => {
                self.downloads.active = downloads.downloads();
                if result == Ok(true) {
                    self.downloads.saved.insert(id);
                    return self.reload_downloads();
                }
                Task::none()
            }
            Msg::Cancel(id) => {
                downloads.cancel(&id);
                self.downloads.active = downloads.downloads();
                Task::none()
            }
            Msg::Delete(id) => Task::perform(
                self.services.clone().delete_download(id.clone()),
                move |r| Message::Download(Msg::Deleted(id.clone(), r)),
            ),
            Msg::Deleted(id, result) => {
                if let Err(error) = result {
                    eprintln!("Could not delete the download {id}: {error}");
                }
                self.downloads.saved.remove(&id);
                self.reload_downloads()
            }
            Msg::Saved(ids) => {
                self.downloads.saved = ids.into_iter().collect();
                Task::none()
            }
            Msg::Tick => {
                self.downloads.active = downloads.downloads();
                Task::none()
            }
        }
    }

    /// The Downloads page reads the database: reload it in place when it is
    /// on screen, so it keeps its rows until the new ones arrive.
    fn reload_downloads(&mut self) -> Task<Message> {
        const KEY: &str = "downloads";
        if *self.route() != Route::Downloads {
            self.details.remove(KEY);
            return Task::none();
        }
        Task::perform(
            self.services.clone().load_detail(Route::Downloads),
            |result| Message::DetailLoaded(KEY.to_owned(), result.map(Arc::new)),
        )
    }

    /// Dev aid: WOOF_DOWNLOAD=1 downloads each song that starts playing.
    pub(crate) fn dev_download(&self) -> Task<Message> {
        match self.player.song() {
            Some(song)
                if std::env::var("WOOF_DOWNLOAD").is_ok()
                    && !self.downloads.saved.contains(&song.video_id)
                    && self.downloads.state(&song.video_id).is_none() =>
            {
                Task::done(Message::Download(Msg::Start(Box::new(song.clone()))))
            }
            _ => Task::none(),
        }
    }
}

/// The player bar's download button: download, its progress, or saved.
/// Progress and saved open the Downloads page.
pub fn bar_button<'a>(downloads: &Downloads, song: &SongItem) -> Element<'a, Message> {
    let open = Message::Navigate(Route::Downloads);
    let start = Message::Download(Msg::Start(Box::new(song.clone())));
    let (content, message, tip): (Element<'a, Message>, _, String) =
        if downloads.saved.contains(&song.video_id) {
            (
                tinted(icons::SAVED, 20.0, theme::accent()),
                open,
                "Downloaded. Show Downloads".to_owned(),
            )
        } else {
            match downloads.state(&song.video_id) {
                Some(DownloadState::Downloading(progress)) => (
                    text(percent(*progress))
                        .size(11)
                        .font(theme::weight(Weight::Bold))
                        .color(theme::accent())
                        .into(),
                    open,
                    "Downloading. Show Downloads".to_owned(),
                ),
                Some(DownloadState::Failed(reason)) => (
                    tinted(icons::DOWNLOAD, 20.0, theme::CLOSE),
                    start,
                    format!("{reason} Click to try again."),
                ),
                None => (
                    ui::icon(icons::DOWNLOAD, 20.0).into(),
                    start,
                    "Download for offline play".to_owned(),
                ),
            }
        };
    let control = button(container(content).center_x(28).center_y(20))
        .padding([6, 2])
        .on_press(message)
        .style(|_, status| button::Style {
            background: (status == button::Status::Hovered)
                .then_some(Background::Color(theme::PRESSED)),
            border: Border::default().rounded(22),
            ..button::Style::default()
        });
    hint(control, tip)
}

/// The Downloads page: saved songs, with downloads in progress above them.
pub fn page<'a>(
    covers: &'a Covers,
    songs: &Arc<Vec<SongItem>>,
    downloads: &'a Downloads,
    playing: Option<&str>,
) -> Element<'a, Message> {
    let length: i64 = songs.iter().map(|song| song.duration).sum();
    let line = [
        crate::pages::count(songs.len() as i64, "song"),
        ui::total_length(length),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" • ");
    let mut page = vec![ui::header(
        covers,
        songs.first().and_then(|song| song.thumbnail_url.as_deref()),
        false,
        "Downloads",
        vec![line],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
    )];
    if !downloads.active.is_empty() {
        let rows = downloads
            .active
            .iter()
            .map(|download| active_row(covers, download));
        page.push(
            column![ui::section_title("Downloading"), column(rows).spacing(2)]
                .spacing(14)
                .into(),
        );
    }
    if songs.is_empty() {
        if downloads.active.is_empty() {
            page.push(
                text(
                    "No downloads yet. Play a song and press the download button \
                     in the player bar to keep it for offline listening.",
                )
                .size(15)
                .color(theme::SECONDARY)
                .into(),
            );
        }
        return ui::scroll_page(page);
    }
    let rows = (0..songs.len()).map(|index| {
        let song = &songs[index];
        let is_playing = playing == Some(song.video_id.as_str());
        let track = ui::track_row(
            covers,
            songs,
            index,
            true,
            is_playing,
            ui::artists(&song.artists),
            None,
        );
        let delete = icon_button(
            icons::TRASH,
            Message::Download(Msg::Delete(song.video_id.clone())),
        );
        row![
            container(track).width(Length::Fill),
            hint(delete, "Remove download".to_owned())
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .into()
    });
    let list = column(rows).spacing(2);
    page.push(if downloads.active.is_empty() {
        list.into()
    } else {
        column![ui::section_title("Songs"), list].spacing(14).into()
    });
    ui::scroll_page(page)
}

/// A running or failed download: cover, title, progress or why it failed, actions.
fn active_row<'a>(covers: &'a Covers, download: &'a ActiveDownload) -> Element<'a, Message> {
    let song = &download.song;
    let id = song.video_id.clone();
    let (status, actions): (Element<'a, Message>, Element<'a, Message>) = match &download.state {
        DownloadState::Downloading(progress) => (
            progress_bar(0.0..=1.0, *progress as f32)
                .girth(4)
                .style(|_| progress_bar::Style {
                    background: Background::Color(theme::PRESSED),
                    bar: Background::Color(theme::accent()),
                    border: Border::default().rounded(2),
                })
                .into(),
            row![
                text(percent(*progress))
                    .size(13)
                    .color(theme::SECONDARY)
                    .width(40)
                    .align_x(Alignment::End),
                ui::pill_button("Cancel", Message::Download(Msg::Cancel(id))),
            ]
            .spacing(12)
            .align_y(Alignment::Center)
            .into(),
        ),
        DownloadState::Failed(reason) => (
            text(reason.as_str()).size(13).color(theme::CLOSE).into(),
            row![
                ui::pill_button(
                    "Try again",
                    Message::Download(Msg::Start(Box::new(song.clone())))
                ),
                ui::pill_button("Dismiss", Message::Download(Msg::Cancel(id))),
            ]
            .spacing(8)
            .into(),
        ),
    };
    let content = row![
        ui::cover(covers, song.thumbnail_url.as_deref(), 40.0, 6.0),
        column![
            text(song.title.as_str())
                .size(14)
                .font(theme::weight(Weight::Medium))
                .wrapping(text::Wrapping::None),
            text(ui::artists(&song.artists))
                .size(13)
                .color(theme::SECONDARY)
                .wrapping(text::Wrapping::None),
            status,
        ]
        .spacing(4)
        .width(Length::Fill)
        .clip(true),
        actions,
    ]
    .spacing(14)
    .align_y(Alignment::Center);
    container(content).padding([8, 8]).into()
}

fn percent(progress: f64) -> String {
    format!("{}%", (progress * 100.0) as u32)
}

fn tinted<'a>(glyph: &'static [u8], size: f32, color: Color) -> Element<'a, Message> {
    ui::icon(glyph, size)
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

/// A small round icon button with the hover wash.
pub(crate) fn icon_button<'a>(glyph: &'static [u8], message: Message) -> Element<'a, Message> {
    button(ui::icon(glyph, 18.0))
        .padding(8)
        .on_press(message)
        .style(|_, status| button::Style {
            background: match status {
                button::Status::Hovered => Some(Background::Color(theme::HOVER)),
                button::Status::Pressed => Some(Background::Color(theme::PRESSED)),
                _ => None,
            },
            border: Border::default().rounded(18),
            ..button::Style::default()
        })
        .into()
}

/// A label shown above `control` on hover: the icon buttons have no text.
pub(crate) fn hint<'a>(
    control: impl Into<Element<'a, Message>>,
    label: String,
) -> Element<'a, Message> {
    tooltip(
        control,
        container(text(label).size(12))
            .padding([6, 10])
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgb8(0x26, 0x26, 0x29))),
                border: Border::default().rounded(8),
                text_color: Some(theme::TEXT),
                ..container::Style::default()
            }),
        tooltip::Position::Top,
    )
    .gap(6)
    .into()
}

/// Glyphs, white; `ui::icon` tints them.
pub mod icons {
    /// An arrow down onto a line.
    pub const DOWNLOAD: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 4v11M7 10.5l5 5 5-5M5 20h14"/></g></svg>"##;
    /// A filled circle with a check cut out.
    pub const SAVED: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill-rule="evenodd" d="M12 2.5a9.5 9.5 0 1 1 0 19 9.5 9.5 0 0 1 0-19zm4.2 6.3a1 1 0 0 0-1.4.1l-4 4.6-1.6-1.6a1 1 0 1 0-1.4 1.4l2.4 2.4a1 1 0 0 0 1.5-.1l4.6-5.4a1 1 0 0 0-.1-1.4z" fill="#fff"/></svg>"##;
    pub const TRASH: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h16M10 3.5h4M6.5 7l.8 12a1.5 1.5 0 0 0 1.5 1.4h6.4a1.5 1.5 0 0 0 1.5-1.4l.8-12M10 11v6M14 11v6"/></g></svg>"##;
}
