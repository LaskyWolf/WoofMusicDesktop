// SwiftyTube/Sync/LibraryParsedModels.swift.

/// A song row from a library shelf.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ParsedSong {
    pub video_id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub artist_ids: Vec<String>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub duration: i64,
    pub thumbnail_url: Option<String>,
    pub is_liked: bool,
    pub library_add_token: Option<String>,
    pub library_remove_token: Option<String>,
}

/// A saved album row.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ParsedAlbum {
    pub browse_id: String,
    pub title: String,
    pub artist: Option<String>,
    pub thumbnail_url: Option<String>,
    pub song_count: i64,
    pub duration: i64,
    pub playlist_id: Option<String>,
}

/// A subscribed artist row.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ParsedArtist {
    pub browse_id: String,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub is_subscribed: bool,
    pub channel_id: Option<String>,
}

/// A liked playlist row.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ParsedPlaylist {
    pub browse_id: String,
    pub title: String,
    pub song_count: Option<i64>,
    pub thumbnail_url: Option<String>,
}

/// A subscribed podcast row.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ParsedPodcast {
    pub browse_id: String,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub is_subscribed: bool,
}
