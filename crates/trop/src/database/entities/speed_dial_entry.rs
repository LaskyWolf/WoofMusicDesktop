use std::time::SystemTime;

use rusqlite::Row;
use rusqlite::types::Value;

use crate::database::{Record, date_value, get_date};
use crate::networking::models::yt_item::{SongItem, YTArtist};

/// A song pinned to the homepage Speed Dial row.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SpeedDialEntry {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub thumbnail_url: Option<String>,
    pub duration: i64,
    pub pinned_at: SystemTime,
}

impl SpeedDialEntry {
    pub fn new(song: &SongItem) -> Self {
        Self {
            video_id: song.video_id.clone(),
            title: song.title.clone(),
            artist: song
                .artists
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            album: song.album.clone(),
            thumbnail_url: song.thumbnail_url.clone(),
            duration: song.duration,
            pinned_at: SystemTime::now(),
        }
    }

    pub fn to_song_item(&self) -> SongItem {
        SongItem {
            video_id: self.video_id.clone(),
            title: self.title.clone(),
            artists: vec![YTArtist {
                name: self.artist.clone(),
                id: None,
            }],
            album: self.album.clone(),
            duration: self.duration,
            thumbnail_url: self.thumbnail_url.clone(),
            ..SongItem::default()
        }
    }
}

impl Record for SpeedDialEntry {
    const TABLE: &'static str = "speed_dial";
    const COLUMNS: &'static [&'static str] = &[
        "video_id",
        "title",
        "artist",
        "album",
        "thumbnail_url",
        "duration",
        "pinned_at",
    ];
    const PRIMARY_KEY: &'static str = "video_id";

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            video_id: row.get("video_id")?,
            title: row.get("title")?,
            artist: row.get("artist")?,
            album: row.get("album")?,
            thumbnail_url: row.get("thumbnail_url")?,
            duration: row.get("duration")?,
            pinned_at: get_date(row, "pinned_at")?,
        })
    }

    fn values(&self) -> Vec<Value> {
        vec![
            self.video_id.clone().into(),
            self.title.clone().into(),
            self.artist.clone().into(),
            self.album.clone().into(),
            self.thumbnail_url.clone().into(),
            self.duration.into(),
            date_value(Some(self.pinned_at)),
        ]
    }
}
