//! Playlist editing (Swift `PlaylistActionsMenu`, `PlaylistEditSheet`,
//! `AddSongToPlaylistView`, `PlaylistPeopleChip`, `NewPlaylistSheet`): the
//! actions under the playlist header, the song rows' edit controls and the
//! Library's New playlist field. `trop::services::playlist_*` does the work.
//!
//! Desktop differences: actions are pills instead of a "⋯" menu, rename and
//! delete confirm inline instead of in sheets and alerts, a picked cover is
//! center-cropped (no crop sheet), and every playlist is synced (no local
//! playlists, so no sync switch).

use std::sync::{Arc, LazyLock};

use iced::font::Weight;
use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Length, Task};
use trop::database::entities::song_entity::SongEntity;
use trop::detail::detail_models::PlaylistDetailInfo;
use trop::networking::models::yt_item::SongItem;
use trop::services::playlist_collaborators::{PlaylistCollaborator, PlaylistCollaborators};
use trop::services::playlist_cover_uploader::PlaylistCoverUploader;
use trop::services::playlist_removal::PlaylistRemoval;
use trop::services::playlist_song_edits::PlaylistSongEdits;

use crate::services::Services;
use crate::ui::{self, Covers};
use crate::{App, Detail, Message, Route, theme};

pub const NEW_PLAYLIST_INPUT: &str = "new-playlist";
const RENAME_INPUT: &str = "rename-playlist";
const FILTER_INPUT: &str = "add-songs-filter";
/// Songs the Add songs panel lists at once.
const ADD_LIMIT: usize = 30;

#[derive(Default)]
pub struct State {
    /// The Library's New playlist field; `None` while closed.
    new_name: Option<String>,
    creating: bool,
    create_error: Option<String>,
    page: Page,
}

/// What the playlist page `id` has open. Opening another playlist starts fresh.
#[derive(Default)]
pub struct Page {
    id: String,
    rename: Option<String>,
    confirm_delete: bool,
    /// The rows show move and remove buttons.
    editing: bool,
    /// The Add songs panel's filter; `None` while closed.
    adding: Option<String>,
    /// Library songs for the Add songs panel, newest first.
    library_songs: Vec<SongItem>,
    /// `Some` while the people list is open; empty until it loads.
    people: Option<Vec<PlaylistCollaborator>>,
    people_loaded: bool,
    /// A request is running; edits wait so the row ids stay current.
    busy: bool,
    error: Option<String>,
}

static CLOSED: LazyLock<Page> = LazyLock::new(Page::default);

impl State {
    /// The rows of playlist `id` show their edit buttons.
    pub fn editing(&self, id: &str) -> bool {
        self.page(id).editing || dev_editable()
    }

    fn page(&self, id: &str) -> &Page {
        if self.page.id == id {
            &self.page
        } else {
            &CLOSED
        }
    }

    #[cfg(test)]
    pub fn open_rename(&mut self, id: &str) {
        self.page_mut(id).rename = Some(String::new());
    }

    #[cfg(test)]
    pub fn rename_open(&self, id: &str) -> bool {
        self.page(id).rename.is_some()
    }

    #[cfg(test)]
    pub fn busy(&self, id: &str) -> bool {
        self.page(id).busy
    }

    /// The open page of playlist `id`, for an answer that arrives later: `None`
    /// when another playlist is open now, so the answer does not reset it.
    fn reply_page(&mut self, id: &str) -> Option<&mut Page> {
        (self.page.id == id).then_some(&mut self.page)
    }

    fn page_mut(&mut self, id: &str) -> &mut Page {
        if self.page.id != id {
            self.page = Page {
                id: id.to_owned(),
                ..Page::default()
            };
        }
        &mut self.page
    }
}

#[derive(Debug, Clone)]
pub enum Msg {
    NewPlaylist,
    NewName(String),
    Create,
    Created(Result<String, String>),
    Rename,
    Name(String),
    SaveName,
    Delete,
    ConfirmDelete,
    Deleted(String, Result<(), String>),
    EditSongs,
    Move(usize, bool),
    Remove(usize),
    AddSongs,
    Filter(String),
    LibrarySongs(Vec<SongItem>),
    Add(String),
    PickCover,
    /// The file dialog closed: the playlist it was opened for, and the image.
    CoverPicked(String, Option<Vec<u8>>),
    People,
    PeopleLoaded(String, Vec<PlaylistCollaborator>),
    /// A change reached YouTube Music, or failed: the page reloads.
    Done(String, Result<(), String>),
    Reloaded(String, Result<Arc<Detail>, String>),
    /// Closes the open field or confirmation.
    Cancel,
}

fn msg(message: Msg) -> Message {
    Message::PlaylistEdit(message)
}

impl App {
    /// The playlist on screen, with its id.
    fn shown_playlist(&self) -> Option<(String, &PlaylistDetailInfo)> {
        let Route::Playlist(id) = self.route() else {
            return None;
        };
        match self.details.get(&self.route().key()?) {
            Some(crate::Load::Loaded(detail)) => match detail.as_ref() {
                Detail::Playlist { info, .. } => Some((id.clone(), info)),
                _ => None,
            },
            _ => None,
        }
    }

    pub(crate) fn update_playlist_edit(&mut self, message: Msg) -> Task<Message> {
        let services = self.services.clone();
        let edit = &mut self.playlist_edit;
        match message {
            Msg::NewPlaylist => {
                edit.new_name = Some(String::new());
                edit.create_error = None;
                return iced::widget::operation::focus(NEW_PLAYLIST_INPUT);
            }
            Msg::NewName(name) => edit.new_name = Some(name),
            Msg::Create => {
                let name = edit
                    .new_name
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .to_owned();
                if name.is_empty() || edit.creating {
                    return Task::none();
                }
                edit.creating = true;
                edit.create_error = None;
                return Task::perform(
                    async move {
                        services
                            .sync
                            .mutations
                            .create_playlist(&name, None)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    |result| msg(Msg::Created(result)),
                );
            }
            Msg::Created(result) => {
                edit.creating = false;
                match result {
                    Ok(_) => {
                        edit.new_name = None;
                        return Task::perform(services.load_library(), Message::LibraryLoaded);
                    }
                    Err(error) => edit.create_error = Some(error),
                }
            }
            Msg::Cancel => {
                edit.new_name = None;
                edit.create_error = None;
                let page = &mut edit.page;
                page.rename = None;
                page.confirm_delete = false;
                page.error = None;
            }
            Msg::CoverPicked(_, None) => {}
            // For the playlist the dialog was opened on, not the one on screen now.
            Msg::CoverPicked(id, Some(bytes)) => {
                let page = edit.page_mut(&id);
                if page.busy {
                    return Task::none();
                }
                page.busy = true;
                page.error = None;
                return Task::perform(
                    async move {
                        let jpeg = tokio::task::spawn_blocking(move || square_jpeg(&bytes))
                            .await
                            .ok()
                            .flatten();
                        let result = match jpeg {
                            Some(jpeg) => {
                                PlaylistCoverUploader::set_cover(&services.sync, jpeg, &id)
                                    .await
                                    .map(drop)
                                    .map_err(|error| error.to_string())
                            }
                            None => Err("This image can't be used. Try a different one.".into()),
                        };
                        (id, result)
                    },
                    |(id, result)| msg(Msg::Done(id, result)),
                );
            }
            Msg::Deleted(id, result) => {
                // A reply for a playlist that is no longer open leaves the open one alone.
                let page = edit.reply_page(&id);
                if let Some(page) = page {
                    page.busy = false;
                }
                match result {
                    Ok(()) => {
                        self.details.remove(&format!("playlist:{id}"));
                        if *self.route() == Route::Playlist(id) {
                            return Task::done(Message::Back);
                        }
                    }
                    Err(error) => {
                        if let Some(page) = edit.reply_page(&id) {
                            page.error = Some(error);
                        }
                    }
                }
            }
            Msg::PeopleLoaded(id, people) => {
                let urls: Vec<_> = people.iter().filter_map(|p| p.avatar_url.clone()).collect();
                if let Some(page) = edit.reply_page(&id) {
                    page.people_loaded = true;
                    if page.people.is_some() {
                        page.people = Some(people);
                    }
                }
                return self.fetch_covers(urls);
            }
            Msg::Done(id, result) => {
                if let (Err(error), Some(page)) = (result, edit.reply_page(&id)) {
                    page.error = Some(error);
                }
                // Reloaded even after a failure: part of a change may have landed.
                let route = Route::Playlist(id.clone());
                return Task::perform(services.load_detail(route), move |result| {
                    msg(Msg::Reloaded(id.clone(), result.map(Arc::new)))
                });
            }
            Msg::Reloaded(id, result) => {
                if let Some(page) = edit.reply_page(&id) {
                    page.busy = false;
                }
                return self.update(Message::DetailLoaded(format!("playlist:{id}"), result));
            }
            Msg::LibrarySongs(songs) => {
                let urls: Vec<_> = songs
                    .iter()
                    .take(ADD_LIMIT)
                    .filter_map(|s| s.thumbnail_url.clone())
                    .collect();
                edit.page.library_songs = songs;
                return self.fetch_covers(urls);
            }
            message => return self.update_shown_playlist(message),
        }
        Task::none()
    }

    /// Messages from the playlist page on screen.
    fn update_shown_playlist(&mut self, message: Msg) -> Task<Message> {
        let Some((id, info)) = self.shown_playlist() else {
            return Task::none();
        };
        let (title, panel) = (info.title.clone(), info.collaborators_panel_params.clone());
        let row = |index: usize| {
            let song = info.songs.get(index)?.video_id.clone();
            Some((song, info.set_video_ids.get(index).cloned().flatten()))
        };
        let row = match message {
            Msg::Move(index, _) | Msg::Remove(index) => row(index),
            _ => None,
        };
        let set_ids = info.set_video_ids.clone();
        let services = self.services.clone();
        let page = self.playlist_edit.page_mut(&id);
        let done = {
            let id = id.clone();
            move |result| msg(Msg::Done(id.clone(), result))
        };
        match message {
            Msg::Rename => {
                page.rename = Some(title);
                page.confirm_delete = false;
                return iced::widget::operation::focus(RENAME_INPUT);
            }
            Msg::Name(name) => page.rename = Some(name),
            Msg::SaveName => {
                let name = page.rename.as_deref().unwrap_or_default().trim().to_owned();
                if name.is_empty() || page.busy {
                    return Task::none();
                }
                page.rename = None;
                if name == title {
                    return Task::none();
                }
                page.busy = true;
                return Task::perform(
                    async move {
                        PlaylistSongEdits::rename(&services.sync, &id, &name)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    done,
                );
            }
            Msg::Delete => {
                page.confirm_delete = true;
                page.rename = None;
            }
            Msg::ConfirmDelete => {
                page.confirm_delete = false;
                page.busy = true;
                page.error = None;
                let owned = info_owned(&self.details, &id);
                return Task::perform(
                    async move {
                        let result = PlaylistRemoval::remove(&services.sync, &id, owned)
                            .await
                            .map_err(|error| error.to_string());
                        (id, result)
                    },
                    |(id, result)| msg(Msg::Deleted(id, result)),
                );
            }
            Msg::EditSongs => page.editing = !page.editing,
            Msg::Move(index, up) => {
                if page.busy {
                    return Task::none();
                }
                page.busy = true;
                page.error = None;
                return Task::perform(
                    async move {
                        PlaylistSongEdits::move_song(&services.sync, &id, &set_ids, index, up)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    done,
                );
            }
            Msg::Remove(_) => {
                let Some((song, set_id)) = row.filter(|_| !page.busy) else {
                    return Task::none();
                };
                let Some(set_id) = set_id else {
                    page.error = Some(
                        "YouTube Music sent no id for this row, so it can't be removed.".into(),
                    );
                    return Task::none();
                };
                page.busy = true;
                page.error = None;
                return Task::perform(
                    async move {
                        PlaylistSongEdits::remove(&services.sync, &song, &set_id, &id)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    done,
                );
            }
            Msg::AddSongs => {
                if page.adding.take().is_some() {
                    return Task::none();
                }
                page.adding = Some(String::new());
                return Task::batch([
                    Task::perform(library_songs(services), |songs| {
                        msg(Msg::LibrarySongs(songs))
                    }),
                    iced::widget::operation::focus(FILTER_INPUT),
                ]);
            }
            Msg::Filter(filter) => page.adding = Some(filter),
            Msg::Add(song) => {
                if page.busy {
                    return Task::none();
                }
                page.busy = true;
                page.error = None;
                return Task::perform(
                    async move {
                        PlaylistSongEdits::add(&services.sync, &song, &id)
                            .await
                            .map_err(|error| error.to_string())
                    },
                    done,
                );
            }
            Msg::PickCover => {
                if page.busy {
                    return Task::none();
                }
                return Task::perform(pick_image(), move |bytes| {
                    msg(Msg::CoverPicked(id.clone(), bytes))
                });
            }
            Msg::People => {
                if page.people.take().is_some() {
                    return Task::none();
                }
                page.people = Some(Vec::new());
                let Some(params) = panel.filter(|_| !page.people_loaded) else {
                    return Task::none();
                };
                return Task::perform(
                    async move {
                        (
                            id,
                            PlaylistCollaborators::fetch(&services.client, &params).await,
                        )
                    },
                    |(id, people)| msg(Msg::PeopleLoaded(id, people)),
                );
            }
            _ => {}
        }
        Task::none()
    }
}

fn info_owned(
    details: &std::collections::HashMap<String, crate::Load<Arc<Detail>>>,
    id: &str,
) -> bool {
    matches!(
        details.get(&format!("playlist:{id}")),
        Some(crate::Load::Loaded(detail)) if matches!(detail.as_ref(), Detail::Playlist { info, .. } if info.is_editable)
    )
}

/// Dev aid: WOOF_EDITABLE=1 shows every playlist as yours with its rows in
/// edit mode, for snapshots without an account. Edits still need one.
fn dev_editable() -> bool {
    std::env::var("WOOF_EDITABLE").is_ok()
}

/// Swift `AddSongToPlaylistView.loadSongs`: the newest library songs with a title.
async fn library_songs(services: Arc<Services>) -> Vec<SongItem> {
    tokio::task::spawn_blocking(move || {
        services
            .db
            .fetch_all_sql::<SongEntity>(
                "SELECT * FROM song WHERE title != '' ORDER BY create_date DESC LIMIT 200",
                [],
            )
            .unwrap_or_default()
            .iter()
            .map(crate::services::song_item)
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// The system file picker, for a cover image.
async fn pick_image() -> Option<Vec<u8>> {
    let file = rfd::AsyncFileDialog::new()
        .set_title("Choose a playlist cover")
        .add_filter("Images", &["jpg", "jpeg", "png", "webp"])
        .pick_file()
        .await?;
    Some(file.read().await)
}

/// Swift `PlaylistCoverUploader.squareJPEG`: center-cropped square JPEG, at
/// most `COVER_SIDE` pixels wide.
fn square_jpeg(bytes: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(bytes).ok()?;
    let side = image.width().min(image.height());
    if side == 0 {
        return None;
    }
    let out = side.min(PlaylistCoverUploader::COVER_SIDE);
    let square = image
        .crop_imm(
            (image.width() - side) / 2,
            (image.height() - side) / 2,
            side,
            side,
        )
        .resize_exact(out, out, image::imageops::FilterType::Lanczos3)
        .to_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode_image(&square)
        .ok()?;
    Some(jpeg)
}

// MARK: - Views

/// The Library's New playlist button, or its name field.
pub fn new_playlist(state: &State) -> Element<'_, Message> {
    let Some(name) = &state.new_name else {
        return pill(
            "New playlist",
            Some(icons::PLUS),
            Some(msg(Msg::NewPlaylist)),
            false,
        );
    };
    let ready = !name.trim().is_empty() && !state.creating;
    let mut field = row![
        text_input("Playlist name", name)
            .id(NEW_PLAYLIST_INPUT)
            .on_input(|name| msg(Msg::NewName(name)))
            .on_submit(msg(Msg::Create))
            .size(14)
            .padding([9, 14])
            .width(280)
            .style(ui::field_style),
        ui::accent_button(
            if state.creating {
                "Creating…"
            } else {
                "Create"
            },
            None,
            ready.then(|| msg(Msg::Create)),
        ),
        pill(
            "Cancel",
            None,
            (!state.creating).then(|| msg(Msg::Cancel)),
            false
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if let Some(error) = &state.create_error {
        field = field.push(error_text(error));
    }
    field.into()
}

/// The pills under the playlist header and whatever they opened.
pub fn actions<'a>(
    covers: &'a Covers,
    state: &'a State,
    info: &'a PlaylistDetailInfo,
    in_library: bool,
) -> Element<'a, Message> {
    let page = state.page(&info.playlist_id);
    let owned = info.is_editable || dev_editable();
    let can_edit_songs = owned || info.is_collaborative;
    let idle = |message: Msg| (!page.busy).then(|| msg(message));
    let mut pills = row![].spacing(8).align_y(Alignment::Center);
    if can_edit_songs {
        pills = pills.push(pill(
            "Add songs",
            Some(icons::PLUS),
            Some(msg(Msg::AddSongs)),
            page.adding.is_some(),
        ));
        if !info.songs.is_empty() {
            pills = pills.push(pill(
                if page.editing { "Done" } else { "Edit songs" },
                Some(icons::EDIT),
                Some(msg(Msg::EditSongs)),
                page.editing,
            ));
        }
    }
    if owned {
        pills = pills
            .push(pill("Rename", None, idle(Msg::Rename), false))
            .push(pill(
                "Change cover",
                Some(icons::IMAGE),
                idle(Msg::PickCover),
                false,
            ));
    }
    if info.collaborators_panel_params.is_some() {
        let count = info.collaborator_avatar_urls.len();
        pills = pills.push(pill(
            &format!("People · {count}"),
            Some(icons::PEOPLE),
            Some(msg(Msg::People)),
            page.people.is_some(),
        ));
    }
    if owned || in_library {
        pills = pills.push(pill(
            if owned {
                "Delete playlist"
            } else {
                "Remove from Library"
            },
            Some(icons::TRASH),
            idle(Msg::Delete),
            false,
        ));
    }
    if page.busy {
        pills = pills.push(text("Saving…").size(13).color(theme::SECONDARY));
    }
    let mut parts = column![pills].spacing(14);

    if let Some(name) = &page.rename {
        parts = parts.push(
            row![
                text_input("Playlist name", name)
                    .id(RENAME_INPUT)
                    .on_input(|name| msg(Msg::Name(name)))
                    .on_submit(msg(Msg::SaveName))
                    .size(14)
                    .padding([9, 14])
                    .width(320)
                    .style(ui::field_style),
                ui::accent_button(
                    "Save",
                    None,
                    (!name.trim().is_empty()).then(|| msg(Msg::SaveName))
                ),
                pill("Cancel", None, Some(msg(Msg::Cancel)), false),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }
    if page.confirm_delete {
        let question = if owned {
            format!(
                "Delete \u{201C}{}\u{201D}? This can't be undone.",
                info.title
            )
        } else {
            format!(
                "Remove \u{201C}{}\u{201D} from your library? The playlist itself stays on YouTube Music.",
                info.title
            )
        };
        parts = parts.push(
            row![
                text(question).size(14),
                danger_button(
                    if owned { "Delete" } else { "Remove" },
                    msg(Msg::ConfirmDelete)
                ),
                pill("Cancel", None, Some(msg(Msg::Cancel)), false),
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        );
    }
    if let Some(error) = &page.error {
        parts = parts.push(
            row![
                error_text(error),
                pill("Dismiss", None, Some(msg(Msg::Cancel)), false)
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        );
    }
    if let Some(people) = &page.people {
        parts = parts.push(people_list(covers, people, page.people_loaded));
    }
    if let Some(filter) = &page.adding {
        parts = parts.push(add_panel(covers, page, info, filter));
    }
    parts.into()
}

fn people_list<'a>(
    covers: &'a Covers,
    people: &'a [PlaylistCollaborator],
    loaded: bool,
) -> Element<'a, Message> {
    if people.is_empty() {
        let note = if loaded {
            "Couldn't load the people on this playlist."
        } else {
            "Loading people…"
        };
        return text(note).size(13).color(theme::SECONDARY).into();
    }
    let rows = people.iter().map(|person| {
        let mut line = row![
            ui::cover(covers, person.avatar_url.as_deref(), 32.0, 16.0),
            text(&person.name)
                .size(14)
                .font(theme::weight(Weight::Medium)),
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        if person.is_owner {
            line = line.push(text("Owner").size(12).color(theme::accent()));
        }
        let open = person
            .channel_id
            .clone()
            .map(|id| Message::Navigate(Route::Artist(id)));
        ui::clickable(container(line).padding([4, 8]), open, 0.0)
    });
    container(column(rows).spacing(2)).max_width(420).into()
}

/// Swift `AddSongToPlaylistView`: library songs not yet on the playlist,
/// filtered by title or artist.
fn add_panel<'a>(
    covers: &'a Covers,
    page: &'a Page,
    info: &'a PlaylistDetailInfo,
    filter: &'a str,
) -> Element<'a, Message> {
    let query = filter.to_lowercase();
    let matches = page
        .library_songs
        .iter()
        .filter(|song| !info.songs.iter().any(|s| s.video_id == song.video_id))
        .filter(|song| {
            query.is_empty()
                || song.title.to_lowercase().contains(&query)
                || ui::artists(&song.artists).to_lowercase().contains(&query)
        })
        .take(ADD_LIMIT);
    let rows: Vec<Element<'a, Message>> = matches
        .map(|song| {
            row![
                ui::cover(covers, song.thumbnail_url.as_deref(), 40.0, 6.0),
                column![
                    text(ui::fit(&song.title, 360.0, 14.0))
                        .size(14)
                        .font(theme::weight(Weight::Medium))
                        .wrapping(text::Wrapping::None),
                    text(ui::fit(&ui::artists(&song.artists), 360.0, 13.0))
                        .size(13)
                        .color(theme::SECONDARY)
                        .wrapping(text::Wrapping::None),
                ]
                .spacing(2)
                .width(Length::Fill),
                pill(
                    "Add",
                    Some(icons::PLUS),
                    (!page.busy).then(|| msg(Msg::Add(song.video_id.clone()))),
                    false
                ),
            ]
            .spacing(14)
            .align_y(Alignment::Center)
            .height(ui::TRACK_ROW)
            .padding([0, 8])
            .into()
        })
        .collect();
    let list: Element<'a, Message> = if rows.is_empty() {
        text(if page.library_songs.is_empty() {
            "Songs you like will appear here."
        } else {
            "No library songs match."
        })
        .size(13)
        .color(theme::SECONDARY)
        .into()
    } else {
        column(rows).spacing(2).into()
    };
    container(
        column![
            text_input("Search your library songs", filter)
                .id(FILTER_INPUT)
                .on_input(|filter| msg(Msg::Filter(filter)))
                .size(14)
                .padding([9, 14])
                .style(ui::field_style),
            list,
        ]
        .spacing(10),
    )
    .padding(14)
    .max_width(640)
    .style(|_| container::Style {
        background: Some(Background::Color(theme::PLACEHOLDER)),
        border: Border::default().rounded(16),
        ..container::Style::default()
    })
    .into()
}

/// The playlist's rows with move up, move down and remove buttons.
pub fn edit_tracks<'a>(
    covers: &'a Covers,
    state: &'a State,
    info: &'a PlaylistDetailInfo,
    songs: &Arc<Vec<SongItem>>,
    playing: Option<&str>,
) -> Element<'a, Message> {
    let page = state.page(&info.playlist_id);
    let last = songs.len().saturating_sub(1);
    let idle = |message: Msg, allowed: bool| (allowed && !page.busy).then(|| msg(message));
    column((0..songs.len()).map(|index| {
        let song = &songs[index];
        let is_playing = playing == Some(song.video_id.as_str());
        row![
            ui::track_row(
                covers,
                songs,
                index,
                true,
                is_playing,
                ui::artists(&song.artists),
                None
            ),
            round_button(
                icons::UP,
                "Move up",
                idle(Msg::Move(index, true), index > 0)
            ),
            round_button(
                icons::DOWN,
                "Move down",
                idle(Msg::Move(index, false), index < last)
            ),
            round_button(icons::REMOVE, "Remove", idle(Msg::Remove(index), true)),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .into()
    }))
    .spacing(2)
    .into()
}

/// A pill like `ui::pill_button`, with an optional icon, a selected look and
/// a dimmed look while it does nothing.
pub(crate) fn pill<'a>(
    label: &str,
    glyph: Option<&'static [u8]>,
    message: Option<Message>,
    selected: bool,
) -> Element<'a, Message> {
    let enabled = message.is_some();
    let ink = if selected {
        theme::BACKGROUND
    } else {
        theme::TEXT
    };
    let mut content = row![].spacing(6).align_y(Alignment::Center);
    if let Some(glyph) = glyph {
        content = content.push(
            ui::icon(glyph, 15.0)
                .opacity(if enabled { 1.0_f32 } else { 0.4 })
                .style(move |_, _| iced::widget::svg::Style { color: Some(ink) }),
        );
    }
    let label = text(label.to_owned())
        .size(14)
        .font(theme::weight(Weight::Medium));
    button(content.push(label))
        .padding([8, 16])
        .on_press_maybe(message)
        .style(move |_, status| button::Style {
            background: Some(Background::Color(match (selected, status) {
                (true, _) => theme::TEXT,
                (false, button::Status::Hovered) => theme::PRESSED,
                _ => theme::HOVER,
            })),
            border: Border::default().rounded(18),
            text_color: match (selected, enabled) {
                (true, _) => theme::BACKGROUND,
                (false, true) => theme::TEXT,
                (false, false) => theme::SECONDARY,
            },
            ..button::Style::default()
        })
        .into()
}

fn danger_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(14).font(theme::weight(Weight::Bold)))
        .padding([8, 18])
        .on_press(message)
        .style(|_, status| button::Style {
            background: Some(Background::Color(if status == button::Status::Hovered {
                theme::CLOSE_PRESSED
            } else {
                theme::CLOSE
            })),
            border: Border::default().rounded(18),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
        .into()
}

fn round_button<'a>(
    glyph: &'static [u8],
    label: &'a str,
    message: Option<Message>,
) -> Element<'a, Message> {
    let opacity = if message.is_some() { 1.0_f32 } else { 0.25 };
    iced::widget::tooltip(
        button(ui::icon(glyph, 16.0).opacity(opacity))
            .padding(8)
            .on_press_maybe(message)
            .style(|_, status| button::Style {
                background: (status == button::Status::Hovered)
                    .then_some(Background::Color(theme::HOVER)),
                border: Border::default().rounded(16),
                ..button::Style::default()
            }),
        container(text(label).size(12))
            .padding([4, 8])
            .style(|_| container::Style {
                background: Some(Background::Color(Color::from_rgb(0.16, 0.16, 0.17))),
                border: Border::default().rounded(8),
                ..container::Style::default()
            }),
        iced::widget::tooltip::Position::Top,
    )
    .into()
}

fn error_text(error: &str) -> Element<'_, Message> {
    text(error).size(13).color(theme::CLOSE).into()
}

/// Glyphs, filled or stroked white; `ui::icon` tints them.
mod icons {
    pub const PLUS: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 5v14M5 12h14" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round"/></svg>"##;
    pub const EDIT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><path d="M4 7h10M4 12h10M4 17h6"/><path d="M18 9l2.5 3L18 15" stroke-linejoin="round"/></g></svg>"##;
    pub const IMAGE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linejoin="round"><rect x="3.5" y="4.5" width="17" height="15" rx="2.5"/><path d="M4 17l5-5 4 4 2.5-2.5L20 18"/></g><circle cx="15.5" cy="9" r="1.6" fill="#fff"/></svg>"##;
    pub const PEOPLE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round"><circle cx="9" cy="8" r="3.2"/><path d="M3.5 19c.6-3.2 2.8-5 5.5-5s4.9 1.8 5.5 5"/><path d="M15.5 5.2a3 3 0 0 1 0 5.6M17.5 14.3c1.6.6 2.7 2.2 3 4.7"/></g></svg>"##;
    pub const TRASH: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h16M9.5 7V4.8h5V7M6.5 7l1 12.2a1 1 0 0 0 1 .8h7a1 1 0 0 0 1-.8l1-12.2M10 11v5M14 11v5"/></g></svg>"##;
    pub const UP: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 14.5l6-6 6 6" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
    pub const DOWN: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M6 9.5l6 6 6-6" fill="none" stroke="#fff" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/></svg>"##;
    pub const REMOVE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5" fill="none" stroke="#fff" stroke-width="2"/><path d="M8 12h8" stroke="#fff" stroke-width="2.2" stroke-linecap="round"/></svg>"##;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_jpeg_crops_the_middle_and_caps_the_side() {
        let encode = |width, height| {
            let mut png = Vec::new();
            image::DynamicImage::new_rgb8(width, height)
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .unwrap();
            png
        };
        let side = |png: Vec<u8>| {
            let jpeg = image::load_from_memory(&square_jpeg(&png).unwrap()).unwrap();
            (jpeg.width(), jpeg.height())
        };
        assert_eq!(side(encode(300, 200)), (200, 200));
        assert_eq!(side(encode(1500, 2000)), (1024, 1024));
        assert!(square_jpeg(b"not an image").is_none());
    }
}
