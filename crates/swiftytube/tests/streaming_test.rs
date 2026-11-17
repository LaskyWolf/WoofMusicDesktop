//! Port of the Dart `test/swiftytube/streaming_test.dart`, plus `fetch_duration`.

use swiftytube::auth::auth_state::AuthState;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::core::youtube_client::YouTubeClient;
use swiftytube::networking::http_client::{HttpResponse, Session};
use swiftytube::networking::inner_tube_client::InnerTubeClient;
use swiftytube::streaming::client_fallback_chain::ClientFallbackChain;
use swiftytube::streaming::duration_cache::DurationCache;
use swiftytube::streaming::format_selector::FormatSelector;
use swiftytube::streaming::playback_quality::{AudioQuality, DownloadQuality, VideoQuality};
use swiftytube::streaming::player_response::{Format, PlayerResponse};
use swiftytube::streaming::stream_fallback::StreamFallback;
use swiftytube::streaming::stream_resolver::{
    StreamResolveOptions, StreamResolveProviders, StreamResolver,
};

/// Anonymized VISIONOS /player response ("Beat It", art track): URLs replaced, no visitorData.
fn fixture_text() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/player_visionos.json"
    ))
    .unwrap()
}

fn fixture() -> PlayerResponse {
    serde_json::from_slice(&fixture_text()).unwrap()
}

fn mocked_inner_tube() -> InnerTubeClient {
    let body = fixture_text();
    let session = Session::mock(move |_| HttpResponse {
        status: 200,
        data: body.clone(),
        ..HttpResponse::default()
    });
    InnerTubeClient::with_session(SwiftyTubeConfig::music(), AuthState::guest(), session)
}

fn muxed(itag: i64, width: i64, height: i64, mime_type: &str) -> Format {
    Format {
        itag: Some(itag),
        width: Some(width),
        height: Some(height),
        audio_channels: Some(2),
        mime_type: Some(mime_type.into()),
        url: Some("u".into()),
        ..Format::default()
    }
}

#[test]
fn parses_a_real_visionos_response() {
    let response = fixture();
    assert!(response.is_playable());
    let details = response.video_details.as_ref().unwrap();
    assert_eq!(details.video_id.as_deref(), Some("WlTlUseVt7E"));
    assert_eq!(details.length_seconds.as_deref(), Some("258"));
    let streaming = response.streaming_data.as_ref().unwrap();
    assert_eq!(streaming.expires_in_seconds_value(), Some(21540));
    let audio = streaming
        .adaptive_formats
        .as_ref()
        .unwrap()
        .iter()
        .find(|f| f.itag == Some(251))
        .unwrap();
    assert!(audio.is_audio_only());
    assert_eq!(audio.codec(), "opus");
    assert_eq!(audio.audio_quality.as_deref(), Some("AUDIO_QUALITY_MEDIUM"));
    assert!((audio.loudness_db.unwrap() - 4.44).abs() < 0.01);
    assert!(audio.url.as_deref().unwrap().starts_with("https://"));
    let tracking = response.playback_tracking.as_ref().unwrap();
    assert!(
        tracking
            .videostats_playback_url
            .as_ref()
            .unwrap()
            .base_url
            .is_some()
    );
}

#[test]
fn codec_falls_back_to_the_mime_type_without_codecs() {
    let format = Format {
        mime_type: Some("audio/mp4".into()),
        ..Format::default()
    };
    assert_eq!(format.codec(), "audio/mp4");
    assert_eq!(Format::default().codec(), "");
}

#[test]
fn a_field_with_the_wrong_json_type_fails_like_a_swift_decode() {
    let result =
        serde_json::from_str::<PlayerResponse>(r#"{"videoDetails":{"lengthSeconds":258}}"#);
    assert!(result.is_err());
}

#[test]
fn best_audio_prefers_quality_tier_then_opus_over_aac() {
    let formats = fixture().streaming_data.unwrap().adaptive_formats.unwrap();
    let itag = |q| FormatSelector::best_audio_format(&formats, q).and_then(|f| f.itag);
    assert_eq!(itag(AudioQuality::Auto), Some(251));
    assert_eq!(itag(AudioQuality::Low), Some(250));
    // No HIGH tier in this response: falls back to the best available.
    assert_eq!(itag(AudioQuality::High), Some(251));
}

#[test]
fn download_prefers_aac_standard_caps_bitrate_and_picks_the_smallest() {
    let formats = fixture().streaming_data.unwrap().adaptive_formats.unwrap();
    let itag = |q| FormatSelector::best_download_format(&formats, q, 96000).and_then(|f| f.itag);
    assert_eq!(itag(DownloadQuality::High), Some(140));
    assert_eq!(itag(DownloadQuality::Standard), Some(139));
}

#[test]
fn falls_back_to_muxed_formats_when_no_audio_only_exists() {
    let formats = [
        muxed(18, 640, 360, "video/mp4; codecs=\"avc1, mp4a\""),
        muxed(22, 1280, 720, "video/mp4; codecs=\"avc1, mp4a\""),
    ];
    let best = FormatSelector::best_audio_format(&formats, AudioQuality::Auto);
    assert_eq!(best.and_then(|f| f.itag), Some(18));
    let video_only = [Format {
        itag: Some(1),
        width: Some(1),
        ..Format::default()
    }];
    assert!(FormatSelector::best_audio_format(&video_only, AudioQuality::Auto).is_none());
}

#[test]
fn video_picks_highest_muxed_capped_by_preference() {
    let formats = [
        muxed(18, 640, 360, "video/mp4"),
        muxed(22, 1280, 720, "video/mp4"),
    ];
    let itag = |q| FormatSelector::best_video_format(&formats, q).and_then(|f| f.itag);
    assert_eq!(itag(VideoQuality::Auto), Some(22));
    assert_eq!(itag(VideoQuality::P480), Some(18));
    // Nothing at or below 240: smallest above it.
    assert_eq!(itag(VideoQuality::P240), Some(18));
}

#[test]
fn video_only_prefers_h264() {
    let formats = fixture().streaming_data.unwrap().adaptive_formats.unwrap();
    let video_only = FormatSelector::best_video_only_format(&formats).unwrap();
    assert!(video_only.codec().starts_with("avc"));
    assert!(video_only.audio_channels.is_none());
}

#[test]
fn appending_po_token_escapes_everything_but_alphanumerics() {
    assert_eq!(
        StreamResolver::appending_po_token("https://x/v?a=1", "a+b/c="),
        "https://x/v?a=1&pot=a%2Bb%2Fc%3D"
    );
    assert_eq!(
        StreamResolver::appending_po_token("https://x/v", "ab"),
        "https://x/v?pot=ab"
    );
}

#[tokio::test]
async fn resolve_picks_the_best_audio_from_a_mocked_player() {
    let inner_tube = mocked_inner_tube();
    let result = StreamResolver::resolve(
        "WlTlUseVt7E",
        YouTubeClient::VISION_OS,
        &inner_tube,
        &StreamResolveOptions::default(),
        &StreamResolveProviders::none(),
    )
    .await
    .unwrap();
    assert_eq!(result.itag, 251);
    assert_eq!(result.client_name, "VISIONOS");
    assert_eq!(result.duration, Some(258));
    assert!(result.has_video_content);
    assert!((result.loudness_db.unwrap() - 4.44).abs() < 0.01);
    assert_eq!(DurationCache::get("WlTlUseVt7E"), Some(258));
}

#[tokio::test]
async fn resolve_duration_reads_length_seconds_and_caches_it() {
    let inner_tube = mocked_inner_tube();
    assert_eq!(inner_tube.fetch_duration("WlTlUseVt7E").await.unwrap(), 258);
    assert_eq!(inner_tube.resolve_duration("WlTlUseVt7E").await, Some(258));
}

#[test]
fn preferred_chain_mirrors_swift_order() {
    let names: Vec<&str> = ClientFallbackChain::preferred()
        .iter()
        .map(|e| e.client.client_name)
        .collect();
    assert_eq!(
        names,
        [
            "VISIONOS",
            "ANDROID_VR",
            "ANDROID_VR",
            "ANDROID_VR",
            "TVHTML5_SIMPLY_EMBEDDED_PLAYER",
            "IOS",
            "TVHTML5",
            "ANDROID",
            "WEB_REMIX",
        ]
    );
}

#[test]
fn edl_combines_tracks_with_byte_lengths() {
    assert_eq!(
        StreamFallback::combine_video_and_audio("v", "aa", Some(3)),
        "edl://!new_stream;!no_clip;!no_chapters;%1%v,length=3;!new_stream;!no_clip;!no_chapters;%2%aa,length=3"
    );
}
