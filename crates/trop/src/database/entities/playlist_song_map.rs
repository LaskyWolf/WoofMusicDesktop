use rusqlite::Row;
use rusqlite::types::Value;

use crate::database::Record;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlaylistSongMap {
    /// `None` until inserted; SQLite assigns it.
    pub id: Option<i64>,
    pub playlist_id: String,
    pub song_id: String,
    pub position: i64,
    pub set_video_id: Option<String>,
}

impl Record for PlaylistSongMap {
    const TABLE: &'static str = "playlist_song_map";
    const COLUMNS: &'static [&'static str] =
        &["id", "playlist_id", "song_id", "position", "set_video_id"];

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            playlist_id: row.get("playlist_id")?,
            song_id: row.get("song_id")?,
            position: row.get("position")?,
            set_video_id: row.get("set_video_id")?,
        })
    }

    fn values(&self) -> Vec<Value> {
        vec![
            self.id.into(),
            self.playlist_id.clone().into(),
            self.song_id.clone().into(),
            self.position.into(),
            self.set_video_id.clone().into(),
        ]
    }
}
