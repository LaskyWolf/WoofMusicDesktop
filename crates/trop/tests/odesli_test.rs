//! Odesli, offline: a mock session answers from `crates/fixtures/odesli/` and
//! keyring-core's mock store stands in for the system keyring.

use std::sync::{Arc, Mutex};

use swiftytube::networking::http_client::{HttpResponse, Session};
use trop::odesli::odesli::{Odesli, OdesliConnectError};
use trop::odesli::odesli_service::{OdesliError, OdesliService};
use trop::utilities::music_link::MusicLink;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../fixtures/odesli/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Answers every request with `status` and `data`, recording its URL.
fn session(status: u16, data: Vec<u8>) -> (Session, Arc<Mutex<Vec<String>>>) {
    let urls = Arc::new(Mutex::new(Vec::new()));
    let log = urls.clone();
    let session = Session::mock(move |request| {
        log.lock().unwrap().push(request.url().to_string());
        HttpResponse {
            status,
            data: data.clone(),
            ..HttpResponse::default()
        }
    });
    (session, urls)
}

#[test]
fn links_url_encodes_the_link_and_plus() {
    assert_eq!(
        OdesliService::links_url("https://music.apple.com/us/album/a+b?i=1&x=y", "k+1").as_str(),
        "https://api.song.link/v1-alpha.1/links?url=https%3A%2F%2Fmusic.apple.com%2Fus%2Falbum%2Fa%2Bb%3Fi%3D1%26x%3Dy&songIfSingle=true&key=k%2B1"
    );
}

#[test]
fn youtube_link_prefers_youtube_music_then_youtube() {
    assert_eq!(
        Odesli::youtube_link(&fixture("links_youtube")).as_deref(),
        Some("https://music.youtube.com/watch?v=FGBhQbmPwH8")
    );
    assert_eq!(
        Odesli::youtube_link(&fixture("links_youtube_only")).as_deref(),
        Some("https://www.youtube.com/watch?v=FGBhQbmPwH8")
    );
    assert_eq!(Odesli::youtube_link(br#"{"linksByPlatform": {}}"#), None);
    assert_eq!(Odesli::youtube_link(b"not json"), None);
}

#[test]
fn only_spotify_and_apple_music_links_go_to_odesli() {
    assert!(Odesli::is_convertible(
        "https://open.spotify.com/track/0DiWol3AO6WpXZgp0goxAV"
    ));
    assert!(Odesli::is_convertible(
        "https://MUSIC.apple.com/us/album/x/1"
    ));
    assert!(Odesli::is_convertible("http://geo.music.apple.com/album/1"));
    assert!(!Odesli::is_convertible(
        "spotify:track:0DiWol3AO6WpXZgp0goxAV"
    ));
    assert!(!Odesli::is_convertible(
        "https://music.youtube.com/watch?v=FGBhQbmPwH8"
    ));
    assert!(!Odesli::is_convertible("not a link"));
}

#[tokio::test]
async fn links_maps_status_codes() {
    for (status, expected) in [
        (401, OdesliError::InvalidKey),
        (403, OdesliError::InvalidKey),
        (429, OdesliError::RateLimited),
        (404, OdesliError::NotFound),
    ] {
        let (session, _) = session(status, Vec::new());
        let refused = expected == OdesliError::InvalidKey;
        assert_eq!(
            OdesliService::links(&session, "https://open.spotify.com/track/x", "k").await,
            Err(expected)
        );
        // Only a refused key fails validation.
        assert_eq!(
            OdesliService::validate(&session, "k").await.is_err(),
            refused
        );
    }
}

#[tokio::test]
async fn connect_validates_with_the_test_song_then_converts() {
    keyring_core::set_default_store(keyring_core::mock::Store::new().unwrap());

    let (refusing, _) = session(401, Vec::new());
    let mut odesli = Odesli::new(refusing);
    assert!(!odesli.is_connected());
    assert_eq!(
        odesli.convert("https://open.spotify.com/track/x").await,
        Err(OdesliError::NotConnected)
    );
    assert!(matches!(
        odesli.connect("bad").await,
        Err(OdesliConnectError::Service(OdesliError::InvalidKey))
    ));
    assert!(!odesli.is_connected());

    let (accepting, urls) = session(200, fixture("links_youtube"));
    let mut odesli = Odesli::new(accepting.clone());
    odesli.connect("  key1  ").await.unwrap();
    assert!(odesli.is_connected());
    assert!(
        urls.lock().unwrap()[0]
            .contains("url=https%3A%2F%2Fopen.spotify.com%2Ftrack%2F4cOdK2wGLETKBW3PvgPWqT")
    );
    assert!(urls.lock().unwrap()[0].ends_with("&key=key1"));
    assert_eq!(
        odesli
            .convert("https://open.spotify.com/track/0DiWol3AO6WpXZgp0goxAV")
            .await,
        Ok(MusicLink::Song("FGBhQbmPwH8".into()))
    );

    // The key survives a restart, until disconnect.
    let mut reloaded = Odesli::new(accepting);
    assert!(reloaded.is_connected());
    reloaded.disconnect();
    assert!(!Odesli::new(session(200, Vec::new()).0).is_connected());
}
