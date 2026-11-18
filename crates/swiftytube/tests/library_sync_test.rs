//! Port of SwiftyTube `Tests/Sync/LibrarySyncTests.swift` and
//! `AccountMenuTests.swift` (the parts on the sync files), plus the Dart
//! `test/swiftytube/library_sync_test.dart` fixture, request-body and store checks.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use swiftytube::auth::auth_state::AuthState;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::networking::http_client::{HttpResponse, Session};
use swiftytube::networking::inner_tube_client::InnerTubeClient;
use swiftytube::networking::inner_tube_decode::JsonDict;
use swiftytube::sync::browse_lens::BrowseLens;
use swiftytube::sync::incremental_sync_service::IncrementalSyncService;
use swiftytube::sync::library_browse_parser::LibraryBrowseParser;
use swiftytube::sync::library_sync_models::{
    LibrarySyncOptions, SongMergeArgs, SongMetadata, SyncArtist, SyncPlaylist, SyncPlaylistEntry,
    SyncSong,
};
use swiftytube::sync::library_sync_service::LibrarySyncService;
use swiftytube::sync::library_sync_store::{JsonLibrarySyncStore, LibrarySyncStore};
use swiftytube::sync::mutation_service::MutationService;
use swiftytube::sync::playlist_detail_service::PlaylistDetailService;

// MARK: - Fixtures

fn dict(value: Value) -> JsonDict {
    match value {
        Value::Object(dict) => dict,
        other => panic!("not an object: {other}"),
    }
}

/// Synthetic library responses in `crates/fixtures/sync`: realistic shapes, no account data.
fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/../fixtures/sync/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn flex_column(runs: &[&str]) -> Value {
    let runs: Vec<Value> = runs.iter().map(|t| json!({ "text": t })).collect();
    json!({ "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": runs } } })
}

fn list_item(
    title: &str,
    subtitle: &[&str],
    browse_id: Option<&str>,
    video_id: Option<&str>,
    menu: Option<Value>,
) -> Value {
    let mut renderer = json!({
        "flexColumns": [flex_column(&[title]), flex_column(subtitle)],
        "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [{ "url": "https://x/y.jpg" }] } } },
    });
    if let Some(browse_id) = browse_id {
        renderer["navigationEndpoint"] = json!({ "browseEndpoint": { "browseId": browse_id } });
    }
    if let Some(video_id) = video_id {
        renderer["videoId"] = json!(video_id);
    }
    if let Some(menu) = menu {
        renderer["menu"] = menu;
    }
    json!({ "musicResponsiveListItemRenderer": renderer })
}

fn shelf_response(items: Vec<Value>, token: Option<&str>) -> JsonDict {
    let mut shelf = json!({ "contents": items });
    if let Some(token) = token {
        shelf["continuations"] = json!([{ "nextContinuationData": { "continuation": token } }]);
    }
    dict(json!({
        "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
            "sectionListRenderer": { "contents": [{
                "itemSectionRenderer": { "contents": [{ "musicShelfRenderer": shelf }] },
            }] },
        } } }] } },
    }))
}

/// A request the fake YouTube Music received: endpoint and body without `context`.
type Sent = Arc<Mutex<Vec<(String, Value)>>>;

/// Client answering every POST with `route(endpoint, body)`; `None` means HTTP 400.
fn fake_client(
    route: impl Fn(&str, &Value) -> Option<Value> + Send + Sync + 'static,
) -> (Arc<InnerTubeClient>, Sent) {
    let sent: Sent = Arc::default();
    let recorder = sent.clone();
    let session = Session::mock(move |request| {
        let endpoint = request
            .url()
            .path()
            .trim_start_matches("/youtubei/v1/")
            .to_string();
        let mut body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        body.as_object_mut().unwrap().remove("context");
        let answer = route(&endpoint, &body);
        recorder.lock().unwrap().push((endpoint, body));
        match answer {
            Some(json) => HttpResponse {
                status: 200,
                data: serde_json::to_vec(&json).unwrap(),
                ..HttpResponse::default()
            },
            None => HttpResponse {
                status: 400,
                data: br#"{"error":{"code":400}}"#.to_vec(),
                ..HttpResponse::default()
            },
        }
    });
    let client =
        InnerTubeClient::with_session(SwiftyTubeConfig::music(), AuthState::guest(), session);
    (Arc::new(client), sent)
}

fn signed_in_library(endpoint: &str, body: &Value) -> Option<Value> {
    if endpoint == "account/account_menu" {
        return Some(fixture("account_menu"));
    }
    if body["continuation"] == "artists-page-2" {
        return Some(fixture("library_artists_page2"));
    }
    let page = match body["browseId"].as_str() {
        Some("FEmusic_library_corpus_artists") => "library_artists",
        Some("FEmusic_liked_playlists") => "library_playlists",
        Some("FEmusic_liked_albums") => "library_albums",
        Some("FEmusic_library_non_music_audio_list") => "library_podcasts",
        Some("VLLM") => "liked_music",
        _ => return Some(json!({})),
    };
    Some(fixture(page))
}

fn last_sent(sent: &Sent) -> (String, Value) {
    sent.lock().unwrap().last().unwrap().clone()
}

/// A fresh path under the system temp dir, unique per test.
fn temp_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("swiftytube-sync-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("library.json")
}

fn open_store(name: &str) -> Arc<JsonLibrarySyncStore> {
    Arc::new(JsonLibrarySyncStore::open(temp_path(name)).unwrap())
}

fn liked_song_ids(store: &JsonLibrarySyncStore) -> Vec<String> {
    let mut liked: Vec<SyncSong> = store
        .snapshot()
        .songs
        .into_values()
        .filter(|s| s.liked)
        .collect();
    liked.sort_by_key(|s| std::cmp::Reverse(s.create_date));
    liked.into_iter().map(|s| s.id).collect()
}

// MARK: - Library browse parsing

#[test]
fn artists_parse_with_continuation() {
    let json = shelf_response(
        vec![
            list_item("Boards of Canada", &["Artist"], Some("UCx"), None, None),
            list_item("No Endpoint Row", &["Artist"], None, None, None),
        ],
        Some("next-page"),
    );
    let artists = LibraryBrowseParser::parse_artists(&json);
    assert_eq!(artists.len(), 1);
    assert_eq!(artists[0].browse_id, "UCx");
    assert_eq!(artists[0].name, "Boards of Canada");
    assert_eq!(
        LibraryBrowseParser::extract_continuation_token(&json).as_deref(),
        Some("next-page")
    );
}

#[test]
fn playlists_skip_rows_without_browse_id() {
    let json = shelf_response(
        vec![list_item(
            "Gym",
            &["Playlist", "42"],
            Some("VLPL1"),
            None,
            None,
        )],
        None,
    );
    let playlists = LibraryBrowseParser::parse_playlists(&json);
    assert_eq!(playlists.len(), 1);
    assert_eq!(playlists[0].song_count, Some(42));
    assert_eq!(LibraryBrowseParser::extract_continuation_token(&json), None);
}

#[test]
fn song_keeps_title_and_first_artist() {
    let item = dict(list_item(
        "Dayvan Cowboy",
        &["Boards of Canada", " • ", "The Campfire Headphase"],
        None,
        Some("abc123"),
        None,
    ));
    let song = LibraryBrowseParser::parse_song(&item).unwrap();
    assert_eq!(song.video_id, "abc123");
    assert_eq!(song.title, "Dayvan Cowboy");
    assert_eq!(
        song.artists.first().map(String::as_str),
        Some("Boards of Canada")
    );
}

#[test]
fn song_length_comes_from_fixed_columns_not_the_explicit_badge() {
    let mut item = list_item(
        "Dayvan Cowboy",
        &["Boards of Canada"],
        None,
        Some("abc123"),
        None,
    );
    let renderer = &mut item["musicResponsiveListItemRenderer"];
    renderer["fixedColumns"] = json!([
        { "musicResponsiveListItemFixedColumnRenderer": { "text": { "runs": [{ "text": "5:00" }] } } }
    ]);
    renderer["badges"] =
        json!([{ "musicInlineBadgeRenderer": { "text": { "runs": [{ "text": "E" }] } } }]);
    assert_eq!(
        LibraryBrowseParser::parse_song(&dict(item))
            .unwrap()
            .duration,
        300
    );
}

#[test]
fn albums_and_podcasts_parse() {
    let albums = LibraryBrowseParser::parse_albums(&shelf_response(
        vec![list_item(
            "Geogaddi",
            &["Album", "Boards of Canada"],
            Some("MPREb_x"),
            None,
            None,
        )],
        None,
    ));
    assert_eq!(albums.len(), 1);
    assert_eq!(albums[0].browse_id, "MPREb_x");

    let podcasts = LibraryBrowseParser::parse_podcasts(&shelf_response(
        vec![list_item(
            "My Podcast",
            &["Podcast"],
            Some("MPSP_x"),
            None,
            None,
        )],
        None,
    ));
    assert_eq!(podcasts.len(), 1);
    assert_eq!(podcasts[0].name, "My Podcast");
}

#[test]
fn like_toggle_tokens_surface_is_liked() {
    let menu = json!({ "menuRenderer": { "items": [{ "toggleMenuServiceItemRenderer": {
        "defaultIcon": { "iconType": "CHECK_CHECK" },
        "defaultServiceEndpoint": { "feedbackEndpoint": { "feedbackToken": "add-tok" } },
        "toggledServiceEndpoint": { "feedbackEndpoint": { "feedbackToken": "remove-tok" } },
    } }] } });
    let item = dict(list_item(
        "Loved",
        &["Artist"],
        None,
        Some("v1"),
        Some(menu),
    ));
    let song = LibraryBrowseParser::parse_song(&item).unwrap();
    assert!(song.is_liked);
    assert_eq!(song.library_add_token.as_deref(), Some("add-tok"));
    assert_eq!(song.library_remove_token.as_deref(), Some("remove-tok"));
}

#[test]
fn fixture_artists_paginate_and_skip_rows_without_endpoint() {
    let json = dict(fixture("library_artists"));
    let artists = LibraryBrowseParser::parse_artists(&json);
    let ids: Vec<&str> = artists.iter().map(|a| a.browse_id.as_str()).collect();
    assert_eq!(
        ids,
        ["UCbocTestChannel000000001", "UCnilsTestChannel00000001"]
    );
    assert_eq!(artists[0].name, "Boards of Canada");
    assert!(artists[0].is_subscribed);
    assert!(
        artists[0]
            .thumbnail_url
            .as_deref()
            .unwrap()
            .starts_with("https://lh3.googleusercontent.com/")
    );
    assert_eq!(
        LibraryBrowseParser::extract_continuation_token(&json).as_deref(),
        Some("artists-page-2")
    );

    let page2 = dict(fixture("library_artists_page2"));
    let page2_artists = LibraryBrowseParser::parse_artists(&page2);
    assert_eq!(page2_artists.len(), 1);
    assert_eq!(page2_artists[0].name, "Ólafur Arnalds");
    assert_eq!(
        LibraryBrowseParser::extract_continuation_token(&page2),
        None
    );
}

#[test]
fn fixture_playlists_albums_and_podcasts_from_grids() {
    let playlists = LibraryBrowseParser::parse_playlists(&dict(fixture("library_playlists")));
    let ids: Vec<&str> = playlists.iter().map(|p| p.browse_id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "VLLM",
            "VLPLfocusTestPlaylist000001",
            "VLPLroadTestPlaylist0000001"
        ]
    );
    assert_eq!(playlists[1].title, "Late Night Focus");

    let albums = LibraryBrowseParser::parse_albums(&dict(fixture("library_albums")));
    let ids: Vec<&str> = albums.iter().map(|a| a.browse_id.as_str()).collect();
    assert_eq!(ids, ["MPREb_geogaddiTest01", "MPREb_allMelodyTest01"]);
    assert_eq!(albums[0].title, "Geogaddi");

    let podcasts = LibraryBrowseParser::parse_podcasts(&dict(fixture("library_podcasts")));
    assert!(
        podcasts
            .iter()
            .any(|p| p.browse_id == "MPSPPLexploderTest000001")
    );
}

#[test]
fn fixture_songs_read_liked_state_and_library_tokens() {
    let songs = LibraryBrowseParser::parse_songs(&dict(fixture("library_songs")));
    let ids: Vec<&str> = songs.iter().map(|s| s.video_id.as_str()).collect();
    assert_eq!(ids, ["dQw4w9WgXcQ", "kJQP7kiw5Fk"]);
    let first = &songs[0];
    assert_eq!(first.title, "Dayvan Cowboy");
    assert_eq!(first.artists, ["Boards of Canada"]);
    assert_eq!(first.album.as_deref(), Some("The Campfire Headphase"));
    assert_eq!(first.duration, 300);
    assert!(first.is_liked);
    assert_eq!(first.library_add_token.as_deref(), Some("add-dQw4w9WgXcQ"));
    assert_eq!(
        first.library_remove_token.as_deref(),
        Some("remove-dQw4w9WgXcQ")
    );
    assert!(!songs[1].is_liked);
    assert_eq!(
        songs[1].library_add_token.as_deref(),
        Some("add-kJQP7kiw5Fk")
    );
}

#[test]
fn community_playlists_skip_radio_mixes_and_albums() {
    let json = shelf_response(
        vec![
            list_item("Fans", &["Playlist"], Some("VLPLfans"), None, None),
            list_item("Radio", &["Playlist"], Some("VLRDradio"), None, None),
            list_item("Album", &["Album"], Some("MPREb_x"), None, None),
        ],
        None,
    );
    // The shelf sits inside itemSectionRenderer here; community playlists read plain shelves.
    assert!(LibraryBrowseParser::extract_community_playlists(&json).is_empty());
    let plain = dict(
        json!({ "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{
        "tabRenderer": { "content": { "sectionListRenderer": { "contents": [{ "musicShelfRenderer": {
            "contents": [
                list_item("Fans", &["Playlist"], Some("VLPLfans"), None, None),
                list_item("Radio", &["Playlist"], Some("VLRDradio"), None, None),
                list_item("Album", &["Album"], Some("MPREb_x"), None, None),
            ],
        } }] } } },
    }] } } }),
    );
    let playlists = LibraryBrowseParser::extract_community_playlists(&plain);
    assert_eq!(playlists.len(), 1);
    assert_eq!(playlists[0].browse_id, "VLPLfans");
    assert_eq!(LibraryBrowseParser::normalize_playlist_id("VLPLx"), "PLx");
}

// MARK: - Browse lens and account menu

#[test]
fn browse_lens_sections_and_account_runs() {
    let json = shelf_response(vec![list_item("A", &[], Some("UC1"), None, None)], None);
    assert_eq!(BrowseLens::browse_sections(&json).unwrap().len(), 1);
    assert!(BrowseLens::first_browse_section(&json).is_some());
    assert!(BrowseLens::first_section_item(&json).is_some());

    let account = dict(json!({ "header": { "musicAccountHeaderRenderer": {
        "accountName": { "runs": [{ "text": "Ada" }] },
    } } }));
    assert_eq!(BrowseLens::account_name_runs(&account).unwrap().len(), 1);
    assert!(BrowseLens::account_name_runs(&JsonDict::new()).is_none());
}

fn runs_header(account_name: &str, email: Option<&str>) -> Value {
    let mut renderer = json!({
        "accountName": { "runs": [{ "text": account_name }] },
        "accountPhoto": { "thumbnails": [{ "url": "https://x/p.jpg" }] },
    });
    if let Some(email) = email {
        renderer["email"] = json!({ "runs": [{ "text": email }] });
    }
    renderer
}

fn menu_popup(popup: Value) -> JsonDict {
    dict(json!({ "actions": [{ "openPopupAction": { "popup": popup } }] }))
}

#[test]
fn signed_in_account_reads_every_menu_shape() {
    let chrome = menu_popup(json!({ "multiPageMenuRenderer": { "header": {
        "activeAccountHeaderRenderer": runs_header("Test User", Some("t@example.com")),
    } } }));
    let identity = BrowseLens::signed_in_account(&chrome).unwrap();
    assert_eq!(identity.name.as_deref(), Some("Test User"));
    assert_eq!(identity.email.as_deref(), Some("t@example.com"));
    assert!(identity.has_photo);

    let music = menu_popup(json!({ "musicMultiPageMenuRenderer": { "header": {
        "activeAccountHeaderRenderer": { "accountName": { "runs": [{ "text": "YTM User" }] } },
    } } }));
    assert_eq!(
        BrowseLens::signed_in_account(&music)
            .unwrap()
            .name
            .as_deref(),
        Some("YTM User")
    );

    let legacy = dict(
        json!({ "header": { "musicAccountHeaderRenderer": runs_header("Legacy User", None) } }),
    );
    assert_eq!(
        BrowseLens::signed_in_account(&legacy)
            .unwrap()
            .name
            .as_deref(),
        Some("Legacy User")
    );

    let simple = dict(json!({ "header": { "musicAccountHeaderRenderer": {
        "accountName": { "simpleText": "Simple User" },
        "email": { "simpleText": "s@example.com" },
    } } }));
    let identity = BrowseLens::signed_in_account(&simple).unwrap();
    assert_eq!(identity.name.as_deref(), Some("Simple User"));
    assert_eq!(identity.email.as_deref(), Some("s@example.com"));
}

#[test]
fn signed_out_and_real_guest_menus_have_no_account() {
    let signed_out = menu_popup(json!({ "multiPageMenuRenderer": { "sections": [{
        "multiPageMenuSectionRenderer": { "items": [{
            "signInButtonRenderer": { "text": { "runs": [{ "text": "Sign in" }] } },
        }] },
    }] } }));
    assert_eq!(BrowseLens::signed_in_account(&signed_out), None);

    // HTTP 200 with only compact links: what a session with stale rotating cookies gets.
    let links: Vec<Value> = ["Get Music Premium", "Settings", "Help"]
        .iter()
        .map(|t| json!({ "compactLinkRenderer": { "title": { "runs": [{ "text": t }] } } }))
        .collect();
    let mut guest = menu_popup(json!({ "multiPageMenuRenderer": {
        "style": "MENU_STYLE_SHORT",
        "sections": [{ "multiPageMenuSectionRenderer": { "items": links } }],
    } }));
    guest.insert("trackingParams".into(), json!("CAE"));
    assert_eq!(BrowseLens::signed_in_account(&guest), None);
    let info = InnerTubeClient::extract_account_info(&guest);
    assert_eq!(info.name, "Guest");
    assert_eq!(info.thumbnail_url, None);
}

#[test]
fn extract_account_info_reads_the_chrome_header_and_channel_command() {
    let json = menu_popup(
        json!({ "multiPageMenuRenderer": { "header": { "activeAccountHeaderRenderer": {
        "accountName": { "runs": [{ "text": "Real User" }] },
        "email": { "runs": [{ "text": "r@example.com" }] },
        "channelHandle": { "runs": [{ "text": "@realuser" }] },
        "accountPhoto": { "thumbnails": [
            { "url": "https://yt3.ggpht.com/small", "width": 32 },
            { "url": "https://yt3.ggpht.com/big", "width": 96 },
        ] },
        "channelCommand": { "innertubeCommand": {
            "browseEndpoint": { "browseId": "UC0123456789abcdefghijk" },
        } },
    } } } }),
    );
    let info = InnerTubeClient::extract_account_info(&json);
    assert_eq!(info.name, "Real User");
    assert_eq!(info.email.as_deref(), Some("r@example.com"));
    assert_eq!(info.channel_handle.as_deref(), Some("@realuser"));
    assert_eq!(
        info.thumbnail_url.as_deref(),
        Some("https://yt3.ggpht.com/big")
    );
    assert_eq!(info.channel_id.as_deref(), Some("UC0123456789abcdefghijk"));
}

#[tokio::test]
async fn verify_session_accepts_the_account_header_and_rejects_a_guest_menu() {
    let (client, sent) = fake_client(|_, _| Some(fixture("account_menu")));
    assert!(client.verify_session(None, None).await.unwrap());
    assert_eq!(last_sent(&sent).0, "account/account_menu");

    let (guest, _) = fake_client(|_, _| Some(json!({ "actions": [] })));
    assert!(!guest.verify_session(None, None).await.unwrap());
}

// MARK: - Sync store values

#[test]
fn skeleton_starts_empty_and_unliked() {
    let song = SyncSong::skeleton("v", false, "");
    assert!(song.title.is_empty());
    assert!(!song.liked);
    assert_eq!(song.duration, 0);
}

#[test]
fn merging_preserves_stored_state() {
    let mut stored = SyncSong::skeleton("v", true, "");
    stored.title = "Kept".into();
    stored.total_play_time = 99;
    let merged = SyncSong::merging(
        Some(&stored),
        SongMergeArgs {
            id: "v".into(),
            title: "Fresh".into(),
            artist_name: Some("New Artist".into()),
            thumbnail_url: Some("https://x/new.jpg".into()),
            ..SongMergeArgs::default()
        },
    );
    assert_eq!(merged.title, "Fresh");
    assert!(merged.liked);
    assert_eq!(merged.total_play_time, 99);
    assert_eq!(merged.artist_name.as_deref(), Some("New Artist"));
    assert_eq!(merged.thumbnail_url.as_deref(), Some("https://x/new.jpg"));
    assert_eq!(merged.create_date, stored.create_date);
}

#[test]
fn metadata_fills_only_blanks() {
    let mut song = SyncSong::skeleton("v", false, "");
    song.artist_name = Some("Stored".into());
    let enriched = SyncSong::merging_metadata(
        &song,
        &SongMetadata {
            title: "T".into(),
            artist_name: Some("Meta".into()),
            thumbnail_url: Some("https://x/m.jpg".into()),
            duration: 200,
            album_name: None,
        },
    );
    assert_eq!(enriched.title, "T");
    assert_eq!(enriched.artist_name.as_deref(), Some("Stored"));
    assert_eq!(enriched.duration, 200);
}

#[test]
fn playlist_merging_keeps_the_users_name() {
    let existing = SyncPlaylist {
        id: "PL1".into(),
        browse_id: Some("VLPL1".into()),
        name: "Mine".into(),
        thumbnail_url: None,
        is_editable: true,
        bookmarked_at: Some(SystemTime::now()),
        remote_song_count: Some(3),
        is_auto_sync: false,
    };
    let merged = SyncPlaylist::merging(Some(&existing), "PL1", Some("VLPL1"), "Remote", 5);
    assert_eq!(merged.name, "Mine");
    assert_eq!(merged.remote_song_count, Some(5));
}

#[test]
fn options_count_enabled_sections() {
    assert_eq!(LibrarySyncOptions::default().enabled_count(), 5);
    let some = LibrarySyncOptions {
        sync_albums: false,
        sync_songs: false,
        ..LibrarySyncOptions::default()
    };
    assert_eq!(some.enabled_count(), 3);
}

// MARK: - JSON store

#[test]
fn json_store_round_trips_every_table_through_the_file() {
    let path = temp_path("round-trip");
    let store = JsonLibrarySyncStore::open(&path).unwrap();
    assert_eq!(store.snapshot(), Default::default());

    let mut song = SyncSong::skeleton("v1", true, "add");
    song.title = "Song".into();
    store.save_song(&song).unwrap();
    store
        .save_artist(&SyncArtist {
            id: "UC1".into(),
            name: "Artist".into(),
            thumbnail_url: None,
            bookmarked_at: Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            is_podcast_channel: false,
            channel_id: Some("UC1".into()),
        })
        .unwrap();
    store
        .insert_or_replace_playlist(&SyncPlaylist::merging(None, "PL1", Some("VLPL1"), "Mix", 1))
        .unwrap();
    let entry = SyncPlaylistEntry {
        playlist_id: "PL1".into(),
        song_id: "v1".into(),
        position: 0,
        set_video_id: Some("SET".into()),
    };
    store.insert_map_ignoring_conflicts(&entry).unwrap();
    store
        .insert_map_ignoring_conflicts(&SyncPlaylistEntry {
            position: 9,
            ..entry.clone()
        })
        .unwrap();
    store
        .insert_song_ignoring_conflicts(&SyncSong::skeleton("v1", false, ""))
        .unwrap();

    let reopened = JsonLibrarySyncStore::open(&path).unwrap();
    assert_eq!(reopened.snapshot(), store.snapshot());
    assert_eq!(reopened.fetch_song("v1").unwrap(), Some(song));
    assert_eq!(reopened.fetch_map_entry("PL1", "v1").unwrap(), Some(entry));
    assert_eq!(reopened.snapshot().playlist_maps.len(), 1);
    assert_eq!(
        reopened.fetch_artist("UC1").unwrap().unwrap().name,
        "Artist"
    );

    std::fs::write(&path, b"not json").unwrap();
    assert!(JsonLibrarySyncStore::open(&path).is_err());
}

#[test]
fn json_store_sweeps_missing_artists_and_stale_likes() {
    let store = open_store("sweep");
    store
        .save_artist(&SyncArtist {
            id: "UCgone".into(),
            name: "Gone".into(),
            thumbnail_url: None,
            bookmarked_at: Some(UNIX_EPOCH),
            is_podcast_channel: false,
            channel_id: None,
        })
        .unwrap();
    let remote = store
        .apply_artist_sync(&LibraryBrowseParser::parse_artists(&dict(fixture(
            "library_artists",
        ))))
        .unwrap();
    assert_eq!(remote.len(), 2);
    assert_eq!(
        store.fetch_artist("UCgone").unwrap().unwrap().bookmarked_at,
        None
    );
    assert!(
        store
            .fetch_artist("UCbocTestChannel000000001")
            .unwrap()
            .unwrap()
            .bookmarked_at
            .is_some()
    );

    let mut old = SyncSong::skeleton("old", true, "");
    old.modify_date = UNIX_EPOCH;
    store.save_song(&old).unwrap();
    store
        .save_song(&SyncSong::skeleton("fresh", true, ""))
        .unwrap();
    let cutoff = SystemTime::now() - Duration::from_secs(60);
    store
        .unmark_stale_liked_songs(&HashSet::new(), cutoff)
        .unwrap();
    assert!(!store.fetch_song("old").unwrap().unwrap().liked);
    assert!(store.fetch_song("fresh").unwrap().unwrap().liked);
    assert_eq!(store.fetch_orphan_song_ids().unwrap(), Vec::<String>::new());
}

// MARK: - Services

#[tokio::test]
async fn disabled_sections_skip_network_but_still_refresh_likes() {
    let (client, sent) = fake_client(signed_in_library);
    let store = open_store("gating");
    let service = LibrarySyncService::new(client, store.clone());
    let refreshed = AtomicBool::new(false);
    let none = LibrarySyncOptions {
        sync_artists: false,
        sync_playlists: false,
        sync_albums: false,
        sync_podcasts: false,
        sync_songs: false,
    };
    let result = service
        .sync_all(none, async { refreshed.store(true, Ordering::SeqCst) })
        .await;
    assert_eq!(result.completed_sections, 0);
    assert!(refreshed.load(Ordering::SeqCst));
    assert!(sent.lock().unwrap().is_empty());
    assert_eq!(store.snapshot(), Default::default());
}

#[tokio::test]
async fn playlist_detail_service_saves_the_liked_music_page() {
    let (client, _) = fake_client(signed_in_library);
    let store = open_store("liked-music");
    let count = PlaylistDetailService::new(&client)
        .fetch_playlist(&*store, "LM", 100, false)
        .await
        .unwrap();
    assert_eq!(count, 3);
    assert_eq!(
        store.liked_playlist_song_ids_ordered("LM").unwrap(),
        ["liked00000a", "liked00000b", "liked00000c"]
    );
    assert_eq!(
        store
            .fetch_playlist("LM")
            .unwrap()
            .unwrap()
            .browse_id
            .as_deref(),
        Some("VLLM")
    );
}

#[tokio::test]
async fn signed_in_sync_merges_every_section_into_the_store() {
    let (client, sent) = fake_client(signed_in_library);
    let store = open_store("signed-in");
    let defaults = temp_path("signed-in-defaults").with_file_name("sync_defaults.json");
    let incremental = IncrementalSyncService::new(
        client.clone(),
        LibrarySyncService::new(client, store.clone()),
        defaults.clone(),
    );

    // A stale bookmark and a stale like that YouTube no longer lists.
    store
        .save_artist(&SyncArtist {
            id: "UCgone".into(),
            name: "Gone".into(),
            thumbnail_url: None,
            bookmarked_at: Some(UNIX_EPOCH),
            is_podcast_channel: false,
            channel_id: None,
        })
        .unwrap();
    let mut stale = SyncSong::skeleton("stale000000", true, "");
    stale.modify_date = UNIX_EPOCH;
    store.save_song(&stale).unwrap();

    incremental
        .check_and_sync_if_stale(LibrarySyncOptions::default(), async {})
        .await;

    let data = store.snapshot();
    let bookmarked: HashSet<&str> = data
        .artists
        .values()
        .filter(|a| a.bookmarked_at.is_some())
        .map(|a| a.id.as_str())
        .collect();
    assert_eq!(
        bookmarked,
        HashSet::from([
            "UCbocTestChannel000000001",
            "UCnilsTestChannel00000001",
            "UColafurTestChannel000001"
        ])
    );
    let playlists: Vec<&str> = data.playlists.keys().map(String::as_str).collect();
    // Liked Music merges into Liked Songs; it never becomes a playlist row.
    assert_eq!(
        playlists,
        ["VLPLfocusTestPlaylist000001", "VLPLroadTestPlaylist0000001"]
    );
    let albums: Vec<&str> = data.albums.keys().map(String::as_str).collect();
    assert_eq!(albums, ["MPREb_allMelodyTest01", "MPREb_geogaddiTest01"]);
    assert!(data.podcasts.contains_key("MPSPPLexploderTest000001"));
    assert_eq!(
        liked_song_ids(&store),
        ["liked00000a", "liked00000b", "liked00000c"]
    );
    assert!(data.playlist_maps.is_empty());
    assert!(incremental.last_sync_date().is_some());
    assert!(defaults.exists());

    // Within 30 minutes a second call does nothing.
    let requests = sent.lock().unwrap().len();
    incremental
        .check_and_sync_if_stale(LibrarySyncOptions::default(), async {})
        .await;
    assert_eq!(sent.lock().unwrap().len(), requests);
}

#[tokio::test]
async fn guest_session_writes_nothing() {
    let (client, sent) = fake_client(|endpoint, body| {
        if endpoint == "account/account_menu" {
            Some(json!({ "actions": [] }))
        } else {
            signed_in_library(endpoint, body)
        }
    });
    let store = open_store("guest");
    let incremental = IncrementalSyncService::new(
        client.clone(),
        LibrarySyncService::new(client, store.clone()),
        temp_path("guest-defaults"),
    );
    incremental
        .force_full_sync(LibrarySyncOptions::default(), async {})
        .await;
    let endpoints: Vec<String> = sent.lock().unwrap().iter().map(|s| s.0.clone()).collect();
    assert_eq!(endpoints, ["account/account_menu"]);
    assert_eq!(store.snapshot(), Default::default());
    assert_eq!(incremental.last_sync_date(), None);
}

// MARK: - MutationService request bodies

fn mutation_service(
    name: &str,
    fail: Arc<AtomicBool>,
) -> (MutationService, Arc<JsonLibrarySyncStore>, Sent) {
    let (client, sent) = fake_client(move |endpoint, _| {
        if fail.load(Ordering::SeqCst) {
            return None;
        }
        if endpoint == "playlist/create" {
            return Some(json!({ "playlistId": "PLnewTestPlaylist" }));
        }
        Some(json!({}))
    });
    let store = open_store(name);
    (MutationService::new(client, store.clone()), store, sent)
}

#[tokio::test]
async fn like_and_unlike() {
    let (mutations, store, sent) = mutation_service("like", Arc::default());
    mutations.like_song("abcdefghijk").await.unwrap();
    assert_eq!(
        last_sent(&sent),
        (
            "like/like".into(),
            json!({ "target": { "videoId": "abcdefghijk" } })
        )
    );
    assert_eq!(liked_song_ids(&store), ["abcdefghijk"]);

    mutations.unlike_song("abcdefghijk").await.unwrap();
    assert_eq!(
        last_sent(&sent),
        (
            "like/removelike".into(),
            json!({ "target": { "videoId": "abcdefghijk" } })
        )
    );
    assert!(liked_song_ids(&store).is_empty());
}

#[tokio::test]
async fn like_rolls_back_when_youtube_refuses() {
    let (mutations, store, _) = mutation_service("like-refused", Arc::new(AtomicBool::new(true)));
    assert!(mutations.like_song("abcdefghijk").await.is_err());
    assert!(liked_song_ids(&store).is_empty());
}

#[tokio::test]
async fn create_rename_add_to_and_remove_from_a_playlist() {
    let (mutations, store, sent) = mutation_service("playlist", Arc::default());
    let id = mutations.create_playlist("Mix", None).await.unwrap();
    assert_eq!(id, "PLnewTestPlaylist");
    assert_eq!(
        last_sent(&sent),
        ("playlist/create".into(), json!({ "title": "Mix" }))
    );

    mutations.rename_playlist(&id, "Mix 2").await.unwrap();
    assert_eq!(
        last_sent(&sent),
        (
            "browse/edit_playlist".into(),
            json!({ "playlistId": id, "actions": [{ "action": "ACTION_SET_PLAYLIST_NAME", "name": "Mix 2" }] })
        )
    );
    assert_eq!(store.fetch_playlist(&id).unwrap().unwrap().name, "Mix 2");

    mutations
        .add_to_playlist(&id, "abcdefghijk", None)
        .await
        .unwrap();
    let (endpoint, body) = last_sent(&sent);
    assert_eq!(endpoint, "browse/edit_playlist");
    assert_eq!(
        body["actions"],
        json!([{ "action": "ACTION_ADD_VIDEO", "addedVideoId": "abcdefghijk" }])
    );
    assert!(store.fetch_map_entry(&id, "abcdefghijk").unwrap().is_some());

    mutations
        .remove_from_playlist(&id, "abcdefghijk", "SET1")
        .await
        .unwrap();
    assert_eq!(
        last_sent(&sent).1["actions"],
        json!([{ "action": "ACTION_REMOVE_VIDEO", "setVideoId": "SET1" }])
    );
    assert!(store.fetch_map_entry(&id, "abcdefghijk").unwrap().is_none());
}

#[tokio::test]
async fn subscribe_and_unsubscribe_artist() {
    let (mutations, _, sent) = mutation_service("subscribe", Arc::default());
    mutations.subscribe_artist("UCx", "UCx").await.unwrap();
    assert_eq!(
        last_sent(&sent),
        (
            "subscription/subscribe".into(),
            json!({ "channelIds": ["UCx"] })
        )
    );
    mutations.unsubscribe_artist("UCx", "UCx").await.unwrap();
    assert_eq!(
        last_sent(&sent),
        (
            "subscription/unsubscribe".into(),
            json!({ "channelIds": ["UCx"] })
        )
    );
}

#[tokio::test]
async fn save_and_unsave_album_rates_its_playlist() {
    let (mutations, store, sent) = mutation_service("album", Arc::default());
    mutations
        .save_album("MPREb_x", "A", None, Some("OLAK5uy_x"), 0, 0)
        .await
        .unwrap();
    assert_eq!(
        last_sent(&sent),
        (
            "like/like".into(),
            json!({ "target": { "playlistId": "OLAK5uy_x" } })
        )
    );
    assert!(
        store
            .fetch_album("MPREb_x")
            .unwrap()
            .unwrap()
            .bookmarked_at
            .is_some()
    );

    mutations.unsave_album("MPREb_x", None).await.unwrap();
    assert_eq!(
        last_sent(&sent),
        (
            "like/removelike".into(),
            json!({ "target": { "playlistId": "OLAK5uy_x" } })
        )
    );
    assert_eq!(
        store.fetch_album("MPREb_x").unwrap().unwrap().bookmarked_at,
        None
    );
}

#[test]
fn album_playlist_id_comes_from_the_like_endpoint_or_the_canonical_url() {
    let page = dict(json!({ "header": { "buttons": [{ "likeButtonRenderer": {
        "likeEndpoint": { "target": { "playlistId": "OLAK5uy_like" } },
    } }] } }));
    assert_eq!(
        MutationService::album_playlist_id_in(&page).as_deref(),
        Some("OLAK5uy_like")
    );
    let canonical = dict(json!({ "microformat": { "microformatDataRenderer": {
        "urlCanonical": "https://music.youtube.com/playlist?list=OLAK5uy_url",
    } } }));
    assert_eq!(
        MutationService::album_playlist_id_in(&canonical).as_deref(),
        Some("OLAK5uy_url")
    );
    assert_eq!(
        MutationService::album_playlist_id_in(&JsonDict::new()),
        None
    );
}
