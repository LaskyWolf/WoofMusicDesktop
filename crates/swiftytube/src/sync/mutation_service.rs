// SwiftyTube/Sync/MutationService.swift. Each mutation writes the store
// first, then calls YouTube Music, and rolls the store back if that fails.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use reqwest::Url;
use serde_json::Value;

use super::library_sync_models::{
    SongMetadata, SyncAlbum, SyncError, SyncPlaylist, SyncPlaylistEntry, SyncPodcast, SyncSong,
};
use super::library_sync_store::LibrarySyncStore;
use crate::core::swiftytube_log::SwiftyTubeLog;
use crate::networking::inner_tube_client::{BrowseArgs, InnerTubeClient, NextArgs};
use crate::networking::inner_tube_decode::{InnerTubeDecode, JsonDict, as_dict, as_string};
use crate::networking::inner_tube_error::InnerTubeError;
use crate::networking::inner_tube_json::InnerTubeJSON;
use crate::networking::playlist_edits::{OptimisticMutation, PlaylistEditAction};

#[derive(Default)]
struct OrphanRepairState {
    last_orphan_repair: Option<Instant>,
    unrecoverable_orphans: HashSet<String>,
}

pub struct MutationService {
    client: Arc<InnerTubeClient>,
    store: Arc<dyn LibrarySyncStore>,
    /// Swift actor state; locked only between `.await`s.
    orphans: Mutex<OrphanRepairState>,
}

impl MutationService {
    const ORPHAN_REPAIR_INTERVAL: Duration = Duration::from_secs(10 * 60);
    const ORPHAN_REPAIR_BATCH_SIZE: usize = 10;

    pub fn new(client: Arc<InnerTubeClient>, store: Arc<dyn LibrarySyncStore>) -> Self {
        Self {
            client,
            store,
            orphans: Mutex::default(),
        }
    }

    fn orphans(&self) -> std::sync::MutexGuard<'_, OrphanRepairState> {
        self.orphans.lock().unwrap_or_else(|e| e.into_inner())
    }

    async fn enrich_empty_song(&self, song: SyncSong) -> SyncSong {
        if !song.title.is_empty() {
            return song;
        }
        match self.fetch_song_metadata(&song.id).await {
            Ok(Some(metadata)) => SyncSong::merging_metadata(&song, &metadata),
            _ => song,
        }
    }

    /// The stored song, or a skeleton filled from `next` metadata.
    async fn stored_or_enriched(
        &self,
        video_id: &str,
        liked: bool,
        add_token: &str,
    ) -> Result<SyncSong, SyncError> {
        Ok(match self.store.fetch_song(video_id)? {
            Some(existing) => existing,
            None => {
                self.enrich_empty_song(SyncSong::skeleton(video_id, liked, add_token))
                    .await
            }
        })
    }

    async fn fetch_song_metadata(
        &self,
        video_id: &str,
    ) -> Result<Option<SongMetadata>, InnerTubeError> {
        let json = self
            .client
            .next(NextArgs {
                video_id: Some(video_id.to_string()),
                ..NextArgs::default()
            })
            .await?;
        Ok(Self::song_metadata(&json))
    }

    /// Title, owner, artwork and length from a `next` response.
    pub fn song_metadata(json: &JsonDict) -> Option<SongMetadata> {
        let first_tab = InnerTubeDecode::array(
            &[
                "contents",
                "singleColumnMusicWatchNextResultsRenderer",
                "tabbedRenderer",
                "watchNextTabbedResultsRenderer",
                "tabs",
            ],
            json,
        )?
        .into_iter()
        .next()?;
        let results = InnerTubeDecode::dict(&["tabRenderer", "content", "results"], first_tab)?;
        let primary = as_dict(results.get("primaryInfoRenderer"))?;

        let title = InnerTubeJSON::runs_text(as_dict(primary.get("title"))).unwrap_or_default();
        let byline =
            InnerTubeDecode::dict(&["secondaryInfoRenderer", "videoOwnerRenderer"], results);
        let artist_name = InnerTubeJSON::raw_runs(byline.and_then(|b| as_dict(b.get("title"))))
            .first()
            .and_then(|run| as_string(run.get("text")))
            .map(str::to_string);
        let thumbnails = InnerTubeDecode::array(&["thumbnail", "thumbnails"], primary);
        let thumbnail_url = InnerTubeJSON::last_thumbnail_url(thumbnails.as_deref());
        let duration = as_string(primary.get("lengthSeconds"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        Some(SongMetadata {
            title,
            artist_name,
            thumbnail_url,
            duration,
            album_name: None,
        })
    }

    /// Fills up to 10 songs the store references but has no row for, at most
    /// once every 10 minutes. Songs whose metadata comes back empty are not retried.
    pub async fn repair_orphan_songs(&self) {
        let unrecoverable = {
            let mut state = self.orphans();
            if let Some(last) = state.last_orphan_repair
                && last.elapsed() < Self::ORPHAN_REPAIR_INTERVAL
            {
                return;
            }
            state.last_orphan_repair = Some(Instant::now());
            state.unrecoverable_orphans.clone()
        };
        let Ok(ids) = self.store.fetch_orphan_song_ids() else {
            return;
        };
        let orphans: Vec<String> = ids
            .into_iter()
            .filter(|id| !unrecoverable.contains(id))
            .take(Self::ORPHAN_REPAIR_BATCH_SIZE)
            .collect();
        for id in orphans {
            match self.fetch_song_metadata(&id).await {
                Ok(Some(metadata)) if !metadata.title.is_empty() => {
                    let repaired =
                        SyncSong::merging_metadata(&SyncSong::skeleton(&id, false, ""), &metadata);
                    let _ = self.store.insert_song_ignoring_conflicts(&repaired);
                }
                Ok(_) => {
                    self.orphans().unrecoverable_orphans.insert(id);
                }
                Err(_) => {}
            }
        }
    }

    pub async fn like_song(&self, video_id: &str) -> Result<(), SyncError> {
        let mut song = self.stored_or_enriched(video_id, false, "").await?;
        song.liked = true;
        song.modify_date = SystemTime::now();
        self.store.save_song(&song)?;
        OptimisticMutation::attempting_remote(
            async {
                self.client.like(video_id, None, None).await?;
                Ok(())
            },
            async {
                let mut restored = song.clone();
                restored.liked = false;
                let _ = self.store.save_song(&restored);
            },
        )
        .await
    }

    pub async fn unlike_song(&self, video_id: &str) -> Result<(), SyncError> {
        let mut song = self.stored_or_enriched(video_id, true, "").await?;
        song.liked = false;
        song.modify_date = SystemTime::now();
        self.store.save_song(&song)?;
        OptimisticMutation::attempting_remote(
            async {
                self.client.unlike(video_id, None, None).await?;
                Ok(())
            },
            async {
                let mut restored = song.clone();
                restored.liked = true;
                let _ = self.store.save_song(&restored);
            },
        )
        .await
    }

    pub async fn add_to_library(&self, video_id: &str, add_token: &str) -> Result<(), SyncError> {
        let mut song = self.stored_or_enriched(video_id, false, add_token).await?;
        song.in_library = song.in_library.or(Some(SystemTime::now()));
        song.modify_date = SystemTime::now();
        self.store.save_song(&song)?;
        if add_token.is_empty() {
            return Ok(());
        }
        OptimisticMutation::attempting_remote(
            async {
                self.client
                    .feedback(&[add_token.to_string()], None, None)
                    .await?;
                Ok(())
            },
            async {
                let mut restored = song.clone();
                restored.in_library = None;
                let _ = self.store.save_song(&restored);
            },
        )
        .await
    }

    pub async fn remove_from_library(
        &self,
        video_id: &str,
        remove_token: &str,
    ) -> Result<(), SyncError> {
        let mut applied = self.store.fetch_song(video_id)?;
        if let Some(existing) = &mut applied {
            existing.in_library = None;
            existing.modify_date = SystemTime::now();
            self.store.save_song(existing)?;
        }
        if remove_token.is_empty() {
            return Ok(());
        }
        OptimisticMutation::attempting_remote(
            async {
                self.client
                    .feedback(&[remove_token.to_string()], None, None)
                    .await?;
                Ok(())
            },
            async {
                if let Some(mut restored) = applied.clone() {
                    restored.in_library = Some(SystemTime::now());
                    restored.modify_date = SystemTime::now();
                    let _ = self.store.save_song(&restored);
                }
            },
        )
        .await
    }

    /// A playlist without a browse id (or unknown to the store) lives only on this device.
    fn is_local_playlist(&self, playlist_id: &str) -> bool {
        self.store
            .fetch_playlist(playlist_id)
            .ok()
            .flatten()
            .is_none_or(|p| p.browse_id.is_none())
    }

    /// Swift default: `set_video_id: None`.
    pub async fn add_to_playlist(
        &self,
        playlist_id: &str,
        song_id: &str,
        set_video_id: Option<&str>,
    ) -> Result<(), SyncError> {
        let is_local = self.is_local_playlist(playlist_id);
        self.store
            .insert_map_ignoring_conflicts(&SyncPlaylistEntry {
                playlist_id: playlist_id.to_string(),
                song_id: song_id.to_string(),
                position: 0,
                set_video_id: set_video_id.map(str::to_string),
            })?;
        if is_local {
            return Ok(());
        }
        let actions = PlaylistEditAction::dictionaries(&[PlaylistEditAction::AddVideo {
            video_id: song_id.to_string(),
            set_video_id: set_video_id.map(str::to_string),
        }]);
        OptimisticMutation::attempting_remote(
            async {
                self.client
                    .edit_playlist(playlist_id, actions, None, None)
                    .await?;
                Ok(())
            },
            async {
                let _ = self.store.delete_map_entry(playlist_id, song_id);
            },
        )
        .await
    }

    pub async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        song_id: &str,
        set_video_id: &str,
    ) -> Result<(), SyncError> {
        if !self.is_local_playlist(playlist_id) {
            let actions = PlaylistEditAction::dictionaries(&[PlaylistEditAction::RemoveVideo {
                set_video_id: set_video_id.to_string(),
            }]);
            self.client
                .edit_playlist(playlist_id, actions, None, None)
                .await?;
        }
        self.store.delete_map_entry(playlist_id, song_id)?;
        Ok(())
    }

    /// Swift default: `description: None`. Returns the new playlist id.
    pub async fn create_playlist(
        &self,
        title: &str,
        description: Option<&str>,
    ) -> Result<String, SyncError> {
        let json = self
            .client
            .create_playlist(title, description, None, None)
            .await?;
        let playlist_id = PlaylistEditAction::extract_playlist_id(&json)
            .ok_or(SyncError::PlaylistCreationFailed)?;
        self.store.insert_or_replace_playlist(&SyncPlaylist {
            id: playlist_id.clone(),
            browse_id: Some(format!("VL{playlist_id}")),
            name: title.to_string(),
            thumbnail_url: None,
            is_editable: true,
            bookmarked_at: Some(SystemTime::now()),
            remote_song_count: Some(0),
            is_auto_sync: false,
        })?;
        Ok(playlist_id)
    }

    pub async fn delete_playlist(&self, playlist_id: &str) -> Result<(), SyncError> {
        if !self.is_local_playlist(playlist_id) {
            self.client.delete_playlist(playlist_id, None, None).await?;
        }
        self.store.delete_maps_for_playlist(playlist_id)?;
        self.store.delete_playlist(playlist_id)?;
        Ok(())
    }

    pub async fn rename_playlist(
        &self,
        playlist_id: &str,
        new_name: &str,
    ) -> Result<(), SyncError> {
        let Some(mut entity) = self.store.fetch_playlist(playlist_id).ok().flatten() else {
            return Err(SyncError::PlaylistNotFound);
        };
        if entity.browse_id.is_some() {
            let actions = PlaylistEditAction::dictionaries(&[PlaylistEditAction::RenamePlaylist {
                name: new_name.to_string(),
            }]);
            self.client
                .edit_playlist(playlist_id, actions, None, None)
                .await?;
        }
        entity.name = new_name.to_string();
        self.store.save_playlist(&entity)?;
        Ok(())
    }

    pub async fn subscribe_artist(
        &self,
        channel_id: &str,
        artist_id: &str,
    ) -> Result<(), SyncError> {
        let mut applied = self.store.fetch_artist(artist_id)?;
        if let Some(existing) = &mut applied {
            existing.bookmarked_at = Some(SystemTime::now());
            existing.channel_id = Some(channel_id.to_string());
            self.store.save_artist(existing)?;
        }
        OptimisticMutation::attempting_remote(
            async {
                self.client.subscribe(channel_id, None, None).await?;
                Ok(())
            },
            async {
                if let Some(mut restored) = applied.clone() {
                    restored.bookmarked_at = None;
                    restored.channel_id = None;
                    let _ = self.store.save_artist(&restored);
                }
            },
        )
        .await
    }

    pub async fn unsubscribe_artist(
        &self,
        channel_id: &str,
        artist_id: &str,
    ) -> Result<(), SyncError> {
        let mut applied = self.store.fetch_artist(artist_id)?;
        if let Some(existing) = &mut applied {
            existing.bookmarked_at = None;
            self.store.save_artist(existing)?;
        }
        OptimisticMutation::attempting_remote(
            async {
                self.client.unsubscribe(channel_id, None, None).await?;
                Ok(())
            },
            async {
                if let Some(mut restored) = applied.clone() {
                    restored.bookmarked_at = restored.bookmarked_at.or(Some(SystemTime::now()));
                    let _ = self.store.save_artist(&restored);
                }
            },
        )
        .await
    }

    // MARK: - Albums

    /// Bookmarks the album and rates its `OLAK…` playlist. Swift defaults:
    /// `thumbnail_url`/`playlist_id: None`, `song_count`/`duration: 0`.
    pub async fn save_album(
        &self,
        browse_id: &str,
        title: &str,
        thumbnail_url: Option<&str>,
        playlist_id: Option<&str>,
        song_count: i64,
        duration: i64,
    ) -> Result<(), SyncError> {
        let mut resolved_playlist_id = playlist_id.filter(|p| !p.is_empty()).map(str::to_string);
        let fetched_album = self.store.fetch_album(browse_id)?;
        if resolved_playlist_id.is_none() {
            resolved_playlist_id = fetched_album
                .as_ref()
                .and_then(|a| a.playlist_id.clone())
                .filter(|p| !p.is_empty());
        }
        if resolved_playlist_id.is_none() {
            resolved_playlist_id = self.album_playlist_id(browse_id).await?;
        }
        let entity = match fetched_album {
            Some(mut existing) => {
                existing.bookmarked_at = Some(SystemTime::now());
                if let Some(thumbnail_url) = thumbnail_url {
                    existing.thumbnail_url = Some(thumbnail_url.to_string());
                }
                if resolved_playlist_id.is_some() {
                    existing.playlist_id = resolved_playlist_id.clone();
                }
                if !title.is_empty() {
                    existing.title = title.to_string();
                }
                if song_count > 0 {
                    existing.song_count = song_count;
                }
                if duration > 0 {
                    existing.duration = duration;
                }
                existing
            }
            None => SyncAlbum {
                id: browse_id.to_string(),
                title: title.to_string(),
                playlist_id: resolved_playlist_id.clone(),
                thumbnail_url: thumbnail_url.map(str::to_string),
                song_count,
                duration,
                bookmarked_at: Some(SystemTime::now()),
                is_uploaded: false,
            },
        };
        self.store.save_album(&entity)?;
        let Some(pid) = resolved_playlist_id else {
            SwiftyTubeLog::error(&format!(
                "saveAlbum: no playlistId for {browse_id}, local-only"
            ));
            return Ok(());
        };
        OptimisticMutation::attempting_remote(
            async {
                self.client.rate_playlist(&pid, "LIKE", None, None).await?;
                Ok(())
            },
            async {
                if let Ok(Some(mut restored)) = self.store.fetch_album(browse_id) {
                    restored.bookmarked_at = None;
                    let _ = self.store.save_album(&restored);
                }
            },
        )
        .await
    }

    /// Swift default: `playlist_id: None`.
    pub async fn unsave_album(
        &self,
        browse_id: &str,
        playlist_id: Option<&str>,
    ) -> Result<(), SyncError> {
        let mut had_bookmark = false;
        let mut existing_playlist_id = playlist_id.filter(|p| !p.is_empty()).map(str::to_string);
        if let Some(mut existing) = self.store.fetch_album(browse_id)? {
            had_bookmark = existing.bookmarked_at.is_some();
            existing.bookmarked_at = None;
            if existing_playlist_id.is_none() {
                existing_playlist_id = existing.playlist_id.clone().filter(|p| !p.is_empty());
            }
            self.store.save_album(&existing)?;
        }
        if existing_playlist_id.is_none() {
            existing_playlist_id = self.album_playlist_id(browse_id).await?;
        }
        let Some(pid) = existing_playlist_id else {
            SwiftyTubeLog::error(&format!(
                "unsaveAlbum: no playlistId for {browse_id}, local-only"
            ));
            return Ok(());
        };
        OptimisticMutation::attempting_remote(
            async {
                self.client
                    .rate_playlist(&pid, "INDIFFERENT", None, None)
                    .await?;
                Ok(())
            },
            async {
                if had_bookmark && let Ok(Some(mut restored)) = self.store.fetch_album(browse_id) {
                    restored.bookmarked_at = Some(SystemTime::now());
                    let _ = self.store.save_album(&restored);
                }
            },
        )
        .await
    }

    // MARK: - Album playlist resolution

    async fn album_playlist_id(&self, browse_id: &str) -> Result<Option<String>, SyncError> {
        let json = self.client.browse(BrowseArgs::id(browse_id)).await?;
        Ok(Self::album_playlist_id_in(&json))
    }

    /// The album's `OLAK…` playlist: the first `likeEndpoint.target.playlistId`
    /// anywhere in the page, else the `list` query of `microformat`'s canonical URL.
    pub fn album_playlist_id_in(json: &JsonDict) -> Option<String> {
        fn walk(dict: &JsonDict) -> Option<&str> {
            if let Some(pid) =
                InnerTubeDecode::string(&["likeEndpoint", "target", "playlistId"], dict)
                && pid.starts_with("OLAK")
            {
                return Some(pid);
            }
            dict.values().find_map(walk_value)
        }
        fn walk_value(value: &Value) -> Option<&str> {
            match value {
                Value::Object(dict) => walk(dict),
                Value::Array(list) => list.iter().find_map(walk_value),
                _ => None,
            }
        }
        if let Some(found) = walk(json) {
            return Some(found.to_string());
        }
        let url = InnerTubeDecode::string(
            &["microformat", "microformatDataRenderer", "urlCanonical"],
            json,
        )?;
        let url = Url::parse("https://music.youtube.com/")
            .ok()?
            .join(url)
            .ok()?;
        url.query_pairs()
            .find(|(name, _)| name == "list")
            .map(|(_, value)| value.into_owned())
            .filter(|list| list.starts_with("OLAK"))
    }

    // MARK: - Podcasts

    /// Without a feedback token the remote subscribe is best-effort through
    /// the channel endpoint. Swift defaults: every optional is `None`.
    pub async fn subscribe_podcast(
        &self,
        browse_id: &str,
        name: &str,
        thumbnail_url: Option<&str>,
        channel_id: Option<&str>,
        feedback_token: Option<&str>,
    ) -> Result<(), SyncError> {
        let podcast = match self.store.fetch_podcast(browse_id)? {
            Some(mut existing) => {
                existing.subscribed_at = Some(SystemTime::now());
                if !name.is_empty() {
                    existing.name = name.to_string();
                }
                if let Some(thumbnail_url) = thumbnail_url {
                    existing.thumbnail_url = Some(thumbnail_url.to_string());
                }
                existing
            }
            None => SyncPodcast {
                id: browse_id.to_string(),
                name: name.to_string(),
                thumbnail_url: thumbnail_url.map(str::to_string),
                subscribed_at: Some(SystemTime::now()),
            },
        };
        self.store.save_podcast(&podcast)?;
        let Some(token) = feedback_token else {
            let channel = channel_id.unwrap_or(browse_id);
            if !channel.is_empty() {
                // Podcast channels share the subscription endpoint.
                let _ = self.client.subscribe(channel, None, None).await;
            }
            return Ok(());
        };
        if token.is_empty() {
            return Ok(());
        }
        OptimisticMutation::attempting_remote(
            async {
                self.client
                    .feedback(&[token.to_string()], None, None)
                    .await?;
                Ok(())
            },
            async {
                if let Ok(Some(mut restored)) = self.store.fetch_podcast(browse_id) {
                    restored.subscribed_at = None;
                    let _ = self.store.save_podcast(&restored);
                }
            },
        )
        .await
    }

    /// Swift defaults: `channel_id`/`feedback_token: None`.
    pub async fn unsubscribe_podcast(
        &self,
        browse_id: &str,
        channel_id: Option<&str>,
        feedback_token: Option<&str>,
    ) -> Result<(), SyncError> {
        if let Some(mut existing) = self.store.fetch_podcast(browse_id)? {
            existing.subscribed_at = None;
            self.store.save_podcast(&existing)?;
        }
        let Some(token) = feedback_token else {
            let channel = channel_id.unwrap_or(browse_id);
            if !channel.is_empty() {
                let _ = self.client.unsubscribe(channel, None, None).await;
            }
            return Ok(());
        };
        if token.is_empty() {
            return Ok(());
        }
        OptimisticMutation::attempting_remote(
            async {
                self.client
                    .feedback(&[token.to_string()], None, None)
                    .await?;
                Ok(())
            },
            async {
                if let Ok(Some(mut restored)) = self.store.fetch_podcast(browse_id) {
                    restored.subscribed_at = restored.subscribed_at.or(Some(SystemTime::now()));
                    let _ = self.store.save_podcast(&restored);
                }
            },
        )
        .await
    }
}
