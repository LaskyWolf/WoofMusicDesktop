//! Trop/Services/TropLibraryStore.swift.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rusqlite::types::Value;
use rusqlite::{Connection, params, params_from_iter};
use swiftytube::sync::library_parsed_models::{
    ParsedAlbum, ParsedArtist, ParsedPlaylist, ParsedPodcast,
};
use swiftytube::sync::library_sync_models::{
    SyncAlbum, SyncArtist, SyncPlaylist, SyncPlaylistEntry, SyncPodcast, SyncSong,
};
use swiftytube::sync::library_sync_store::{LibrarySyncStore, StoreError};

use crate::database::database_service::{ConflictResolution, DatabaseService};
use crate::database::entities::album_entity::AlbumEntity;
use crate::database::entities::artist_entity::ArtistEntity;
use crate::database::entities::playlist_entity::PlaylistEntity;
use crate::database::entities::playlist_song_map::PlaylistSongMap;
use crate::database::entities::podcast_entity::PodcastEntity;
use crate::database::entities::song_entity::SongEntity;
use crate::database::{DbDate, date_text};

/// Swift `DatabaseService.chunked` default size.
const CHUNK_SIZE: usize = 500;

/// `LibrarySyncStore` over Trop's database.
///
/// Every list the `apply_*` methods receive is complete: SwiftyTube's
/// `LibrarySyncService` fetches every page and fails the whole section when
/// one page fails, so the sweeps never see a partial list. Swift fetches the
/// pages again here through `LibraryPages`; this store does not, so it needs
/// no network client and the synchronous trait needs no async bridge.
///
/// Swift posts `libraryDidChange` after each section; Rust has no change
/// notification yet, so nothing is announced.
pub struct TropLibraryStore {
    db: Arc<DatabaseService>,
}

impl TropLibraryStore {
    pub fn new(db: Arc<DatabaseService>) -> Self {
        Self { db }
    }

    /// Stops syncing a playlist: it keeps its songs and is edited only on this
    /// computer. The playlist on YouTube Music is left as it is. The row keeps
    /// its YouTube id without a browse id, which `apply_playlist_sync` skips.
    pub fn detach_playlist(&self, id: &str) -> rusqlite::Result<()> {
        self.db.write(|db| {
            db.execute(
                "UPDATE playlist SET browse_id = NULL, is_auto_sync = 0, is_editable = 1 WHERE id = ?",
                [id],
            )?;
            Ok(())
        })
    }

    /// Syncs a detached playlist again. Its songs reload from YouTube Music the
    /// next time it opens, so local changes must reach YouTube Music first.
    pub fn reattach_playlist(&self, id: &str) -> rusqlite::Result<()> {
        let browse_id = if id.starts_with("VL") {
            id.to_string()
        } else {
            format!("VL{id}")
        };
        self.db.write(|db| {
            db.execute(
                "UPDATE playlist SET browse_id = ? WHERE id = ?",
                [&browse_id, id],
            )?;
            Ok(())
        })
    }

    fn execute(&self, sql: &str, arguments: impl rusqlite::Params) -> Result<(), StoreError> {
        self.db.write(|db| db.execute(sql, arguments))?;
        Ok(())
    }
}

fn is_hidden(browse_id: &str) -> bool {
    PlaylistEntity::HIDDEN_IDS.contains(&browse_id)
}

/// Runs `sql` with `ids` bound to its `{}` placeholder list.
fn execute_in(db: &Connection, sql: &str, ids: &HashSet<String>) -> rusqlite::Result<()> {
    let sql = sql.replace("{}", &DatabaseService::placeholders(ids.len()));
    db.execute(&sql, params_from_iter(ids))?;
    Ok(())
}

// MARK: - Library sections

impl LibrarySyncStore for TropLibraryStore {
    fn apply_artist_sync(&self, items: &[ParsedArtist]) -> Result<HashSet<String>, StoreError> {
        let remote_ids: HashSet<String> = items.iter().map(|i| i.browse_id.clone()).collect();
        let now = DbDate(SystemTime::now());
        self.db.write(|db| {
            for item in items {
                db.execute(
                    "INSERT INTO artist (id, name, thumbnail_url, bookmarked_at, is_podcast_channel, channel_id)
                     VALUES (?, ?, ?, ?, ?, ?)
                     ON CONFLICT(id) DO UPDATE SET
                         name = excluded.name,
                         thumbnail_url = excluded.thumbnail_url,
                         channel_id = excluded.channel_id,
                         bookmarked_at = COALESCE(artist.bookmarked_at, excluded.bookmarked_at)",
                    params![
                        item.browse_id,
                        item.name,
                        item.thumbnail_url,
                        now,
                        false,
                        item.channel_id
                    ],
                )?;
            }
            if !remote_ids.is_empty() {
                execute_in(
                    db,
                    "UPDATE artist SET bookmarked_at = NULL WHERE bookmarked_at IS NOT NULL AND id NOT IN ({})",
                    &remote_ids,
                )?;
            }
            Ok(())
        })?;
        Ok(remote_ids)
    }

    fn apply_playlist_sync(&self, items: &[ParsedPlaylist]) -> Result<HashSet<String>, StoreError> {
        // Dropping "Episodes for Later" here also clears a copy saved by an older sync.
        let items: Vec<&ParsedPlaylist> =
            items.iter().filter(|i| !is_hidden(&i.browse_id)).collect();
        let remote_ids: HashSet<String> = items.iter().map(|i| i.browse_id.clone()).collect();
        let now = SystemTime::now();
        self.db.write(|db| {
            for item in items {
                let existing: Option<PlaylistEntity> =
                    DatabaseService::fetch_one_in(db, &item.browse_id)?;
                let existing = existing.as_ref();
                // A YouTube id without a browse id is a playlist you stopped syncing.
                if existing.is_some_and(|e| e.browse_id.is_none()) {
                    continue;
                }
                let entity = PlaylistEntity {
                    id: item.browse_id.clone(),
                    browse_id: Some(item.browse_id.clone()),
                    name: item.title.clone(),
                    thumbnail_url: item
                        .thumbnail_url
                        .clone()
                        .or_else(|| existing.and_then(|e| e.thumbnail_url.clone())),
                    is_editable: existing.is_some_and(|e| e.is_editable),
                    bookmarked_at: existing.and_then(|e| e.bookmarked_at).or(Some(now)),
                    remote_song_count: item
                        .song_count
                        .or_else(|| existing.and_then(|e| e.remote_song_count)),
                    is_auto_sync: true,
                };
                DatabaseService::save_in(db, &entity)?;
            }
            if !remote_ids.is_empty() {
                execute_in(
                    db,
                    "DELETE FROM playlist_song_map WHERE playlist_id IN (
                         SELECT id FROM playlist WHERE is_auto_sync = 1 AND browse_id IS NOT NULL
                         AND browse_id NOT IN ({})
                     )",
                    &remote_ids,
                )?;
                execute_in(
                    db,
                    "DELETE FROM playlist WHERE is_auto_sync = 1 AND browse_id IS NOT NULL
                     AND browse_id NOT IN ({})",
                    &remote_ids,
                )?;
            }
            Ok(())
        })?;
        Ok(remote_ids)
    }

    fn apply_album_sync(&self, items: &[ParsedAlbum]) -> Result<HashSet<String>, StoreError> {
        let remote_ids: HashSet<String> = items.iter().map(|i| i.browse_id.clone()).collect();
        let now = SystemTime::now();
        self.db.write(|db| {
            for item in items {
                let existing: Option<AlbumEntity> =
                    DatabaseService::fetch_one_in(db, &item.browse_id)?;
                let existing = existing.as_ref();
                let entity = AlbumEntity {
                    id: item.browse_id.clone(),
                    title: item.title.clone(),
                    playlist_id: item
                        .playlist_id
                        .clone()
                        .or_else(|| existing.and_then(|e| e.playlist_id.clone())),
                    thumbnail_url: item
                        .thumbnail_url
                        .clone()
                        .or_else(|| existing.and_then(|e| e.thumbnail_url.clone())),
                    song_count: if item.song_count != 0 {
                        item.song_count
                    } else {
                        existing.map_or(0, |e| e.song_count)
                    },
                    duration: if item.duration != 0 {
                        item.duration
                    } else {
                        existing.map_or(0, |e| e.duration)
                    },
                    bookmarked_at: existing.and_then(|e| e.bookmarked_at).or(Some(now)),
                    is_uploaded: existing.is_some_and(|e| e.is_uploaded),
                };
                DatabaseService::save_in(db, &entity)?;
            }
            if !remote_ids.is_empty() {
                execute_in(
                    db,
                    "UPDATE album SET bookmarked_at = NULL WHERE bookmarked_at IS NOT NULL AND id NOT IN ({})",
                    &remote_ids,
                )?;
            }
            Ok(())
        })?;
        Ok(remote_ids)
    }

    fn apply_podcast_sync(&self, items: &[ParsedPodcast]) -> Result<HashSet<String>, StoreError> {
        // Dropping "Episodes for Later" here also clears a copy saved by an older sync.
        let items: Vec<&ParsedPodcast> =
            items.iter().filter(|i| !is_hidden(&i.browse_id)).collect();
        let remote_ids: HashSet<String> = items.iter().map(|i| i.browse_id.clone()).collect();
        let now = SystemTime::now();
        self.db.write(|db| {
            for item in items {
                let existing: Option<PodcastEntity> =
                    DatabaseService::fetch_one_in(db, &item.browse_id)?;
                let existing = existing.as_ref();
                let entity = PodcastEntity {
                    id: item.browse_id.clone(),
                    name: item.name.clone(),
                    thumbnail_url: item
                        .thumbnail_url
                        .clone()
                        .or_else(|| existing.and_then(|e| e.thumbnail_url.clone())),
                    subscribed_at: existing.and_then(|e| e.subscribed_at).or(Some(now)),
                };
                DatabaseService::save_in(db, &entity)?;
            }
            if !remote_ids.is_empty() {
                execute_in(
                    db,
                    "UPDATE podcast SET subscribed_at = NULL WHERE subscribed_at IS NOT NULL AND id NOT IN ({})",
                    &remote_ids,
                )?;
            }
            Ok(())
        })?;
        Ok(remote_ids)
    }

    // MARK: - Liked songs

    fn liked_playlist_song_ids_ordered(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<String>, StoreError> {
        Ok(self.db.read(|db| {
            db.prepare(
                "SELECT song_id FROM playlist_song_map WHERE playlist_id = ? ORDER BY position",
            )?
            .query_map([playlist_id], |row| row.get(0))?
            .collect()
        })?)
    }

    fn mirror_liked_songs(
        &self,
        ordered_ids: &[String],
        base_date: SystemTime,
    ) -> Result<(), StoreError> {
        if ordered_ids.is_empty() {
            return Ok(());
        }
        self.db.write(|db| {
            for (index, song_id) in ordered_ids.iter().enumerate() {
                let create_date = base_date - Duration::from_secs(index as u64);
                db.execute(
                    "UPDATE song SET liked = 1, modify_date = ?, create_date = ? WHERE id = ?",
                    params![DbDate(base_date), DbDate(create_date), song_id],
                )?;
            }
            Ok(())
        })?;
        Ok(())
    }

    fn unmark_stale_liked_songs(
        &self,
        excluding: &HashSet<String>,
        grace_cutoff: SystemTime,
    ) -> Result<(), StoreError> {
        // A guest-shaped answer lists nothing too: it must not unlike the whole library.
        if excluding.is_empty() {
            return Ok(());
        }
        self.db.write(|db| {
            let sql = format!(
                "SELECT id FROM song WHERE liked = 1 AND id NOT IN ({}) AND modify_date < ?",
                DatabaseService::placeholders(excluding.len())
            );
            let arguments = excluding
                .iter()
                .map(|id| Value::Text(id.clone()))
                .chain([Value::Text(date_text(grace_cutoff))]);
            let stale_ids: Vec<String> = db
                .prepare(&sql)?
                .query_map(params_from_iter(arguments), |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for chunk in DatabaseService::chunked(&stale_ids, CHUNK_SIZE) {
                let sql = format!(
                    "UPDATE song SET liked = 0 WHERE id IN ({})",
                    DatabaseService::placeholders(chunk.len())
                );
                db.execute(&sql, params_from_iter(&chunk))?;
            }
            Ok(())
        })?;
        Ok(())
    }

    fn delete_playlist_rows(&self, playlist_id: &str) -> Result<(), StoreError> {
        self.db.write(|db| {
            db.execute(
                "DELETE FROM playlist_song_map WHERE playlist_id = ?",
                [playlist_id],
            )?;
            db.execute("DELETE FROM playlist WHERE id = ?", [playlist_id])
        })?;
        Ok(())
    }

    // MARK: - Playlists

    fn fetch_playlist(&self, id: &str) -> Result<Option<SyncPlaylist>, StoreError> {
        Ok(self.db.fetch_one::<PlaylistEntity>(id)?.map(Into::into))
    }

    fn save_playlist(&self, playlist: &SyncPlaylist) -> Result<(), StoreError> {
        Ok(self.db.save(&PlaylistEntity::from(playlist))?)
    }

    fn insert_or_replace_playlist(
        &self,
        playlist: &SyncPlaylist,
    ) -> Result<SyncPlaylist, StoreError> {
        self.db.insert_or_replace(&PlaylistEntity::from(playlist))?;
        Ok(playlist.clone())
    }

    fn delete_playlist(&self, id: &str) -> Result<(), StoreError> {
        self.execute("DELETE FROM playlist WHERE id = ?", [id])
    }

    fn delete_maps_for_playlist(&self, playlist_id: &str) -> Result<(), StoreError> {
        self.execute(
            "DELETE FROM playlist_song_map WHERE playlist_id = ?",
            [playlist_id],
        )
    }

    fn insert_map_ignoring_conflicts(&self, entry: &SyncPlaylistEntry) -> Result<(), StoreError> {
        self.db
            .insert_on_conflict(&PlaylistSongMap::from(entry), ConflictResolution::Ignore)?;
        Ok(())
    }

    fn fetch_map_entry(
        &self,
        playlist_id: &str,
        song_id: &str,
    ) -> Result<Option<SyncPlaylistEntry>, StoreError> {
        let maps: Vec<PlaylistSongMap> = self.db.fetch_all_sql(
            "SELECT * FROM playlist_song_map WHERE playlist_id = ? AND song_id = ? LIMIT 1",
            [playlist_id, song_id],
        )?;
        Ok(maps.into_iter().next().map(Into::into))
    }

    fn delete_map_entry(&self, playlist_id: &str, song_id: &str) -> Result<(), StoreError> {
        self.execute(
            "DELETE FROM playlist_song_map WHERE playlist_id = ? AND song_id = ?",
            [playlist_id, song_id],
        )
    }

    // MARK: - Songs

    fn fetch_song(&self, id: &str) -> Result<Option<SyncSong>, StoreError> {
        Ok(self.db.fetch_one::<SongEntity>(id)?.map(Into::into))
    }

    fn save_song(&self, song: &SyncSong) -> Result<(), StoreError> {
        self.db.write(|db| {
            let mut entity = SongEntity::from(song);
            // Sync doesn't know this flag: keep the stored one.
            entity.is_unavailable = DatabaseService::fetch_one_in::<SongEntity>(db, &song.id)?
                .is_some_and(|stored| stored.is_unavailable);
            DatabaseService::save_in(db, &entity)
        })?;
        Ok(())
    }

    fn insert_song_ignoring_conflicts(&self, song: &SyncSong) -> Result<(), StoreError> {
        self.db
            .insert_on_conflict(&SongEntity::from(song), ConflictResolution::Ignore)?;
        Ok(())
    }

    fn fetch_orphan_song_ids(&self) -> Result<Vec<String>, StoreError> {
        Ok(self.db.orphan_song_ids()?)
    }

    // MARK: - Artists / albums / podcasts

    fn fetch_artist(&self, id: &str) -> Result<Option<SyncArtist>, StoreError> {
        Ok(self.db.fetch_one::<ArtistEntity>(id)?.map(Into::into))
    }

    fn save_artist(&self, artist: &SyncArtist) -> Result<(), StoreError> {
        Ok(self.db.save(&ArtistEntity::from(artist))?)
    }

    fn fetch_album(&self, id: &str) -> Result<Option<SyncAlbum>, StoreError> {
        Ok(self.db.fetch_one::<AlbumEntity>(id)?.map(Into::into))
    }

    fn save_album(&self, album: &SyncAlbum) -> Result<(), StoreError> {
        Ok(self.db.save(&AlbumEntity::from(album))?)
    }

    fn fetch_podcast(&self, id: &str) -> Result<Option<SyncPodcast>, StoreError> {
        Ok(self.db.fetch_one::<PodcastEntity>(id)?.map(Into::into))
    }

    fn save_podcast(&self, podcast: &SyncPodcast) -> Result<(), StoreError> {
        Ok(self.db.save(&PodcastEntity::from(podcast))?)
    }
}

// MARK: - Entity mapping

impl From<&SyncSong> for SongEntity {
    /// `is_unavailable` is not part of sync: it starts false.
    fn from(sync: &SyncSong) -> Self {
        Self {
            id: sync.id.clone(),
            title: sync.title.clone(),
            artist_name: sync.artist_name.clone(),
            album_name: sync.album_name.clone(),
            duration: sync.duration,
            thumbnail_url: sync.thumbnail_url.clone(),
            liked: sync.liked,
            total_play_time: sync.total_play_time,
            in_library: sync.in_library,
            library_add_token: sync.library_add_token.clone(),
            library_remove_token: sync.library_remove_token.clone(),
            is_episode: sync.is_episode,
            is_uploaded: sync.is_uploaded,
            is_video: sync.is_video,
            create_date: sync.create_date,
            modify_date: sync.modify_date,
            is_unavailable: false,
        }
    }
}

impl From<SongEntity> for SyncSong {
    fn from(entity: SongEntity) -> Self {
        Self {
            id: entity.id,
            title: entity.title,
            artist_name: entity.artist_name,
            album_name: entity.album_name,
            duration: entity.duration,
            thumbnail_url: entity.thumbnail_url,
            liked: entity.liked,
            total_play_time: entity.total_play_time,
            in_library: entity.in_library,
            library_add_token: entity.library_add_token,
            library_remove_token: entity.library_remove_token,
            is_episode: entity.is_episode,
            is_uploaded: entity.is_uploaded,
            is_video: entity.is_video,
            create_date: entity.create_date,
            modify_date: entity.modify_date,
        }
    }
}

impl From<&SyncArtist> for ArtistEntity {
    fn from(sync: &SyncArtist) -> Self {
        Self {
            id: sync.id.clone(),
            name: sync.name.clone(),
            thumbnail_url: sync.thumbnail_url.clone(),
            bookmarked_at: sync.bookmarked_at,
            is_podcast_channel: sync.is_podcast_channel,
            channel_id: sync.channel_id.clone(),
        }
    }
}

impl From<ArtistEntity> for SyncArtist {
    fn from(entity: ArtistEntity) -> Self {
        Self {
            id: entity.id,
            name: entity.name,
            thumbnail_url: entity.thumbnail_url,
            bookmarked_at: entity.bookmarked_at,
            is_podcast_channel: entity.is_podcast_channel,
            channel_id: entity.channel_id,
        }
    }
}

impl From<&SyncAlbum> for AlbumEntity {
    fn from(sync: &SyncAlbum) -> Self {
        Self {
            id: sync.id.clone(),
            title: sync.title.clone(),
            playlist_id: sync.playlist_id.clone(),
            thumbnail_url: sync.thumbnail_url.clone(),
            song_count: sync.song_count,
            duration: sync.duration,
            bookmarked_at: sync.bookmarked_at,
            is_uploaded: sync.is_uploaded,
        }
    }
}

impl From<AlbumEntity> for SyncAlbum {
    fn from(entity: AlbumEntity) -> Self {
        Self {
            id: entity.id,
            title: entity.title,
            playlist_id: entity.playlist_id,
            thumbnail_url: entity.thumbnail_url,
            song_count: entity.song_count,
            duration: entity.duration,
            bookmarked_at: entity.bookmarked_at,
            is_uploaded: entity.is_uploaded,
        }
    }
}

impl From<&SyncPlaylist> for PlaylistEntity {
    fn from(sync: &SyncPlaylist) -> Self {
        Self {
            id: sync.id.clone(),
            browse_id: sync.browse_id.clone(),
            name: sync.name.clone(),
            thumbnail_url: sync.thumbnail_url.clone(),
            is_editable: sync.is_editable,
            bookmarked_at: sync.bookmarked_at,
            remote_song_count: sync.remote_song_count,
            is_auto_sync: sync.is_auto_sync,
        }
    }
}

impl From<PlaylistEntity> for SyncPlaylist {
    fn from(entity: PlaylistEntity) -> Self {
        Self {
            id: entity.id,
            browse_id: entity.browse_id,
            name: entity.name,
            thumbnail_url: entity.thumbnail_url,
            is_editable: entity.is_editable,
            bookmarked_at: entity.bookmarked_at,
            remote_song_count: entity.remote_song_count,
            is_auto_sync: entity.is_auto_sync,
        }
    }
}

impl From<&SyncPodcast> for PodcastEntity {
    fn from(sync: &SyncPodcast) -> Self {
        Self {
            id: sync.id.clone(),
            name: sync.name.clone(),
            thumbnail_url: sync.thumbnail_url.clone(),
            subscribed_at: sync.subscribed_at,
        }
    }
}

impl From<PodcastEntity> for SyncPodcast {
    fn from(entity: PodcastEntity) -> Self {
        Self {
            id: entity.id,
            name: entity.name,
            thumbnail_url: entity.thumbnail_url,
            subscribed_at: entity.subscribed_at,
        }
    }
}

impl From<&SyncPlaylistEntry> for PlaylistSongMap {
    fn from(entry: &SyncPlaylistEntry) -> Self {
        Self {
            id: None,
            playlist_id: entry.playlist_id.clone(),
            song_id: entry.song_id.clone(),
            position: entry.position,
            set_video_id: entry.set_video_id.clone(),
        }
    }
}

impl From<PlaylistSongMap> for SyncPlaylistEntry {
    fn from(map: PlaylistSongMap) -> Self {
        Self {
            playlist_id: map.playlist_id,
            song_id: map.song_id,
            position: map.position,
            set_video_id: map.set_video_id,
        }
    }
}
