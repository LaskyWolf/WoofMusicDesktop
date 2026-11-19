//! History and Activity: their state and messages. The pages are in `pages.rs`;
//! `trop::history` and `trop::activity` do the reading and merging.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::Task;
use trop::activity::activity_store::{ActivityItem, SeenIds};
use trop::activity::release_feed::ReleaseFeed;
use trop::database::database_service::HistoryEntry;
use trop::history::history_view_model::{HistoryViewModel, RemoteSection};

use crate::services::Services;
use crate::{Message, Route};

/// Activity's last load is reused for this long when the page opens again.
const FRESH_FOR: Duration = Duration::from_secs(60 * 60);

#[derive(Default)]
pub struct Feeds {
    pub history: HistoryViewModel,
    /// The History page's search field.
    pub query: String,
    /// The new releases; `None` until the first load ends.
    pub activity: Option<Vec<ActivityItem>>,
    /// Counts Activity loads, so one started before a sign-out is dropped.
    load: u64,
    loading: bool,
    loaded_at: Option<Instant>,
    /// Activity is the page on screen.
    viewing: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    LocalLoaded(Result<Vec<HistoryEntry>, String>),
    /// Tagged with the session's load count, so an answer for a previous session is dropped.
    RemoteLoaded(u64, Result<Vec<RemoteSection>, String>),
    Query(String),
    /// The next page of older plays from this app.
    More,
    /// Removes a play from this app's history.
    Delete(i64),
    ActivityLoaded(u64, Option<Vec<ActivityItem>>),
    /// Opens a release's album and marks it seen.
    Open(String, String),
}

impl Feeds {
    /// History opened: plays from this app, then YouTube Music's own history when signed in.
    pub fn enter_history(&mut self, services: &Arc<Services>, signed_in: bool) -> Task<Message> {
        let remote = if signed_in {
            let load = self.load;
            Task::perform(
                HistoryViewModel::read_remote(services.client.clone()),
                move |result| Message::Feeds(Msg::RemoteLoaded(load, result)),
            )
        } else {
            Task::none()
        };
        Task::batch([self.read_local(services), remote])
    }

    fn read_local(&self, services: &Arc<Services>) -> Task<Message> {
        let db = services.db.clone();
        let limit = self.history.local_limit();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || HistoryViewModel::read_local(&db, limit))
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())
            },
            |result| Message::Feeds(Msg::LocalLoaded(result)),
        )
    }

    /// Activity opened: loads unless the last load is under an hour old.
    pub fn enter_activity(&mut self, services: &Arc<Services>) -> Task<Message> {
        if self.loaded_at.is_some_and(|at| at.elapsed() < FRESH_FOR) {
            return Task::none();
        }
        self.refresh_activity(services)
    }

    /// Swift `ActivityStore.refresh`. A load already running is not started twice.
    pub fn refresh_activity(&mut self, services: &Arc<Services>) -> Task<Message> {
        if self.loading {
            return Task::none();
        }
        self.loading = true;
        let load = self.load;
        let (client, db) = (services.client.clone(), services.db.clone());
        Task::perform(
            async move {
                let (saved, seen) = tokio::task::spawn_blocking(move || {
                    let seen = SeenIds::default_path()
                        .map(|path| SeenIds::load(&path))
                        .unwrap_or_default();
                    (ReleaseFeed::saved_artists(&db), seen)
                })
                .await
                .ok()?;
                ReleaseFeed::items(client, saved, seen.into_iter().collect()).await
            },
            move |items| Message::Feeds(Msg::ActivityLoaded(load, items)),
        )
    }

    /// Signed out or another account: forgets both lists.
    pub fn reset(&mut self) {
        let query = std::mem::take(&mut self.query);
        *self = Self {
            load: self.load + 1,
            query,
            viewing: self.viewing,
            ..Self::default()
        };
    }

    /// Unread releases, for the sidebar badge.
    pub fn unseen(&self) -> usize {
        self.activity
            .as_deref()
            .map_or(0, trop::activity::activity_store::unseen)
    }

    /// Called on every page change: leaving Activity counts what it showed as
    /// seen (Swift `onDisappear`), which clears the badge.
    pub fn entered(&mut self, route: &Route) {
        let left = self.viewing && *route != Route::Activity;
        self.viewing = *route == Route::Activity;
        if !left {
            return;
        }
        let ids = self
            .activity
            .iter()
            .flatten()
            .filter(|item| !item.is_read)
            .map(|item| item.id.clone())
            .collect();
        self.mark_seen(ids);
    }

    fn mark_seen(&mut self, ids: HashSet<String>) {
        if ids.is_empty() {
            return;
        }
        for item in self.activity.iter_mut().flatten() {
            item.is_read |= ids.contains(&item.id);
        }
        if let Some(path) = SeenIds::default_path() {
            // A small file write: off the UI thread, nothing waits for it.
            std::thread::spawn(move || SeenIds::save(&path, &ids));
        }
    }

    /// Covers of the rows on the two pages.
    pub fn cover_urls(&self) -> Vec<String> {
        let history = self.history.days.iter().flat_map(|day| &day.rows);
        history
            .filter_map(|row| row.song.thumbnail_url.clone())
            .chain(
                self.activity
                    .iter()
                    .flatten()
                    .filter_map(|item| item.thumbnail.clone()),
            )
            .take(crate::MAX_ROW_COVERS)
            .collect()
    }

    pub fn update(&mut self, message: Msg, services: &Arc<Services>) -> Task<Message> {
        match message {
            Msg::LocalLoaded(Ok(entries)) => {
                self.history.set_local(entries);
                Task::none()
            }
            Msg::LocalLoaded(Err(error)) => {
                eprintln!("History could not be read: {error}");
                self.history.is_loading = false;
                Task::none()
            }
            Msg::RemoteLoaded(load, _) if load != self.load => Task::none(),
            Msg::RemoteLoaded(_, Ok(sections)) => {
                self.history.set_remote(sections);
                Task::none()
            }
            // Signed out or offline: the list stays local.
            Msg::RemoteLoaded(_, Err(error)) => {
                log::info!("YouTube Music history not loaded: {error}");
                Task::none()
            }
            Msg::Query(query) => {
                self.query = query;
                Task::none()
            }
            Msg::More => match self.history.load_more_local() {
                Some(_) => self.read_local(services),
                None => Task::none(),
            },
            Msg::Delete(id) => {
                let db = services.db.clone();
                let delete = Task::future(tokio::task::spawn_blocking(move || {
                    if let Err(error) = HistoryViewModel::delete_events(&db, &[id]) {
                        eprintln!("Could not delete the play: {error}");
                    }
                }))
                .discard();
                delete.chain(self.read_local(services))
            }
            Msg::ActivityLoaded(load, items) => {
                if load != self.load {
                    return Task::none();
                }
                self.loading = false;
                match items {
                    Some(items) => {
                        self.activity = Some(items);
                        self.loaded_at = Some(Instant::now());
                    }
                    // Failed (offline, signed out): keep what is shown; the next visit tries again.
                    None => {
                        self.activity.get_or_insert_default();
                    }
                }
                Task::none()
            }
            Msg::Open(id, album) => {
                self.mark_seen(HashSet::from([id]));
                Task::done(Message::Navigate(Route::Album(album)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_of_a_previous_session_is_dropped_after_sign_out() {
        let services = Services::for_test();
        let mut feeds = Feeds::default();
        // A remote history read starts, then the session changes before it answers.
        let old_load = feeds.load;
        feeds.reset();
        let song = trop::networking::models::yt_item::SongItem {
            video_id: "old".into(),
            ..Default::default()
        };
        let _ = feeds.update(
            Msg::RemoteLoaded(old_load, Ok(vec![("Today".into(), vec![song])])),
            &services,
        );
        let rows: usize = feeds.history.days.iter().map(|day| day.rows.len()).sum();
        assert_eq!(rows, 0);
    }
}
