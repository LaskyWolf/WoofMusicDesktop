//! Trop/Audio/Lyrics/Providers/KugouProvider.swift.
//! https://github.com/MetrolistGroup/Metrolist/blob/main/kugou/src/main/kotlin/com/metrolist/kugou/KuGou.kt

use std::sync::LazyLock;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use regex::Regex;
use serde_json::Value;
use swiftytube::networking::http_client::Session;

use super::lrclib_provider::LyricsError;
use super::lyrics_http::LyricsHTTP;
use super::lyrics_provider::{LyricsFuture, LyricsParsing, LyricsProvider, LyricsQuery};
use crate::audio::lyrics::lyrics_service::LyricLine;

/// Greedy bracket pairs removed from the title, as in Swift.
static TITLE_BRACKETS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"\(.*\)",
        r"（.*）",
        r"「.*」",
        r"『.*』",
        r"<.*>",
        r"《.*》",
        r"〈.*〉",
        r"＜.*＞",
    ]
    .into_iter()
    .map(|p| Regex::new(p).unwrap())
    .collect()
});
static ARTIST_BRACKETS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [r"\(.*\)", r"（.*）"]
        .into_iter()
        .map(|p| Regex::new(p).unwrap())
        .collect()
});

struct Keyword {
    title: String,
    artist: String,
    album: Option<String>,
}

/// One `candidates` entry of a lyrics search.
struct Candidate {
    id: String,
    accesskey: String,
}

pub struct KugouProvider;

impl KugouProvider {
    /// Seconds.
    const DURATION_TOLERANCE: i64 = 8;
    const LYRICS_SEARCH_URL: &str = "https://lyrics.kugou.com/search";

    async fn fetch_lines(
        query: &LyricsQuery,
        session: &Session,
    ) -> Result<Vec<LyricLine>, LyricsError> {
        let keyword = Self::generate_keyword(&query.title, &query.artist, query.album.clone());
        let candidate = Self::get_lyrics_candidate(&keyword, query.duration_seconds(), session)
            .await?
            .ok_or(LyricsError::NotFound)?;

        // KRC carries word timing; plain LRC is the fallback.
        if let Ok(krc) = Self::download_lyrics(&candidate, "krc", session).await
            && let Some(text) = LyricsParsing::decode_krc(&krc)
        {
            let lines = LyricsParsing::parse_word_timed_lines(&text, true);
            if !lines.is_empty() {
                return Ok(lines);
            }
        }
        let data = Self::download_lyrics(&candidate, "lrc", session).await?;
        let raw = String::from_utf8(data).map_err(|_| LyricsError::DecodingFailed)?;
        let lines = LyricsParsing::parse_lrc(&raw);
        if lines.is_empty() {
            return Err(LyricsError::NotFound);
        }
        Ok(lines)
    }

    async fn get_lyrics_candidate(
        keyword: &Keyword,
        duration: i64,
        session: &Session,
    ) -> Result<Option<Candidate>, LyricsError> {
        // Try matching by song hash first.
        for (song_duration, hash) in Self::search_songs(keyword, session).await? {
            if duration != -1 && (song_duration - duration).abs() > Self::DURATION_TOLERANCE {
                continue;
            }
            let by_hash = Self::search_lyrics(&[("hash", &hash)], session).await?;
            if let Some(candidate) = by_hash {
                return Ok(Some(candidate));
            }
        }
        // Fall back to the keyword search.
        let query = Self::build_search_query(keyword);
        let duration_ms = (duration * 1000).to_string();
        let mut items = vec![("keyword", query.as_str())];
        if duration != -1 {
            items.push(("duration", &duration_ms));
        }
        Self::search_lyrics(&items, session).await
    }

    /// `(duration, hash)` of each found song.
    async fn search_songs(
        keyword: &Keyword,
        session: &Session,
    ) -> Result<Vec<(i64, String)>, LyricsError> {
        let query = Self::build_search_query(keyword);
        let request = LyricsHTTP::request(
            "https://mobileservice.kugou.com/api/v3/search/song",
            &[
                ("version", "9108"),
                ("plat", "0"),
                ("pagesize", "8"),
                ("showtype", "0"),
                ("keyword", &query),
            ],
            &[],
        )?;
        let json = LyricsHTTP::get_json(request, session).await?;
        let info = json
            .get("data")
            .and_then(|d| d.get("info"))
            .and_then(Value::as_array)
            .ok_or(LyricsError::DecodingFailed)?;
        Ok(info
            .iter()
            .filter_map(|song| {
                let duration = song.get("duration")?.as_i64()?;
                Some((duration, song.get("hash")?.as_str()?.to_string()))
            })
            .collect())
    }

    /// The first candidate of a lyrics search by `hash` or `keyword` (+ `duration`).
    async fn search_lyrics(
        items: &[(&str, &str)],
        session: &Session,
    ) -> Result<Option<Candidate>, LyricsError> {
        let mut query = vec![("ver", "1"), ("man", "yes"), ("client", "pc")];
        query.extend_from_slice(items);
        let request = LyricsHTTP::request(Self::LYRICS_SEARCH_URL, &query, &[])?;
        let json = LyricsHTTP::get_json(request, session).await?;
        let candidates = json
            .get("candidates")
            .and_then(Value::as_array)
            .ok_or(LyricsError::DecodingFailed)?;
        Ok(candidates.first().and_then(|c| {
            Some(Candidate {
                id: c.get("id")?.as_str()?.to_string(),
                accesskey: c.get("accesskey")?.as_str()?.to_string(),
            })
        }))
    }

    /// The base64-decoded `content` of a lyrics download in `format` ("lrc" or "krc").
    async fn download_lyrics(
        candidate: &Candidate,
        format: &str,
        session: &Session,
    ) -> Result<Vec<u8>, LyricsError> {
        let request = LyricsHTTP::request(
            "https://lyrics.kugou.com/download",
            &[
                ("fmt", format),
                ("charset", "utf8"),
                ("client", "pc"),
                ("ver", "1"),
                ("id", &candidate.id),
                ("accesskey", &candidate.accesskey),
            ],
            &[],
        )?;
        let json = LyricsHTTP::get_json(request, session).await?;
        json.get("content")
            .and_then(Value::as_str)
            .and_then(|content| STANDARD.decode(content).ok())
            .ok_or(LyricsError::DecodingFailed)
    }

    fn build_search_query(keyword: &Keyword) -> String {
        let mut q = format!("{} - {}", keyword.title, keyword.artist);
        if let Some(album) = keyword.album.as_deref().filter(|a| !a.is_empty()) {
            q += " ";
            q += album;
        }
        q
    }

    fn generate_keyword(title: &str, artist: &str, album: Option<String>) -> Keyword {
        Keyword {
            title: Self::normalize_title(title),
            artist: Self::normalize_artist(artist),
            album,
        }
    }

    fn normalize_title(title: &str) -> String {
        let mut s = title.to_string();
        for re in TITLE_BRACKETS.iter() {
            s = re.replace_all(&s, "").into_owned();
        }
        s.trim().to_string()
    }

    fn normalize_artist(artist: &str) -> String {
        let mut s = artist
            .replace(", ", "、")
            .replace(" & ", "、")
            .replace('.', "")
            .replace('和', "、");
        for re in ARTIST_BRACKETS.iter() {
            s = re.replace_all(&s, "").into_owned();
        }
        s.trim().to_string()
    }
}

impl LyricsProvider for KugouProvider {
    fn id(&self) -> &'static str {
        "kugou"
    }

    fn name(&self) -> &'static str {
        "KuGou"
    }

    fn fetch<'a>(&'a self, query: &'a LyricsQuery, session: &'a Session) -> LyricsFuture<'a> {
        Box::pin(Self::fetch_lines(query, session))
    }
}
