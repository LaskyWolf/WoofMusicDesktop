use std::time::SystemTime;

use rusqlite::Row;
use rusqlite::types::Value;

use crate::database::{Record, date_value, get_opt_date};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArtistEntity {
    pub id: String,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub bookmarked_at: Option<SystemTime>,
    pub is_podcast_channel: bool,
    pub channel_id: Option<String>,
}

impl Record for ArtistEntity {
    const TABLE: &'static str = "artist";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "name",
        "thumbnail_url",
        "bookmarked_at",
        "is_podcast_channel",
        "channel_id",
    ];

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            thumbnail_url: row.get("thumbnail_url")?,
            bookmarked_at: get_opt_date(row, "bookmarked_at")?,
            is_podcast_channel: row.get("is_podcast_channel")?,
            channel_id: row.get("channel_id")?,
        })
    }

    fn values(&self) -> Vec<Value> {
        vec![
            self.id.clone().into(),
            self.name.clone().into(),
            self.thumbnail_url.clone().into(),
            date_value(self.bookmarked_at),
            self.is_podcast_channel.into(),
            self.channel_id.clone().into(),
        ]
    }
}
