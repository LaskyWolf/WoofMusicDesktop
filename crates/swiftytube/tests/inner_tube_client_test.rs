//! Port of the Dart `test/swiftytube/inner_tube_client_test.dart`, plus two fixture checks.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use swiftytube::auth::auth_state::AuthState;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::core::youtube_client::YouTubeClient;
use swiftytube::core::youtube_feeds::YouTubeFeeds;
use swiftytube::core::youtube_locale::YouTubeLocale;
use swiftytube::networking::http_client::{HttpResponse, Session};
use swiftytube::networking::inner_tube_client::{
    BrowseArgs, InnerTubeClient, NextArgs, PlayerArgs, SearchArgs,
};
use swiftytube::networking::inner_tube_decode::JsonDict;
use swiftytube::networking::inner_tube_error::InnerTubeError;
use swiftytube::networking::retry_policy::RetryPolicy;

type Sent = Arc<Mutex<Vec<reqwest::Request>>>;

/// Client whose session records each request and answers `response` with `status`.
fn make_client(auth: AuthState, response: Vec<u8>, status: u16) -> (InnerTubeClient, Sent) {
    let sent: Sent = Arc::default();
    let recorder = sent.clone();
    let session = Session::mock(move |request| {
        recorder.lock().unwrap().push(request);
        HttpResponse {
            status,
            data: response.clone(),
            ..HttpResponse::default()
        }
    });
    (
        InnerTubeClient::with_session(SwiftyTubeConfig::music(), auth, session),
        sent,
    )
}

fn header<'a>(request: &'a reqwest::Request, name: &str) -> Option<&'a str> {
    request.headers().get(name).map(|v| v.to_str().unwrap())
}

fn body(request: &reqwest::Request) -> Value {
    serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap()
}

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(format!("{}/../fixtures/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[tokio::test]
async fn web_remix_browse_url_headers_and_context_match_swift_request_builder() {
    let auth = AuthState {
        cookies: BTreeMap::from([("SID".into(), "a".into()), ("HSID".into(), "b".into())]),
        sapisid: Some("S".into()),
        visitor_data: Some("VD".into()),
        data_sync_id: None,
    };
    let (client, sent) = make_client(auth, b"{}".to_vec(), 200);
    client.browse(BrowseArgs::id("FEmusic_home")).await.unwrap();

    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    let request = &sent[0];
    assert_eq!(request.method(), "POST");
    assert_eq!(
        request.url().as_str(),
        "https://music.youtube.com/youtubei/v1/browse"
    );
    assert_eq!(header(request, "Content-Type"), Some("application/json"));
    assert_eq!(header(request, "X-Goog-Api-Format-Version"), Some("1"));
    assert_eq!(header(request, "X-YouTube-Client-Name"), Some("67"));
    assert_eq!(
        header(request, "X-YouTube-Client-Version"),
        Some(YouTubeClient::WEB_REMIX.client_version)
    );
    assert_eq!(
        header(request, "X-Origin"),
        Some("https://music.youtube.com")
    );
    assert_eq!(
        header(request, "Referer"),
        Some("https://music.youtube.com/")
    );
    assert_eq!(
        header(request, "User-Agent"),
        Some(YouTubeClient::WEB_USER_AGENT)
    );
    assert_eq!(header(request, "Accept-Language"), Some("en-US,en;q=0.9"));
    assert_eq!(header(request, "X-Goog-Visitor-Id"), Some("VD"));
    assert_eq!(header(request, "X-Goog-AuthUser"), Some("0"));
    // Cookies are sorted by name (BTreeMap); Dart kept insertion order "SID=a; HSID=b".
    assert_eq!(header(request, "Cookie"), Some("HSID=b; SID=a"));
    let authorization = header(request, "Authorization").unwrap();
    let (ts, hash) = authorization
        .strip_prefix("SAPISIDHASH ")
        .unwrap()
        .split_once('_')
        .unwrap();
    assert!(ts.chars().all(|c| c.is_ascii_digit()));
    assert!(
        hash.len() == 40
            && hash
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );

    let body = body(request);
    assert_eq!(body["browseId"], "FEmusic_home");
    assert_eq!(
        body["context"],
        json!({
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": "1.20260707.12.00",
                "gl": "US",
                "hl": "en",
                "visitorData": "VD",
            },
            "request": { "internalExperimentFlags": [], "useSsl": true },
            "user": { "lockedSafetyMode": false },
        })
    );
}

#[tokio::test]
async fn guest_request_sends_no_cookie_authorization_or_visitor_id() {
    let (client, sent) = make_client(AuthState::guest(), b"{}".to_vec(), 200);
    client
        .search(SearchArgs {
            query: "q".into(),
            params: Some("P".into()),
            ..SearchArgs::default()
        })
        .await
        .unwrap();
    let sent = sent.lock().unwrap();
    let request = &sent[0];
    assert!(header(request, "Cookie").is_none());
    assert!(header(request, "Authorization").is_none());
    assert!(header(request, "X-Goog-Visitor-Id").is_none());
    assert_eq!(header(request, "X-Goog-AuthUser"), Some("0"));
    let body = body(request);
    assert_eq!(body["query"], "q");
    assert_eq!(body["params"], "P");
}

#[tokio::test]
async fn visitor_data_from_response_context_is_kept_for_the_next_request() {
    let response = br#"{"responseContext":{"visitorData":"FRESH"}}"#.to_vec();
    let (client, sent) = make_client(AuthState::guest(), response, 200);
    client.ensure_visitor_data(None, None).await.unwrap();
    assert_eq!(client.current_auth().visitor_data.as_deref(), Some("FRESH"));
    assert_eq!(body(&sent.lock().unwrap()[0])["browseId"], "FEmusic_home");

    client.ensure_visitor_data(None, None).await.unwrap(); // already set: no request
    assert_eq!(sent.lock().unwrap().len(), 1);

    client
        .next(NextArgs {
            video_id: Some("v".into()),
            ..NextArgs::default()
        })
        .await
        .unwrap();
    let sent = sent.lock().unwrap();
    let last = sent.last().unwrap();
    assert_eq!(header(last, "X-Goog-Visitor-Id"), Some("FRESH"));
    assert_eq!(body(last)["context"]["client"]["visitorData"], "FRESH");
}

#[tokio::test]
async fn update_locale_changes_gl_hl_and_accept_language() {
    let (client, sent) = make_client(AuthState::guest(), b"{}".to_vec(), 200);
    client.update_locale(YouTubeLocale::new("IT", "it"));
    client.browse(BrowseArgs::id("x")).await.unwrap();
    let sent = sent.lock().unwrap();
    assert_eq!(header(&sent[0], "Accept-Language"), Some("en-IT,en;q=0.9"));
    let ctx = &body(&sent[0])["context"]["client"];
    assert_eq!((&ctx["gl"], &ctx["hl"]), (&json!("IT"), &json!("it")));
}

#[tokio::test]
async fn player_body_for_an_embedded_client() {
    let (client, sent) = make_client(AuthState::guest(), b"{}".to_vec(), 200);
    client
        .player(PlayerArgs {
            video_id: "abc".into(),
            client: Some(YouTubeClient::TV_HTML5_SIMPLY_EMBEDDED),
            ..PlayerArgs::default()
        })
        .await
        .unwrap();
    let sent = sent.lock().unwrap();
    let body = body(&sent[0]);
    assert_eq!(body["videoCheckOk"], true);
    assert_eq!(
        body["thirdParty"],
        json!({ "embedUrl": "https://www.youtube.com/embed/abc" })
    );
    assert_eq!(
        body["context"]["client"]["embedUrl"],
        "https://www.reddit.com/"
    );
    assert!(body["context"]["client"].get("userAgent").is_none());
    assert_eq!(header(&sent[0], "X-YouTube-Client-Name"), Some("85"));
}

#[tokio::test]
async fn http_4xx_fails_with_http_error_without_retry() {
    let (client, sent) = make_client(AuthState::guest(), b"{}".to_vec(), 403);
    let error = client
        .post("browse", JsonDict::new(), None, RetryPolicy::INNER_TUBE)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        InnerTubeError::HttpError {
            status_code: 403,
            ..
        }
    ));
    assert_eq!(sent.lock().unwrap().len(), 1);
}

#[test]
fn extract_account_info_reads_the_active_account_header() {
    let json = json!({
        "actions": [{
            "openPopupAction": { "popup": { "multiPageMenuRenderer": { "header": { "activeAccountHeaderRenderer": {
                "accountName": { "runs": [{ "text": "Woof" }] },
                "accountPhoto": { "thumbnails": [{ "url": "small" }, { "url": "big" }] },
                "manageAccountTitle": { "runs": [{ "navigationEndpoint": { "commandMetadata": {
                    "webCommandMetadata": { "url": "/channel/UC123/featured" },
                } } }] },
            } } } } },
        }],
    });
    let Value::Object(json) = json else {
        unreachable!()
    };
    let info = InnerTubeClient::extract_account_info(&json);
    assert_eq!(info.name, "Woof");
    assert_eq!(info.thumbnail_url.as_deref(), Some("big"));
    assert_eq!(info.channel_id.as_deref(), Some("UC123"));
    assert_eq!(
        InnerTubeClient::extract_account_info(&JsonDict::new()).name,
        "Guest"
    );
}

#[tokio::test]
async fn account_info_from_the_account_menu_fixture() {
    let (client, sent) = make_client(AuthState::guest(), fixture("sync/account_menu.json"), 200);
    let info = client.account_info(None, None).await.unwrap();
    assert_eq!(
        sent.lock().unwrap()[0].url().path(),
        "/youtubei/v1/account/account_menu"
    );
    assert_eq!(info.name, "Test Listener");
    assert_eq!(info.email.as_deref(), Some("listener@example.com"));
    assert_eq!(
        info.thumbnail_url.as_deref(),
        Some("https://yt3.ggpht.com/test=s88")
    );
}

#[tokio::test]
async fn browse_returns_the_home_fixture_tabs() {
    let (client, _) = make_client(AuthState::guest(), fixture("trop/home.json"), 200);
    let home = client.browse(BrowseArgs::id("FEmusic_home")).await.unwrap();
    let tabs = YouTubeFeeds::channel_tabs(&home);
    assert_eq!(tabs[0].title, "Home");
    assert_eq!(tabs[0].browse_id.as_deref(), Some("FEmusic_home"));
    assert!(tabs[0].selected);
}
