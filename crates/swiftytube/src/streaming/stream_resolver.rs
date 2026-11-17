use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use super::duration_cache::DurationCache;
use super::format_selector::FormatSelector;
use super::playback_quality::{AudioQuality, DownloadQuality, VideoQuality};
use super::playback_result::PlaybackResult;
use super::player_request_options::PlayerRequestOptions;
use super::player_response::{Format, PlayerResponse};
use super::stream_error::StreamError;
use crate::cipher::cipher_error::CipherError;
use crate::core::swiftytube_log::SwiftyTubeLog;
use crate::core::youtube_client::YouTubeClient;
use crate::networking::http_client::{HttpClient, Session};
use crate::networking::inner_tube_client::{InnerTubeClient, PlayerArgs};
use crate::po_token::po_token_result::PoTokenResult;

/// Per-call knobs for `StreamResolver::resolve`.
#[derive(Clone, Debug, Default)]
pub struct StreamResolveOptions {
    /// PoToken for web clients (playerRequestPoToken).
    pub po_token: Option<String>,
    /// PoToken appended to stream URLs (`&pot=`).
    pub streaming_data_po_token: Option<String>,
    /// Caller-preferred format (used when still present in the response).
    pub preferred_format: Option<Format>,
    pub for_download: bool,
    pub audio_quality: AudioQuality,
    pub download_quality: DownloadQuality,
    /// Resolution cap for the returned muxed stream.
    pub video_quality: VideoQuality,
}

/// Swift `async` closure: a boxed, sendable future.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Optional cipher/PoToken hooks; None steps are skipped.
#[derive(Clone, Default)]
#[allow(clippy::type_complexity)]
pub struct StreamResolveProviders {
    /// Player.js signature timestamp for clients with `useSignatureTimestamp`.
    pub signature_timestamp: Option<Arc<dyn Fn() -> BoxFuture<Option<i64>> + Send + Sync>>,
    /// (cipherText, playerJs) → URL.
    pub cipher_url:
        Option<Arc<dyn Fn(String, String) -> BoxFuture<Result<String, CipherError>> + Send + Sync>>,
    /// Fetches player.js content for cipher resolution.
    pub player_js: Option<Arc<dyn Fn() -> BoxFuture<Result<String, CipherError>> + Send + Sync>>,
    /// Called when a stream URL gets HTTP 403 during validation.
    pub on_stream_rejection: Option<Arc<dyn Fn() -> BoxFuture<()> + Send + Sync>>,
    /// Tokens per videoId; None proceeds tokenless.
    pub po_token_provider:
        Option<Arc<dyn Fn(String) -> BoxFuture<Option<PoTokenResult>> + Send + Sync>>,
    /// Persist hook per resolve. Never fails.
    pub resolved_format_handler:
        Option<Arc<dyn Fn(ResolvedFormatInfo) -> BoxFuture<()> + Send + Sync>>,
}

impl StreamResolveProviders {
    /// Swift `StreamResolveProviders.none`.
    pub fn none() -> Self {
        Self::default()
    }
}

/// Format facts for app persistence.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFormatInfo {
    pub video_id: String,
    pub itag: i64,
    pub mime_type: String,
    pub codecs: String,
    pub bitrate: i64,
    pub content_length: i64,
    pub loudness_db: Option<f64>,
    pub playback_url: Option<String>,
}

// One client, one /player call.
pub struct StreamResolver;

impl StreamResolver {
    /// Resolves the best stream URL for a video with a specific client.
    pub async fn resolve(
        video_id: &str,
        client: YouTubeClient,
        inner_tube: &InnerTubeClient,
        options: &StreamResolveOptions,
        providers: &StreamResolveProviders,
    ) -> Result<PlaybackResult, StreamError> {
        SwiftyTubeLog::debug(&format!(
            "Resolving videoId={video_id} client={} v{}",
            client.client_name, client.client_version
        ));

        let mut signature_timestamp = None;
        if client.use_signature_timestamp
            && let Some(provider) = &providers.signature_timestamp
        {
            signature_timestamp = provider().await;
            if let Some(sts) = signature_timestamp {
                SwiftyTubeLog::debug(&format!("Using signatureTimestamp={sts}"));
            }
        }

        let response: PlayerResponse = inner_tube
            .player_response(PlayerArgs {
                video_id: video_id.to_string(),
                client: Some(client),
                options: PlayerRequestOptions {
                    signature_timestamp,
                    po_token: options.po_token.clone(),
                },
                ..PlayerArgs::default()
            })
            .await?;

        let Some(playability_status) = &response.playability_status else {
            return Err(StreamError::Unplayable {
                reason: "No playability status in response".into(),
            });
        };

        let status = playability_status.status.as_deref();
        if status != Some("OK") {
            let reason = playability_status.reason.as_deref().unwrap_or("Unknown");
            // Tolerate non-OK clients that still return formats.
            if !client.skip_player_response_validation {
                SwiftyTubeLog::error(&format!(
                    "Not playable: status={} reason={reason}",
                    status.unwrap_or("?")
                ));
                return Err(StreamError::Unplayable {
                    reason: reason.to_string(),
                });
            }
            SwiftyTubeLog::notice(&format!(
                "Status={} ({reason}) - attempting formats anyway",
                status.unwrap_or("?")
            ));
        } else {
            SwiftyTubeLog::debug("Playable: status=OK");
        }

        let Some(streaming_data) = &response.streaming_data else {
            return Err(StreamError::NoStreams);
        };

        let adaptive_formats = streaming_data
            .adaptive_formats
            .as_deref()
            .unwrap_or_default();
        SwiftyTubeLog::debug(&format!(
            "Got {} adaptive formats, expiresIn={}s",
            adaptive_formats.len(),
            streaming_data.expires_in_seconds.as_deref().unwrap_or("?")
        ));

        let all_formats: Vec<Format> = streaming_data
            .formats
            .iter()
            .flatten()
            .chain(adaptive_formats)
            .cloned()
            .collect();
        let max_video_height = all_formats
            .iter()
            .filter_map(|f| f.height)
            .max()
            .unwrap_or(0);
        let has_video_content = max_video_height >= 480;
        SwiftyTubeLog::debug(&format!(
            "hasVideoContent={has_video_content} (maxVideoHeight={max_video_height}, video formats: {})",
            all_formats.iter().filter(|f| f.width.is_some()).count()
        ));

        let fresh = options
            .preferred_format
            .as_ref()
            .and_then(|preferred| adaptive_formats.iter().find(|f| f.itag == preferred.itag));
        let download_format = if fresh.is_none() && options.for_download {
            FormatSelector::best_download_format(&all_formats, options.download_quality, 96000)
        } else {
            None
        };
        let selected_format = if let Some(fresh) = fresh {
            SwiftyTubeLog::debug(&format!(
                "Using preferred format itag={}",
                fresh.itag.unwrap_or(0)
            ));
            fresh
        } else if let Some(download_format) = download_format {
            download_format
        } else if let Some(best) =
            FormatSelector::best_audio_format(&all_formats, options.audio_quality)
        {
            best
        } else {
            let format_infos: Vec<String> = all_formats
                .iter()
                .map(|f| {
                    format!(
                        "itag={} mime={} audio={} url={} cipher={}",
                        f.itag.unwrap_or(0),
                        f.mime_type.as_deref().unwrap_or("?"),
                        f.audio_channels.is_some(),
                        f.url.is_some(),
                        f.signature_cipher.is_some() || f.cipher.is_some()
                    )
                })
                .collect();
            SwiftyTubeLog::error(&format!(
                "No suitable format. Formats: {}",
                format_infos.join(", ")
            ));
            return Err(StreamError::NoSuitableFormat);
        };

        let stream_url = if let Some(direct_url) =
            selected_format.url.as_deref().filter(|u| !u.is_empty())
        {
            SwiftyTubeLog::debug(&format!("Direct URL: {}...", prefix(direct_url, 120)));
            direct_url.to_string()
        } else if let Some(cipher_text) = selected_format
            .signature_cipher
            .as_ref()
            .or(selected_format.cipher.as_ref())
        {
            SwiftyTubeLog::debug("Format requires cipher deobfuscation");
            let (Some(cipher_url), Some(player_js)) = (&providers.cipher_url, &providers.player_js)
            else {
                return Err(StreamError::NoStreamUrl);
            };
            let player_js_content = player_js().await?;
            let url = cipher_url(cipher_text.clone(), player_js_content).await?;
            SwiftyTubeLog::debug(&format!("Resolved cipher URL: {}...", prefix(&url, 120)));
            url
        } else {
            return Err(StreamError::NoStreamUrl);
        };

        let mut final_stream_url = stream_url;
        let pot = options
            .streaming_data_po_token
            .as_deref()
            .filter(|_| client.use_web_po_tokens);
        if let Some(pot) = pot {
            final_stream_url = Self::appending_po_token(&final_stream_url, pot);
            SwiftyTubeLog::debug("Appended &pot= to stream URL");
        }

        let video_details = response.video_details.as_ref();
        let duration = video_details
            .and_then(|d| d.length_seconds.as_deref())
            .and_then(|s| s.parse::<i64>().ok());
        let music_video_type = video_details.and_then(|d| d.music_video_type.clone());

        let muxed_format = FormatSelector::best_video_format(&all_formats, options.video_quality);
        let muxed_stream_url = muxed_format
            .and_then(|f| f.url.as_deref())
            .filter(|u| !u.is_empty())
            .map(|url| match pot {
                Some(pot) => Self::appending_po_token(url, pot),
                None => url.to_string(),
            });

        let result = PlaybackResult {
            stream_url: final_stream_url,
            itag: selected_format.itag.unwrap_or(0),
            mime_type: selected_format.mime_type.clone().unwrap_or_default(),
            bitrate: selected_format.bitrate.unwrap_or(0),
            audio_quality: selected_format.audio_quality.clone().unwrap_or_default(),
            video_id: video_details
                .and_then(|d| d.video_id.clone())
                .unwrap_or_else(|| video_id.to_string()),
            title: video_details.and_then(|d| d.title.clone()),
            author: video_details.and_then(|d| d.author.clone()),
            duration,
            expires_in_seconds: streaming_data.expires_in_seconds_value().unwrap_or(0),
            client_name: client.client_name.to_string(),
            music_video_type,
            has_video_content,
            muxed_stream_url,
            muxed_width: muxed_format.and_then(|f| f.width),
            muxed_height: muxed_format.and_then(|f| f.height),
            loudness_db: selected_format.loudness_db,
        };
        SwiftyTubeLog::debug(&format!(
            "musicVideoType={}",
            result.music_video_type.as_deref().unwrap_or("nil")
        ));

        if let (Some(duration), Some(resolved_video_id)) =
            (duration, video_details.and_then(|d| d.video_id.as_deref()))
        {
            DurationCache::set(resolved_video_id, duration);
        }

        if let Some(handler) = &providers.resolved_format_handler {
            handler(ResolvedFormatInfo {
                video_id: result.video_id.clone(),
                itag: result.itag,
                mime_type: result.mime_type.clone(),
                codecs: selected_format.codec(),
                bitrate: result.bitrate,
                content_length: selected_format
                    .content_length
                    .as_deref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
                loudness_db: result.loudness_db,
                playback_url: response
                    .playback_tracking
                    .as_ref()
                    .and_then(|t| t.videostats_playback_url.as_ref())
                    .and_then(|u| u.base_url.clone()),
            })
            .await;
        }

        SwiftyTubeLog::debug(&format!(
            "Result: title=\"{}\" author=\"{}\" itag={} quality={} bitrate={}",
            result.title.as_deref().unwrap_or("?"),
            result.author.as_deref().unwrap_or("?"),
            result.itag,
            result.audio_quality,
            result.bitrate
        ));

        Ok(result)
    }

    /// Query-escape incl. +, =, &, /. Swift keeps only `.alphanumerics` unescaped.
    pub fn encode_query_value(value: &str) -> String {
        value
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect()
    }

    /// Appends the po_token with the right separator (cipher-resolved URLs
    /// don't always carry a query string).
    pub fn appending_po_token(url: &str, pot: &str) -> String {
        let separator = if url.contains('?') { '&' } else { '?' };
        format!("{url}{separator}pot={}", Self::encode_query_value(pot))
    }

    /// 1 KiB Range GET check.
    pub async fn validate_stream(
        url: &str,
        session: &Session,
        on_stream_rejection: Option<&Arc<dyn Fn() -> BoxFuture<()> + Send + Sync>>,
    ) -> bool {
        let Ok(url) = reqwest::Url::parse(url) else {
            SwiftyTubeLog::error("Invalid URL for validation");
            return false;
        };

        let mut request = reqwest::Request::new(reqwest::Method::GET, url);
        request
            .headers_mut()
            .insert("Range", "bytes=0-1023".parse().expect("valid header"));

        let response =
            match tokio::time::timeout(Duration::from_secs(5), HttpClient::data(request, session))
                .await
            {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    SwiftyTubeLog::error(&format!("Range validation failed: {error}"));
                    return false;
                }
                Err(_) => {
                    SwiftyTubeLog::error("Range validation failed: timed out");
                    return false;
                }
            };
        let valid = (200..=299).contains(&response.status);
        if !valid {
            SwiftyTubeLog::debug(&format!(
                "Range validation: status={} valid=false",
                response.status
            ));
        }
        if response.status == 403
            && let Some(on_stream_rejection) = on_stream_rejection
        {
            on_stream_rejection().await;
        }
        valid
    }
}

fn prefix(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map_or(s, |(i, _)| &s[..i])
}
