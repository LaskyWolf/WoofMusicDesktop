//! Pages and cards (Swift `PlaylistActionsMenu`, `CardContextMenu`, `BulkSongActions`,
//! `ArtworkMosaicView`, `PodcastDetailView`): the actions row of a detail page,
//! the right-click menu of album, playlist and artist cards, multi-select with
//! bulk actions, and the podcast page extras.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use iced::font::Weight;
use iced::widget::{Row, Space, button, column, container, mouse_area, row, text};
use iced::{Alignment, Background, Border, Element, Length, Padding, Point, Task};
use swiftytube::sync::library_sync_models::SyncArtist;
use swiftytube::sync::library_sync_store::LibrarySyncStore;
use trop::database::entities::playlist_entity::PlaylistEntity;
use trop::networking::models::yt_item::{EpisodeItem, SongItem, YTItem};
use trop::playback::episode_playback_store::{EpisodePlaybackStore, EpisodePosition};
use trop::services::playlist_song_edits::PlaylistSongEdits;

use crate::playlist_edit::pill;
use crate::song_menu::icons;
use crate::{
    App, Detail, Load, Message, Route, Session, feeds, playlist_edit, song_menu, theme, ui,
};

/// What a page action does with songs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Do {
    Play,
    Shuffle,
    PlayNext,
    Queue,
    Like,
    AddToPlaylist,
    /// Own playlists only.
    Remove,
    /// History only.
    Delete,
}

#[derive(Clone, Debug)]
pub enum Msg {
    /// The page's songs, or only the picked ones.
    Run(Do, bool),
    Share(String),
    /// Save the album or subscribe to the artist or podcast on show.
    ToggleSaved,
    Saved(String, bool, Result<(), String>),
    Select(bool),
    Pick(usize),
    PickAll,
    Describe,
    Card(Box<YTItem>),
    CardRun(Do),
    CardSongs(Do, Result<Arc<Detail>, String>),
    Open(Route),
    Playlists(Vec<PlaylistEntity>),
    AddTo(String, String),
    Close,
}

#[derive(Default)]
pub struct State {
    pub selecting: bool,
    /// Indices into the page's song list.
    pub picked: HashSet<usize>,
    /// Saved or subscribed, by page key, over what the page loaded with.
    saved: HashMap<String, bool>,
    /// The podcast description is open in full.
    described: bool,
    overlay: Option<Overlay>,
}

struct Overlay {
    at: Point,
    kind: Kind,
}

enum Kind {
    Card(Box<YTItem>),
    /// The songs to add, and the playlists to pick from.
    Playlists(Vec<SongItem>, Option<Vec<PlaylistEntity>>),
}

impl State {
    /// A new page starts without a selection (the saved marks stay).
    pub fn reset(&mut self) {
        self.selecting = false;
        self.picked.clear();
        self.described = false;
        self.overlay = None;
    }

    fn toggle(&mut self, index: usize) {
        if !self.picked.remove(&index) {
            self.picked.insert(index);
        }
    }
}

/// What a page's view needs from the app.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub state: &'a State,
    /// `Some` when the page can be saved or subscribed to, and whether it is.
    pub saved: Option<bool>,
    pub account: bool,
}

// ---------------------------------------------------------------- pure helpers

/// The songs in a random order (Swift `shuffled()`).
pub fn shuffled(songs: &[SongItem]) -> Vec<SongItem> {
    let mut songs = songs.to_vec();
    fastrand::shuffle(&mut songs);
    songs
}

/// The songs at `picked`, in page order.
pub fn picked_songs(songs: &[SongItem], picked: &HashSet<usize>) -> Vec<SongItem> {
    songs
        .iter()
        .enumerate()
        .filter(|(index, _)| picked.contains(index))
        .map(|(_, song)| song.clone())
        .collect()
}

/// Swift `EpisodeTag`: "N MIN LEFT" for an episode that is part way through.
pub fn minutes_left(seconds: f64) -> String {
    format!("{} MIN LEFT", ((seconds / 60.0).round() as i64).max(1))
}

/// ON AIR while it plays, PLAYED once finished, minutes left when started.
pub fn episode_tag(saved: Option<EpisodePosition>, on_air: bool) -> Option<String> {
    if on_air {
        return Some("ON AIR".to_owned());
    }
    let saved = saved?;
    if saved.is_finished() {
        Some("PLAYED".to_owned())
    } else {
        Some(minutes_left(saved.remaining()))
    }
}

/// The episode listened to last that is not finished (Swift `latestInProgress`).
pub fn latest_in_progress(
    episodes: &[EpisodeItem],
    store: &EpisodePlaybackStore,
) -> Option<(usize, EpisodePosition)> {
    episodes
        .iter()
        .enumerate()
        .filter_map(|(index, episode)| {
            let saved = store.position(&episode.video_id)?;
            saved.resume_at().map(|_| (index, saved))
        })
        .max_by(|a, b| a.1.updated_at.total_cmp(&b.1.updated_at))
}

/// The description cut to `limit` characters with an ellipsis, `None` when it fits.
pub fn collapsed(description: &str, limit: usize) -> Option<String> {
    (description.chars().count() > limit).then(|| {
        let cut: String = description.chars().take(limit).collect();
        format!("{}…", cut.trim_end())
    })
}

fn route_of(item: &YTItem) -> Option<Route> {
    match item {
        YTItem::Album(album) => Some(Route::Album(album.browse_id.clone())),
        YTItem::Artist(artist) => Some(Route::Artist(artist.browse_id.clone())),
        YTItem::Playlist(playlist) => Some(Route::Playlist(playlist.id.clone())),
        _ => None,
    }
}

/// A link to the page that opens in YouTube Music.
pub fn share_url(route: &Route, playlist_id: Option<&str>) -> Option<String> {
    let base = "https://music.youtube.com";
    match route {
        Route::Album(id) => Some(match playlist_id.filter(|p| !p.is_empty()) {
            Some(list) => format!("{base}/playlist?list={list}"),
            None => format!("{base}/browse/{id}"),
        }),
        Route::Playlist(id) => Some(format!("{base}/playlist?list={id}")),
        Route::Artist(id) | Route::Podcast(id) => Some(format!("{base}/browse/{id}")),
        _ => None,
    }
}

fn songs_of(detail: &Detail) -> Vec<SongItem> {
    match detail {
        Detail::Album { songs, .. }
        | Detail::Artist { songs, .. }
        | Detail::Playlist { songs, .. }
        | Detail::Podcast { songs, .. }
        | Detail::Songs(songs) => songs.to_vec(),
        Detail::Mood(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------- app side

/// The page being saved or subscribed to, copied out so the task owns it.
enum Target {
    Album {
        id: String,
        title: String,
        thumbnail: Option<String>,
        playlist_id: Option<String>,
        count: i64,
        duration: i64,
    },
    Artist {
        id: String,
        name: String,
        thumbnail: Option<String>,
    },
    Podcast {
        id: String,
        name: String,
        thumbnail: Option<String>,
    },
}

impl App {
    pub(crate) fn update_collections(&mut self, message: Msg) -> Task<Message> {
        match message {
            Msg::Run(what, picked) => {
                let all = self.page_songs();
                let songs = if picked {
                    picked_songs(&all, &self.collections.picked)
                } else {
                    all
                };
                let task = match (what, picked) {
                    (Do::Remove, _) => self.remove_picked(),
                    (Do::Delete, _) => self.delete_picked(),
                    _ => self.run_songs(what, songs),
                };
                if picked && what != Do::AddToPlaylist {
                    self.collections.selecting = false;
                    self.collections.picked.clear();
                }
                task
            }
            Msg::Share(url) => {
                self.collections.overlay = None;
                Task::batch([iced::clipboard::write(url), self.notify("Link copied")])
            }
            Msg::ToggleSaved => self.toggle_saved(),
            Msg::Saved(key, on, result) => match result {
                Ok(()) => Task::none(),
                Err(error) => {
                    self.collections.saved.insert(key, !on);
                    self.notify(&error)
                }
            },
            Msg::Select(on) => {
                self.collections.selecting = on;
                self.collections.picked.clear();
                Task::none()
            }
            Msg::Pick(index) => {
                self.collections.toggle(index);
                Task::none()
            }
            Msg::PickAll => {
                let all = self.page_songs().len();
                let picked = &mut self.collections.picked;
                if picked.len() == all {
                    picked.clear();
                } else {
                    *picked = (0..all).collect();
                }
                Task::none()
            }
            Msg::Describe => {
                self.collections.described = !self.collections.described;
                Task::none()
            }
            Msg::Card(item) => {
                self.collections.overlay = Some(Overlay {
                    at: self.right_click,
                    kind: Kind::Card(item),
                });
                Task::none()
            }
            Msg::CardRun(what) => {
                let Some(Overlay {
                    kind: Kind::Card(item),
                    ..
                }) = self.collections.overlay.take()
                else {
                    return Task::none();
                };
                let Some(route) = route_of(&item) else {
                    return Task::none();
                };
                let cached = route.key().and_then(|key| self.details.get(&key));
                if let Some(Load::Loaded(detail)) = cached {
                    let songs = songs_of(detail);
                    return self.run_songs(what, songs);
                }
                Task::perform(self.services.clone().load_detail(route), move |result| {
                    Message::Pages(Msg::CardSongs(what, result.map(Arc::new)))
                })
            }
            Msg::CardSongs(what, result) => match result {
                Ok(detail) => self.run_songs(what, songs_of(&detail)),
                Err(error) => self.notify(&error),
            },
            Msg::Open(route) => {
                self.collections.overlay = None;
                Task::done(Message::Navigate(route))
            }
            Msg::Playlists(list) => {
                if let Some(Overlay {
                    kind: Kind::Playlists(_, slot),
                    ..
                }) = &mut self.collections.overlay
                {
                    *slot = Some(list);
                }
                Task::none()
            }
            Msg::AddTo(playlist_id, name) => {
                let Some(Overlay {
                    kind: Kind::Playlists(songs, _),
                    ..
                }) = self.collections.overlay.take()
                else {
                    return Task::none();
                };
                self.collections.selecting = false;
                self.collections.picked.clear();
                let services = self.services.clone();
                Task::perform(
                    async move {
                        for song in songs {
                            services
                                .clone()
                                .add_to_playlist(playlist_id.clone(), song.video_id)
                                .await?;
                        }
                        Ok(())
                    },
                    move |result| Message::AddedToPlaylist(name.clone(), result),
                )
            }
            Msg::Close => {
                self.collections.overlay = None;
                Task::none()
            }
        }
    }

    /// The songs of the page on show, in the order its rows have.
    fn page_songs(&self) -> Vec<SongItem> {
        let route = self.route();
        if *route == Route::History {
            return self
                .feeds
                .history
                .filtered(&self.feeds.query)
                .iter()
                .flat_map(|day| day.rows.iter().map(|row| row.song.clone()))
                .collect();
        }
        match route.key().and_then(|key| self.details.get(&key)) {
            Some(Load::Loaded(detail)) => songs_of(detail),
            _ => Vec::new(),
        }
    }

    fn run_songs(&mut self, what: Do, songs: Vec<SongItem>) -> Task<Message> {
        if songs.is_empty() {
            return Task::none();
        }
        match what {
            Do::Play => self.play(Arc::new(songs), 0),
            // ponytail: shuffles the list once; the queue's own shuffle switch stays off.
            Do::Shuffle => self.play(Arc::new(shuffled(&songs)), 0),
            Do::PlayNext => {
                for song in songs.into_iter().rev() {
                    self.player.queue.enqueue_next(song);
                }
                self.notify("Playing next")
            }
            Do::Queue => {
                self.player.queue.append(&songs);
                self.notify("Added to the queue")
            }
            Do::Like => Task::batch(songs.into_iter().map(|song| self.like(song, true))),
            Do::AddToPlaylist => {
                let at = Point::new((self.window.width / 2.0 - 116.0).max(0.0), 96.0);
                self.collections.overlay = Some(Overlay {
                    at,
                    kind: Kind::Playlists(songs, None),
                });
                Task::perform(self.services.clone().playlists(), |list| {
                    Message::Pages(Msg::Playlists(list))
                })
            }
            Do::Remove | Do::Delete => Task::none(),
        }
    }

    /// Removes the picked songs from the playlist on show.
    fn remove_picked(&mut self) -> Task<Message> {
        let Route::Playlist(id) = self.route().clone() else {
            return Task::none();
        };
        let Some(Load::Loaded(detail)) = self.details.get(&format!("playlist:{id}")) else {
            return Task::none();
        };
        let Detail::Playlist { info, songs } = detail.as_ref() else {
            return Task::none();
        };
        let mut picked: Vec<_> = self.collections.picked.iter().copied().collect();
        picked.sort_unstable();
        let items: Vec<(String, String)> = picked
            .into_iter()
            .filter_map(|index| {
                let set_id = info.set_video_ids.get(index)?.clone()?;
                Some((songs.get(index)?.video_id.clone(), set_id))
            })
            .collect();
        self.collections.selecting = false;
        self.collections.picked.clear();
        let services = self.services.clone();
        Task::perform(
            async move {
                for (song_id, set_id) in items {
                    if let Err(error) =
                        PlaylistSongEdits::remove(&services.sync, &song_id, &set_id, &id).await
                    {
                        return (id, Err(error.to_string()));
                    }
                }
                (id, Ok(()))
            },
            |(id, result)| Message::PlaylistEdit(playlist_edit::Msg::Done(id, result)),
        )
    }

    /// Deletes the picked plays from History.
    fn delete_picked(&mut self) -> Task<Message> {
        let ids: Vec<i64> = self
            .feeds
            .history
            .filtered(&self.feeds.query)
            .iter()
            .flat_map(|day| day.rows.iter())
            .enumerate()
            .filter(|(index, _)| self.collections.picked.contains(index))
            .filter_map(|(_, row)| row.event.as_ref()?.id)
            .collect();
        self.collections.selecting = false;
        self.collections.picked.clear();
        if ids.is_empty() {
            return Task::none();
        }
        Task::done(Message::Feeds(feeds::Msg::Delete(ids)))
    }

    /// Whether the page on show is saved or subscribed to; `None` when it cannot be.
    pub(crate) fn saved(&self, route: &Route) -> Option<bool> {
        if !matches!(self.session, Session::SignedIn(_)) {
            return None;
        }
        let key = route.key()?;
        let Some(Load::Loaded(detail)) = self.details.get(&key) else {
            return None;
        };
        let library = |items: &dyn Fn(&crate::services::Library) -> &Vec<(YTItem, String)>,
                       id: &str| {
            matches!(&self.library, Some(Load::Loaded(library))
                if items(library).iter().any(|(item, _)| item.id() == id))
        };
        let initial = match detail.as_ref() {
            Detail::Album { info, .. } => library(&|l| &l.albums, &info.browse_id),
            Detail::Artist { info, .. } => {
                info.is_subscribed || library(&|l| &l.artists, &info.browse_id)
            }
            Detail::Podcast { info, .. } => {
                info.is_subscribed || library(&|l| &l.podcasts, &info.browse_id)
            }
            _ => return None,
        };
        Some(self.collections.saved.get(&key).copied().unwrap_or(initial))
    }

    fn toggle_saved(&mut self) -> Task<Message> {
        let route = self.route().clone();
        let (Some(key), Some(now)) = (route.key(), self.saved(&route)) else {
            return Task::none();
        };
        let Some(Load::Loaded(detail)) = self.details.get(&key) else {
            return Task::none();
        };
        let target = match detail.as_ref() {
            Detail::Album { info, .. } => Target::Album {
                id: info.browse_id.clone(),
                title: info.title.clone(),
                thumbnail: info.thumbnail_url.clone(),
                playlist_id: info.playlist_id.clone(),
                count: info.song_count.max(info.songs.len() as i64),
                duration: info.duration,
            },
            Detail::Artist { info, .. } => Target::Artist {
                id: info.browse_id.clone(),
                name: info.name.clone(),
                thumbnail: info.thumbnail_url.clone(),
            },
            Detail::Podcast { info, .. } => Target::Podcast {
                id: info.browse_id.clone(),
                name: info.title.clone(),
                thumbnail: info.thumbnail_url.clone(),
            },
            _ => return Task::none(),
        };
        let on = !now;
        self.collections.saved.insert(key.clone(), on);
        let services = self.services.clone();
        Task::perform(
            async move {
                let sync = &services.sync;
                let result = match target {
                    Target::Album {
                        id,
                        title,
                        thumbnail,
                        playlist_id,
                        count,
                        duration,
                    } if on => {
                        sync.mutations
                            .save_album(
                                &id,
                                &title,
                                thumbnail.as_deref(),
                                playlist_id.as_deref(),
                                count,
                                duration,
                            )
                            .await
                    }
                    Target::Album {
                        id, playlist_id, ..
                    } => {
                        sync.mutations
                            .unsave_album(&id, playlist_id.as_deref())
                            .await
                    }
                    Target::Artist {
                        id,
                        name,
                        thumbnail,
                    } => {
                        // `subscribe_artist` only updates a stored artist: store it first.
                        if on && !matches!(sync.store.fetch_artist(&id), Ok(Some(_))) {
                            let _ = sync.store.save_artist(&SyncArtist {
                                id: id.clone(),
                                name,
                                thumbnail_url: thumbnail,
                                bookmarked_at: None,
                                is_podcast_channel: false,
                                channel_id: Some(id.clone()),
                            });
                        }
                        if on {
                            sync.mutations.subscribe_artist(&id, &id).await
                        } else {
                            sync.mutations.unsubscribe_artist(&id, &id).await
                        }
                    }
                    Target::Podcast {
                        id,
                        name,
                        thumbnail,
                    } => {
                        if on {
                            sync.mutations
                                .subscribe_podcast(&id, &name, thumbnail.as_deref(), None, None)
                                .await
                        } else {
                            sync.mutations.unsubscribe_podcast(&id, None, None).await
                        }
                    }
                };
                result.map_err(|error| error.to_string())
            },
            move |result| Message::Pages(Msg::Saved(key.clone(), on, result)),
        )
    }
}

// ---------------------------------------------------------------- views

/// The page's action pills under the Play button.
pub fn actions<'a>(
    cx: Ctx<'a>,
    songs: &Arc<Vec<SongItem>>,
    link: Option<String>,
    labels: (&'static str, &'static str),
    shuffle: bool,
    select: bool,
) -> Element<'a, Message> {
    let any = !songs.is_empty();
    let run = |what| any.then_some(Message::Pages(Msg::Run(what, false)));
    let mut pills = Row::new().spacing(8);
    if shuffle {
        pills = pills.push(pill(
            "Shuffle",
            Some(icons::SHUFFLE),
            run(Do::Shuffle),
            false,
        ));
    }
    pills = pills.push(pill(
        "Add to Queue",
        Some(icons::ADD_QUEUE),
        run(Do::Queue),
        false,
    ));
    if let Some(on) = cx.saved {
        let (off_label, on_label) = labels;
        pills = pills.push(pill(
            if on { on_label } else { off_label },
            Some(if on { icons::CHECK } else { icons::PLUS }),
            Some(Message::Pages(Msg::ToggleSaved)),
            on,
        ));
    }
    if let Some(link) = link {
        pills = pills.push(pill(
            "Copy Link",
            Some(icons::LINK),
            Some(Message::Pages(Msg::Share(link))),
            false,
        ));
    }
    if select && any {
        pills = pills.push(pill(
            "Select",
            Some(icons::CHECK),
            Some(Message::Pages(Msg::Select(!cx.state.selecting))),
            cx.state.selecting,
        ));
    }
    pills.wrap().vertical_spacing(8).into()
}

/// What a bulk bar can do beyond play, shuffle and queue.
#[derive(Clone, Copy, Default)]
pub struct Caps {
    pub remove: bool,
    pub delete: bool,
}

/// The bar above a list in select mode: count, select all, bulk actions.
pub fn bulk_bar<'a>(cx: Ctx<'a>, total: usize, caps: Caps) -> Element<'a, Message> {
    let picked = cx.state.picked.len();
    let run = |what| (picked > 0).then_some(Message::Pages(Msg::Run(what, true)));
    let mut pills = Row::new().spacing(8);
    pills = pills
        .push(pill(
            if picked == total {
                "Deselect All"
            } else {
                "Select All"
            },
            None,
            Some(Message::Pages(Msg::PickAll)),
            false,
        ))
        .push(pill("Play", Some(crate::icons::PLAY), run(Do::Play), false))
        .push(pill(
            "Shuffle",
            Some(icons::SHUFFLE),
            run(Do::Shuffle),
            false,
        ))
        .push(pill(
            "Play Next",
            Some(icons::PLAY_NEXT),
            run(Do::PlayNext),
            false,
        ))
        .push(pill(
            "Add to Queue",
            Some(icons::ADD_QUEUE),
            run(Do::Queue),
            false,
        ));
    if cx.account {
        pills = pills
            .push(pill("Like", Some(icons::HEART), run(Do::Like), false))
            .push(pill(
                "Add to Playlist",
                Some(icons::PLAYLIST),
                run(Do::AddToPlaylist),
                false,
            ));
    }
    if caps.remove {
        pills = pills.push(pill("Remove", Some(icons::REMOVE), run(Do::Remove), false));
    }
    if caps.delete {
        pills = pills.push(pill("Delete", Some(icons::REMOVE), run(Do::Delete), false));
    }
    pills = pills.push(pill(
        "Done",
        None,
        Some(Message::Pages(Msg::Select(false))),
        false,
    ));
    column![
        text(format!("{picked} selected"))
            .size(14)
            .color(theme::SECONDARY),
        pills.wrap().vertical_spacing(8)
    ]
    .spacing(8)
    .into()
}

/// A song row in select mode: a tick circle, cover, title and artists. A click picks it.
pub fn pick_row<'a>(
    covers: &'a ui::Covers,
    song: &SongItem,
    index: usize,
    on: bool,
) -> Element<'a, Message> {
    let mark: Element<'a, Message> = if on {
        container(
            ui::icon(icons::CHECK, 14.0).style(|_, _| iced::widget::svg::Style {
                color: Some(theme::BACKGROUND),
            }),
        )
        .center(22)
        .style(|_| container::Style {
            background: Some(Background::Color(theme::accent())),
            border: Border::default().rounded(11),
            ..container::Style::default()
        })
        .into()
    } else {
        container(Space::new())
            .width(22)
            .height(22)
            .style(|_| container::Style {
                border: Border::default()
                    .rounded(11)
                    .width(2)
                    .color(theme::SECONDARY),
                ..container::Style::default()
            })
            .into()
    };
    let content = row![
        mark,
        ui::cover(covers, song.thumbnail_url.as_deref(), 40.0, 6.0),
        column![
            text(song.title.clone())
                .size(14)
                .font(theme::weight(Weight::Medium))
                .wrapping(text::Wrapping::None),
            text(ui::artists(&song.artists))
                .size(13)
                .color(theme::SECONDARY)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(2)
        .width(Length::Fill)
        .clip(true),
    ]
    .spacing(14)
    .align_y(Alignment::Center)
    .height(ui::TRACK_ROW);
    ui::clickable(
        container(content).padding([0, 8]),
        Some(Message::Pages(Msg::Pick(index))),
        0.0,
    )
}

/// All the songs as pick rows.
pub fn pick_rows<'a>(
    covers: &'a ui::Covers,
    songs: &[SongItem],
    state: &State,
) -> Element<'a, Message> {
    column(
        songs
            .iter()
            .enumerate()
            .map(|(index, song)| pick_row(covers, song, index, state.picked.contains(&index))),
    )
    .spacing(2)
    .into()
}

/// 2x2 song covers for a playlist that has none (Swift `ArtworkMosaicView`).
pub fn mosaic<'a>(covers: &'a ui::Covers, songs: &[SongItem], size: f32) -> Element<'a, Message> {
    const GAP: f32 = 3.0;
    let cell = (size - GAP) / 2.0;
    let cover = |index: usize| {
        ui::cover(
            covers,
            songs.get(index).and_then(|s| s.thumbnail_url.as_deref()),
            cell,
            6.0,
        )
    };
    container(
        column![
            row![cover(0), cover(1)].spacing(GAP),
            row![cover(2), cover(3)].spacing(GAP)
        ]
        .spacing(GAP),
    )
    .clip(true)
    .style(|_| container::Style {
        border: Border::default().rounded(16),
        ..container::Style::default()
    })
    .into()
}

/// Podcast description: 110 characters and "More", or all of it and "Less".
pub fn description<'a>(state: &State, about: &'a str) -> Element<'a, Message> {
    let short = collapsed(about, 110);
    let shown = match (&short, state.described) {
        (Some(short), false) => short.clone(),
        _ => about.to_owned(),
    };
    let mut body = column![text(shown).size(14).color(theme::SECONDARY)].spacing(4);
    if short.is_some() {
        body = body.push(
            button(
                text(if state.described { "Less" } else { "More" })
                    .size(14)
                    .font(theme::weight(Weight::Medium)),
            )
            .padding(0)
            .on_press(Message::Pages(Msg::Describe))
            .style(|_, _| button::Style {
                text_color: theme::accent(),
                ..button::Style::default()
            }),
        );
    }
    container(body).max_width(720).into()
}

/// The episode status chip: ON AIR, PLAYED or minutes left.
pub fn tag<'a>(label: String) -> Element<'a, Message> {
    container(
        text(label)
            .size(11)
            .font(theme::weight(Weight::Bold))
            .color(theme::SECONDARY),
    )
    .padding([3, 8])
    .style(|_| container::Style {
        background: Some(Background::Color(theme::HOVER)),
        border: Border::default().rounded(8),
        ..container::Style::default()
    })
    .into()
}

/// The episode to resume: cover, title, minutes left, a progress bar and Resume.
pub fn resume_card<'a>(
    covers: &'a ui::Covers,
    songs: &Arc<Vec<SongItem>>,
    index: usize,
    saved: EpisodePosition,
) -> Element<'a, Message> {
    let song = &songs[index];
    let progress = (saved.position / saved.duration.max(1.0)).clamp(0.0, 1.0) as f32;
    let bar = iced::widget::progress_bar(0.0..=1.0, progress)
        .girth(4)
        .style(|_| iced::widget::progress_bar::Style {
            background: Background::Color(theme::PRESSED),
            bar: Background::Color(theme::accent()),
            border: Border::default().rounded(2),
        });
    let info = column![
        text("Continue listening")
            .size(12)
            .font(theme::weight(Weight::Bold))
            .color(theme::SECONDARY),
        text(ui::fit(&song.title, 420.0, 15.0))
            .size(15)
            .font(theme::weight(Weight::Medium))
            .wrapping(text::Wrapping::None),
        text(minutes_left(saved.remaining()))
            .size(12)
            .color(theme::SECONDARY),
        bar,
    ]
    .spacing(4)
    .width(Length::Fill);
    container(
        row![
            ui::cover(covers, song.thumbnail_url.as_deref(), 64.0, 8.0),
            info,
            ui::accent_button(
                "Resume",
                Some(crate::icons::PLAY),
                Some(Message::PlayQueue(songs.clone(), index)),
            ),
        ]
        .spacing(16)
        .align_y(Alignment::Center),
    )
    .padding(14)
    .max_width(720)
    .style(|_| container::Style {
        background: Some(Background::Color(theme::HOVER)),
        border: Border::default().rounded(14),
        ..container::Style::default()
    })
    .into()
}

/// The card and bulk-playlist overlay, when one is open.
pub fn overlay<'a>(state: &'a State, window: iced::Size) -> Option<Element<'a, Message>> {
    const WIDTH: f32 = 232.0;
    let overlay = state.overlay.as_ref()?;
    let item = |glyph, label: &str, message| song_menu::row_button(glyph, label, message);
    let items: Vec<Element<'a, Message>> = match &overlay.kind {
        Kind::Card(card) => {
            let run = |what| Message::Pages(Msg::CardRun(what));
            let mut items = vec![
                item(crate::icons::PLAY, "Play", run(Do::Play)),
                item(icons::SHUFFLE, "Shuffle", run(Do::Shuffle)),
                item(icons::PLAY_NEXT, "Play Next", run(Do::PlayNext)),
                item(icons::ADD_QUEUE, "Add to Queue", run(Do::Queue)),
            ];
            if let Some(route) = route_of(card) {
                items.push(item(icons::ALBUM, "Open", Message::Pages(Msg::Open(route))));
            }
            if let YTItem::Album(album) = card.as_ref()
                && let Some(id) = album.artists.iter().find_map(|a| a.id.clone())
            {
                items.push(item(
                    icons::ARTIST,
                    "Go to Artist",
                    Message::Pages(Msg::Open(Route::Artist(id))),
                ));
            }
            if let Some(url) = card.web_url() {
                items.push(item(
                    icons::SHARE,
                    "Copy Link",
                    Message::Pages(Msg::Share(url)),
                ));
            }
            items
        }
        Kind::Playlists(_, list) => match list {
            None => vec![song_menu::note("Loading playlists…")],
            Some(list) if list.is_empty() => vec![song_menu::note("No playlists")],
            Some(list) => list
                .iter()
                .map(|playlist| {
                    item(
                        icons::PLAYLIST,
                        &playlist.name,
                        Message::Pages(Msg::AddTo(playlist.id.clone(), playlist.name.clone())),
                    )
                })
                .collect(),
        },
    };
    let height = (items.len() as f32 * song_menu::ITEM + 2.0 * song_menu::PAD)
        .min(window.height - 2.0 * theme::INSET);
    let panel = ui::glass_panel(
        song_menu::GLASS,
        14.0,
        Length::Fixed(WIDTH),
        Length::Fixed(height),
        iced::widget::scrollable(column(items).padding(song_menu::PAD)).direction(
            iced::widget::scrollable::Direction::Vertical(
                iced::widget::scrollable::Scrollbar::hidden(),
            ),
        ),
    );
    let x = overlay
        .at
        .x
        .min(window.width - WIDTH - theme::INSET)
        .max(theme::INSET);
    let y = if overlay.at.y + height > window.height - theme::INSET {
        (overlay.at.y - height).max(theme::INSET)
    } else {
        overlay.at.y
    };
    let backdrop = mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
        .on_press(Message::Pages(Msg::Close))
        .on_right_press(Message::Pages(Msg::Close));
    Some(
        iced::widget::stack![
            backdrop,
            container(panel).padding(Padding {
                top: y,
                left: x,
                ..Padding::ZERO
            }),
        ]
        .into(),
    )
}

/// A card opens its menu on right-click.
pub fn on_right_click<'a>(
    content: impl Into<Element<'a, Message>>,
    item: &YTItem,
) -> Element<'a, Message> {
    mouse_area(content)
        .on_right_press(Message::Pages(Msg::Card(Box::new(item.clone()))))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(id: &str) -> SongItem {
        SongItem {
            video_id: id.to_owned(),
            ..SongItem::default()
        }
    }

    fn ids(songs: &[SongItem]) -> Vec<&str> {
        songs.iter().map(|s| s.video_id.as_str()).collect()
    }

    #[test]
    fn shuffle_keeps_every_song() {
        let songs: Vec<_> = (0..20).map(|i| song(&i.to_string())).collect();
        let mut out = ids(&shuffled(&songs))
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        out.sort();
        let mut expected: Vec<_> = (0..20).map(|i| i.to_string()).collect();
        expected.sort();
        assert_eq!(out, expected);
    }

    #[test]
    fn picked_songs_keep_page_order() {
        let songs = [song("a"), song("b"), song("c")];
        let picked = HashSet::from([2, 0]);
        assert_eq!(ids(&picked_songs(&songs, &picked)), ["a", "c"]);
    }

    #[test]
    fn selection_toggles() {
        let mut state = State::default();
        state.toggle(1);
        state.toggle(1);
        state.toggle(2);
        assert_eq!(state.picked, HashSet::from([2]));
        state.selecting = true;
        state.reset();
        assert!(!state.selecting && state.picked.is_empty());
    }

    #[test]
    fn share_links() {
        assert_eq!(
            share_url(&Route::Album("MPRE1".into()), Some("OLAK5")).as_deref(),
            Some("https://music.youtube.com/playlist?list=OLAK5")
        );
        assert_eq!(
            share_url(&Route::Album("MPRE1".into()), None).as_deref(),
            Some("https://music.youtube.com/browse/MPRE1")
        );
        assert_eq!(
            share_url(&Route::Playlist("PL1".into()), None).as_deref(),
            Some("https://music.youtube.com/playlist?list=PL1")
        );
        assert_eq!(share_url(&Route::Home, None), None);
    }

    #[test]
    fn episode_tags() {
        let at = |position| EpisodePosition {
            position,
            duration: 3600.0,
            updated_at: 1.0,
        };
        assert_eq!(episode_tag(None, false), None);
        assert_eq!(episode_tag(None, true).as_deref(), Some("ON AIR"));
        assert_eq!(
            episode_tag(Some(at(3590.0)), false).as_deref(),
            Some("PLAYED")
        );
        assert_eq!(
            episode_tag(Some(at(1800.0)), false).as_deref(),
            Some("30 MIN LEFT")
        );
        assert_eq!(minutes_left(5.0), "1 MIN LEFT");
    }

    #[test]
    fn resume_picks_latest_unfinished() {
        let episode = |id: &str| EpisodeItem {
            video_id: id.to_owned(),
            ..EpisodeItem::default()
        };
        let episodes = [episode("a"), episode("b"), episode("c")];
        let mut store = EpisodePlaybackStore::default();
        store.save_position("a", 100.0, 3600.0, 10.0);
        store.save_position("b", 200.0, 3600.0, 20.0);
        // Finished: never offered.
        store.save_position("c", 3590.0, 3600.0, 30.0);
        let (index, _) = latest_in_progress(&episodes, &store).unwrap();
        assert_eq!(index, 1);
        assert!(latest_in_progress(&episodes[2..], &store).is_none());
    }

    #[test]
    fn long_descriptions_collapse() {
        assert_eq!(collapsed("short", 110), None);
        let long = "word ".repeat(40);
        let cut = collapsed(&long, 110).unwrap();
        assert!(cut.ends_with('…') && cut.chars().count() <= 111);
    }
}
