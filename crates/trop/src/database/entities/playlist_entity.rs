use std::time::SystemTime;

use rusqlite::Row;
use rusqlite::types::Value;

use crate::database::{Record, date_value, get_opt_date};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlaylistEntity {
    pub id: String,
    pub browse_id: Option<String>,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub is_editable: bool,
    pub bookmarked_at: Option<SystemTime>,
    pub remote_song_count: Option<i64>,
    pub is_auto_sync: bool,
}

impl PlaylistEntity {
    /// YouTube Music's built-in "Episodes for Later" list ("SE", or "VLSE" as a
    /// browse id). Trop has no use for it, so no playlist list shows it.
    pub const HIDDEN_IDS: [&'static str; 2] = ["SE", "VLSE"];

    /// Merges fetched playlist metadata over the stored row, preserving the
    /// user's name/editability/bookmark/thumbnail. Used when materializing a
    /// playlist's contents (sync + detail); the liked-playlists sync keeps
    /// its own policy (name/thumbnail always refresh, bookmark defaults
    /// to now). `is_auto_sync` resets to false, as in Swift.
    pub fn merging(
        existing: Option<&PlaylistEntity>,
        id: &str,
        browse_id: Option<String>,
        name: &str,
        remote_song_count: i64,
        thumbnail_url: Option<String>,
    ) -> PlaylistEntity {
        PlaylistEntity {
            id: id.to_string(),
            browse_id,
            name: existing.map_or_else(|| name.to_string(), |e| e.name.clone()),
            thumbnail_url: existing
                .and_then(|e| e.thumbnail_url.clone())
                .or(thumbnail_url),
            is_editable: existing.is_some_and(|e| e.is_editable),
            bookmarked_at: existing.and_then(|e| e.bookmarked_at),
            remote_song_count: Some(remote_song_count),
            is_auto_sync: false,
        }
    }
}

impl Record for PlaylistEntity {
    const TABLE: &'static str = "playlist";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "browse_id",
        "name",
        "thumbnail_url",
        "is_editable",
        "bookmarked_at",
        "remote_song_count",
        "is_auto_sync",
    ];

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            browse_id: row.get("browse_id")?,
            name: row.get("name")?,
            thumbnail_url: row.get("thumbnail_url")?,
            is_editable: row.get("is_editable")?,
            bookmarked_at: get_opt_date(row, "bookmarked_at")?,
            remote_song_count: row.get("remote_song_count")?,
            is_auto_sync: row.get("is_auto_sync")?,
        })
    }

    fn values(&self) -> Vec<Value> {
        vec![
            self.id.clone().into(),
            self.browse_id.clone().into(),
            self.name.clone().into(),
            self.thumbnail_url.clone().into(),
            self.is_editable.into(),
            date_value(self.bookmarked_at),
            self.remote_song_count.into(),
            self.is_auto_sync.into(),
        ]
    }
}
