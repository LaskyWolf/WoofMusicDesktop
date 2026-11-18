use std::time::SystemTime;

use rusqlite::Row;
use rusqlite::types::Value;

use crate::database::{Record, date_value, get_date};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DownloadedTrackEntity {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub duration: i64,
    pub thumbnail_url: Option<String>,
    pub local_path: String,
    pub downloaded_at: SystemTime,
}

impl Record for DownloadedTrackEntity {
    const TABLE: &'static str = "downloaded_track";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "title",
        "artist",
        "duration",
        "thumbnail_url",
        "local_path",
        "downloaded_at",
    ];

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            artist: row.get("artist")?,
            duration: row.get("duration")?,
            thumbnail_url: row.get("thumbnail_url")?,
            local_path: row.get("local_path")?,
            downloaded_at: get_date(row, "downloaded_at")?,
        })
    }

    fn values(&self) -> Vec<Value> {
        vec![
            self.id.clone().into(),
            self.title.clone().into(),
            self.artist.clone().into(),
            self.duration.into(),
            self.thumbnail_url.clone().into(),
            self.local_path.clone().into(),
            date_value(Some(self.downloaded_at)),
        ]
    }
}
