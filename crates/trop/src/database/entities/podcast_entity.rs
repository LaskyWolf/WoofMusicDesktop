use std::time::SystemTime;

use rusqlite::Row;
use rusqlite::types::Value;

use crate::database::{Record, date_value, get_opt_date};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PodcastEntity {
    pub id: String,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub subscribed_at: Option<SystemTime>,
}

impl Record for PodcastEntity {
    const TABLE: &'static str = "podcast";
    const COLUMNS: &'static [&'static str] = &["id", "name", "thumbnail_url", "subscribed_at"];

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
            thumbnail_url: row.get("thumbnail_url")?,
            subscribed_at: get_opt_date(row, "subscribed_at")?,
        })
    }

    fn values(&self) -> Vec<Value> {
        vec![
            self.id.clone().into(),
            self.name.clone().into(),
            self.thumbnail_url.clone().into(),
            date_value(self.subscribed_at),
        ]
    }
}
