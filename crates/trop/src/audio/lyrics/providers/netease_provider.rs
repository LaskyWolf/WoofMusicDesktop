//! Trop/Audio/Lyrics/Providers/NeteaseProvider.swift.
//! https://github.com/spicetify/cli/blob/main/CustomApps/lyrics-plus/ProviderNetease.js
//! yrc word timing: https://github.com/chenmozhijin/LDDC/blob/main/LDDC/core/parser/yrc.py

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use swiftytube::networking::http_client::Session;
use swiftytube::networking::inner_tube_decode::JsonDict;

use super::lrclib_provider::LyricsError;
use super::lyrics_http::LyricsHTTP;
use super::lyrics_provider::{LyricsFuture, LyricsParsing, LyricsProvider, LyricsQuery};
use crate::audio::lyrics::lyrics_service::LyricLine;

static PARENS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\(.*?\)\s*").unwrap());
static BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\[.*?\]\s*").unwrap());

pub struct NeteaseProvider;

impl NeteaseProvider {
    // Netease's own public web API; needs no login.
    const SEARCH_URL: &str = "https://music.163.com/api/search/get";
    const LYRIC_URL: &str = "https://music.163.com/api/song/lyric";

    async fn fetch_lines(
        query: &LyricsQuery,
        session: &Session,
    ) -> Result<Vec<LyricLine>, LyricsError> {
        let clean_title = Self::normalize(&query.title);
        let keyword = format!("{clean_title} {}", query.artist);
        let request = LyricsHTTP::request(
            Self::SEARCH_URL,
            &[("limit", "10"), ("type", "1"), ("s", &keyword)],
            &[],
        )?;
        let json = LyricsHTTP::get_json(request, session).await?;
        let songs: Vec<&JsonDict> = json
            .get("result")
            .and_then(|r| r.get("songs"))
            .and_then(Value::as_array)
            .map(|songs| songs.iter().filter_map(Value::as_object).collect())
            .unwrap_or_default();
        if songs.is_empty() {
            return Err(LyricsError::NotFound);
        }
        let song_id =
            Self::best_song_id(&songs, query, &clean_title).ok_or(LyricsError::NotFound)?;
        Self::fetch_lyrics(&song_id, session).await
    }

    /// Scores every candidate and picks the best; `None` for a weak match.
    fn best_song_id(songs: &[&JsonDict], query: &LyricsQuery, clean_title: &str) -> Option<String> {
        let expected_album = Self::normalize(query.album.as_deref().unwrap_or(""));
        let dur = (query.duration * 1000.0) as i64;
        let query_artist = Self::normalize(&query.artist);

        let mut best_index = 0;
        let mut best_score = f64::NEG_INFINITY;
        for (i, song) in songs.iter().enumerate() {
            let name = Self::normalize(song.get("name").and_then(Value::as_str).unwrap_or(""));
            let artists: Vec<String> = song
                .get("artists")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|a| a.get("name").and_then(Value::as_str).map(Self::normalize))
                .collect();
            let album = Self::normalize(
                song.get("album")
                    .and_then(|a| a.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            );
            let sdur = song.get("duration").and_then(Value::as_i64).unwrap_or(0);

            let mut score = 0.0;
            if name == clean_title {
                score += 100.0;
            } else if name.contains(clean_title) || clean_title.contains(&name) {
                score += 60.0;
            } else {
                score -= 30.0;
            }
            if !query_artist.is_empty()
                && artists
                    .iter()
                    .any(|a| a.contains(&query_artist) || query_artist.contains(a.as_str()))
            {
                score += 80.0;
            } else if !query_artist.is_empty() {
                score -= 20.0;
            }
            if !expected_album.is_empty() && album == expected_album {
                score += 40.0;
            }
            if dur > 0 && sdur > 0 {
                score += (30.0 - (sdur - dur).abs() as f64 / 1000.0).max(0.0);
            }
            if score > best_score {
                best_score = score;
                best_index = i;
            }
        }

        if best_score <= 0.0 {
            return None;
        }
        match songs[best_index].get("id")? {
            Value::String(id) => Some(id.clone()),
            id => Some(id.to_string()),
        }
    }

    async fn fetch_lyrics(song_id: &str, session: &Session) -> Result<Vec<LyricLine>, LyricsError> {
        // -1 asks for each lyric kind: lrc, klyric, tlyric and yrc (word-timed).
        let request = LyricsHTTP::request(
            Self::LYRIC_URL,
            &[
                ("lv", "-1"),
                ("kv", "-1"),
                ("tv", "-1"),
                ("yv", "-1"),
                ("id", song_id),
            ],
            &[],
        )?;
        let json = LyricsHTTP::get_json(request, session).await?;
        let lyric = |key: &str| {
            json.get(key)
                .and_then(|k| k.get("lyric"))
                .and_then(Value::as_str)
                .filter(|l| !l.is_empty())
        };
        // Word-timed lyrics: the current yrc format, then the legacy klyric.
        // Then synced lyrics (LRC), then the translation as a last resort.
        let parse = |key: &str| {
            let raw = lyric(key)?;
            let lines = match key {
                "yrc" => LyricsParsing::parse_word_timed_lines(raw, false),
                "klyric" => LyricsParsing::parse_klyric(raw),
                _ => LyricsParsing::parse_lrc(raw),
            };
            (!lines.is_empty()).then_some(lines)
        };
        ["yrc", "klyric", "lrc", "tlyric"]
            .into_iter()
            .find_map(parse)
            .ok_or(LyricsError::NotFound)
    }

    /// Lowercase, trim, strip common extra info.
    fn normalize(text: &str) -> String {
        let lower = text.to_lowercase();
        let s = PARENS.replace_all(lower.trim(), " ");
        BRACKETS.replace_all(&s, " ").trim().to_string()
    }
}

impl LyricsProvider for NeteaseProvider {
    fn id(&self) -> &'static str {
        "netease"
    }

    fn name(&self) -> &'static str {
        "Netease"
    }

    fn fetch<'a>(&'a self, query: &'a LyricsQuery, session: &'a Session) -> LyricsFuture<'a> {
        Box::pin(Self::fetch_lines(query, session))
    }
}
