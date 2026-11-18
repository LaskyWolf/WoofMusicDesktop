use std::time::SystemTime;

use rusqlite::Row;
use rusqlite::types::Value;

use crate::database::{Record, date_value, get_opt_date};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AlbumEntity {
    pub id: String,
    pub title: String,
    pub playlist_id: Option<String>,
    pub thumbnail_url: Option<String>,
    pub song_count: i64,
    pub duration: i64,
    pub bookmarked_at: Option<SystemTime>,
    pub is_uploaded: bool,
}

impl Record for AlbumEntity {
    const TABLE: &'static str = "album";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "title",
        "playlist_id",
        "thumbnail_url",
        "song_count",
        "duration",
        "bookmarked_at",
        "is_uploaded",
    ];

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            playlist_id: row.get("playlist_id")?,
            thumbnail_url: row.get("thumbnail_url")?,
            song_count: row.get("song_count")?,
            duration: row.get("duration")?,
            bookmarked_at: get_opt_date(row, "bookmarked_at")?,
            is_uploaded: row.get("is_uploaded")?,
        })
    }

    fn values(&self) -> Vec<Value> {
        vec![
            self.id.clone().into(),
            self.title.clone().into(),
            self.playlist_id.clone().into(),
            self.thumbnail_url.clone().into(),
            self.song_count.into(),
            self.duration.into(),
            date_value(self.bookmarked_at),
            self.is_uploaded.into(),
        ]
    }
}
