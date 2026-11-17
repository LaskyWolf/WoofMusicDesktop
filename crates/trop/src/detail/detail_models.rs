// `PlaylistHeaderCache` (UserDefaults-backed) lands with local storage (R5);
// `PlaylistHeaderSnapshot` and `apply` are here so the UI can use them.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::networking::models::yt_item::{
    AlbumItem, ArtistItem, EpisodeItem, PlaylistItem, SongItem, YTArtist,
};

/// Parsed detail data for an album page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AlbumDetailInfo {
    pub title: String,
    pub artists: Vec<YTArtist>,
    pub year: Option<i64>,
    pub song_count: i64,
    pub duration: i64,
    pub thumbnail_url: Option<String>,
    pub playlist_id: Option<String>,
    pub browse_id: String,
    pub songs: Vec<SongItem>,
}

/// Parsed detail data for an artist page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArtistDetailInfo {
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub subscriber_count_text: Option<String>,
    pub description_text: Option<String>,
    pub is_subscribed: bool,
    pub browse_id: String,
    pub songs: Vec<SongItem>,
    pub albums: Vec<AlbumItem>,
    pub singles: Vec<AlbumItem>,
    pub videos: Vec<SongItem>,
    pub playlists: Vec<PlaylistItem>,
    pub related_artists: Vec<ArtistItem>,
}

impl ArtistDetailInfo {
    /// False for a listener's channel: no albums or singles, and the songs
    /// shelf (a profile lists songs it uploaded or played) has none credited to it.
    pub fn has_music(&self) -> bool {
        !self.albums.is_empty()
            || !self.singles.is_empty()
            || self.songs.iter().any(|song| {
                song.artists.iter().any(|a| {
                    a.id.as_deref() == Some(self.browse_id.as_str()) || a.name == self.name
                })
            })
    }

    /// "@handle" from a profile's description ("Profile • @handle").
    pub fn handle(&self) -> Option<&str> {
        self.description_text
            .as_deref()
            .unwrap_or_default()
            .split(' ')
            .find(|word| word.starts_with('@') && word.len() > 1)
    }
}

/// Parsed detail data for a playlist page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistDetailInfo {
    pub title: String,
    pub author_name: Option<String>,
    pub author_browse_id: Option<String>,
    pub author_avatar_url: Option<String>,
    pub description_text: Option<String>,
    pub song_count: i64,
    pub duration: i64,
    pub thumbnail_url: Option<String>,
    pub playlist_id: String,
    pub songs: Vec<SongItem>,

    /// Avatars of the people on a shared playlist, owner first. Empty otherwise.
    pub collaborator_avatar_urls: Vec<String>,

    /// Token for the request that lists them by name.
    pub collaborators_panel_params: Option<String>,

    /// The signed-in account owns this playlist (YouTube sends an editable header).
    pub is_editable: bool,

    /// The header facepile opens the "PAplaylist_collaborate" panel: a shared
    /// playlist whose collaborators can add and remove songs without owning it.
    pub is_collaborative: bool,
}

impl PlaylistDetailInfo {
    pub fn apply(&mut self, snapshot: &PlaylistHeaderSnapshot) {
        self.is_collaborative = snapshot.is_collaborative;
        self.collaborator_avatar_urls = snapshot.collaborator_avatar_urls.clone();
        self.collaborators_panel_params = snapshot.collaborators_panel_params.clone();
        fill(&mut self.description_text, &snapshot.description_text);
        fill(&mut self.author_name, &snapshot.author_name);
        fill(&mut self.author_browse_id, &snapshot.author_browse_id);
        fill(&mut self.author_avatar_url, &snapshot.author_avatar_url);
    }
}

/// Swift `a = a ?? b`.
fn fill(target: &mut Option<String>, fallback: &Option<String>) {
    if target.is_none() {
        target.clone_from(fallback);
    }
}

/// Parsed detail data for a podcast page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PodcastDetailInfo {
    pub title: String,
    pub author: Option<String>,
    pub description_text: Option<String>,
    pub thumbnail_url: Option<String>,
    pub browse_id: String,
    pub is_subscribed: bool,
    pub episodes: Vec<EpisodeItem>,
}

/// Header fields of a playlist from its last network load. Lets the edit
/// buttons and the people chip show on reopen before the refresh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistHeaderSnapshot {
    pub is_collaborative: bool,
    pub collaborator_avatar_urls: Vec<String>,
    pub collaborators_panel_params: Option<String>,
    pub description_text: Option<String>,
    pub author_name: Option<String>,
    pub author_browse_id: Option<String>,
    pub author_avatar_url: Option<String>,

    /// Owner name and head count from the collaborators list, so a shared
    /// playlist's chip opens with its final label.
    pub owner_name: Option<String>,
    pub people_count: Option<i64>,
}

// ponytail: unbounded, one entry per artist opened this session; add an LRU cap if memory grows.
static ARTIST_CACHE: LazyLock<Mutex<HashMap<String, (ArtistDetailInfo, Instant)>>> =
    LazyLock::new(Mutex::default);

/// Artist pages from recent loads, so reopening one shows it at once.
/// Kept in memory only; entries older than `LIFETIME` still show but get refreshed.
pub struct ArtistDetailCache;

impl ArtistDetailCache {
    pub const LIFETIME: Duration = Duration::from_secs(10 * 60);

    /// The cached page and whether it is younger than `LIFETIME`.
    pub fn entry(browse_id: &str) -> Option<(ArtistDetailInfo, bool)> {
        let cache = ARTIST_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let (info, loaded_at) = cache.get(browse_id)?;
        Some((info.clone(), loaded_at.elapsed() < Self::LIFETIME))
    }

    pub fn store(info: ArtistDetailInfo) {
        let mut cache = ARTIST_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(info.browse_id.clone(), (info, Instant::now()));
    }

    /// Keeps the cached copy in step with a local change without resetting its age.
    pub fn update(info: ArtistDetailInfo) {
        let mut cache = ARTIST_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get_mut(&info.browse_id) {
            entry.0 = info;
        }
    }
}
