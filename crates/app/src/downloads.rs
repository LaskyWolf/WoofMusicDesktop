//! Downloads for offline play: the player bar button, the Downloads page and
//! their messages. `trop::services::download_manager` does the saving.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, RwLock};

use iced::font::Weight;
use iced::widget::{
    Space, button, column, container, progress_bar, row, stack, svg, text, tooltip,
};
use iced::{Alignment, Background, Border, Color, Element, Length, Task};
use trop::database::entities::downloaded_track_entity::DownloadedTrackEntity;
use trop::networking::models::yt_item::{SongItem, YTArtist};
use trop::services::download_manager::{ActiveDownload, BatchProgress, DownloadState};
use trop::services::downloaded_collections::DownloadedCollections;

use crate::services::Services;
use crate::ui::{self, Covers};
use crate::{App, Message, Route, theme};

/// What the app knows about downloads between two reads of the manager.
#[derive(Clone, Default)]
pub struct Downloads {
    /// Running, queued and failed downloads, read again on every `Tick`.
    pub active: Vec<ActiveDownload>,
    /// Ids of the saved songs.
    pub saved: HashSet<String>,
    /// The run of several songs the Library card follows.
    pub batch: Option<BatchProgress>,
    /// Which albums and playlists have their songs saved.
    pub collections: DownloadedCollections,
    /// What a pending "Remove downloads?" is for: a page's id, or `ALL`.
    pub confirm: Option<String>,
}

/// `Downloads::confirm` of the Downloads page's Remove All Downloads.
const ALL: &str = "all";

/// What the page widgets read. The pages draw from `&self` fields they are
/// passed, but the download buttons and badges appear on many pages (album,
/// playlist, cards, Library), so `update_download` copies the state here and
/// they call `collection_actions`, `library_card` and `is_complete` without a
/// parameter on each page.
// ponytail: a global instead of a parameter per page; thread it through if a
// second window ever needs its own downloads.
static VIEW: LazyLock<RwLock<Downloads>> = LazyLock::new(RwLock::default);

fn view<T>(read: impl FnOnce(&Downloads) -> T) -> T {
    read(
        &VIEW
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

/// A fully saved album or playlist: its card shows the badge.
pub fn is_complete(id: &str) -> bool {
    view(|d| d.collections.is_complete(id))
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
    /// A page's songs as one run (Swift `download(songs:)`).
    StartAll(Arc<Vec<SongItem>>),
    /// Closes the Library card of a finished run.
    CloseBatch,
    /// The Library card: Downloads opens, and a finished run's card closes.
    OpenDownloads,
    Collections(DownloadedCollections),
    /// Opens or closes a "Remove downloads?" question: a page's id, or `ALL`.
    Ask(Option<String>),
    /// Removes these saved songs: an album's or playlist's.
    RemoveSongs(Vec<String>),
    /// Removes the saved songs of the playlist with this id (a card menu knows only the id).
    RemovePlaylist(String),
    RemoveAll,
    Removed(Result<(), String>),
    /// The Downloads page's Add to Queue.
    QueueAll(Arc<Vec<SongItem>>),
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
        let task = self.step_download(message);
        self.downloads.batch = self.services.downloads.batch_progress();
        *VIEW
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = self.downloads.clone();
        task
    }

    /// Which albums and playlists have their songs saved, read again.
    pub(crate) fn refresh_collections(&self) -> Task<Message> {
        Task::perform(self.services.clone().downloaded_collections(), |c| {
            Message::Download(Msg::Collections(c))
        })
    }

    /// Swift `AlbumDetailView`: remembers the album's songs, so its card can
    /// tell when all of them are saved.
    /// A podcast page loaded: its episodes are remembered, and the new ones
    /// download when Auto-Download New Episodes is on (Swift `PodcastDetailView.load`).
    pub(crate) fn podcast_seen(&self, detail: &crate::Detail) -> Task<Message> {
        let crate::Detail::Podcast { info, .. } = detail else {
            return Task::none();
        };
        let auto = self.settings.prefs.auto_download_new_episodes;
        Task::perform(
            self.services
                .clone()
                .new_episodes(info.clone(), info.browse_id.clone()),
            move |songs| {
                if auto && !songs.is_empty() {
                    Message::Download(Msg::StartAll(Arc::new(songs)))
                } else {
                    Message::Noop
                }
            },
        )
    }

    pub(crate) fn album_seen(&self, detail: &crate::Detail) -> Task<Message> {
        let crate::Detail::Album { info, songs } = detail else {
            return Task::none();
        };
        let ids: Vec<String> = songs
            .iter()
            .filter(|song| song.is_available())
            .map(|song| song.video_id.clone())
            .collect();
        if ids.is_empty() {
            return Task::none();
        }
        let browse_id = info.browse_id.clone();
        let services = self.services.clone();
        Task::perform(
            async move {
                services.clone().save_album_songs(browse_id, ids).await;
                services.downloaded_collections().await
            },
            |c| Message::Download(Msg::Collections(c)),
        )
    }

    fn step_download(&mut self, message: Msg) -> Task<Message> {
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
                    return Task::batch([self.reload_downloads(), self.refresh_collections()]);
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
                Task::batch([self.reload_downloads(), self.refresh_collections()])
            }
            Msg::Saved(ids) => {
                self.downloads.saved = ids.into_iter().collect();
                self.refresh_collections()
            }
            Msg::StartAll(songs) => {
                let songs: Vec<_> = songs.iter().filter(|song| song.is_available()).collect();
                let ids: Vec<String> = songs.iter().map(|song| song.video_id.clone()).collect();
                downloads.join_batch(&ids);
                let starts: Vec<_> = songs
                    .into_iter()
                    .map(|song| self.step_download(Msg::Start(Box::new(song.clone()))))
                    .collect();
                Task::batch(starts)
            }
            Msg::CloseBatch => {
                downloads.clear_finished_batch();
                Task::none()
            }
            Msg::OpenDownloads => {
                if self
                    .downloads
                    .batch
                    .as_ref()
                    .is_some_and(BatchProgress::is_done)
                {
                    downloads.clear_finished_batch();
                }
                Task::done(Message::Navigate(Route::Downloads))
            }
            Msg::Collections(collections) => {
                self.downloads.collections = collections;
                Task::none()
            }
            Msg::Ask(key) => {
                self.downloads.confirm = key;
                Task::none()
            }
            Msg::RemoveSongs(ids) => {
                self.downloads.confirm = None;
                Task::perform(self.services.clone().delete_downloads(Some(ids)), |r| {
                    Message::Download(Msg::Removed(r))
                })
            }
            Msg::RemovePlaylist(id) => {
                self.downloads.confirm = None;
                Task::perform(self.services.clone().playlist_song_ids(id), |ids| {
                    Message::Download(Msg::RemoveSongs(ids))
                })
            }
            Msg::RemoveAll => {
                self.downloads.confirm = None;
                Task::perform(self.services.clone().delete_downloads(None), |r| {
                    Message::Download(Msg::Removed(r))
                })
            }
            Msg::Removed(result) => {
                if let Err(error) = result {
                    eprintln!("Could not remove the downloads: {error}");
                }
                self.downloads.active = downloads.downloads();
                Task::batch([load(&self.services), self.reload_downloads()])
            }
            Msg::QueueAll(songs) => {
                self.player.queue.append(&songs);
                self.notify("Added to the queue")
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
    // Swift `DownloadsMoreSheet`: Add to Queue and Remove All Downloads.
    if !songs.is_empty() {
        page.push(more_actions(
            songs,
            downloads.confirm.as_deref() == Some(ALL),
        ));
    }
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

/// Download, Downloaded or Remove downloads for the songs of an album or
/// playlist page: `id` is the page's id, `songs` its songs. Pages put it next
/// to their other header buttons; nothing shows for a page without songs.
pub fn collection_actions(id: &str, songs: &Arc<Vec<SongItem>>) -> Element<'static, Message> {
    let available: Vec<&SongItem> = songs.iter().filter(|song| song.is_available()).collect();
    if available.is_empty() {
        return Space::new().into();
    }
    view(|d| {
        let saved: Vec<String> = available
            .iter()
            .filter(|song| d.saved.contains(&song.video_id))
            .map(|song| song.video_id.clone())
            .collect();
        let downloading = available
            .iter()
            .any(|song| matches!(d.state(&song.video_id), Some(DownloadState::Downloading(_))));
        let mut pills = row![].spacing(8).align_y(Alignment::Center);
        pills = pills.push(if saved.len() == available.len() {
            ui::accent_button("Downloaded", Some(icons::SAVED), None)
        } else if downloading {
            ui::accent_button("Downloading…", Some(icons::DOWNLOAD), None)
        } else {
            ui::pill_button("Download", Message::Download(Msg::StartAll(songs.clone())))
        });
        if saved.is_empty() {
            return pills.into();
        }
        if d.confirm.as_deref() == Some(id) {
            return column![
                text("Remove the offline copies of these songs? The page stays in your library.")
                    .size(14),
                row![
                    crate::playlist_edit::danger_button(
                        "Remove downloads",
                        Message::Download(Msg::RemoveSongs(saved)),
                    ),
                    ui::pill_button("Cancel", Message::Download(Msg::Ask(None))),
                ]
                .spacing(8),
            ]
            .spacing(10)
            .into();
        }
        pills
            .push(ui::pill_button(
                "Remove downloads",
                Message::Download(Msg::Ask(Some(id.to_owned()))),
            ))
            .into()
    })
}

/// Swift `LibraryDownloadCard`: follows a download of several songs. It stays
/// after the last song until it is clicked or closed, so the result isn't
/// missed. Clicking opens Downloads.
pub fn library_card() -> Option<Element<'static, Message>> {
    let batch = view(|d| d.batch.clone())?;
    let done = batch.is_done();
    let failed_done = done && batch.failed > 0;
    let tint = if failed_done {
        theme::WARNING
    } else {
        theme::accent()
    };
    let (title, detail) = if !done {
        (
            format!("Downloading {} songs", batch.total),
            format!("{} of {} done", batch.completed + batch.failed, batch.total),
        )
    } else if failed_done {
        (
            format!("{} of {} songs downloaded", batch.completed, batch.total),
            match &batch.failure_reason {
                Some(reason) => format!("{} failed. {reason}", batch.failed),
                None => format!("{} failed.", batch.failed),
            },
        )
    } else {
        (
            format!("{} songs downloaded", batch.total),
            "Ready to play offline".to_owned(),
        )
    };
    let mut content = row![
        tinted(
            if failed_done {
                icons::WARNING
            } else if done {
                icons::SAVED
            } else {
                icons::DOWNLOAD
            },
            22.0,
            tint
        ),
        column![
            text(title).size(14).font(theme::weight(Weight::Bold)),
            text(detail).size(12).color(theme::SECONDARY),
        ]
        .spacing(1)
        .width(Length::Fill),
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    if !done {
        content = content.push(
            text(percent(batch.fraction))
                .size(14)
                .font(theme::weight(Weight::Bold))
                .color(tint),
        );
    }
    // The song rows' download look: a tinted fill growing with a solid bar under it.
    let growth = (batch.fraction * 1000.0) as u16;
    let fill: Element<'static, Message> = if done {
        Space::new().into()
    } else {
        row![
            column![
                container(Space::new())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(move |_| container::Style {
                        background: Some(Background::Color(Color { a: 0.18, ..tint })),
                        ..container::Style::default()
                    }),
                container(Space::new())
                    .width(Length::Fill)
                    .height(3)
                    .style(move |_| container::Style {
                        background: Some(Background::Color(tint)),
                        ..container::Style::default()
                    }),
            ]
            .width(Length::FillPortion(growth.max(1))),
            Space::new().width(Length::FillPortion(1000 - growth.min(999))),
        ]
        .into()
    };
    let card = container(stack![
        fill,
        container(content).padding([12, 14]).width(Length::Fill)
    ])
    .clip(true)
    .style(|_| container::Style {
        background: Some(Background::Color(theme::HOVER)),
        border: Border::default().rounded(14),
        ..container::Style::default()
    });
    let open = button(card)
        .padding(0)
        .width(Length::Fill)
        .on_press(Message::Download(Msg::OpenDownloads))
        .style(|_, _| button::Style::default());
    let mut line = row![open].align_y(Alignment::Center);
    if done {
        line = line.push(hint(
            icon_button(icons::CLOSE, Message::Download(Msg::CloseBatch)),
            "Close".to_owned(),
        ));
    }
    Some(container(line).max_width(744).into())
}

fn more_actions(songs: &Arc<Vec<SongItem>>, confirming: bool) -> Element<'static, Message> {
    if confirming {
        return column![
            text("Remove All Downloads? This permanently deletes all offline tracks from your device.")
                .size(14),
            row![
                crate::playlist_edit::danger_button("Remove All", Message::Download(Msg::RemoveAll)),
                ui::pill_button("Cancel", Message::Download(Msg::Ask(None))),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .into();
    }
    row![
        ui::pill_button(
            "Add to Queue",
            Message::Download(Msg::QueueAll(songs.clone()))
        ),
        ui::pill_button(
            "Remove All Downloads",
            Message::Download(Msg::Ask(Some(ALL.to_owned()))),
        ),
    ]
    .spacing(8)
    .into()
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
    pub const CLOSE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 6l12 12M18 6L6 18" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round"/></svg>"##;
    /// A circle with an exclamation mark.
    pub const WARNING: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill-rule="evenodd" d="M12 2.5a9.5 9.5 0 1 1 0 19 9.5 9.5 0 0 1 0-19zm0 4.7a1 1 0 0 0-1 1v5a1 1 0 1 0 2 0v-5a1 1 0 0 0-1-1zm0 9.1a1.2 1.2 0 1 0 0 2.4 1.2 1.2 0 0 0 0-2.4z" fill="#fff"/></svg>"##;
    pub const TRASH: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h16M10 3.5h4M6.5 7l.8 12a1.5 1.5 0 0 0 1.5 1.4h6.4a1.5 1.5 0 0 0 1.5-1.4l.8-12M10 11v6M14 11v6"/></g></svg>"##;
}
