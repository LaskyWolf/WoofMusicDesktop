use super::client_fallback_chain::{ClientFallbackChain, FallbackClient};
use super::format_selector::FormatSelector;
use super::playback_quality::VideoQuality;
use super::playback_result::PlaybackResult;
use super::player_request_options::PlayerRequestOptions;
use super::player_response::{Format, PlayerResponse};
use super::stream_cache::StreamCache;
use super::stream_error::StreamError;
use super::stream_resolver::{StreamResolveOptions, StreamResolveProviders, StreamResolver};
use crate::core::swiftytube_log::SwiftyTubeLog;
use crate::networking::http_client::Session;
use crate::networking::inner_tube_client::{InnerTubeClient, PlayerArgs};
use crate::po_token::po_token_result::PoTokenResult;

/// Loop behavior for `StreamFallback::resolve_first_valid`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FallbackPolicy {
    /// Return a fresh cache entry, no network.
    pub use_cache: bool,
    /// Store hits in the cache.
    pub store_in_cache: bool,
    /// Validate URLs (honor `skip_validation`).
    pub validate_streams: bool,
    /// Expired-cache fallback on total failure.
    pub expired_cache_fallback: bool,
    /// Prefer AAC, keep first non-AAC aside.
    pub prefer_aac: bool,
}

impl FallbackPolicy {
    /// Playback: cache, validate, expired-cache fallback.
    pub const PLAYBACK: Self = Self {
        use_cache: true,
        store_in_cache: true,
        validate_streams: true,
        expired_cache_fallback: true,
        prefer_aac: false,
    };

    /// Download: no cache, validate, AAC preferred with non-AAC fallback.
    pub const DOWNLOAD: Self = Self {
        use_cache: false,
        store_in_cache: false,
        validate_streams: true,
        expired_cache_fallback: false,
        prefer_aac: true,
    };
}

/// What to resolve, in what order, how to behave.
#[derive(Clone, Debug)]
pub struct StreamResolveRequest {
    pub video_id: String,
    pub chain: Vec<FallbackClient>,
    pub options: StreamResolveOptions,
    pub policy: FallbackPolicy,
}

impl StreamResolveRequest {
    /// Playback request for a video.
    pub fn playback(video_id: impl Into<String>, options: StreamResolveOptions) -> Self {
        Self {
            video_id: video_id.into(),
            chain: ClientFallbackChain::preferred(),
            options,
            policy: FallbackPolicy::PLAYBACK,
        }
    }

    /// Download request for a video.
    pub fn download(video_id: impl Into<String>, options: StreamResolveOptions) -> Self {
        Self {
            video_id: video_id.into(),
            chain: ClientFallbackChain::for_download(),
            options: StreamResolveOptions {
                for_download: true,
                ..options
            },
            policy: FallbackPolicy::DOWNLOAD,
        }
    }
}

/// Muxed URL or video+audio pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoStream {
    Muxed(String),
    Split {
        video: String,
        audio: String,
        duration: Option<i64>,
    },
    Hls(String),
}

/// Shared fallback loops (player wiring stays in apps).
pub struct StreamFallback;

impl StreamFallback {
    /// First valid stream across the chain. PoTokens minted lazily.
    /// `cache`: Swift default `.shared`.
    pub async fn resolve_first_valid(
        request: &StreamResolveRequest,
        inner_tube: &InnerTubeClient,
        cache: &StreamCache,
        providers: &StreamResolveProviders,
    ) -> Result<PlaybackResult, StreamError> {
        if request.policy.use_cache
            && let Some(cached) = cache.get(&request.video_id)
        {
            return Ok(cached);
        }

        // ponytail: Swift starts minting in a Task before the loop; here the
        // token is minted when the first token client is reached, then reused.
        // Spawn it ahead when a real PoToken generator exists.
        let wants_po_token = request.chain.iter().any(|e| e.client.use_web_po_tokens);
        let po_token_provider = providers
            .po_token_provider
            .as_ref()
            .filter(|_| wants_po_token);
        let mut minted: Option<Option<PoTokenResult>> = None;

        let mut last_error = None;
        let mut non_aac_fallback: Option<PlaybackResult> = None;

        for entry in &request.chain {
            let mut options = request.options.clone();
            if entry.client.use_web_po_tokens
                && let Some(provider) = po_token_provider
            {
                if minted.is_none() {
                    minted = Some(provider(request.video_id.clone()).await);
                }
                if let Some(Some(token)) = &minted {
                    options.po_token = Some(token.player_request_po_token.clone());
                    options.streaming_data_po_token = Some(token.streaming_data_po_token.clone());
                }
            }

            let result = match StreamResolver::resolve(
                &request.video_id,
                entry.client,
                inner_tube,
                &options,
                providers,
            )
            .await
            {
                Ok(result) => result,
                Err(error) => {
                    SwiftyTubeLog::error(&format!("{} failed: {error}", entry.client.client_name));
                    last_error = Some(error);
                    continue;
                }
            };

            if request.policy.validate_streams && !entry.skip_validation {
                let valid = StreamResolver::validate_stream(
                    &result.stream_url,
                    &Session::shared(),
                    providers.on_stream_rejection.as_ref(),
                )
                .await;
                if !valid {
                    SwiftyTubeLog::debug(&format!(
                        "{} Range validation failed, trying next",
                        result.client_name
                    ));
                    last_error = Some(StreamError::ValidationFailed(result.client_name));
                    continue;
                }
            }

            if request.policy.prefer_aac {
                let mime = result.mime_type.to_lowercase();
                if !(mime.contains("mp4a") || mime.contains("aac")) {
                    SwiftyTubeLog::debug(&format!(
                        "{} returned non-AAC ({})",
                        result.client_name, result.mime_type
                    ));
                    non_aac_fallback.get_or_insert(result);
                    continue;
                }
            } else if request.policy.store_in_cache {
                cache.set(&request.video_id, result.clone());
            }
            return Ok(result);
        }

        if request.policy.prefer_aac
            && let Some(fallback) = non_aac_fallback
        {
            SwiftyTubeLog::debug(&format!(
                "No AAC stream found - falling back to {}",
                fallback.mime_type
            ));
            return Ok(fallback);
        }

        if request.policy.expired_cache_fallback
            && let Some(expired) = cache.get_expired(&request.video_id)
        {
            SwiftyTubeLog::notice(&format!(
                "All clients failed for {}, falling back to expired cache",
                request.video_id
            ));
            return Ok(expired);
        }
        Err(last_error.unwrap_or(StreamError::AllClientsFailed))
    }

    /// Muxed URL or split pair. Fails with `NoSuitableFormat`.
    pub async fn resolve_video(
        request: &StreamResolveRequest,
        inner_tube: &InnerTubeClient,
        providers: &StreamResolveProviders,
    ) -> Result<VideoStream, StreamError> {
        let mut last_error = None;

        for entry in &request.chain {
            match Self::resolve_video_with(request, entry, inner_tube, providers).await {
                Ok(Some(stream)) => return Ok(stream),
                Ok(None) => {}
                Err(error) => {
                    SwiftyTubeLog::error(&format!(
                        "Video resolution failed for {}: {error}",
                        entry.client.client_name
                    ));
                    last_error = Some(error);
                }
            }
        }

        Err(last_error.unwrap_or(StreamError::NoSuitableFormat))
    }

    /// One client of `resolve_video` (Swift's loop body). Ok(None): try the next client.
    async fn resolve_video_with(
        request: &StreamResolveRequest,
        entry: &FallbackClient,
        inner_tube: &InnerTubeClient,
        providers: &StreamResolveProviders,
    ) -> Result<Option<VideoStream>, StreamError> {
        let signature_timestamp = match &providers.signature_timestamp {
            Some(provider) if entry.client.use_signature_timestamp => provider().await,
            _ => None,
        };

        let response: PlayerResponse = inner_tube
            .player_response(PlayerArgs {
                video_id: request.video_id.clone(),
                client: Some(entry.client),
                options: PlayerRequestOptions {
                    signature_timestamp,
                    po_token: request.options.po_token.clone(),
                },
                ..PlayerArgs::default()
            })
            .await?;

        let Some(streaming_data) = &response.streaming_data else {
            return Err(StreamError::NoStreams);
        };

        if let Some(status) = response
            .playability_status
            .as_ref()
            .and_then(|s| s.status.as_deref())
            .filter(|s| *s != "OK")
        {
            let reason = response
                .playability_status
                .as_ref()
                .and_then(|s| s.reason.clone())
                .unwrap_or_else(|| status.to_string());
            return Err(StreamError::Unplayable { reason });
        }

        // Keep HLS as a fallback. VOD responses can expose both an
        // HLS manifest and direct formats; direct URLs start much
        // faster in embedded players. Live responses often expose
        // only HLS, so it remains available below.
        let hls_url = streaming_data.hls_manifest_url.clone();

        let all_formats: Vec<Format> = streaming_data
            .formats
            .iter()
            .chain(&streaming_data.adaptive_formats)
            .flatten()
            .cloned()
            .collect();

        if let Some(muxed_url) = FormatSelector::best_video_format(&all_formats, VideoQuality::Auto)
            .and_then(|f| f.url.clone())
        {
            return Ok(Some(VideoStream::Muxed(muxed_url)));
        }

        let video_url =
            FormatSelector::best_video_only_format(&all_formats).and_then(|f| f.url.clone());
        let audio = FormatSelector::best_audio_format(&all_formats, request.options.audio_quality);
        let mut split_error = None;
        if let (Some(video), Some(audio)) = (video_url, audio) {
            match Self::resolve_format_url(audio, providers).await {
                Ok(audio) => {
                    let duration = response
                        .video_details
                        .as_ref()
                        .and_then(|d| d.length_seconds.as_deref())
                        .and_then(|s| s.parse().ok());
                    return Ok(Some(VideoStream::Split {
                        video,
                        audio,
                        duration,
                    }));
                }
                Err(error) => split_error = Some(error),
            }
        }

        if let Some(hls_url) = hls_url.filter(|u| !u.is_empty()) {
            return Ok(Some(VideoStream::Hls(hls_url)));
        }
        // Swift keeps the split error as `lastError` and moves on.
        match split_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    /// Resolves a single format's stream URL, going through the cipher if needed.
    pub async fn resolve_format_url(
        format: &Format,
        providers: &StreamResolveProviders,
    ) -> Result<String, StreamError> {
        if let Some(url) = format.url.as_deref().filter(|u| !u.is_empty()) {
            return Ok(url.to_string());
        }
        let cipher_text = format.signature_cipher.as_ref().or(format.cipher.as_ref());
        if let (Some(cipher_text), Some(cipher_url), Some(player_js)) =
            (cipher_text, &providers.cipher_url, &providers.player_js)
        {
            let player_js = player_js().await?;
            return Ok(cipher_url(cipher_text.clone(), player_js).await?);
        }
        Err(StreamError::NoStreamUrl)
    }

    /// mpv EDL for separate DASH tracks.
    pub fn combine_video_and_audio(
        video_url: &str,
        audio_url: &str,
        duration: Option<i64>,
    ) -> String {
        let length_param = duration.map_or(String::new(), |d| format!(",length={d}"));
        let video = format!("%{}%{video_url}{length_param}", video_url.len());
        let audio = format!("%{}%{audio_url}{length_param}", audio_url.len());
        format!(
            "edl://!new_stream;!no_clip;!no_chapters;{video};!new_stream;!no_clip;!no_chapters;{audio}"
        )
    }
}
