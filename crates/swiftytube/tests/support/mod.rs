//! `MemoryLibrarySyncStore`: the `LibrarySyncStore` the sync tests run against.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use swiftytube::sync::library_parsed_models::{
    ParsedAlbum, ParsedArtist, ParsedPlaylist, ParsedPodcast,
};
use swiftytube::sync::library_sync_models::{
    SyncAlbum, SyncArtist, SyncPlaylist, SyncPlaylistEntry, SyncPodcast, SyncSong,
};
use swiftytube::sync::library_sync_store::{LibrarySyncStore, StoreError};

/// Every table of `MemoryLibrarySyncStore`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LibraryData {
    pub songs: BTreeMap<String, SyncSong>,
    pub artists: BTreeMap<String, SyncArtist>,
    pub albums: BTreeMap<String, SyncAlbum>,
    pub playlists: BTreeMap<String, SyncPlaylist>,
    pub podcasts: BTreeMap<String, SyncPodcast>,
    /// One row per (playlist, song): a second insert of the same pair is ignored.
    pub playlist_maps: Vec<SyncPlaylistEntry>,
}

/// Every list it receives counts as complete, so the sweeps always run.
#[derive(Default)]
pub struct MemoryLibrarySyncStore {
    data: Mutex<LibraryData>,
}

impl MemoryLibrarySyncStore {
    /// A copy of every table.
    pub fn snapshot(&self) -> LibraryData {
        self.lock().clone()
    }

    fn lock(&self) -> MutexGuard<'_, LibraryData> {
        self.data.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn read<R>(&self, read: impl FnOnce(&LibraryData) -> R) -> Result<R, StoreError> {
        Ok(read(&self.lock()))
    }

    fn write<R>(&self, change: impl FnOnce(&mut LibraryData) -> R) -> Result<R, StoreError> {
        Ok(change(&mut self.lock()))
    }
}

fn remote_ids<'a>(ids: impl Iterator<Item = &'a String>) -> HashSet<String> {
    ids.cloned().collect()
}

impl LibrarySyncStore for MemoryLibrarySyncStore {
    fn apply_artist_sync(&self, items: &[ParsedArtist]) -> Result<HashSet<String>, StoreError> {
        let remote = remote_ids(items.iter().map(|i| &i.browse_id));
        self.write(|data| {
            let now = SystemTime::now();
            for item in items {
                let existing = data.artists.get(&item.browse_id);
                let artist = SyncArtist {
                    id: item.browse_id.clone(),
                    name: item.name.clone(),
                    thumbnail_url: item.thumbnail_url.clone(),
                    bookmarked_at: existing.and_then(|e| e.bookmarked_at).or(Some(now)),
                    is_podcast_channel: existing.is_some_and(|e| e.is_podcast_channel),
                    channel_id: item.channel_id.clone(),
                };
                data.artists.insert(artist.id.clone(), artist);
            }
            if !remote.is_empty() {
                for artist in data.artists.values_mut() {
                    if !remote.contains(&artist.id) {
                        artist.bookmarked_at = None;
                    }
                }
            }
        })?;
        Ok(remote)
    }

    fn apply_playlist_sync(&self, items: &[ParsedPlaylist]) -> Result<HashSet<String>, StoreError> {
        let remote = remote_ids(items.iter().map(|i| &i.browse_id));
        self.write(|data| {
            let now = SystemTime::now();
            for item in items {
                let existing = data.playlists.get(&item.browse_id);
                if existing.is_some_and(|e| e.browse_id.is_none()) {
                    continue;
                }
                let playlist = SyncPlaylist {
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
                data.playlists.insert(playlist.id.clone(), playlist);
            }
            if !remote.is_empty() {
                let gone: HashSet<String> = data
                    .playlists
                    .values()
                    .filter(|p| {
                        p.is_auto_sync && p.browse_id.as_ref().is_some_and(|b| !remote.contains(b))
                    })
                    .map(|p| p.id.clone())
                    .collect();
                data.playlist_maps
                    .retain(|m| !gone.contains(&m.playlist_id));
                data.playlists.retain(|id, _| !gone.contains(id));
            }
        })?;
        Ok(remote)
    }

    fn apply_album_sync(&self, items: &[ParsedAlbum]) -> Result<HashSet<String>, StoreError> {
        let remote = remote_ids(items.iter().map(|i| &i.browse_id));
        self.write(|data| {
            let now = SystemTime::now();
            for item in items {
                let existing = data.albums.get(&item.browse_id);
                let album = SyncAlbum {
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
                data.albums.insert(album.id.clone(), album);
            }
            if !remote.is_empty() {
                for album in data.albums.values_mut() {
                    if !remote.contains(&album.id) {
                        album.bookmarked_at = None;
                    }
                }
            }
        })?;
        Ok(remote)
    }

    fn apply_podcast_sync(&self, items: &[ParsedPodcast]) -> Result<HashSet<String>, StoreError> {
        let remote = remote_ids(items.iter().map(|i| &i.browse_id));
        self.write(|data| {
            let now = SystemTime::now();
            for item in items {
                let existing = data.podcasts.get(&item.browse_id);
                let podcast = SyncPodcast {
                    id: item.browse_id.clone(),
                    name: item.name.clone(),
                    thumbnail_url: item
                        .thumbnail_url
                        .clone()
                        .or_else(|| existing.and_then(|e| e.thumbnail_url.clone())),
                    subscribed_at: existing.and_then(|e| e.subscribed_at).or(Some(now)),
                };
                data.podcasts.insert(podcast.id.clone(), podcast);
            }
            if !remote.is_empty() {
                for podcast in data.podcasts.values_mut() {
                    if !remote.contains(&podcast.id) {
                        podcast.subscribed_at = None;
                    }
                }
            }
        })?;
        Ok(remote)
    }

    fn liked_playlist_song_ids_ordered(
        &self,
        playlist_id: &str,
    ) -> Result<Vec<String>, StoreError> {
        self.read(|data| {
            let mut maps: Vec<&SyncPlaylistEntry> = data
                .playlist_maps
                .iter()
                .filter(|m| m.playlist_id == playlist_id)
                .collect();
            maps.sort_by_key(|m| m.position);
            maps.into_iter().map(|m| m.song_id.clone()).collect()
        })
    }

    fn mirror_liked_songs(
        &self,
        ordered_ids: &[String],
        base_date: SystemTime,
    ) -> Result<(), StoreError> {
        self.write(|data| {
            for (index, id) in ordered_ids.iter().enumerate() {
                if let Some(song) = data.songs.get_mut(id) {
                    song.liked = true;
                    song.modify_date = base_date;
                    song.create_date = base_date - Duration::from_secs(index as u64);
                }
            }
        })
    }

    fn unmark_stale_liked_songs(
        &self,
        excluding: &HashSet<String>,
        grace_cutoff: SystemTime,
    ) -> Result<(), StoreError> {
        self.write(|data| {
            for song in data.songs.values_mut() {
                if song.liked && !excluding.contains(&song.id) && song.modify_date < grace_cutoff {
                    song.liked = false;
                }
            }
        })
    }

    fn delete_playlist_rows(&self, playlist_id: &str) -> Result<(), StoreError> {
        self.write(|data| {
            data.playlist_maps.retain(|m| m.playlist_id != playlist_id);
            data.playlists.remove(playlist_id);
        })
    }

    fn fetch_playlist(&self, id: &str) -> Result<Option<SyncPlaylist>, StoreError> {
        self.read(|data| data.playlists.get(id).cloned())
    }

    fn save_playlist(&self, playlist: &SyncPlaylist) -> Result<(), StoreError> {
        self.write(|data| {
            data.playlists.insert(playlist.id.clone(), playlist.clone());
        })
    }

    fn insert_or_replace_playlist(
        &self,
        playlist: &SyncPlaylist,
    ) -> Result<SyncPlaylist, StoreError> {
        self.save_playlist(playlist)?;
        Ok(playlist.clone())
    }

    fn delete_playlist(&self, id: &str) -> Result<(), StoreError> {
        self.write(|data| {
            data.playlists.remove(id);
        })
    }

    fn delete_maps_for_playlist(&self, playlist_id: &str) -> Result<(), StoreError> {
        self.write(|data| data.playlist_maps.retain(|m| m.playlist_id != playlist_id))
    }

    fn insert_map_ignoring_conflicts(&self, entry: &SyncPlaylistEntry) -> Result<(), StoreError> {
        self.write(|data| {
            let exists = data
                .playlist_maps
                .iter()
                .any(|m| m.playlist_id == entry.playlist_id && m.song_id == entry.song_id);
            if !exists {
                data.playlist_maps.push(entry.clone());
            }
        })
    }

    fn fetch_map_entry(
        &self,
        playlist_id: &str,
        song_id: &str,
    ) -> Result<Option<SyncPlaylistEntry>, StoreError> {
        self.read(|data| {
            data.playlist_maps
                .iter()
                .find(|m| m.playlist_id == playlist_id && m.song_id == song_id)
                .cloned()
        })
    }

    fn delete_map_entry(&self, playlist_id: &str, song_id: &str) -> Result<(), StoreError> {
        self.write(|data| {
            data.playlist_maps
                .retain(|m| m.playlist_id != playlist_id || m.song_id != song_id)
        })
    }

    fn fetch_song(&self, id: &str) -> Result<Option<SyncSong>, StoreError> {
        self.read(|data| data.songs.get(id).cloned())
    }

    fn save_song(&self, song: &SyncSong) -> Result<(), StoreError> {
        self.write(|data| {
            data.songs.insert(song.id.clone(), song.clone());
        })
    }

    fn insert_song_ignoring_conflicts(&self, song: &SyncSong) -> Result<(), StoreError> {
        self.write(|data| {
            data.songs
                .entry(song.id.clone())
                .or_insert_with(|| song.clone());
        })
    }

    /// Song ids a playlist lists but no song row describes.
    fn fetch_orphan_song_ids(&self) -> Result<Vec<String>, StoreError> {
        self.read(|data| {
            let mut seen = HashSet::new();
            data.playlist_maps
                .iter()
                .map(|m| &m.song_id)
                .filter(|id| !data.songs.contains_key(*id) && seen.insert(*id))
                .cloned()
                .collect()
        })
    }

    fn fetch_artist(&self, id: &str) -> Result<Option<SyncArtist>, StoreError> {
        self.read(|data| data.artists.get(id).cloned())
    }

    fn save_artist(&self, artist: &SyncArtist) -> Result<(), StoreError> {
        self.write(|data| {
            data.artists.insert(artist.id.clone(), artist.clone());
        })
    }

    fn fetch_album(&self, id: &str) -> Result<Option<SyncAlbum>, StoreError> {
        self.read(|data| data.albums.get(id).cloned())
    }

    fn save_album(&self, album: &SyncAlbum) -> Result<(), StoreError> {
        self.write(|data| {
            data.albums.insert(album.id.clone(), album.clone());
        })
    }

    fn fetch_podcast(&self, id: &str) -> Result<Option<SyncPodcast>, StoreError> {
        self.read(|data| data.podcasts.get(id).cloned())
    }

    fn save_podcast(&self, podcast: &SyncPodcast) -> Result<(), StoreError> {
        self.write(|data| {
            data.podcasts.insert(podcast.id.clone(), podcast.clone());
        })
    }
}
