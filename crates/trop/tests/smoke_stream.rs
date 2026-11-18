//! Live, anonymous smoke test of R3 (no cookies, no account), like the Dart
//! `tool/smoke_stream.dart`. Resolves 10 songs from the Home fixture through the
//! client fallback chain, checks each stream URL with a ranged GET, forces
//! WEB_REMIX once to exercise the QuickJS cipher, then plays the first song
//! muted through the libmpv deck and checks that `time-pos` advances.
//!
//! cargo test -p trop --test smoke_stream -- --ignored --nocapture

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use swiftytube::auth::auth_state::AuthState;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::core::swiftytube_log::SwiftyTubeLog;
use swiftytube::core::youtube_client::YouTubeClient;
use swiftytube::networking::inner_tube_client::InnerTubeClient;
use swiftytube::streaming::stream_cache::StreamCache;
use swiftytube::streaming::stream_fallback::{StreamFallback, StreamResolveRequest};
use swiftytube::streaming::stream_resolver::{
    StreamResolveOptions, StreamResolveProviders, StreamResolver,
};
use trop::audio::player_controller::player_controller::{PlayerController, State};

/// The first `count` distinct `videoId`s in the Home fixture.
fn home_video_ids(count: usize) -> Vec<String> {
    let text = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/trop/home.json"
    ))
    .unwrap();
    let home: Value = serde_json::from_slice(&text).unwrap();
    let mut ids = Vec::new();
    let mut stack = vec![&home];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                if let Some(Value::String(id)) = map.get("videoId")
                    && !ids.contains(id)
                {
                    ids.push(id.clone());
                }
                stack.extend(map.values().rev());
            }
            Value::Array(list) => stack.extend(list.iter().rev()),
            _ => {}
        }
    }
    ids.truncate(count);
    ids
}

async fn range_status(url: &str) -> u16 {
    reqwest::Client::new()
        .get(url)
        .header("Range", "bytes=0-1023")
        .send()
        .await
        .map_or(0, |r| r.status().as_u16())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs network and libmpv with an audio output"]
async fn smoke_resolve_range_check_and_play() {
    SwiftyTubeLog::set_handler(Some(Arc::new(|message, _| {
        if message.contains("ipher") || message.contains("QuickJS") || message.contains("failed") {
            println!("  log: {}", message.chars().take(160).collect::<String>());
        }
    })));

    let client = InnerTubeClient::new(SwiftyTubeConfig::music(), AuthState::guest());
    // VISIONOS answers anonymous /player calls only with a visitorData.
    client.ensure_visitor_data(None, None).await.unwrap();
    let providers = StreamResolveProviders::live();

    let ids = home_video_ids(10);
    assert_eq!(ids.len(), 10);
    let mut urls = Vec::new();
    let mut passed = 0;
    println!("| videoId | client | itag | mime | range GET |");
    println!("|---|---|---|---|---|");
    for id in &ids {
        let request = StreamResolveRequest::playback(id, StreamResolveOptions::default());
        match StreamFallback::resolve_first_valid(
            &request,
            &client,
            StreamCache::shared(),
            &providers,
        )
        .await
        {
            Ok(result) => {
                let status = range_status(&result.stream_url).await;
                if status == 206 {
                    passed += 1;
                }
                println!(
                    "| {id} | {} | {} | {} | {status} |",
                    result.client_name, result.itag, result.mime_type
                );
                urls.push((id.clone(), result.stream_url));
            }
            Err(error) => println!("| {id} | FAILED: {error} | | | |"),
        }
    }
    println!("passed: {passed}/{}", ids.len());

    // Cipher path on its own: WEB_REMIX returns signatureCipher formats.
    match StreamResolver::resolve(
        &ids[0],
        YouTubeClient::WEB_REMIX,
        &client,
        &StreamResolveOptions::default(),
        &providers,
    )
    .await
    {
        Ok(result) => println!(
            "WEB_REMIX forced: itag={} mime={} range GET={}",
            result.itag,
            result.mime_type,
            range_status(&result.stream_url).await
        ),
        Err(error) => println!("WEB_REMIX forced: FAILED: {error}"),
    }

    // Play the first stream muted for ~3 s.
    let (id, url) = urls.first().expect("one resolved stream").clone();
    let played = tokio::task::spawn_blocking(move || {
        let player = PlayerController::new().expect("libmpv starts");
        player.active_deck().set("mute", "yes").unwrap();
        player.play(&url, Some(&id), None).unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while player.current_time() <= 0.0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        let start = player.current_time();
        std::thread::sleep(Duration::from_secs(3));
        let end = player.current_time();
        println!(
            "playback: state={:?} videoId={:?} time-pos {start:.2} -> {end:.2} duration={:.0}",
            player.play_state(),
            player.current_video_id(),
            player.duration()
        );
        player.set_paused(true);
        let paused = player.play_state();
        player.stop();
        (start, end, paused, player.play_state())
    })
    .await
    .unwrap();
    let (start, end, paused, stopped) = played;

    assert_eq!(passed, ids.len());
    assert!(end - start > 2.0, "time-pos advanced {start} -> {end}");
    assert_eq!(paused, State::Paused);
    assert_eq!(stopped, State::Stopped);
}
