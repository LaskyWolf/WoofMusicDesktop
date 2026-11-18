use rusqlite::Row;
use rusqlite::types::Value;

use crate::database::Record;

#[derive(Clone, Debug, PartialEq)]
pub struct FormatEntity {
    pub id: String,
    pub itag: i64,
    pub mime_type: String,
    pub codecs: String,
    pub bitrate: i64,
    pub sample_rate: i64,
    pub content_length: i64,
    pub loudness_db: Option<f64>,
    pub perceptual_loudness_db: Option<f64>,
    pub playback_url: Option<String>,
}

impl Record for FormatEntity {
    const TABLE: &'static str = "format";
    const COLUMNS: &'static [&'static str] = &[
        "id",
        "itag",
        "mime_type",
        "codecs",
        "bitrate",
        "sample_rate",
        "content_length",
        "loudness_db",
        "perceptual_loudness_db",
        "playback_url",
    ];

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            itag: row.get("itag")?,
            mime_type: row.get("mime_type")?,
            codecs: row.get("codecs")?,
            bitrate: row.get("bitrate")?,
            sample_rate: row.get("sample_rate")?,
            content_length: row.get("content_length")?,
            loudness_db: row.get("loudness_db")?,
            perceptual_loudness_db: row.get("perceptual_loudness_db")?,
            playback_url: row.get("playback_url")?,
        })
    }

    fn values(&self) -> Vec<Value> {
        vec![
            self.id.clone().into(),
            self.itag.into(),
            self.mime_type.clone().into(),
            self.codecs.clone().into(),
            self.bitrate.into(),
            self.sample_rate.into(),
            self.content_length.into(),
            self.loudness_db.into(),
            self.perceptual_loudness_db.into(),
            self.playback_url.clone().into(),
        ]
    }
}
