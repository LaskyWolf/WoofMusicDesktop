use rusqlite::Row;
use rusqlite::types::Value;

use crate::database::Record;

/// The table's `saved_at` column is not part of the entity, as in Swift.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct EpisodeEntity {
    pub id: String,
    pub title: String,
    pub duration: i64,
    pub thumbnail_url: Option<String>,
    pub podcast_id: Option<String>,
    pub podcast_name: Option<String>,
    pub is_played: bool,
}

impl Record for EpisodeEntity {
    const TABLE: &'static str = "episode";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "title",
        "duration",
        "thumbnail_url",
        "podcast_id",
        "podcast_name",
        "is_played",
    ];

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            duration: row.get("duration")?,
            thumbnail_url: row.get("thumbnail_url")?,
            podcast_id: row.get("podcast_id")?,
            podcast_name: row.get("podcast_name")?,
            is_played: row.get("is_played")?,
        })
    }

    fn values(&self) -> Vec<Value> {
        vec![
            self.id.clone().into(),
            self.title.clone().into(),
            self.duration.into(),
            self.thumbnail_url.clone().into(),
            self.podcast_id.clone().into(),
            self.podcast_name.clone().into(),
            self.is_played.into(),
        ]
    }
}
