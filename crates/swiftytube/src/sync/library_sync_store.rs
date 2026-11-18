// SwiftyTube/Sync/LibrarySyncStore.swift.

use std::collections::HashSet;
use std::time::SystemTime;

use super::library_parsed_models::{ParsedAlbum, ParsedArtist, ParsedPlaylist, ParsedPodcast};
use super::library_sync_models::{
    SyncAlbum, SyncArtist, SyncPlaylist, SyncPlaylistEntry, SyncPodcast, SyncSong,
};

/// Whatever an app's store fails with (I/O, JSON, SQL).
pub type StoreError = Box<dyn std::error::Error + Send + Sync>;

/// App-side persistence for library sync, implemented by the app (Trop:
/// `TropLibraryStore`). None of the methods perform network I/O, so they are
/// synchronous.
pub trait LibrarySyncStore: Send + Sync {
    // MARK: - Library sections (batch merge + sweep)

    /// Upserts subscribed artists and clears bookmarks missing remotely.
    /// Returns the remote ids.
    fn apply_artist_sync(&self, items: &[ParsedArtist]) -> Result<HashSet<String>, StoreError>;

    /// Upserts liked playlists (never materializing the Liked Music
    /// pseudo-playlist) and deletes auto-synced rows missing remotely.
    /// Returns the remote ids.
    fn apply_playlist_sync(&self, items: &[ParsedPlaylist]) -> Result<HashSet<String>, StoreError>;

    /// Upserts saved albums and clears bookmarks missing remotely.
    /// Returns the remote ids.
    fn apply_album_sync(&self, items: &[ParsedAlbum]) -> Result<HashSet<String>, StoreError>;

    /// Upserts subscribed podcasts and clears subscriptions missing remotely.
    /// Returns the remote ids.
    fn apply_podcast_sync(&self, items: &[ParsedPodcast]) -> Result<HashSet<String>, StoreError>;

    // MARK: - Liked songs (Liked Music auto-playlist merge)

    /// Song ids of a materialized playlist, in remote shelf order.
    fn liked_playlist_song_ids_ordered(&self, playlist_id: &str)
    -> Result<Vec<String>, StoreError>;

    /// Mirrors the remote liked order into the permanent liked list.
    fn mirror_liked_songs(
        &self,
        ordered_ids: &[String],
        base_date: SystemTime,
    ) -> Result<(), StoreError>;

    /// Clears the liked flag for songs missing remotely, sparing recently
    /// modified rows newer than `grace_cutoff`.
    fn unmark_stale_liked_songs(
        &self,
        excluding: &HashSet<String>,
        grace_cutoff: SystemTime,
    ) -> Result<(), StoreError>;

    /// Deletes a materialized playlist's mappings and its playlist row.
    fn delete_playlist_rows(&self, playlist_id: &str) -> Result<(), StoreError>;

    // MARK: - Playlist contents

    fn fetch_playlist(&self, id: &str) -> Result<Option<SyncPlaylist>, StoreError>;
    fn save_playlist(&self, playlist: &SyncPlaylist) -> Result<(), StoreError>;
    fn insert_or_replace_playlist(
        &self,
        playlist: &SyncPlaylist,
    ) -> Result<SyncPlaylist, StoreError>;
    fn delete_playlist(&self, id: &str) -> Result<(), StoreError>;
    fn delete_maps_for_playlist(&self, playlist_id: &str) -> Result<(), StoreError>;
    fn insert_map_ignoring_conflicts(&self, entry: &SyncPlaylistEntry) -> Result<(), StoreError>;
    fn fetch_map_entry(
        &self,
        playlist_id: &str,
        song_id: &str,
    ) -> Result<Option<SyncPlaylistEntry>, StoreError>;
    fn delete_map_entry(&self, playlist_id: &str, song_id: &str) -> Result<(), StoreError>;

    // MARK: - Songs

    fn fetch_song(&self, id: &str) -> Result<Option<SyncSong>, StoreError>;
    fn save_song(&self, song: &SyncSong) -> Result<(), StoreError>;
    fn insert_song_ignoring_conflicts(&self, song: &SyncSong) -> Result<(), StoreError>;
    fn fetch_orphan_song_ids(&self) -> Result<Vec<String>, StoreError>;

    // MARK: - Artists / albums / podcasts

    fn fetch_artist(&self, id: &str) -> Result<Option<SyncArtist>, StoreError>;
    fn save_artist(&self, artist: &SyncArtist) -> Result<(), StoreError>;
    fn fetch_album(&self, id: &str) -> Result<Option<SyncAlbum>, StoreError>;
    fn save_album(&self, album: &SyncAlbum) -> Result<(), StoreError>;
    fn fetch_podcast(&self, id: &str) -> Result<Option<SyncPodcast>, StoreError>;
    fn save_podcast(&self, podcast: &SyncPodcast) -> Result<(), StoreError>;
}
