//! Playlist editing: song edits, rename, reorder, removal, collaborators, cover reply.

use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use serde_json::{Value, json};
use swiftytube::auth::auth_state::AuthState;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::networking::http_client::{HttpResponse, Session};
use swiftytube::networking::inner_tube_client::InnerTubeClient;
use swiftytube::sync::library_sync_models::SyncPlaylist;
use swiftytube::sync::library_sync_store::LibrarySyncStore;
use trop::database::database_service::DatabaseService;
use trop::services::playlist_collaborators::PlaylistCollaborators;
use trop::services::playlist_cover_uploader::PlaylistCoverUploader;
use trop::services::playlist_removal::PlaylistRemoval;
use trop::services::playlist_song_edits::PlaylistSongEdits;
use trop::services::sync_bridge::SyncBridge;

type Sent = Arc<Mutex<Vec<(String, Value)>>>;

/// A signed-in bridge whose requests answer `status(endpoint)` with `{}`.
fn bridge(status: impl Fn(&str) -> u16 + Send + Sync + 'static) -> (SyncBridge, Sent) {
    let sent: Sent = Arc::default();
    let log = sent.clone();
    let session = Session::mock(move |request| {
        let endpoint = request
            .url()
            .path()
            .trim_start_matches("/youtubei/v1/")
            .to_owned();
        let body = serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        let status = status(&endpoint);
        log.lock().unwrap().push((endpoint, body));
        HttpResponse {
            status,
            data: b"{}".to_vec(),
            ..HttpResponse::default()
        }
    });
    let auth = AuthState {
        sapisid: Some("sapisid".into()),
        cookies: [("SAPISID".to_owned(), "sapisid".to_owned())].into(),
        ..AuthState::default()
    };
    let client = InnerTubeClient::with_session(SwiftyTubeConfig::music(), auth, session);
    let defaults = std::env::temp_dir()
        .join(format!(
            "trop-playlist-edits-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
        .join("sync_defaults.json");
    let db = Arc::new(DatabaseService::open_in_memory());
    (SyncBridge::new(Arc::new(client), db, defaults), sent)
}

fn save(bridge: &SyncBridge, id: &str, browse_id: Option<&str>) {
    bridge
        .store
        .insert_or_replace_playlist(&SyncPlaylist {
            id: id.into(),
            browse_id: browse_id.map(str::to_owned),
            name: "Old".into(),
            thumbnail_url: None,
            is_editable: true,
            bookmarked_at: Some(UNIX_EPOCH),
            remote_song_count: None,
            is_auto_sync: false,
        })
        .unwrap();
}

fn ids(list: &[&str]) -> Vec<Option<String>> {
    list.iter().map(|id| Some((*id).to_owned())).collect()
}

#[test]
fn move_action_names_the_new_successor() {
    let rows = ids(&["a", "b", "c", "d"]);
    let action = |from, up| PlaylistSongEdits::move_action(&rows, from, up).map(Value::Object);
    // Up: before the row above.
    assert_eq!(
        action(2, true),
        Some(
            json!({ "action": "ACTION_MOVE_VIDEO_BEFORE", "setVideoId": "c", "movedSetVideoIdSuccessor": "b" })
        )
    );
    // Down: before the row two below, or to the end without a successor.
    assert_eq!(action(1, false).unwrap()["movedSetVideoIdSuccessor"], "d");
    assert_eq!(
        action(2, false),
        Some(json!({ "action": "ACTION_MOVE_VIDEO_BEFORE", "setVideoId": "c" }))
    );
    // Nowhere to go, or a row without a set id.
    assert_eq!(action(0, true), None);
    assert_eq!(action(3, false), None);
    assert_eq!(
        PlaylistSongEdits::move_action(&[Some("a".into()), None], 1, true),
        None
    );
}

#[test]
fn bare_id_strips_the_browse_prefix() {
    assert_eq!(PlaylistSongEdits::bare_id("VLPLx"), "PLx");
    assert_eq!(PlaylistSongEdits::bare_id("PLx"), "PLx");
}

#[tokio::test]
async fn synced_edits_send_the_bare_id() {
    let (bridge, sent) = bridge(|_| 200);
    save(&bridge, "VLPLx", Some("VLPLx"));

    PlaylistSongEdits::add(&bridge, "song", "VLPLx")
        .await
        .unwrap();
    PlaylistSongEdits::rename(&bridge, "VLPLx", "New")
        .await
        .unwrap();
    PlaylistSongEdits::move_song(&bridge, "VLPLx", &ids(&["a", "b"]), 1, true)
        .await
        .unwrap();

    let sent = sent.lock().unwrap();
    let actions: Vec<_> = sent
        .iter()
        .map(|(endpoint, body)| {
            assert_eq!(endpoint, "browse/edit_playlist");
            assert_eq!(body["playlistId"], "PLx");
            body["actions"][0]["action"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        actions,
        [
            "ACTION_ADD_VIDEO",
            "ACTION_SET_PLAYLIST_NAME",
            "ACTION_MOVE_VIDEO_BEFORE"
        ]
    );
    assert_eq!(
        bridge.store.fetch_playlist("VLPLx").unwrap().unwrap().name,
        "New"
    );
}

#[tokio::test]
async fn removal_falls_back_when_youtube_rejects_the_first_call() {
    let (bridge, sent) = bridge(|endpoint| {
        if endpoint == "playlist/delete" {
            403
        } else {
            200
        }
    });
    save(&bridge, "VLPLx", Some("VLPLx"));

    PlaylistRemoval::remove(&bridge, "VLPLx", true)
        .await
        .unwrap();

    let sent = sent.lock().unwrap();
    let endpoints: Vec<_> = sent.iter().map(|(endpoint, _)| endpoint.as_str()).collect();
    assert_eq!(endpoints, ["playlist/delete", "like/removelike"]);
    assert_eq!(sent[1].1["target"]["playlistId"], "PLx");
    assert!(bridge.store.fetch_playlist("VLPLx").unwrap().is_none());
}

#[tokio::test]
async fn removing_a_local_playlist_stays_offline() {
    let (bridge, sent) = bridge(|_| 500);
    save(&bridge, "local", None);

    PlaylistRemoval::remove(&bridge, "local", true)
        .await
        .unwrap();

    assert!(sent.lock().unwrap().is_empty());
    assert!(bridge.store.fetch_playlist("local").unwrap().is_none());
}

#[test]
fn collaborators_owner_first_with_channel() {
    let person = |name: &str, channel: &str| {
        json!({ "contentListItemViewModel": {
            "title": { "content": name },
            "avatar": { "avatarViewModel": { "image": { "sources": [{ "url": format!("https://a/{name}") }] } } },
            "rendererContext": { "commandContext": { "onTap": { "innertubeCommand": {
                "browseEndpoint": { "browseId": channel } } } } }
        } })
    };
    let json = json!({ "content": { "engagementPanelSectionListRenderer": { "content": {
        "playlistCollaborationViewModel": { "playlistCollaborators": [
            person("Ann", "UCann"), person("Bob", "FEnot-a-channel"), { "other": {} }
        ] } } } } });
    let people = PlaylistCollaborators::parse(json.as_object().unwrap());
    assert_eq!(people.len(), 2);
    assert!(people[0].is_owner && !people[1].is_owner);
    assert_eq!(people[0].channel_id.as_deref(), Some("UCann"));
    assert_eq!(people[1].channel_id, None);
    assert_eq!(people[1].avatar_url.as_deref(), Some("https://a/Bob"));
}

#[test]
fn cover_reply_carries_the_new_url() {
    let reply = json!({ "status": "STATUS_SUCCEEDED", "newHeader": {
        "musicEditablePlaylistDetailHeaderRenderer": { "header": { "musicResponsiveHeaderRenderer": {
            "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                { "url": "https://small" }, { "url": "https://large" }
            ] } } } } } } } });
    assert_eq!(
        PlaylistCoverUploader::new_cover_url(reply.as_object().unwrap()).as_deref(),
        Some("https://large")
    );
}
