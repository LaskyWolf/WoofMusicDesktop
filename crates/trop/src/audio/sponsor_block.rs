//! `Trop/Audio/SponsorBlock.swift`: skips the non-music parts of music videos
//! (talking intros, skits, long outros) with SponsorBlock, a community
//! database of video segments. Only the first four characters of a hash of
//! the video id are sent, so the server can't tell which song is playing.

use std::collections::HashSet;

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
}

/// The segments of the playing song and which ones were already skipped.
#[derive(Debug, Default)]
pub struct SponsorBlock {
    video_id: Option<String>,
    segments: Vec<Segment>,
    /// Skipped in this play; seeking back into one plays it.
    skipped: HashSet<usize>,
}

impl SponsorBlock {
    /// A play of `video_id` starts. True when it is a new song: the caller
    /// fetches its segments. The same song again (repeat) skips them again.
    pub fn track(&mut self, video_id: &str) -> bool {
        self.skipped.clear();
        if self.video_id.as_deref() == Some(video_id) {
            return false;
        }
        self.video_id = Some(video_id.to_owned());
        self.segments.clear();
        true
    }

    /// The fetched segments, kept only if the song is still the same.
    pub fn found(&mut self, video_id: &str, segments: Vec<Segment>) {
        if self.video_id.as_deref() == Some(video_id) {
            self.segments = segments;
        }
    }

    /// Where to jump when `position` is inside a segment not skipped yet.
    pub fn skip_to(&mut self, position: f64) -> Option<f64> {
        let index = self
            .segments
            .iter()
            .enumerate()
            .position(|(index, segment)| {
                // Within the last second the jump isn't worth it.
                !self.skipped.contains(&index)
                    && position >= segment.start
                    && position < segment.end - 1.0
            })?;
        self.skipped.insert(index);
        Some(self.segments[index].end)
    }
}

/// The hash prefix the request names instead of the video id.
pub fn hash_prefix(video_id: &str) -> String {
    let hash = Sha256::digest(video_id.as_bytes());
    hash.iter()
        .take(2)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The non-music segments of `video_id`; empty when there are none, the
/// server is unreachable, or the reply can't be read.
pub async fn fetch_segments(http: &reqwest::Client, video_id: &str) -> Vec<Segment> {
    let url = format!(
        "https://sponsor.ajay.app/api/skipSegments/{}?categories=%5B%22music_offtopic%22%5D",
        hash_prefix(video_id)
    );
    let Ok(response) = http.get(url).send().await else {
        return Vec::new();
    };
    if response.status() != reqwest::StatusCode::OK {
        return Vec::new();
    }
    match response.bytes().await {
        Ok(body) => parse(&body, video_id),
        Err(_) => Vec::new(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoSegments {
    #[serde(rename = "videoID")]
    video_id: String,
    segments: Vec<RawSegment>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSegment {
    category: String,
    action_type: Option<String>,
    segment: Vec<f64>,
}

/// The reply lists every video whose hash starts the same: keep `video_id`'s
/// skippable music_offtopic segments, in order.
pub fn parse(body: &[u8], video_id: &str) -> Vec<Segment> {
    let Ok(videos) = serde_json::from_slice::<Vec<VideoSegments>>(body) else {
        return Vec::new();
    };
    let Some(video) = videos.into_iter().find(|video| video.video_id == video_id) else {
        return Vec::new();
    };
    let mut segments: Vec<Segment> = video
        .segments
        .into_iter()
        .filter(|raw| {
            raw.category == "music_offtopic"
                && raw.action_type.as_deref().unwrap_or("skip") == "skip"
                && raw.segment.len() == 2
        })
        .map(|raw| Segment {
            start: raw.segment[0],
            end: raw.segment[1],
        })
        .collect();
    segments.sort_by(|a, b| a.start.total_cmp(&b.start));
    segments
}
