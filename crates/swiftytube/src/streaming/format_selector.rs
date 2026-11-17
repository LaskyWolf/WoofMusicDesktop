use super::playback_quality::{AudioQuality, DownloadQuality, VideoQuality};
use super::player_response::Format;
use crate::core::swiftytube_log::SwiftyTubeLog;

/// Swift `max(by:)`: the first element with the highest score.
/// (`Iterator::max_by_key` returns the last one, so it runs reversed.)
fn max_by<'a>(formats: &[&'a Format], score: impl Fn(&Format) -> i64) -> Option<&'a Format> {
    formats.iter().rev().max_by_key(|f| score(f)).copied()
}

/// Swift `min(by:)`: the first element with the lowest score.
fn min_by<'a>(formats: &[&'a Format], score: impl Fn(&Format) -> i64) -> Option<&'a Format> {
    formats.iter().min_by_key(|f| score(f)).copied()
}

// Best-format picks. Audio score: quality, channels, codec, bitrate.
// Video prefers H.264 (simulator can't decode VP9).
pub struct FormatSelector;

impl FormatSelector {
    /// Audio-only first, then combined with audio. None if none.
    pub fn audio_bearing_formats(formats: &[Format]) -> Option<Vec<&Format>> {
        let mut audio_formats: Vec<&Format> =
            formats.iter().filter(|f| f.is_audio_only()).collect();
        if audio_formats.is_empty() {
            SwiftyTubeLog::debug(
                "No audio-only formats found - trying combined formats with audio track",
            );
            audio_formats = formats
                .iter()
                .filter(|f| f.audio_channels.is_some())
                .collect();
        }
        if audio_formats.is_empty() {
            SwiftyTubeLog::debug(&format!(
                "No formats with audio track found in {} total formats",
                formats.len()
            ));
            return None;
        }
        Some(audio_formats)
    }

    /// Prefers H.264 (avc/mp4) for compatibility, falling back to the full pool.
    pub fn h264_preferred_pool<'a>(formats: Vec<&'a Format>, kind: &str) -> Vec<&'a Format> {
        let h264: Vec<&Format> = formats
            .iter()
            .filter(|f| {
                f.codec().to_lowercase().contains("avc")
                    || f.mime_type
                        .as_deref()
                        .is_some_and(|m| m.to_lowercase().contains("mp4"))
            })
            .copied()
            .collect();
        if h264.is_empty() {
            SwiftyTubeLog::debug(&format!(
                "No H.264 {kind} format; falling back to any {kind} format"
            ));
            formats
        } else {
            h264
        }
    }

    /// Best audio format, optionally restricted to the user's quality tier first.
    pub fn best_audio_format(formats: &[Format], preference: AudioQuality) -> Option<&Format> {
        if formats.is_empty() {
            SwiftyTubeLog::debug("No formats to select from");
            return None;
        }

        let mut audio_formats = Self::audio_bearing_formats(formats)?;

        if preference != AudioQuality::Auto {
            let tier = Self::quality_tier(preference);
            let tier_formats: Vec<&Format> = audio_formats
                .iter()
                .filter(|f| Self::quality_rank(f.audio_quality.as_deref()) == tier)
                .copied()
                .collect();
            if tier_formats.is_empty() {
                SwiftyTubeLog::debug(&format!(
                    "No {preference:?}-tier format, using best available"
                ));
            } else {
                SwiftyTubeLog::debug(&format!(
                    "Preferring {preference:?} tier: {} format(s)",
                    tier_formats.len()
                ));
                audio_formats = tier_formats;
            }
        }

        SwiftyTubeLog::debug(&format!(
            "Selecting from {} audio-bearing formats",
            audio_formats.len()
        ));

        let selected = max_by(&audio_formats, Self::format_score);

        match selected {
            Some(selected) => SwiftyTubeLog::debug(&format!(
                "Selected: itag={} quality={} channels={} codec={} bitrate={}",
                selected.itag.unwrap_or(0),
                selected.audio_quality.as_deref().unwrap_or("?"),
                selected.audio_channels.unwrap_or(0),
                selected.codec(),
                selected.bitrate.unwrap_or(0),
            )),
            None => SwiftyTubeLog::debug("No format could be selected"),
        }

        selected
    }

    /// Maps the user-facing quality preference to the format quality tier.
    fn quality_tier(preference: AudioQuality) -> i64 {
        match preference {
            AudioQuality::High => 3,
            AudioQuality::Medium => 2,
            AudioQuality::Low => 1,
            AudioQuality::Auto => 0,
        }
    }

    /// Maps a format's `AUDIO_QUALITY_*` string to a comparable tier.
    pub fn quality_rank(quality: Option<&str>) -> i64 {
        match quality {
            Some("AUDIO_QUALITY_HIGH") => 3,
            Some("AUDIO_QUALITY_MEDIUM") => 2,
            Some("AUDIO_QUALITY_LOW") => 1,
            _ => 0,
        }
    }

    /// Best audio for downloads. `Standard` caps near the target, picks smallest.
    /// Swift defaults: `downloadQuality: .high`, `standardTargetBitrate: 96000`.
    pub fn best_download_format(
        formats: &[Format],
        download_quality: DownloadQuality,
        standard_target_bitrate: i64,
    ) -> Option<&Format> {
        if formats.is_empty() {
            SwiftyTubeLog::debug("No formats to select from (download)");
            return None;
        }

        let audio_formats = Self::audio_bearing_formats(formats)?;

        // Prefer formats we can actually fetch (direct URL or cipher).
        let fetchable: Vec<&Format> = audio_formats
            .iter()
            .filter(|f| {
                f.url.as_deref().is_some_and(|u| !u.is_empty())
                    || f.signature_cipher.is_some()
                    || f.cipher.is_some()
            })
            .copied()
            .collect();
        let candidates = if fetchable.is_empty() {
            audio_formats
        } else {
            fetchable
        };

        let aac_formats: Vec<&Format> = candidates
            .iter()
            .filter(|f| {
                let codec = f.codec().to_lowercase();
                codec.contains("mp4a") || codec.contains("aac")
            })
            .copied()
            .collect();

        let no_aac = aac_formats.is_empty();
        let mut pool = if no_aac { candidates } else { aac_formats };
        SwiftyTubeLog::debug(&format!(
            "Download pool: {} format(s){}",
            pool.len(),
            if no_aac {
                " (no AAC, falling back to Opus)"
            } else {
                " (AAC preferred)"
            }
        ));

        let selected = if download_quality == DownloadQuality::Standard {
            let capped: Vec<&Format> = pool
                .iter()
                .filter(|f| {
                    f.bitrate.unwrap_or(standard_target_bitrate) <= standard_target_bitrate + 32000
                })
                .copied()
                .collect();
            if !capped.is_empty() {
                pool = capped;
            }
            min_by(&pool, |f| f.bitrate.unwrap_or(i64::MAX))
        } else {
            max_by(&pool, Self::format_score)
        };

        if let Some(selected) = selected {
            SwiftyTubeLog::debug(&format!(
                "Download selected: itag={} codec={} bitrate={}",
                selected.itag.unwrap_or(0),
                selected.codec(),
                selected.bitrate.unwrap_or(0),
            ));
        }

        selected
    }

    fn format_score(format: &Format) -> i64 {
        let quality_score = Self::quality_rank(format.audio_quality.as_deref()) * 10000;
        let channels_score = format.audio_channels.unwrap_or(2) * 1000;
        let codec_score = Self::score_codec(&format.codec()) * 100;
        let bitrate_score = format.bitrate.unwrap_or(0) / 1000;
        quality_score + channels_score + codec_score + bitrate_score
    }

    fn score_codec(codec: &str) -> i64 {
        let lowercased = codec.to_lowercase();
        if lowercased.contains("opus") {
            2
        } else if lowercased.contains("mp4a") || lowercased.contains("aac") {
            1
        } else {
            0
        }
    }

    // MARK: - Video Format Selection

    /// Best muxed (video+audio) format: highest resolution, H.264 first.
    /// `preference` caps the resolution to the nearest height at or below it.
    pub fn best_video_format(formats: &[Format], preference: VideoQuality) -> Option<&Format> {
        if formats.is_empty() {
            SwiftyTubeLog::debug("No formats to select from (video)");
            return None;
        }

        let muxed_formats: Vec<&Format> = formats
            .iter()
            .filter(|f| {
                !f.is_audio_only()
                    && f.width.is_some()
                    && f.height.is_some()
                    && f.audio_channels.is_some()
                    && f.url.is_some()
            })
            .collect();

        if muxed_formats.is_empty() {
            SwiftyTubeLog::debug(&format!(
                "No muxed (audio+video) formats found in {} total formats",
                formats.len()
            ));
            return None;
        }

        SwiftyTubeLog::debug(&format!(
            "Selecting from {} muxed video formats",
            muxed_formats.len()
        ));

        let pool = Self::h264_preferred_pool(muxed_formats, "muxed");

        let selected = Self::select_by_height(&pool, preference, Self::video_format_score);

        if let Some(selected) = selected {
            SwiftyTubeLog::debug(&format!(
                "Selected video: itag={} resolution={}x{} codec={} bitrate={}",
                selected.itag.unwrap_or(0),
                selected.width.unwrap_or(0),
                selected.height.unwrap_or(0),
                selected.codec(),
                selected.bitrate.unwrap_or(0),
            ));
        }

        selected
    }

    /// Highest-scoring format at or below the preference height; when nothing
    /// fits (only larger streams exist), the smallest one above it.
    pub fn select_by_height<'a>(
        formats: &[&'a Format],
        preference: VideoQuality,
        score: impl Fn(&Format) -> i64,
    ) -> Option<&'a Format> {
        let Some(target) = preference.height() else {
            return max_by(formats, score);
        };
        let at_or_below: Vec<&Format> = formats
            .iter()
            .filter(|f| f.height.unwrap_or(0) <= target)
            .copied()
            .collect();
        if at_or_below.is_empty() {
            return min_by(formats, |f| f.height.unwrap_or(i64::MAX));
        }
        max_by(&at_or_below, score)
    }

    /// Best DASH video-only format: highest resolution, H.264 preferred for
    /// compatibility (software VP9 stalls low-end devices and the simulator
    /// cannot decode it at all).
    pub fn best_video_only_format(formats: &[Format]) -> Option<&Format> {
        if formats.is_empty() {
            SwiftyTubeLog::debug("No formats to select from (video-only)");
            return None;
        }

        let video_only_formats: Vec<&Format> = formats
            .iter()
            .filter(|f| {
                !f.is_audio_only()
                    && f.width.is_some()
                    && f.height.is_some()
                    && f.audio_channels.is_none()
                    && f.url.is_some()
            })
            .collect();

        if video_only_formats.is_empty() {
            SwiftyTubeLog::debug(&format!(
                "No video-only formats found in {} total formats",
                formats.len()
            ));
            return None;
        }

        SwiftyTubeLog::debug(&format!(
            "Selecting from {} video-only formats",
            video_only_formats.len()
        ));

        let pool = Self::h264_preferred_pool(video_only_formats, "video-only");

        let selected = max_by(&pool, Self::video_format_score);

        if let Some(selected) = selected {
            SwiftyTubeLog::debug(&format!(
                "Selected video-only: itag={} resolution={}x{} codec={} bitrate={}",
                selected.itag.unwrap_or(0),
                selected.width.unwrap_or(0),
                selected.height.unwrap_or(0),
                selected.codec(),
                selected.bitrate.unwrap_or(0),
            ));
        }

        selected
    }

    fn video_format_score(format: &Format) -> i64 {
        let width_score = format.width.unwrap_or(0) * 10;
        let height_score = format.height.unwrap_or(0) * 10;
        let bitrate_score = format.bitrate.unwrap_or(0) / 1000;
        width_score + height_score + bitrate_score
    }
}
