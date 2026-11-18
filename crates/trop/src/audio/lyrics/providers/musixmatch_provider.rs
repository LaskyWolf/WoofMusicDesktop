//! Trop/Audio/Lyrics/Providers/MusixmatchProvider.swift.
//! https://github.com/spicetify/cli/blob/main/CustomApps/lyrics-plus/ProviderMusixmatch.js

use std::sync::Mutex;

use serde_json::Value;
use swiftytube::networking::http_client::Session;
use swiftytube::networking::inner_tube_decode::JsonDict;

use super::lrclib_provider::LyricsError;
use super::lyrics_http::{LyricsHTTP, LyricsText, as_bool};
use super::lyrics_provider::{LyricsFuture, LyricsParsing, LyricsProvider, LyricsQuery};
use crate::audio::lyrics::lyrics_service::LyricLine;

/// Guest token, fetched once and reused across requests.
static GUEST_TOKEN: Mutex<Option<String>> = Mutex::new(None);

const HEADERS: [(&str, &str); 8] = [
    ("Host", "apic-appmobile.musixmatch.com"),
    ("authority", "apic-appmobile.musixmatch.com"),
    ("X-Cookie", "x-mxm-token-guid="),
    ("x-mxm-app-version", "10.1.1"),
    (
        "X-User-Agent",
        "Musixmatch/2025120901 CFNetwork/3860.300.31 Darwin/25.2.0",
    ),
    ("Accept-Language", "en-US,en;q=0.9"),
    ("Connection", "keep-alive"),
    ("Accept", "application/json"),
];

/// `json[key]` as an object.
fn dict<'a>(json: &'a JsonDict, key: &str) -> Option<&'a JsonDict> {
    json.get(key)?.as_object()
}

/// `call.message`, when `message.header.status_code` is 200.
fn ok_message(call: &JsonDict) -> Option<&JsonDict> {
    let message = dict(call, "message")?;
    let status = dict(message, "header")?.get("status_code")?.as_i64()?;
    (status == 200).then_some(message)
}

pub struct MusixmatchProvider;

impl MusixmatchProvider {
    const BASE_URL: &str = "https://apic-appmobile.musixmatch.com/ws/1.1/macro.subtitles.get";
    const TOKEN_URL: &str =
        "https://apic-appmobile.musixmatch.com/ws/1.1/token.get?app_id=mac-ios-v2.0&";

    async fn fetch_lines(
        query: &LyricsQuery,
        session: &Session,
    ) -> Result<Vec<LyricLine>, LyricsError> {
        let token = Self::fetch_token(session).await?;
        let dur = query.duration_seconds().to_string();
        let items = [
            ("format", "json"),
            ("namespace", "lyrics_richsynched"),
            ("subtitle_format", "mxm"),
            ("app_id", "mac-ios-v2.0"),
            ("q_track", &query.title),
            ("q_artist", &query.artist),
            ("q_artists", &query.artist),
            ("q_album", query.album.as_deref().unwrap_or("")),
            ("q_duration", &dur),
            ("f_subtitle_length", &dur),
            ("usertoken", &token),
            // Adds word timing ("track.richsync.get") to the same macro response.
            ("optional_calls", "track.richsync"),
            ("richsync_compact_type", "words"),
            (
                "part",
                "track_lyrics_translation_status,track_structure,track_performer_tagging",
            ),
        ];
        let request = LyricsHTTP::request(Self::BASE_URL, &items, &HEADERS)?;
        Self::parse(&LyricsHTTP::get_json(request, session).await?)
    }

    async fn fetch_token(session: &Session) -> Result<String, LyricsError> {
        if let Some(cached) = GUEST_TOKEN.lock().unwrap().clone() {
            return Ok(cached);
        }
        let json = LyricsHTTP::get_json(
            LyricsHTTP::request(Self::TOKEN_URL, &[], &HEADERS)?,
            session,
        )
        .await?;
        let token = dict(&json, "message")
            .and_then(|m| dict(m, "body"))
            .and_then(|b| b.get("user_token"))
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .ok_or(LyricsError::NotFound)?
            .to_string();
        *GUEST_TOKEN.lock().unwrap() = Some(token.clone());
        Ok(token)
    }

    fn parse(json: &JsonDict) -> Result<Vec<LyricLine>, LyricsError> {
        let macro_calls = dict(json, "message")
            .and_then(|m| dict(m, "body"))
            .and_then(|b| dict(b, "macro_calls"))
            .ok_or(LyricsError::NotFound)?;

        // The matcher must answer 200.
        let matcher = dict(macro_calls, "matcher.track.get")
            .and_then(ok_message)
            .ok_or(LyricsError::NotFound)?;
        let track = dict(matcher, "body").and_then(|b| dict(b, "track"));
        if as_bool(track.and_then(|t| t.get("instrumental"))) == Some(true) {
            return Ok(vec![LyricLine::new("♪ Instrumental ♪", None)]);
        }

        let lyrics = dict(macro_calls, "track.lyrics.get")
            .and_then(|c| dict(c, "message"))
            .and_then(|m| dict(m, "body"))
            .and_then(|b| dict(b, "lyrics"));
        // Restricted lyrics cannot be shown.
        if as_bool(lyrics.and_then(|l| l.get("restricted"))) == Some(true) {
            return Err(LyricsError::NotFound);
        }

        // Word-synced.
        let richsync = dict(macro_calls, "track.richsync.get")
            .and_then(ok_message)
            .and_then(|m| dict(m, "body"))
            .and_then(|b| dict(b, "richsync"))
            .and_then(|r| r.get("richsync_body"))
            .and_then(Value::as_str);
        if let Some(body) = richsync {
            let lines = LyricsParsing::parse_richsync(body);
            if !lines.is_empty() {
                return Ok(lines);
            }
        }

        // Synced.
        let subtitle = dict(macro_calls, "track.subtitles.get")
            .and_then(|c| dict(c, "message"))
            .and_then(|m| dict(m, "body"))
            .and_then(|b| b.get("subtitle_list"))
            .and_then(Value::as_array)
            .and_then(|list| list.first())
            .and_then(|first| first.get("subtitle"))
            .and_then(|s| s.get("subtitle_body"))
            .and_then(Value::as_str)
            .and_then(|body| serde_json::from_str::<Value>(body).ok());
        if let Some(Value::Array(items)) = subtitle {
            let lines: Vec<LyricLine> = items
                .iter()
                .filter_map(|item| {
                    let text = item.get("text")?.as_str()?;
                    if text.trim().is_empty() {
                        return None;
                    }
                    let time = item
                        .get("time")
                        .and_then(|t| t.get("total"))
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    Some(LyricLine::new(text, (time > 0.0).then_some(time)))
                })
                .collect();
            if !lines.is_empty() {
                return Ok(lines);
            }
        }

        // Unsynced.
        if let Some(body) = lyrics
            .and_then(|l| l.get("lyrics_body"))
            .and_then(Value::as_str)
        {
            let lines = LyricsText::plain_lines(body);
            if !lines.is_empty() {
                return Ok(lines);
            }
        }
        Err(LyricsError::NotFound)
    }
}

impl LyricsProvider for MusixmatchProvider {
    fn id(&self) -> &'static str {
        "musixmatch"
    }

    fn name(&self) -> &'static str {
        "Musixmatch"
    }

    fn fetch<'a>(&'a self, query: &'a LyricsQuery, session: &'a Session) -> LyricsFuture<'a> {
        Box::pin(Self::fetch_lines(query, session))
    }
}
