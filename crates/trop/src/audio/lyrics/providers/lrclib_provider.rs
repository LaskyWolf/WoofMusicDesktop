//! Trop/Audio/Lyrics/Providers/LRCLIBProvider.swift.

use std::fmt;

use serde_json::Value;
use swiftytube::networking::http_client::{HttpError, Session};

use super::lyrics_http::{LyricsHTTP, LyricsText, as_bool};
use super::lyrics_provider::{LyricsFuture, LyricsParsing, LyricsProvider, LyricsQuery};
use crate::audio::lyrics::lyrics_service::LyricLine;

pub struct LRCLIBProvider;

impl LRCLIBProvider {
    const BASE_URL: &str = "https://lrclib.net/api/get";

    async fn fetch_lines(
        query: &LyricsQuery,
        session: &Session,
    ) -> Result<Vec<LyricLine>, LyricsError> {
        let duration = query.duration_seconds().to_string();
        let mut items = vec![
            ("track_name", query.title.as_str()),
            ("artist_name", query.artist.as_str()),
            ("album_name", query.album.as_deref().unwrap_or("")),
        ];
        if query.duration_seconds() > 0 {
            items.push(("duration", &duration));
        }
        let json = LyricsHTTP::get_json(LyricsHTTP::request(Self::BASE_URL, &items, &[])?, session)
            .await?;

        if as_bool(json.get("instrumental")) == Some(true) {
            return Ok(vec![LyricLine::new("♪ Instrumental ♪", None)]);
        }
        if let Some(synced) = json.get("syncedLyrics").and_then(Value::as_str) {
            let lines = LyricsParsing::parse_lrc(synced);
            if !lines.is_empty() {
                return Ok(lines);
            }
        }
        if let Some(plain) = json.get("plainLyrics").and_then(Value::as_str) {
            let lines = LyricsText::plain_lines(plain);
            if !lines.is_empty() {
                return Ok(lines);
            }
        }
        Err(LyricsError::NotFound)
    }
}

impl LyricsProvider for LRCLIBProvider {
    fn id(&self) -> &'static str {
        "lrclib"
    }

    fn name(&self) -> &'static str {
        "LRCLIB"
    }

    fn fetch<'a>(&'a self, query: &'a LyricsQuery, session: &'a Session) -> LyricsFuture<'a> {
        Box::pin(Self::fetch_lines(query, session))
    }
}

/// Swift `LyricsError` plus the errors Swift throws as other types: a
/// provider that could not be reached (`Transport`, `TimedOut`) is not an
/// answer, so the next fetch tries it again. Swift's unused `requestFailed`
/// has no Rust case.
#[derive(Debug)]
pub enum LyricsError {
    InvalidUrl,
    NotFound,
    DecodingFailed,
    /// Swift `URLError` from `URLSession`.
    Transport(HttpError),
    /// Swift `URLError(.timedOut)` from `LyricsManager.withTimeout`.
    TimedOut,
}

impl LyricsError {
    /// Swift `error is LyricsError`: the provider answered, without lyrics.
    pub fn is_answer(&self) -> bool {
        !matches!(self, Self::Transport(_) | Self::TimedOut)
    }
}

impl From<HttpError> for LyricsError {
    fn from(error: HttpError) -> Self {
        Self::Transport(error)
    }
}

impl fmt::Display for LyricsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl => f.write_str("Invalid lyrics request URL"),
            Self::NotFound => f.write_str("Lyrics not found"),
            Self::DecodingFailed => f.write_str("Failed to decode lyrics response"),
            Self::Transport(error) => write!(f, "Lyrics request failed: {error}"),
            Self::TimedOut => f.write_str("Lyrics request timed out"),
        }
    }
}

impl std::error::Error for LyricsError {}
