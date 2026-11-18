//! Last.fm and ListenBrainz, offline: a mock session answers from
//! `crates/fixtures/{last_fm,listen_brainz}/` and keyring-core's mock store
//! stands in for the system keyring.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use swiftytube::networking::http_client::{HttpResponse, Session};
use trop::last_fm::last_fm_defaults::{LastFMDefaults, LastFMSettings};
use trop::last_fm::last_fm_integration::{LastFMIntegration, PlayingSong};
use trop::last_fm::last_fm_service::{LastFMError, LastFMService};
use trop::last_fm::last_fm_token_store::LastFMTokenStore;
use trop::listen_brainz::listen_brainz::ListenBrainz;
use trop::listen_brainz::listen_brainz_defaults::ListenBrainzSettings;
use trop::listen_brainz::listen_brainz_service::{ListenBrainzError, ListenBrainzService};

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!("{}/../fixtures/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("trop-scrobbling-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("defaults.json")
}

fn mock_keyring() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| keyring_core::set_default_store(keyring_core::mock::Store::new().unwrap()));
}

/// A request the mock session saw.
struct Sent {
    url: String,
    authorization: Option<String>,
    body: String,
}

impl Sent {
    fn form(&self) -> HashMap<String, String> {
        form_urlencoded::parse(self.body.as_bytes())
            .into_owned()
            .collect()
    }
}

/// Answers every request with `reply(url, body)` and records it.
fn recording_session(
    reply: impl Fn(&str, &str) -> (u16, Vec<u8>) + Send + Sync + 'static,
) -> (Session, Arc<Mutex<Vec<Sent>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let sent = log.clone();
    let session = Session::mock(move |request| {
        let body = request
            .body()
            .and_then(|b| b.as_bytes())
            .map(|b| String::from_utf8(b.to_vec()).unwrap())
            .unwrap_or_default();
        let (status, data) = reply(request.url().as_str(), &body);
        sent.lock().unwrap().push(Sent {
            url: request.url().to_string(),
            authorization: request
                .headers()
                .get("authorization")
                .map(|v| v.to_str().unwrap().to_string()),
            body,
        });
        HttpResponse {
            status,
            data,
            ..HttpResponse::default()
        }
    });
    (session, log)
}

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

fn song(video_id: &str, duration: f64, is_playing: bool) -> PlayingSong {
    PlayingSong {
        video_id: video_id.into(),
        title: "One More Time".into(),
        artist: "Daft Punk".into(),
        album: Some("Discovery".into()),
        duration,
        is_playing,
    }
}

// MARK: - Last.fm signing

fn auth_params() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("method".to_string(), "auth.getSession".to_string()),
        ("api_key".to_string(), "key".to_string()),
        ("token".to_string(), "tok".to_string()),
    ])
}

#[test]
fn last_fm_signature_is_md5_of_sorted_params_and_secret() {
    // md5("api_keykeymethodauth.getSessiontokentoksecret")
    assert_eq!(
        LastFMService::api_sig(&auth_params(), "secret"),
        "04e870be4bb79756721b7bc1937fe83d"
    );
}

#[test]
fn last_fm_form_body_adds_signature_and_unsigned_format() {
    assert_eq!(
        LastFMService::form_body(&auth_params(), "secret"),
        "api_key=key&api_sig=04e870be4bb79756721b7bc1937fe83d&format=json&method=auth.getSession&token=tok"
    );
}

#[test]
fn last_fm_auth_url_names_key_and_token() {
    assert_eq!(
        LastFMDefaults::auth_url("key", "tok"),
        "https://www.last.fm/api/auth/?api_key=key&token=tok"
    );
}

// MARK: - Last.fm responses

fn configured_service(session: Session, name: &str) -> LastFMService {
    let mut service = LastFMService::new(session, LastFMTokenStore::new(temp_path(name)));
    service.api_key = "key".into();
    service.secret = "secret".into();
    service
}

#[tokio::test]
async fn last_fm_auth_calls_parse_token_session_and_unauthorized_token() {
    let (session, sent) = recording_session(|_, body| {
        let name = if body.contains("auth.getToken") {
            "get_token"
        } else if body.contains("token=pending") {
            "error_unauthorized_token"
        } else {
            "get_session"
        };
        (200, fixture(&format!("last_fm/{name}.json")))
    });
    let service = configured_service(session, "auth");

    let (token, url) = service.start_sign_in().await.unwrap();
    assert_eq!(token, "cf45fe5a3e3cebe168480a086d7fe481");
    assert_eq!(
        url,
        format!("https://www.last.fm/api/auth/?api_key=key&token={token}")
    );

    assert!(matches!(
        service.get_session("pending").await,
        Err(LastFMError::UnauthorizedToken)
    ));
    let auth = service.get_session(&token).await.unwrap();
    assert_eq!(auth.session.name, "woofuser");
    assert_eq!(auth.session.key, "d580d57f32848f5dcf574d1ce18d78b2");

    let sent = sent.lock().unwrap();
    assert_eq!(sent[0].url, LastFMDefaults::BASE_URL);
    let form = sent[2].form();
    assert_eq!(form["method"], "auth.getSession");
    assert_eq!(form["format"], "json");
    assert_eq!(form["api_sig"].len(), 32);
}

#[tokio::test]
async fn last_fm_needs_keys_and_maps_http_errors() {
    let (session, _) = recording_session(|_, _| (500, b"oops".to_vec()));
    let mut service = configured_service(session, "errors");
    assert!(matches!(
        service.get_token().await,
        Err(LastFMError::Http(500, _))
    ));
    service.secret.clear();
    assert!(matches!(
        service.get_token().await,
        Err(LastFMError::NotConfigured)
    ));
}

#[tokio::test]
async fn last_fm_avatar_prefers_extralarge_over_https() {
    let (session, sent) = recording_session(|_, _| (200, fixture("last_fm/user_get_info.json")));
    let service = configured_service(session, "avatar");
    assert_eq!(
        service.fetch_avatar_url(" woofuser ").await.as_deref(),
        Some("https://lastfm.freetls.fastly.net/i/u/300x300/avatar.png")
    );
    assert_eq!(
        sent.lock().unwrap()[0].url,
        "https://ws.audioscrobbler.com/2.0/?method=user.getInfo&user=woofuser&api_key=key&format=json"
    );
}

// MARK: - Last.fm scrobble rules

#[tokio::test]
async fn last_fm_scrobbles_at_half_the_song_or_the_delay_counting_only_play_time() {
    mock_keyring();
    let (session, sent) = recording_session(|_, _| (200, fixture("last_fm/scrobble.json")));
    let defaults = temp_path("integration");
    LastFMSettings {
        scrobbling_enabled: true,
        use_now_playing: true,
        ..LastFMSettings::default()
    }
    .save(&defaults)
    .unwrap();
    let mut lastfm = LastFMIntegration::new(session, defaults);
    lastfm.service.api_key = "key".into();
    lastfm.service.secret = "secret".into();
    lastfm.service.token_store.store("sk1", "woofuser").unwrap();
    assert_eq!(
        lastfm.service.token_store.retrieve_username().as_deref(),
        Some("woofuser")
    );
    let methods = || -> Vec<String> {
        sent.lock()
            .unwrap()
            .iter()
            .map(|s| s.form()["method"].clone())
            .collect()
    };

    // 180 s song: 50% is 90 s, under the 180 s cap.
    let a = song("aaaaaaaaaaa", 180.0, true);
    lastfm.sync_state(Some(&a), at(1_000)).await;
    assert_eq!(methods(), ["track.updateNowPlaying"]);
    lastfm.sync_state(Some(&a), at(1_089)).await;
    assert_eq!(methods().len(), 1);
    lastfm.sync_state(Some(&a), at(1_090)).await;
    assert_eq!(methods(), ["track.updateNowPlaying", "track.scrobble"]);
    let scrobble = sent.lock().unwrap()[1].form();
    assert_eq!(scrobble["sk"], "sk1");
    assert_eq!(scrobble["artist[0]"], "Daft Punk");
    assert_eq!(scrobble["track[0]"], "One More Time");
    assert_eq!(scrobble["album[0]"], "Discovery");
    assert_eq!(scrobble["timestamp[0]"], "1000");
    assert_eq!(scrobble["duration[0]"], "180");
    // Once per play.
    lastfm.sync_state(Some(&a), at(1_200)).await;
    assert_eq!(methods().len(), 2);

    // 600 s song: capped at 180 s; 100 s paused don't count.
    let b = song("bbbbbbbbbbb", 600.0, true);
    lastfm.sync_state(Some(&b), at(2_000)).await;
    lastfm
        .sync_state(
            Some(&PlayingSong {
                is_playing: false,
                ..b.clone()
            }),
            at(2_100),
        )
        .await;
    lastfm.sync_state(Some(&b), at(2_200)).await;
    lastfm.sync_state(Some(&b), at(2_279)).await;
    assert_eq!(methods().len(), 3);
    lastfm.sync_state(Some(&b), at(2_280)).await;
    assert_eq!(methods().len(), 4);
    assert_eq!(sent.lock().unwrap()[3].form()["timestamp[0]"], "2000");

    // 30 s or shorter: now playing only.
    let c = song("ccccccccccc", 30.0, true);
    lastfm.sync_state(Some(&c), at(3_000)).await;
    lastfm.sync_state(Some(&c), at(3_029)).await;
    assert_eq!(methods()[4..], ["track.updateNowPlaying"]);

    // Incognito sends nothing.
    lastfm.incognito = true;
    let d = song("ddddddddddd", 180.0, true);
    lastfm.sync_state(Some(&d), at(4_000)).await;
    lastfm.sync_state(Some(&d), at(4_100)).await;
    assert_eq!(methods().len(), 5);

    lastfm.logout();
    assert!(!lastfm.is_logged_in());
    assert_eq!(lastfm.service.token_store.retrieve_username(), None);
}

#[test]
fn last_fm_settings_round_trip_under_swift_keys() {
    let path = temp_path("settings");
    assert_eq!(LastFMSettings::load(&path), LastFMSettings::default());
    let settings = LastFMSettings {
        username: Some("woofuser".into()),
        scrobbling_enabled: true,
        use_now_playing: true,
        use_send_likes: true,
        scrobble_delay_percent: 0.7,
        scrobble_min_duration: 40,
        scrobble_delay_seconds: 240,
    };
    settings.save(&path).unwrap();
    assert_eq!(LastFMSettings::load(&path), settings);

    let json: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for key in [
        "lastfmUsername",
        "lastfmScrobblingEnable",
        "lastfmUseNowPlaying",
        "lastfmUseSendLikes",
        "scrobbleDelayPercent",
        "scrobbleMinSongDuration",
        "scrobbleDelaySeconds",
    ] {
        assert!(json.get(key).is_some(), "missing {key}");
    }
    // Unset keys read as the Swift defaults.
    std::fs::write(&path, br#"{"lastfmScrobblingEnable": true}"#).unwrap();
    let partial = LastFMSettings::load(&path);
    assert!(partial.scrobbling_enabled);
    assert_eq!(partial.scrobble_delay_seconds, 180);
}

// MARK: - ListenBrainz

#[tokio::test]
async fn listen_brainz_validate_reads_user_name_or_rejects_token() {
    let (session, sent) =
        recording_session(|_, _| (200, fixture("listen_brainz/validate_token.json")));
    assert_eq!(
        ListenBrainzService::validate(&session, "tok")
            .await
            .unwrap(),
        "woofuser"
    );
    {
        let sent = sent.lock().unwrap();
        assert_eq!(sent[0].url, "https://api.listenbrainz.org/1/validate-token");
        assert_eq!(sent[0].authorization.as_deref(), Some("Token tok"));
    }

    let (session, _) =
        recording_session(|_, _| (200, fixture("listen_brainz/validate_token_invalid.json")));
    assert_eq!(
        ListenBrainzService::validate(&session, "tok").await,
        Err(ListenBrainzError::InvalidToken)
    );
}

#[tokio::test]
async fn listen_brainz_sends_now_playing_then_a_listen_at_half_the_song() {
    mock_keyring();
    let (session, sent) = recording_session(|url, _| {
        if url.ends_with("validate-token") {
            (200, fixture("listen_brainz/validate_token.json"))
        } else {
            (200, br#"{"status": "ok"}"#.to_vec())
        }
    });
    let defaults = temp_path("listen_brainz");
    let mut lb = ListenBrainz::new(session, defaults.clone());
    lb.connect("  tok  ").await.unwrap();
    assert_eq!(lb.user_name(), Some("woofuser"));
    assert_eq!(
        ListenBrainzSettings::load(&defaults).user_name.as_deref(),
        Some("woofuser")
    );
    let submissions = || -> Vec<Value> {
        sent.lock()
            .unwrap()
            .iter()
            .filter(|s| s.url.ends_with("submit-listens"))
            .map(|s| serde_json::from_str(&s.body).unwrap())
            .collect()
    };

    // A 60 s song counts after 30 s of play; the first tick only starts the clock.
    let a = song("aaaaaaaaaaa", 60.0, true);
    for second in 0..=29 {
        lb.tick(Some(&a), at(1_000 + second)).await;
    }
    let sent_so_far = submissions();
    assert_eq!(sent_so_far.len(), 1);
    assert_eq!(
        sent_so_far[0],
        json!({
            "listen_type": "playing_now",
            "payload": [{
                "track_metadata": {
                    "artist_name": "Daft Punk",
                    "track_name": "One More Time",
                    "release_name": "Discovery",
                    "additional_info": {
                        "media_player": "Woof Music",
                        "submission_client": "Woof Music",
                        "music_service": "music.youtube.com",
                        "origin_url": "https://music.youtube.com/watch?v=aaaaaaaaaaa",
                        "duration_ms": 60000
                    }
                }
            }]
        })
    );
    lb.tick(Some(&a), at(1_030)).await;
    let sent_so_far = submissions();
    assert_eq!(sent_so_far.len(), 2);
    assert_eq!(sent_so_far[1]["listen_type"], "single");
    assert_eq!(sent_so_far[1]["payload"][0]["listened_at"], 1_000);
    assert_eq!(
        sent.lock()
            .unwrap()
            .last()
            .unwrap()
            .authorization
            .as_deref(),
        Some("Token tok")
    );

    // Paused time does not count; a 20 min song counts at 4 min.
    let b = PlayingSong {
        album: None,
        ..song("bbbbbbbbbbb", 1_200.0, true)
    };
    lb.tick(Some(&b), at(2_000)).await;
    lb.tick(
        Some(&PlayingSong {
            is_playing: false,
            ..b.clone()
        }),
        at(2_001),
    )
    .await;
    lb.tick(Some(&b), at(2_500)).await;
    for second in 1..=239 {
        lb.tick(Some(&b), at(2_500 + second)).await;
    }
    let sent_so_far = submissions();
    assert_eq!(sent_so_far.len(), 3);
    assert!(
        sent_so_far[2]["payload"][0]["track_metadata"]
            .get("release_name")
            .is_none()
    );
    lb.tick(Some(&b), at(2_740)).await;
    assert_eq!(submissions().len(), 4);

    // Songs under 30 s never count.
    let c = song("ccccccccccc", 20.0, true);
    for second in 0..=20 {
        lb.tick(Some(&c), at(3_000 + second)).await;
    }
    assert_eq!(submissions()[4..].len(), 1);

    lb.disconnect();
    assert_eq!(lb.user_name(), None);
    assert_eq!(
        ListenBrainzSettings::load(&defaults),
        ListenBrainzSettings::default()
    );
}
