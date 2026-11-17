// Swift `Decodable` structs as serde structs. A field with the wrong JSON
// type fails the decode, like Swift.

use serde::Deserialize;

/// Top-level response from POST /player.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerResponse {
    pub response_context: Option<ResponseContext>,
    pub playability_status: Option<PlayabilityStatus>,
    pub streaming_data: Option<StreamingData>,
    pub video_details: Option<VideoDetails>,
    pub playback_tracking: Option<PlaybackTracking>,
    pub captions: Option<Captions>,
}

impl PlayerResponse {
    /// Simplified status check.
    pub fn is_playable(&self) -> bool {
        self.playability_status
            .as_ref()
            .and_then(|s| s.status.as_deref())
            == Some("OK")
    }
}

/// Caption metadata (used to fetch timedtext without a PoToken).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Captions {
    pub player_captions_tracklist_renderer: Option<CaptionTracklist>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionTracklist {
    pub caption_tracks: Option<Vec<CaptionTrack>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptionTrack {
    pub base_url: Option<String>,
    pub name: Option<TextRun>,
    pub language_code: Option<String>,
    /// "asr" for auto-generated captions.
    pub kind: Option<String>,
    pub vss_id: Option<String>,
}

impl CaptionTrack {
    pub fn is_auto_generated(&self) -> bool {
        self.kind.as_deref() == Some("asr")
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRun {
    pub simple_text: Option<String>,
    pub runs: Option<Vec<Run>>,
}

impl TextRun {
    pub fn text(&self) -> Option<String> {
        self.simple_text.clone().or_else(|| {
            self.runs
                .as_ref()
                .map(|runs| runs.iter().filter_map(|r| r.text.as_deref()).collect())
        })
    }
}

/// Swift `TextRun.Run`.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Run {
    pub text: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseContext {
    pub service_tracking_params: Option<Vec<ServiceTrackingParam>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ServiceTrackingParam {
    pub service: Option<String>,
    pub params: Option<Vec<TrackingParam>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct TrackingParam {
    pub key: Option<String>,
    pub value: Option<String>,
}

/// Playability information.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct PlayabilityStatus {
    pub status: Option<String>,
    pub reason: Option<String>,
}

/// Streaming data containing available formats.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamingData {
    pub formats: Option<Vec<Format>>,
    pub adaptive_formats: Option<Vec<Format>>,
    pub expires_in_seconds: Option<String>,
    pub hls_manifest_url: Option<String>,
}

impl StreamingData {
    pub fn expires_in_seconds_value(&self) -> Option<i64> {
        self.expires_in_seconds.as_deref()?.parse().ok()
    }
}

/// Individual stream format (audio or video).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Format {
    pub itag: Option<i64>,
    pub url: Option<String>,
    pub mime_type: Option<String>,
    pub bitrate: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub content_length: Option<String>,
    pub quality: Option<String>,
    pub average_bitrate: Option<i64>,
    pub audio_quality: Option<String>,
    pub audio_channels: Option<i64>,
    pub approx_duration_ms: Option<String>,
    pub loudness_db: Option<f64>,
    pub last_modified: Option<String>,
    pub signature_cipher: Option<String>,
    pub cipher: Option<String>,
}

impl Format {
    pub fn is_audio_only(&self) -> bool {
        self.width.is_none()
    }

    /// Codec from e.g. `audio/webm; codecs="opus"`.
    pub fn codec(&self) -> String {
        let Some(mime_type) = &self.mime_type else {
            return String::new();
        };
        mime_type
            .split_once("codecs=\"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .map_or_else(|| mime_type.clone(), |(codec, _)| codec.to_string())
    }
}

/// Video metadata.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoDetails {
    pub video_id: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub channel_id: Option<String>,
    pub length_seconds: Option<String>,
    pub music_video_type: Option<String>,
    pub view_count: Option<String>,
    pub thumbnail: Option<ThumbnailInfo>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ThumbnailInfo {
    pub thumbnails: Option<Vec<ThumbnailImage>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ThumbnailImage {
    pub url: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

/// Playback tracking URLs.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackTracking {
    pub videostats_playback_url: Option<TrackingUrl>,
    pub videostats_watchtime_url: Option<TrackingUrl>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackingUrl {
    pub base_url: Option<String>,
}
