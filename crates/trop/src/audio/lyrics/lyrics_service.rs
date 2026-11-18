//! Trop/Audio/Lyrics/LyricsService.swift.
//!
//! Swift reads the song from `NowPlaying` and the queue; here the caller passes
//! a `LyricsSong`. `UserDefaults` becomes a JSON object file (`defaults_path`),
//! keyed like the Swift defaults. `LyricsState` is kept inside the service and
//! read with `state()`; the karaoke greeting fields are UI-only and not ported.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::Value;
use swiftytube::networking::inner_tube_decode::JsonDict;
use swiftytube::sync::incremental_sync_service::IncrementalSyncService;

use super::lyrics_manager::LyricsManager;
use super::providers::lrclib_provider::LyricsError;
use super::providers::lyrics_provider::{LyricsParsing, LyricsQuery};
use crate::networking::models::home_page::next_id;

#[derive(Clone, Debug, PartialEq)]
pub struct LyricLine {
    /// Swift `UUID()`: unique per instance in this process.
    pub id: u64,
    pub text: String,
    /// Seconds; `None` for an untimed line.
    pub start_time: Option<f64>,
    /// Word-level timing for karaoke; `None` when the source has only line timing.
    pub words: Option<Vec<LyricWord>>,
}

impl LyricLine {
    pub fn new(text: impl Into<String>, start_time: Option<f64>) -> Self {
        Self::with_words(text, start_time, None)
    }

    pub fn with_words(
        text: impl Into<String>,
        start_time: Option<f64>,
        words: Option<Vec<LyricWord>>,
    ) -> Self {
        Self {
            id: next_id(),
            text: text.into(),
            start_time,
            words,
        }
    }

    pub fn placeholder() -> Self {
        Self::new("♪", None)
    }
}

/// One timed piece of a line (a word or a syllable), in song seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct LyricWord {
    pub text: String,
    pub start_time: f64,
    /// `None` when the source gives no end (a last Enhanced LRC word without a closing tag).
    pub end_time: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct LyricsState {
    /// Songs with confirmed lyrics. Populated by preload/fetch.
    pub available_video_ids: HashSet<String>,
    /// The provider of each song's lyrics. Swift keeps one name, for the song playing.
    pub provider_names: HashMap<String, String>,
    /// Bumped when the lyrics of a song changed and the view should reload them.
    pub refresh_token: u64,
}

/// The song fields Swift `resolveQuery` reads from `NowPlaying` or the queue.
/// For a queue song Swift joins the artist names with ", " and falls back to
/// `DurationCache` when the duration is 0.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricsSong {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    /// `None` when empty.
    pub album: Option<String>,
    /// Seconds.
    pub duration: f64,
}

#[derive(Default)]
struct Inner {
    cache: HashMap<String, Vec<LyricLine>>,
    customs: HashMap<String, (Vec<LyricLine>, String)>,
    /// Held while a song's lyrics are fetched, so callers arriving meanwhile wait for that fetch.
    // ponytail: one entry per song id for the process lifetime; prune if sessions get long.
    in_flight: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
    /// Songs no provider had lyrics for, so reopening or preloading them skips
    /// the providers for a while. Entries expire because lyrics can be
    /// published later.
    not_found_until: HashMap<String, Instant>,
    /// The defaults file, loaded once and written through.
    defaults: JsonDict,
}

pub struct LyricsService {
    pub manager: LyricsManager,
    defaults_path: PathBuf,
    inner: Mutex<Inner>,
    state: Mutex<LyricsState>,
}

static BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]").unwrap());
static PARENS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\([^)]*\)").unwrap());
static CJK_BRACKETS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"【[^】]*】|「[^」]*」|『[^』]*』").unwrap());

impl LyricsService {
    const NOT_FOUND_TTL: Duration = Duration::from_secs(10 * 60);
    /// Defaults key of the karaoke toggle in the lyrics menu.
    pub const KARAOKE_ENABLED_KEY: &str = "settings.lyricsKaraoke";

    /// `defaults_path` stands in for `UserDefaults.standard`; a missing or
    /// unreadable file reads as empty.
    pub fn new(manager: LyricsManager, defaults_path: PathBuf) -> Self {
        let defaults = std::fs::read(&defaults_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            manager,
            defaults_path,
            inner: Mutex::new(Inner {
                defaults,
                ..Inner::default()
            }),
            state: Mutex::new(LyricsState::default()),
        }
    }

    /// `WoofMusic/lyrics_defaults.json` next to SwiftyTube's `sync_defaults.json`.
    pub fn default_defaults_path() -> Option<PathBuf> {
        IncrementalSyncService::default_defaults_path()
            .map(|p| p.with_file_name("lyrics_defaults.json"))
    }

    fn custom_key(video_id: &str) -> String {
        format!("lyrics.custom.{video_id}")
    }

    fn custom_provider_key(video_id: &str) -> String {
        format!("lyrics.customProvider.{video_id}")
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }

    /// A snapshot of the availability, provider names and refresh token.
    pub fn state(&self) -> LyricsState {
        self.state.lock().unwrap().clone()
    }

    pub async fn fetch_lyrics(
        self: &Arc<Self>,
        song: &LyricsSong,
    ) -> Result<Vec<LyricLine>, LyricsError> {
        let video_id = &song.video_id;
        let gate = self
            .inner()
            .in_flight
            .entry(video_id.clone())
            .or_default()
            .clone();
        // ponytail: a caller that waited retries after a network error instead of sharing it, as Swift's shared `Task` does.
        let _turn = gate.lock().await;

        if let Some((lines, provider)) = self.custom_entry(video_id) {
            self.update_availability(video_id, true, Some(provider));
            return Ok(lines);
        }
        let cached = self.inner().cache.get(video_id).cloned();
        if let Some(cached) = cached {
            self.update_availability(video_id, !cached.is_empty(), None);
            return Ok(cached);
        }
        let not_found = self
            .inner()
            .not_found_until
            .get(video_id)
            .is_some_and(|until| *until > Instant::now());
        if not_found {
            self.update_availability(video_id, false, None);
            return Err(LyricsError::NotFound);
        }
        // No query could be built for the song: not cached as not found.
        let Some(query) = Self::resolve_query(song) else {
            self.update_availability(video_id, false, None);
            return Err(LyricsError::NotFound);
        };
        match self.manager.fetch_lyrics_returning_provider(&query).await {
            Err(error) => {
                // `NotFound` means the providers answered; network errors are retried.
                if matches!(error, LyricsError::NotFound) {
                    self.inner()
                        .not_found_until
                        .insert(video_id.clone(), Instant::now() + Self::NOT_FOUND_TTL);
                }
                self.update_availability(video_id, false, None);
                Err(error)
            }
            Ok((lines, provider)) => {
                // Only successful lookups are cached: lyrics can be matched or
                // published after playback starts.
                if !lines.is_empty() {
                    self.inner().cache.insert(video_id.clone(), lines.clone());
                    self.upgrade_to_word_timing(video_id, query, &lines, Some(provider.clone()));
                }
                self.update_availability(video_id, !lines.is_empty(), Some(provider));
                Ok(lines)
            }
        }
    }

    // Manual manipulations (edit / refetch / search selection / copy).

    /// Raw text of the active lyrics for the editor.
    pub fn editable_text(&self, video_id: &str) -> String {
        if let Some((lines, _)) = self.custom_entry(video_id) {
            return LyricsParsing::serialize_lines(&lines);
        }
        self.inner()
            .cache
            .get(video_id)
            .map(|cached| LyricsParsing::serialize_lines(cached))
            .unwrap_or_default()
    }

    /// Plain text without timestamps for the clipboard.
    pub fn plain_text_for_copy(&self, video_id: &str) -> Option<String> {
        if let Some((lines, _)) = self.custom_entry(video_id) {
            return Some(LyricsParsing::plain_text(&lines));
        }
        self.inner()
            .cache
            .get(video_id)
            .map(|cached| LyricsParsing::plain_text(cached))
    }

    /// Stores edited or search-selected lyrics and signals the view to reload.
    /// Edits pass `None` and keep the existing provider; fresh selections pass their own.
    pub fn save_custom(&self, video_id: &str, raw_text: &str, provider_name: Option<&str>) {
        let lines = LyricsParsing::parse_stored_text(raw_text);
        if lines.is_empty() {
            return;
        }
        let provider = {
            let mut inner = self.inner();
            let provider = provider_name
                .map(str::to_string)
                .or_else(|| inner.customs.get(video_id).map(|c| c.1.clone()))
                .or_else(|| Self::string(&inner.defaults, &Self::custom_provider_key(video_id)))
                .unwrap_or_else(|| "Manual".to_string());
            inner
                .customs
                .insert(video_id.to_string(), (lines, provider.clone()));
            inner.cache.remove(video_id);
            provider
        };
        self.set_defaults([
            (Self::custom_key(video_id), raw_text.into()),
            (Self::custom_provider_key(video_id), provider.clone().into()),
        ]);
        let mut state = self.state.lock().unwrap();
        state.available_video_ids.insert(video_id.to_string());
        state.provider_names.insert(video_id.to_string(), provider);
        state.refresh_token += 1;
    }

    /// Re-fetches from providers; replaces any manual override with the fresh
    /// result (explicit user intent).
    pub async fn refetch(&self, song: &LyricsSong) {
        let video_id = &song.video_id;
        {
            let mut inner = self.inner();
            inner.cache.remove(video_id);
            inner.not_found_until.remove(video_id);
        }
        let found = match Self::resolve_query(song) {
            Some(query) => match self.manager.fetch_lyrics_returning_provider(&query).await {
                Ok((lines, provider)) if !lines.is_empty() => Some((query, lines, provider)),
                _ => None,
            },
            None => None,
        };
        let Some((query, mut lines, mut provider)) = found else {
            self.update_availability(video_id, false, None);
            self.state.lock().unwrap().refresh_token += 1;
            return;
        };
        // The result is stored as the song's own copy, which later fetches
        // never replace, so word timing for karaoke is looked up now.
        if self.wants_word_timing(&lines)
            && let Some((word_timed, word_provider)) =
                self.manager.fetch_word_timed(&query, Some(&provider)).await
        {
            lines = word_timed;
            provider = word_provider;
        }
        let raw = LyricsParsing::serialize_lines(&lines);
        {
            let mut inner = self.inner();
            inner.cache.insert(video_id.clone(), lines.clone());
            inner
                .customs
                .insert(video_id.clone(), (lines, provider.clone()));
        }
        self.set_defaults([
            (Self::custom_key(video_id), raw.into()),
            (Self::custom_provider_key(video_id), provider.clone().into()),
        ]);
        self.update_availability(video_id, true, Some(provider));
        self.state.lock().unwrap().refresh_token += 1;
    }

    /// Karaoke was turned on: looks for word timing for the provider lyrics
    /// on screen. A song's own copy (edited or picked) is left alone.
    pub fn request_word_timing(self: &Arc<Self>, song: &LyricsSong) {
        if self.custom_entry(&song.video_id).is_some() {
            return;
        }
        let shown = self.inner().cache.get(&song.video_id).cloned();
        if let (Some(shown), Some(query)) = (shown, Self::resolve_query(song)) {
            self.upgrade_to_word_timing(&song.video_id, query, &shown, None);
        }
    }

    /// Swift `UserDefaults` bool `settings.lyricsKaraoke`, set by the lyrics menu toggle.
    pub fn karaoke_enabled(&self) -> bool {
        self.inner()
            .defaults
            .get(Self::KARAOKE_ENABLED_KEY)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn set_karaoke_enabled(&self, enabled: bool) {
        self.set_defaults([(Self::KARAOKE_ENABLED_KEY.to_string(), enabled.into())]);
    }

    fn wants_word_timing(&self, lines: &[LyricLine]) -> bool {
        self.karaoke_enabled() && !lines.iter().any(|line| line.words.is_some())
    }

    /// When karaoke is on and the shown lyrics are line-only, asks the other
    /// providers for a word-timed copy in the background and swaps it in, so
    /// the line-synced lyrics appear without waiting for it. Needs a tokio runtime.
    fn upgrade_to_word_timing(
        self: &Arc<Self>,
        video_id: &str,
        query: LyricsQuery,
        shown: &[LyricLine],
        excluding: Option<String>,
    ) {
        if !self.wants_word_timing(shown) {
            return;
        }
        let shown_id = shown.first().map(|line| line.id);
        let this = self.clone();
        let video_id = video_id.to_string();
        tokio::spawn(async move {
            let Some((lines, provider)) = this
                .manager
                .fetch_word_timed(&query, excluding.as_deref())
                .await
            else {
                return;
            };
            {
                let mut inner = this.inner();
                // Other lyrics may have been picked or refetched meanwhile.
                let current_id = inner
                    .cache
                    .get(&video_id)
                    .and_then(|c| c.first())
                    .map(|line| line.id);
                if inner.customs.contains_key(&video_id) || current_id != shown_id {
                    return;
                }
                inner.cache.insert(video_id.clone(), lines);
            }
            this.update_availability(&video_id, true, Some(provider));
            // Swift bumps it only while the song is playing.
            this.state.lock().unwrap().refresh_token += 1;
        });
    }

    fn custom_entry(&self, video_id: &str) -> Option<(Vec<LyricLine>, String)> {
        let mut inner = self.inner();
        if let Some(entry) = inner.customs.get(video_id) {
            return Some(entry.clone());
        }
        let raw = Self::string(&inner.defaults, &Self::custom_key(video_id))?;
        let lines = LyricsParsing::parse_stored_text(&raw);
        if lines.is_empty() {
            return None;
        }
        let provider = Self::string(&inner.defaults, &Self::custom_provider_key(video_id))
            .unwrap_or_else(|| "Manual".to_string());
        let entry = (lines, provider);
        inner.customs.insert(video_id.to_string(), entry.clone());
        Some(entry)
    }

    /// Swift default `upcoming`: `[]`.
    pub async fn preload(self: &Arc<Self>, song: &LyricsSong, upcoming: &[LyricsSong]) {
        let _ = self.fetch_lyrics(song).await;
        for next in upcoming {
            let known = self.inner().cache.contains_key(&next.video_id);
            if !known && self.custom_entry(&next.video_id).is_none() {
                let _ = self.fetch_lyrics(next).await;
            }
        }
    }

    /// `None` provider keeps the name recorded earlier for this song.
    fn update_availability(&self, video_id: &str, available: bool, provider_name: Option<String>) {
        let mut state = self.state.lock().unwrap();
        if available {
            state.available_video_ids.insert(video_id.to_string());
            if let Some(provider) = provider_name {
                state.provider_names.insert(video_id.to_string(), provider);
            }
        } else {
            state.available_video_ids.remove(video_id);
        }
    }

    fn string(defaults: &JsonDict, key: &str) -> Option<String> {
        defaults.get(key)?.as_str().map(str::to_string)
    }

    fn set_defaults<const N: usize>(&self, values: [(String, Value); N]) {
        let bytes = {
            let mut inner = self.inner();
            inner.defaults.extend(values);
            serde_json::to_vec(&inner.defaults)
        };
        let saved = bytes.map_err(std::io::Error::from).and_then(|bytes| {
            if let Some(parent) = self.defaults_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&self.defaults_path, bytes)
        });
        if let Err(error) = saved {
            log::error!("Saving the lyrics defaults failed: {error}");
        }
    }

    // Query cleaning.

    /// The provider query for `song`; `None` without a title or an artist.
    pub fn resolve_query(song: &LyricsSong) -> Option<LyricsQuery> {
        if song.title.is_empty() || song.artist.is_empty() {
            return None;
        }
        let (extracted_title, extracted_artist) =
            Self::extract_artist_title(&song.title, &song.artist);
        Some(LyricsQuery {
            title: Self::clean_title(&extracted_title),
            artist: Self::clean_artist(extracted_artist.as_deref().unwrap_or(&song.artist)),
            album: song
                .album
                .as_deref()
                .filter(|a| !a.is_empty())
                .map(Self::clean_album),
            duration: song.duration.max(0.0),
        })
    }

    /// "Artist - Title" uploads from a channel: the real artist is on the left.
    fn extract_artist_title(raw_title: &str, channel_artist: &str) -> (String, Option<String>) {
        if !Self::is_channel_like(channel_artist) {
            return (raw_title.to_string(), None);
        }
        for sep in [" - ", " – ", " — "] {
            if let Some((left, right)) = raw_title.split_once(sep) {
                let (left, right) = (left.trim(), right.trim());
                if left.is_empty() || right.is_empty() {
                    break;
                }
                return (right.to_string(), Some(left.to_string()));
            }
        }
        (raw_title.to_string(), None)
    }

    fn is_channel_like(name: &str) -> bool {
        const CHANNEL_WORDS: [&str; 22] = [
            "music",
            "channel",
            "official",
            "records",
            "record",
            "tv",
            "studio",
            "entertainment",
            "4k",
            "中文",
            "古风",
            "chinese",
            "ort",
            "ehp",
            "vip",
            "lyrics",
            "mv",
            "topic",
            "vevo",
            "remix",
            "dj",
            "radio",
        ];
        let lower = name.to_lowercase();
        !lower.is_empty() && CHANNEL_WORDS.iter().any(|w| lower.contains(w))
    }

    fn clean_title(raw: &str) -> String {
        let s = CJK_BRACKETS.replace_all(raw, "");
        let s = BRACKETS.replace_all(&s, "");
        let mut s = PARENS.replace_all(&s, "").into_owned();
        // ASCII lowercasing keeps byte offsets, so `find` indexes into `s`.
        let lower = s.to_ascii_lowercase();
        for marker in [
            "|", "♪", "mv", "lyrics", "歌词", "動態", "动态", "official", "官方",
        ] {
            if let Some(at) = lower.find(marker) {
                s.truncate(at);
                break;
            }
        }
        s.trim().to_string()
    }

    fn clean_artist(raw: &str) -> String {
        raw.split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| {
                let p = part.split_once(" - ").map_or(part, |(left, _)| left);
                p.replace(" - Topic", "")
                    .replace("-Topic", "")
                    .replace("Topic", "")
                    .trim()
                    .to_string()
            })
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn clean_album(raw: &str) -> String {
        let s = BRACKETS.replace_all(raw, "");
        PARENS.replace_all(&s, "").trim().to_string()
    }
}
