// SwiftyTube/Sync/LibrarySyncModels.swift.

use std::collections::HashSet;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use super::library_sync_store::StoreError;
use crate::networking::inner_tube_error::InnerTubeError;

/// Which library sections a full sync covers. Apps build this from their own
/// settings (Trop maps its five sync toggles onto it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LibrarySyncOptions {
    pub sync_artists: bool,
    pub sync_playlists: bool,
    pub sync_albums: bool,
    pub sync_podcasts: bool,
    pub sync_songs: bool,
}

impl Default for LibrarySyncOptions {
    fn default() -> Self {
        Self {
            sync_artists: true,
            sync_playlists: true,
            sync_albums: true,
            sync_podcasts: true,
            sync_songs: true,
        }
    }
}

impl LibrarySyncOptions {
    pub fn enabled_count(&self) -> usize {
        [
            self.sync_artists,
            self.sync_playlists,
            self.sync_albums,
            self.sync_podcasts,
            self.sync_songs,
        ]
        .into_iter()
        .filter(|on| *on)
        .count()
    }
}

/// Per-section remote ids collected by a full sync.
#[derive(Clone, Debug, Default)]
pub struct LibrarySyncResult {
    pub artist_ids: HashSet<String>,
    pub playlist_ids: HashSet<String>,
    pub album_ids: HashSet<String>,
    pub podcast_ids: HashSet<String>,
    pub song_ids: HashSet<String>,
    pub completed_sections: usize,
}

/// Fetched song metadata used to fill skeleton songs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SongMetadata {
    pub title: String,
    pub artist_name: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration: i64,
    pub album_name: Option<String>,
}

/// A stored song row, without any database framework attached.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SyncSong {
    pub id: String,
    pub title: String,
    pub artist_name: Option<String>,
    pub album_name: Option<String>,
    pub duration: i64,
    pub thumbnail_url: Option<String>,
    pub liked: bool,
    pub total_play_time: i64,
    pub in_library: Option<SystemTime>,
    pub library_add_token: String,
    pub library_remove_token: String,
    pub is_episode: bool,
    pub is_uploaded: bool,
    pub is_video: bool,
    pub create_date: SystemTime,
    pub modify_date: SystemTime,
}

/// Arguments of Swift `SyncSong.merging(existing:id:title:...)`; `Default`
/// gives the Swift defaults (not liked, empty tokens, all flags false).
#[derive(Clone, Debug, Default)]
pub struct SongMergeArgs {
    pub id: String,
    pub title: String,
    pub artist_name: Option<String>,
    pub album_name: Option<String>,
    pub duration: i64,
    pub thumbnail_url: Option<String>,
    pub liked: bool,
    pub library_add_token: String,
    pub library_remove_token: String,
    pub is_episode: bool,
    pub is_uploaded: bool,
    pub is_video: bool,
}

impl SyncSong {
    /// Empty placeholder for a song the store has never seen. Filled in later
    /// from fetched metadata; never surfaced to callers as-is.
    /// Swift default: `add_token: ""`.
    pub fn skeleton(id: &str, liked: bool, add_token: &str) -> SyncSong {
        let now = SystemTime::now();
        SyncSong {
            id: id.to_string(),
            title: String::new(),
            artist_name: None,
            album_name: None,
            duration: 0,
            thumbnail_url: None,
            liked,
            total_play_time: 0,
            in_library: None,
            library_add_token: add_token.to_string(),
            library_remove_token: String::new(),
            is_episode: false,
            is_uploaded: false,
            is_video: false,
            create_date: now,
            modify_date: now,
        }
    }

    /// Swift `merging(_:with:)`. Fills empty fields from metadata, preserving
    /// stored values. Does not touch `modify_date`: callers that persist set it.
    pub fn merging_metadata(song: &SyncSong, metadata: &SongMetadata) -> SyncSong {
        let mut enriched = song.clone();
        if enriched.title.is_empty() {
            enriched.title = metadata.title.clone();
        }
        enriched.artist_name = enriched.artist_name.or(metadata.artist_name.clone());
        enriched.thumbnail_url = enriched.thumbnail_url.or(metadata.thumbnail_url.clone());
        if enriched.duration == 0 {
            enriched.duration = metadata.duration;
        }
        enriched.album_name = enriched.album_name.or(metadata.album_name.clone());
        enriched
    }

    /// Merges freshly fetched metadata over the stored row, preserving user
    /// state (liked, play time, library flags, tokens, original create_date).
    /// Unknown incoming values (zero duration, missing artwork) fall back to
    /// the stored row instead of wiping it.
    pub fn merging(existing: Option<&SyncSong>, args: SongMergeArgs) -> SyncSong {
        let now = SystemTime::now();
        SyncSong {
            id: args.id,
            title: args.title,
            artist_name: existing
                .and_then(|e| e.artist_name.clone())
                .or(args.artist_name),
            album_name: existing
                .and_then(|e| e.album_name.clone())
                .or(args.album_name),
            duration: if args.duration > 0 {
                args.duration
            } else {
                existing.map_or(0, |e| e.duration)
            },
            thumbnail_url: args
                .thumbnail_url
                .or_else(|| existing.and_then(|e| e.thumbnail_url.clone())),
            liked: existing.map_or(args.liked, |e| e.liked),
            total_play_time: existing.map_or(0, |e| e.total_play_time),
            in_library: existing.and_then(|e| e.in_library),
            library_add_token: existing
                .map_or(args.library_add_token, |e| e.library_add_token.clone()),
            library_remove_token: existing.map_or(args.library_remove_token, |e| {
                e.library_remove_token.clone()
            }),
            is_episode: existing.map_or(args.is_episode, |e| e.is_episode),
            is_uploaded: existing.map_or(args.is_uploaded, |e| e.is_uploaded),
            is_video: existing.map_or(args.is_video, |e| e.is_video),
            create_date: existing.map_or(now, |e| e.create_date),
            modify_date: now,
        }
    }
}

/// A stored artist row.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SyncArtist {
    pub id: String,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub bookmarked_at: Option<SystemTime>,
    pub is_podcast_channel: bool,
    pub channel_id: Option<String>,
}

/// A stored album row. Swift default: `is_uploaded: false`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SyncAlbum {
    pub id: String,
    pub title: String,
    pub playlist_id: Option<String>,
    pub thumbnail_url: Option<String>,
    pub song_count: i64,
    pub duration: i64,
    pub bookmarked_at: Option<SystemTime>,
    pub is_uploaded: bool,
}

/// A stored playlist row. Swift defaults: no thumbnail, no remote count,
/// `is_auto_sync: false`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SyncPlaylist {
    pub id: String,
    pub browse_id: Option<String>,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub is_editable: bool,
    pub bookmarked_at: Option<SystemTime>,
    pub remote_song_count: Option<i64>,
    pub is_auto_sync: bool,
}

impl SyncPlaylist {
    /// Merges fetched playlist metadata over the stored row, preserving the
    /// user's name/editability/bookmark. Used when materializing a playlist's
    /// contents (sync + detail); the liked-playlists sync keeps its own
    /// policy (name/thumbnail always refresh, bookmark defaults to now).
    /// Thumbnail and auto-sync survive too: the detail path must not demote
    /// a library row the sweeps select on `is_auto_sync`.
    pub fn merging(
        existing: Option<&SyncPlaylist>,
        id: &str,
        browse_id: Option<&str>,
        name: &str,
        remote_song_count: i64,
    ) -> SyncPlaylist {
        SyncPlaylist {
            id: id.to_string(),
            browse_id: browse_id.map(str::to_string),
            name: existing.map_or_else(|| name.to_string(), |e| e.name.clone()),
            thumbnail_url: existing.and_then(|e| e.thumbnail_url.clone()),
            is_editable: existing.is_some_and(|e| e.is_editable),
            bookmarked_at: existing.and_then(|e| e.bookmarked_at),
            remote_song_count: Some(remote_song_count),
            is_auto_sync: existing.is_some_and(|e| e.is_auto_sync),
        }
    }
}

/// A stored podcast row.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SyncPodcast {
    pub id: String,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub subscribed_at: Option<SystemTime>,
}

/// A playlist membership row.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SyncPlaylistEntry {
    pub playlist_id: String,
    pub song_id: String,
    pub position: i64,
    pub set_video_id: Option<String>,
}

/// Swift `MutationError`, plus the errors the sync services rethrow as
/// `any Error` (client, store) and `renamePlaylist`'s "Playlist not found".
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("Failed to create playlist")]
    PlaylistCreationFailed,
    #[error("Playlist not found")]
    PlaylistNotFound,
    #[error(transparent)]
    InnerTube(#[from] InnerTubeError),
    #[error(transparent)]
    Store(#[from] StoreError),
}
