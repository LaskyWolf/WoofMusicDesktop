use std::time::SystemTime;

use rusqlite::Row;
use rusqlite::types::Value;

use crate::database::{Record, date_value, get_date};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Event {
    /// `None` until inserted; SQLite assigns it.
    pub id: Option<i64>,
    pub song_id: String,
    pub timestamp: SystemTime,
    pub play_time: i64,
}

impl Record for Event {
    const TABLE: &'static str = "event";
    const COLUMNS: &'static [&'static str] = &["id", "song_id", "timestamp", "play_time"];

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            song_id: row.get("song_id")?,
            timestamp: get_date(row, "timestamp")?,
            play_time: row.get("play_time")?,
        })
    }

    fn values(&self) -> Vec<Value> {
        vec![
            self.id.into(),
            self.song_id.clone().into(),
            date_value(Some(self.timestamp)),
            self.play_time.into(),
        ]
    }
}
