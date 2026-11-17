use serde::{Deserialize, Serialize};

/// Result of a successful stream resolution.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackResult {
    pub stream_url: String,
    pub itag: i64,
    pub mime_type: String,
    pub bitrate: i64,
    pub audio_quality: String,
    pub video_id: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub duration: Option<i64>,
    pub expires_in_seconds: i64,
    pub client_name: String,
    pub music_video_type: Option<String>,
    /// Has video dimensions.
    pub has_video_content: bool,
    /// Combined audio-video stream URL for instant toggling without reloads.
    pub muxed_stream_url: Option<String>,
    /// Dimensions of the muxed stream (when present), for a quality picker.
    pub muxed_width: Option<i64>,
    pub muxed_height: Option<i64>,
    /// Loudness of the selected stream (LUFS) for normalization.
    pub loudness_db: Option<f64>,
}

impl PlaybackResult {
    /// Whether a real (watchable) video exists — not just an auto-generated
    /// art-track still. An ATV always ships video streams (cover stills), so
    /// bare stream presence must not imply video; only a non-ATV type counts,
    /// with unknown types keeping the stream fallback.
    pub fn has_real_video(&self) -> bool {
        Self::has_watchable_video(self.music_video_type.as_deref(), self.has_video_content)
    }

    /// Pure form of the verdict, for callers combining a row-level type with
    /// resolve-level stream info.
    pub fn has_watchable_video(music_video_type: Option<&str>, has_video_content: bool) -> bool {
        let type_has_video = music_video_type.is_some_and(|t| {
            t != "MUSIC_VIDEO_TYPE_ATV" && t != "MUSIC_VIDEO_TYPE_PODCAST_EPISODE"
        });
        type_has_video || (has_video_content && music_video_type != Some("MUSIC_VIDEO_TYPE_ATV"))
    }
}
