use std::time::SystemTime;

use rusqlite::Row;
use rusqlite::types::Value;

use crate::database::{Record, date_value, get_date, get_opt_date};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SongEntity {
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
    pub is_unavailable: bool,
}

impl SongEntity {
    /// Merges freshly fetched metadata over the stored row, preserving user
    /// state (liked, play time, library flags, tokens, original create_date).
    /// Unknown incoming values (zero duration, missing artwork) fall back to
    /// the stored row instead of wiping it.
    pub fn merging(
        existing: Option<&SongEntity>,
        id: &str,
        title: &str,
        artist_name: Option<String>,
        album_name: Option<String>,
        duration: i64,
        thumbnail_url: Option<String>,
    ) -> SongEntity {
        let now = SystemTime::now();
        SongEntity {
            id: id.to_string(),
            title: title.to_string(),
            artist_name: existing.and_then(|e| e.artist_name.clone()).or(artist_name),
            album_name: existing.and_then(|e| e.album_name.clone()).or(album_name),
            duration: if duration > 0 {
                duration
            } else {
                existing.map_or(0, |e| e.duration)
            },
            thumbnail_url: thumbnail_url.or_else(|| existing.and_then(|e| e.thumbnail_url.clone())),
            liked: existing.is_some_and(|e| e.liked),
            total_play_time: existing.map_or(0, |e| e.total_play_time),
            in_library: existing.and_then(|e| e.in_library),
            library_add_token: existing
                .map(|e| e.library_add_token.clone())
                .unwrap_or_default(),
            library_remove_token: existing
                .map(|e| e.library_remove_token.clone())
                .unwrap_or_default(),
            is_episode: existing.is_some_and(|e| e.is_episode),
            is_uploaded: existing.is_some_and(|e| e.is_uploaded),
            is_video: existing.is_some_and(|e| e.is_video),
            create_date: existing.map_or(now, |e| e.create_date),
            modify_date: now,
            is_unavailable: existing.is_some_and(|e| e.is_unavailable),
        }
    }
}

impl Record for SongEntity {
    const TABLE: &'static str = "song";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "title",
        "artist_name",
        "album_name",
        "duration",
        "thumbnail_url",
        "liked",
        "total_play_time",
        "in_library",
        "library_add_token",
        "library_remove_token",
        "is_episode",
        "is_uploaded",
        "is_video",
        "create_date",
        "modify_date",
        "is_unavailable",
    ];

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            artist_name: row.get("artist_name")?,
            album_name: row.get("album_name")?,
            duration: row.get("duration")?,
            thumbnail_url: row.get("thumbnail_url")?,
            liked: row.get("liked")?,
            total_play_time: row.get("total_play_time")?,
            in_library: get_opt_date(row, "in_library")?,
            library_add_token: row.get("library_add_token")?,
            library_remove_token: row.get("library_remove_token")?,
            is_episode: row.get("is_episode")?,
            is_uploaded: row.get("is_uploaded")?,
            is_video: row.get("is_video")?,
            create_date: get_date(row, "create_date")?,
            modify_date: get_date(row, "modify_date")?,
            is_unavailable: row.get("is_unavailable")?,
        })
    }

    fn values(&self) -> Vec<Value> {
        vec![
            self.id.clone().into(),
            self.title.clone().into(),
            self.artist_name.clone().into(),
            self.album_name.clone().into(),
            self.duration.into(),
            self.thumbnail_url.clone().into(),
            self.liked.into(),
            self.total_play_time.into(),
            date_value(self.in_library),
            self.library_add_token.clone().into(),
            self.library_remove_token.clone().into(),
            self.is_episode.into(),
            self.is_uploaded.into(),
            self.is_video.into(),
            date_value(Some(self.create_date)),
            date_value(Some(self.modify_date)),
            self.is_unavailable.into(),
        ]
    }
}
