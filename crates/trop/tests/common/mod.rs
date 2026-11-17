//! Port of the Dart `test/trop/fixture_helpers.dart`.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use regex::Regex;
use serde_json::Value;
use swiftytube::auth::auth_state::AuthState;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::networking::http_client::{HttpResponse, Session};
use swiftytube::networking::inner_tube_client::InnerTubeClient;
use swiftytube::networking::inner_tube_decode::JsonDict;
use trop::networking::models::yt_item::YTItem;

/// Anonymous responses recorded by the Dart `tool/fetch_fixtures.dart`.
pub fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../fixtures/trop/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

pub fn fixture(name: &str) -> JsonDict {
    serde_json::from_slice(&fixture_bytes(name)).unwrap()
}

/// Thumbnail URL, non-empty title and a well-formed id for the item's kind.
pub fn expect_well_formed(item: &YTItem) {
    let video_id = Regex::new(r"^[A-Za-z0-9_-]{11}$").unwrap();
    assert!(!item.title().trim().is_empty(), "{item:?} has no title");
    assert!(
        item.thumbnail_url().is_some_and(|u| u.starts_with("http")),
        "{} has no thumbnail",
        item.title()
    );
    let id = item.id();
    let ok = match item {
        YTItem::Song(_) | YTItem::Episode(_) => video_id.is_match(id),
        YTItem::Album(_) => id.starts_with("MPREb_"),
        YTItem::Artist(_) => id.starts_with("UC") || id.starts_with("MPLA"),
        YTItem::Playlist(_) => id.starts_with("VL") || id.starts_with("RD") || id.starts_with("FE"),
        YTItem::Podcast(_) => id.starts_with("MPSP"),
    };
    assert!(ok, "{} has a malformed id {id}", item.title());
}

pub type Sent = Arc<Mutex<Vec<Value>>>;

/// Client whose answers come from `route(body)` (a fixture name). Every
/// request body is recorded in the returned log.
pub fn fixture_client(
    route: impl Fn(&Value) -> &'static str + Send + Sync + 'static,
) -> (Arc<InnerTubeClient>, Sent) {
    let sent: Sent = Arc::default();
    let log = sent.clone();
    let session = Session::mock(move |request| {
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        let name = route(&body);
        log.lock().unwrap().push(body);
        HttpResponse {
            status: 200,
            data: fixture_bytes(name),
            ..HttpResponse::default()
        }
    });
    let client =
        InnerTubeClient::with_session(SwiftyTubeConfig::music(), AuthState::default(), session);
    (Arc::new(client), sent)
}
