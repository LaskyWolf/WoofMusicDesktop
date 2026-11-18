//! Trop/Audio/Lyrics/Providers/LyricsProvider.swift. The Swift `#if DEBUG`
//! `selfCheck` is the `parsing_*` tests in `tests/lyrics_test.rs`.

use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::sync::LazyLock;

use flate2::read::DeflateDecoder;
use regex::{Captures, Regex};
use serde_json::Value;
use swiftytube::networking::http_client::Session;

use super::lrclib_provider::LyricsError;
use crate::audio::lyrics::lyrics_service::{LyricLine, LyricWord};

/// A normalized query used to look up lyrics across providers.
#[derive(Clone, Debug, PartialEq)]
pub struct LyricsQuery {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    /// Seconds.
    pub duration: f64,
}

impl LyricsQuery {
    pub fn duration_seconds(&self) -> i64 {
        self.duration as i64
    }
}

/// Swift `async throws -> [LyricLine]`.
pub type LyricsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<LyricLine>, LyricsError>> + Send + 'a>>;

/// Common interface implemented by every lyrics source. `session` stands in
/// for `URLSession.shared`, so tests can answer without the network.
pub trait LyricsProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn fetch<'a>(&'a self, query: &'a LyricsQuery, session: &'a Session) -> LyricsFuture<'a>;
}

/// One or more leading [..] tags.
static LEADING_TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:\[(\d+):(\d{1,2})(?:\.(\d{1,3}))?\])+").unwrap());
static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[(\d+):(\d{1,2})(?:\.(\d{1,3}))?\]").unwrap());
/// Enhanced LRC word tag: `<mm:ss.xx>`.
static WORD_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(\d+):(\d{1,2})(?:\.(\d{1,3}))?>").unwrap());
/// LRC `[offset:±ms]` header.
static OFFSET_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\[offset:\s*([+-]?\d+)\s*\]").unwrap());
/// LRC metadata line such as `[ar:Artist]` or `[offset:+500]`.
static METADATA_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[[A-Za-z]+:.*\]$").unwrap());
/// Netease klyric / yrc and Kugou KRC line start `[startMs,durationMs]`.
static KLYRIC_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[(\d+),\d+\]").unwrap());
/// Netease klyric word tag `(x,durationMs)`.
static KLYRIC_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(\d+,(\d+)\)").unwrap());
/// Netease yrc word tag `(startMs,durationMs,0)` or Kugou KRC `<offsetMs,durationMs,0>`.
static TIMED_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[(<](\d+),(\d+),-?\d+[)>]").unwrap());

/// Chinese providers answer with a line instead of lyrics when a song has
/// none: "纯音乐，请欣赏" is "instrumental, please enjoy".
const PLACEHOLDER_FRAGMENTS: [&str; 6] = [
    "纯音乐",
    "純音樂",
    "暂无歌词",
    "暫無歌詞",
    "没有歌词",
    "沒有歌詞",
];

const CREDIT_KEYWORDS: [&str; 33] = [
    "词", "詞", "曲", "编", "編", "制作", "製作", "混音", "母带", "录音", "錄音", "和声", "和聲",
    "监制", "監製", "出品", "发行", "發行", "吉他", "贝斯", "鼓", "弦乐", "企划", "统筹", "lyric",
    "compos", "written", "writer", "produc", "arrang", "mix", "master", "vocal",
];

/// Kugou KRC XOR key.
const KRC_KEY: [u8; 16] = [
    64, 71, 97, 119, 94, 50, 116, 71, 81, 54, 49, 45, 206, 210, 110, 105,
];

/// Splits on `\n` and `\r`, dropping empty pieces (Swift `split(whereSeparator: \.isNewline)`)
/// and a leading byte order mark, which would hide the first line's tag.
fn split_lines(raw: &str) -> impl Iterator<Item = &str> {
    let raw = raw.strip_prefix('\u{FEFF}').unwrap_or(raw);
    raw.split(['\n', '\r']).filter(|line| !line.is_empty())
}

pub struct LyricsParsing;

impl LyricsParsing {
    /// Seconds from a tag match whose groups 1–3 are minutes, seconds and an
    /// optional 2 or 3 digit fraction.
    fn tag_seconds(caps: &Captures) -> f64 {
        let int = |i| {
            caps.get(i)
                .and_then(|m| m.as_str().parse::<i64>().ok())
                .unwrap_or(0)
        };
        let frac = caps
            .get(3)
            .and_then(|m| format!("0.{}", m.as_str()).parse::<f64>().ok())
            .unwrap_or(0.0);
        (int(1) * 60 + int(2)) as f64 + frac
    }

    /// Parses the `[mm:ss.xx]` or `[mm:ss]` timestamps at the start of a line.
    /// Compressed LRC gives one text several times: `[00:10][01:20]x` is shown at 10 s and 80 s.
    pub fn parse_lrc_timestamps(line: &str) -> Option<(Vec<f64>, String)> {
        let lead = LEADING_TAGS.find(line)?;
        let times = TAG
            .captures_iter(lead.as_str())
            .map(|tag| Self::tag_seconds(&tag))
            .collect();
        Some((times, line[lead.end()..].trim().to_string()))
    }

    /// Split a raw LRC string into lines sorted by time. Blank timestamped
    /// lines are kept: they mark instrumental spans.
    pub fn parse_lrc(raw: &str) -> Vec<LyricLine> {
        Self::parse_lrc_lines(raw, false)
    }

    /// `keep_untimed` keeps lines without a timestamp, except metadata tags
    /// like `[ar:Artist]`; each stays right after the line before it.
    fn parse_lrc_lines(raw: &str, keep_untimed: bool) -> Vec<LyricLine> {
        let mut keyed: Vec<(f64, LyricLine)> = Vec::new();
        for line in split_lines(raw) {
            let Some((times, text)) = Self::parse_lrc_timestamps(line) else {
                let text = line.trim();
                if keep_untimed && !text.is_empty() && !METADATA_TAG.is_match(text) {
                    let key = keyed.last().map_or(f64::NEG_INFINITY, |(key, _)| *key);
                    keyed.push((key, LyricLine::new(text, None)));
                }
                continue;
            };
            let parsed = Self::parse_word_tags(&text);
            for time in times {
                let line = match &parsed {
                    None => LyricLine::new(text.clone(), Some(time)),
                    Some((plain, words)) => {
                        LyricLine::with_words(plain.clone(), Some(time), words.clone())
                    }
                };
                keyed.push((time, line));
            }
        }
        keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut lines: Vec<LyricLine> = keyed.into_iter().map(|(_, line)| line).collect();
        // `[offset:+500]` in ms: a positive offset shows the lyrics earlier.
        let offset = split_lines(raw)
            .find_map(|line| OFFSET_TAG.captures(line.trim()))
            .and_then(|tag| tag[1].parse::<f64>().ok())
            .map_or(0.0, |ms| ms / 1000.0);
        if offset != 0.0 {
            let shift = |t: f64| (t - offset).max(0.0);
            for line in &mut lines {
                line.start_time = line.start_time.map(shift);
                for word in line.words.iter_mut().flatten() {
                    word.start_time = shift(word.start_time);
                    word.end_time = word.end_time.map(shift);
                }
            }
        }
        lines
    }

    /// Enhanced LRC word tags: `<00:12.00>Never <00:12.50>gonna<00:13.25>`.
    /// Each tag starts the text after it and the next tag ends it. Returns
    /// `None` when the text has no tags; `words` is `None` when no piece has text.
    pub fn parse_word_tags(text: &str) -> Option<(String, Option<Vec<LyricWord>>)> {
        let tags: Vec<Captures> = WORD_TAG.captures_iter(text).collect();
        let first_start = tags.first()?.get(0)?.start();
        let mut words = Vec::new();
        for (i, tag) in tags.iter().enumerate() {
            let next = tags.get(i + 1);
            let from = tag.get(0).map_or(0, |m| m.end());
            let to = next
                .and_then(|n| n.get(0))
                .map_or(text.len(), |m| m.start());
            Self::append_piece(
                &text[from..to],
                Self::tag_seconds(tag),
                next.map(Self::tag_seconds),
                &mut words,
            );
        }
        // Text before the first tag has no time of its own: it joins the first word.
        if let Some(first) = words.first_mut() {
            first.text.insert_str(0, &text[..first_start]);
        }
        match Self::trimmed_words(words) {
            Some(timed) => Some((Self::joined(&timed), Some(timed))),
            None => Some((WORD_TAG.replace_all(text, "").trim().to_string(), None)),
        }
    }

    /// Netease `klyric`: `[startMs,durationMs](0,508)Don't(0,1) (0,151)want`.
    /// The second number of each `(x,ms)` tag is the following word's
    /// duration; words follow each other from the line start.
    pub fn parse_klyric(raw: &str) -> Vec<LyricLine> {
        split_lines(raw)
            .filter_map(|sub| {
                let line = sub.trim();
                let head = KLYRIC_LINE.captures(line)?;
                let start_ms: f64 = head[1].parse().ok()?;
                let body = &line[head.get(0)?.end()..];
                let tags: Vec<Captures> = KLYRIC_WORD.captures_iter(body).collect();
                let mut words = Vec::new();
                let mut cursor = start_ms / 1000.0;
                for (i, tag) in tags.iter().enumerate() {
                    let from = tag.get(0).map_or(0, |m| m.end());
                    let to = tags
                        .get(i + 1)
                        .and_then(|n| n.get(0))
                        .map_or(body.len(), |m| m.start());
                    let duration = tag[1].parse::<f64>().unwrap_or(0.0) / 1000.0;
                    Self::append_piece(
                        &body[from..to],
                        cursor,
                        Some(cursor + duration),
                        &mut words,
                    );
                    cursor += duration;
                }
                let timed = Self::trimmed_words(words)?;
                Some(LyricLine::with_words(
                    Self::joined(&timed),
                    Some(start_ms / 1000.0),
                    Some(timed),
                ))
            })
            .collect()
    }

    /// Netease `yrc`: `[lineMs,durMs](startMs,durMs,0)word`, word starts in song
    /// time. Kugou KRC: `[lineMs,durMs]<offsetMs,durMs,0>word`, word starts
    /// from the line start.
    pub fn parse_word_timed_lines(raw: &str, offsets_from_line_start: bool) -> Vec<LyricLine> {
        split_lines(raw)
            .filter_map(|sub| {
                let line = sub.trim();
                let head = KLYRIC_LINE.captures(line)?;
                let line_ms: f64 = head[1].parse().ok()?;
                let body = &line[head.get(0)?.end()..];
                let tags: Vec<Captures> = TIMED_WORD.captures_iter(body).collect();
                let base_ms = if offsets_from_line_start {
                    line_ms
                } else {
                    0.0
                };
                let mut words = Vec::new();
                for (i, tag) in tags.iter().enumerate() {
                    let from = tag.get(0).map_or(0, |m| m.end());
                    let to = tags
                        .get(i + 1)
                        .and_then(|n| n.get(0))
                        .map_or(body.len(), |m| m.start());
                    let start = (base_ms + tag[1].parse::<f64>().unwrap_or(0.0)) / 1000.0;
                    let duration = tag[2].parse::<f64>().unwrap_or(0.0) / 1000.0;
                    Self::append_piece(&body[from..to], start, Some(start + duration), &mut words);
                }
                let timed = Self::trimmed_words(words)?;
                Some(LyricLine::with_words(
                    Self::joined(&timed),
                    Some(line_ms / 1000.0),
                    Some(timed),
                ))
            })
            .collect()
    }

    /// Kugou KRC file: `krc1` + zlib data XORed with a fixed key.
    pub fn decode_krc(data: &[u8]) -> Option<String> {
        if data.len() <= 10 || !data.starts_with(b"krc1") {
            return None;
        }
        let zlib: Vec<u8> = data[4..]
            .iter()
            .enumerate()
            .map(|(i, byte)| byte ^ KRC_KEY[i % KRC_KEY.len()])
            .collect();
        // Raw DEFLATE: drop the 2-byte zlib header and the 4-byte checksum.
        let mut text = String::new();
        DeflateDecoder::new(&zlib[2..zlib.len() - 4])
            .read_to_string(&mut text)
            .ok()?;
        Some(text)
    }

    /// Musixmatch `richsync_body`: a JSON array of lines `{ts, te, l: [{c, o}]}`
    /// in seconds. Each word starts `o` seconds after its line's `ts` and ends
    /// where the next word starts; the last ends at `te`.
    /// Words are split on spaces, so text without spaces (Japanese, Chinese)
    /// arrives as one piece per line: when most lines are a single piece the
    /// word timing is dropped and the lines stay line-synced, so a provider
    /// with real per-character timing (Kugou KRC) wins the karaoke lookup.
    pub fn parse_richsync(body: &str) -> Vec<LyricLine> {
        let Ok(Value::Array(rows)) = serde_json::from_str::<Value>(body) else {
            return Vec::new();
        };
        let lines: Vec<LyricLine> = rows
            .iter()
            .filter_map(|row| {
                let ts = row.get("ts")?.as_f64()?;
                let te = row.get("te")?.as_f64()?;
                let parts = row.get("l")?.as_array()?;
                let mut words = Vec::new();
                for (i, part) in parts.iter().enumerate() {
                    let (Some(piece), Some(offset)) = (
                        part.get("c").and_then(Value::as_str),
                        part.get("o").and_then(Value::as_f64),
                    ) else {
                        continue;
                    };
                    let next_offset = parts
                        .get(i + 1)
                        .and_then(|p| p.get("o"))
                        .and_then(Value::as_f64);
                    Self::append_piece(
                        piece,
                        ts + offset,
                        Some(next_offset.map_or(te, |o| ts + o)),
                        &mut words,
                    );
                }
                let timed = Self::trimmed_words(words)?;
                Some(LyricLine::with_words(
                    Self::joined(&timed),
                    Some(ts),
                    Some(timed),
                ))
            })
            .collect();
        let single_piece = lines
            .iter()
            .filter(|line| line.words.as_ref().is_some_and(|w| w.len() == 1))
            .count();
        if single_piece * 2 <= lines.len() {
            return lines;
        }
        lines
            .into_iter()
            .map(|line| LyricLine::new(line.text, line.start_time))
            .collect()
    }

    /// Adds a timed piece; whitespace-only (or empty) pieces join the previous word.
    fn append_piece(piece: &str, start: f64, end: Option<f64>, words: &mut Vec<LyricWord>) {
        if piece.trim().is_empty() {
            if let Some(last) = words.last_mut() {
                last.text.push_str(piece);
            }
            return;
        }
        words.push(LyricWord {
            text: piece.to_string(),
            start_time: start,
            end_time: end,
        });
    }

    /// Drops the line's outer spaces from its first and last word; `None` when empty.
    fn trimmed_words(mut words: Vec<LyricWord>) -> Option<Vec<LyricWord>> {
        let first = words.first_mut()?;
        first.text = first.text.trim_start().to_string();
        let last = words.last_mut()?;
        last.text = last.text.trim_end().to_string();
        Some(words)
    }

    fn joined(words: &[LyricWord]) -> String {
        words.iter().map(|w| w.text.as_str()).collect()
    }

    /// Parses stored lyrics text: LRC if timestamped, otherwise plain lines.
    /// Untimed lines in LRC text, such as a line added in the editor, are kept.
    pub fn parse_stored_text(raw: &str) -> Vec<LyricLine> {
        let lrc = Self::parse_lrc_lines(raw, true);
        if lrc.iter().any(|line| line.start_time.is_some()) {
            return lrc;
        }
        split_lines(raw)
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| LyricLine::new(line, None))
            .collect()
    }

    /// Serializes lines back to LRC/plain text for storage and editing.
    pub fn serialize_lines(lines: &[LyricLine]) -> String {
        let synced = lines.iter().any(|line| line.start_time.is_some());
        lines
            .iter()
            .map(|line| {
                let Some(t) = line.start_time.filter(|_| synced) else {
                    return line.text.clone();
                };
                let Some(words) = &line.words else {
                    return format!("[{}]{}", Self::stamp(t), line.text);
                };
                // Word timing is stored as Enhanced LRC; an end tag is written only
                // where a word does not end exactly when the next one starts.
                let mut body = String::new();
                for (i, word) in words.iter().enumerate() {
                    body += &format!("<{}>{}", Self::stamp(word.start_time), word.text);
                    let next_start = words.get(i + 1).map(|w| Self::stamp(w.start_time));
                    if let Some(end) = word.end_time.map(Self::stamp)
                        && Some(&end) != next_start.as_ref()
                    {
                        body += &format!("<{end}>");
                    }
                }
                format!("[{}]{body}", Self::stamp(t))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// `mm:ss.xx`, rounded to the nearest centisecond.
    fn stamp(t: f64) -> String {
        let centis = (t * 100.0).round() as i64;
        format!(
            "{:02}:{:02}.{:02}",
            centis / 6000,
            centis / 100 % 60,
            centis % 100
        )
    }

    /// Drops the credit lines some sources put before the lyrics ("作词：Eve",
    /// "编曲：Eve/Numa", "Lyrics by: X"), plus a leading "Title - Artist" line
    /// when credits follow it. Only the opening lines are checked, so a sung
    /// line with a colon later in the song stays.
    pub fn dropping_credits(lines: Vec<LyricLine>) -> Vec<LyricLine> {
        let mut count = 0;
        let mut saw_credit = false;
        for (index, line) in lines.iter().enumerate() {
            if Self::is_credit_line(&line.text) {
                saw_credit = true;
            } else if !(index == 0 && line.text.contains(" - ")) {
                break;
            }
            count = index + 1;
        }
        if saw_credit {
            lines.into_iter().skip(count).collect()
        } else {
            lines
        }
    }

    /// Empty when the lines are only a note that the song has no lyrics, so the
    /// next provider gets its turn. Real lyrics are longer than three lines.
    pub fn dropping_placeholder(lines: Vec<LyricLine>) -> Vec<LyricLine> {
        let is_note = lines.len() <= 3
            && lines
                .iter()
                .any(|line| PLACEHOLDER_FRAGMENTS.iter().any(|f| line.text.contains(f)));
        if is_note { Vec::new() } else { lines }
    }

    /// "Key: value" with a short key naming a credit role.
    fn is_credit_line(text: &str) -> bool {
        let Some(colon) = text.find([':', '：']) else {
            return false;
        };
        let key = text[..colon].trim().to_lowercase();
        if key.is_empty() || key.chars().count() > 14 {
            return false;
        }
        key == "op" || key == "sp" || CREDIT_KEYWORDS.iter().any(|k| key.contains(k))
    }

    /// Plain text without timestamps: what "Copy" puts on the clipboard.
    pub fn plain_text(lines: &[LyricLine]) -> String {
        lines
            .iter()
            .map(|line| line.text.as_str())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
