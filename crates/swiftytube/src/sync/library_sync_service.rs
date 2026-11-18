// SwiftyTube/Sync/LibrarySyncService.swift.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::library_browse_parser::LibraryBrowseParser;
use super::library_sync_models::{LibrarySyncOptions, LibrarySyncResult, SyncError};
use super::library_sync_store::LibrarySyncStore;
use super::playlist_detail_service::PlaylistDetailService;
use crate::core::swiftytube_log::SwiftyTubeLog;
use crate::networking::inner_tube_client::{BrowseArgs, InnerTubeClient};
use crate::networking::inner_tube_decode::JsonDict;

pub struct LibrarySyncService {
    client: Arc<InnerTubeClient>,
    store: Arc<dyn LibrarySyncStore>,
}

impl LibrarySyncService {
    pub fn new(client: Arc<InnerTubeClient>, store: Arc<dyn LibrarySyncStore>) -> Self {
        Self { client, store }
    }

    /// Syncs every enabled section; a failing section is logged and left
    /// out of `completed_sections`. `on_likes_refresh` runs at the end
    /// (Swift passes `nil` for none; pass `async {}`).
    pub async fn sync_all(
        &self,
        options: LibrarySyncOptions,
        on_likes_refresh: impl Future<Output = ()>,
    ) -> LibrarySyncResult {
        fn log_failure(section: &str, error: &SyncError) {
            SwiftyTubeLog::error(&format!("{section} error: {error:?} ({error})"));
        }
        let mut result = LibrarySyncResult::default();
        if options.sync_artists {
            match self.sync_subscribed_artists().await {
                Ok(ids) => {
                    result.artist_ids = ids;
                    result.completed_sections += 1;
                }
                Err(error) => log_failure("syncSubscribedArtists", &error),
            }
        }
        if options.sync_playlists {
            match self.sync_liked_playlists().await {
                Ok(ids) => {
                    result.playlist_ids = ids;
                    result.completed_sections += 1;
                }
                Err(error) => log_failure("syncLikedPlaylists", &error),
            }
        }
        if options.sync_albums {
            match self.sync_saved_albums().await {
                Ok(ids) => {
                    result.album_ids = ids;
                    result.completed_sections += 1;
                }
                Err(error) => log_failure("syncSavedAlbums", &error),
            }
        }
        if options.sync_podcasts {
            match self.sync_subscribed_podcasts().await {
                Ok(ids) => {
                    result.podcast_ids = ids;
                    result.completed_sections += 1;
                }
                Err(error) => log_failure("syncPodcasts", &error),
            }
        }
        if options.sync_songs {
            match self.sync_liked_songs().await {
                Ok(ids) => {
                    result.song_ids = ids;
                    result.completed_sections += 1;
                }
                Err(error) => log_failure("syncLikedSongs", &error),
            }
        }
        on_likes_refresh.await;
        result
    }
}

// MARK: Section sync

impl LibrarySyncService {
    pub async fn sync_subscribed_artists(&self) -> Result<HashSet<String>, SyncError> {
        let items = self
            .fetch_all_pages(
                "FEmusic_library_corpus_artists",
                LibraryBrowseParser::parse_artists,
            )
            .await?;
        Ok(self.store.apply_artist_sync(&items)?)
    }

    pub async fn sync_liked_playlists(&self) -> Result<HashSet<String>, SyncError> {
        // YTM's Liked Music pseudo-playlist must never materialize as a
        // library row: its members merge into Liked Songs via sync_liked_songs.
        let items = self
            .fetch_all_pages("FEmusic_liked_playlists", |json| {
                LibraryBrowseParser::parse_playlists(json)
                    .into_iter()
                    .filter(|p| p.browse_id != "LM" && p.browse_id != "VLLM")
                    .collect()
            })
            .await?;
        Ok(self.store.apply_playlist_sync(&items)?)
    }

    pub async fn sync_saved_albums(&self) -> Result<HashSet<String>, SyncError> {
        let items = self
            .fetch_all_pages("FEmusic_liked_albums", LibraryBrowseParser::parse_albums)
            .await?;
        Ok(self.store.apply_album_sync(&items)?)
    }

    pub async fn sync_subscribed_podcasts(&self) -> Result<HashSet<String>, SyncError> {
        let items = self
            .fetch_all_pages(
                "FEmusic_library_non_music_audio_list",
                LibraryBrowseParser::parse_podcasts,
            )
            .await?;
        Ok(self.store.apply_podcast_sync(&items)?)
    }

    /// Merges YTM's Liked Music auto-playlist (VLLM) into the permanent local
    /// Liked Songs list, like Metrolist. The LM playlist row itself is
    /// deleted right away so it never appears in the library. Reaching here
    /// means the fetch succeeded, so an empty remote is a verified "unliked
    /// everything" and the stale sweep runs regardless of count.
    pub async fn sync_liked_songs(&self) -> Result<HashSet<String>, SyncError> {
        PlaylistDetailService::new(&self.client)
            .fetch_playlist(&*self.store, "LM", 100, true)
            .await?;
        let ordered_ids = self.store.liked_playlist_song_ids_ordered("LM")?;
        let remote_ids: HashSet<String> = ordered_ids.iter().cloned().collect();
        if !ordered_ids.is_empty() {
            // Mirror YTM's liked order in the local Liked Songs list, which
            // sorts by create_date: newest like first, staggered by shelf position.
            self.store
                .mirror_liked_songs(&ordered_ids, SystemTime::now())?;
        }
        let grace_cutoff = SystemTime::now() - Duration::from_secs(5 * 60);
        self.store
            .unmark_stale_liked_songs(&remote_ids, grace_cutoff)?;
        self.store.delete_playlist_rows("LM")?;
        Ok(remote_ids)
    }
}

// MARK: Pagination

impl LibrarySyncService {
    /// Swift `paginate` default: 50 pages.
    async fn fetch_all_pages<T>(
        &self,
        browse_id: &str,
        parse: impl Fn(&JsonDict) -> Vec<T>,
    ) -> Result<Vec<T>, SyncError> {
        Ok(self
            .client
            .paginate(
                BrowseArgs::id(browse_id),
                50,
                parse,
                LibraryBrowseParser::extract_continuation_token,
            )
            .await?)
    }
}
