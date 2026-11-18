//! `services/`: `TropLibraryStore` against an in-memory database,
//! `LibraryPages` and `SyncBridge` against a mock session. No network.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use swiftytube::auth::auth_state::AuthState;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::networking::http_client::{HttpResponse, Session};
use swiftytube::networking::inner_tube_client::InnerTubeClient;
use swiftytube::sync::library_browse_parser::LibraryBrowseParser;
use swiftytube::sync::library_parsed_models::{ParsedArtist, ParsedPlaylist, ParsedPodcast};
use swiftytube::sync::library_sync_models::{
    SyncArtist, SyncPlaylist, SyncPlaylistEntry, SyncSong,
};
use swiftytube::sync::library_sync_service::LibrarySyncService;
use swiftytube::sync::library_sync_store::LibrarySyncStore;
use swiftytube::sync::mutation_service::MutationService;
use trop::database::database_service::DatabaseService;
use trop::database::entities::song_entity::SongEntity;
use trop::services::library_pages::LibraryPages;
use trop::services::sync_bridge::SyncBridge;
use trop::services::trop_library_store::TropLibraryStore;

// MARK: - Helpers

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/../fixtures/sync/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn store() -> (TropLibraryStore, Arc<DatabaseService>) {
    let db = Arc::new(DatabaseService::open_in_memory());
    (TropLibraryStore::new(db.clone()), db)
}

fn artist(id: &str, bookmarked_at: Option<SystemTime>) -> SyncArtist {
    SyncArtist {
        id: id.into(),
        name: id.into(),
        thumbnail_url: None,
        bookmarked_at,
        is_podcast_channel: false,
        channel_id: None,
    }
}

fn parsed_artist(id: &str) -> ParsedArtist {
    ParsedArtist {
        browse_id: id.into(),
        name: format!("{id} remote"),
        ..ParsedArtist::default()
    }
}

fn playlist(id: &str, browse_id: Option<&str>, is_auto_sync: bool) -> SyncPlaylist {
    SyncPlaylist {
        id: id.into(),
        browse_id: browse_id.map(str::to_string),
        name: id.into(),
        thumbnail_url: None,
        is_editable: false,
        bookmarked_at: Some(UNIX_EPOCH),
        remote_song_count: None,
        is_auto_sync,
    }
}

fn bookmark(store: &TropLibraryStore, id: &str) -> Option<SystemTime> {
    store.fetch_artist(id).unwrap().unwrap().bookmarked_at
}

/// Requests the mock session received: endpoint and body.
type Sent = Arc<Mutex<Vec<(String, Value)>>>;

/// Client answering every POST with `route(endpoint, body)`; `None` means HTTP 400.
fn fake_client(
    route: impl Fn(&str, &Value) -> Option<Value> + Send + Sync + 'static,
) -> (Arc<InnerTubeClient>, Sent) {
    let sent: Sent = Arc::default();
    let log = sent.clone();
    let session = Session::mock(move |request| {
        let endpoint = request
            .url()
            .path()
            .trim_start_matches("/youtubei/v1/")
            .to_string();
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        let answer = route(&endpoint, &body);
        log.lock().unwrap().push((endpoint, body));
        match answer {
            Some(json) => HttpResponse {
                status: 200,
                data: serde_json::to_vec(&json).unwrap(),
                ..HttpResponse::default()
            },
            None => HttpResponse {
                status: 400,
                ..HttpResponse::default()
            },
        }
    });
    let client =
        InnerTubeClient::with_session(SwiftyTubeConfig::music(), AuthState::guest(), session);
    (Arc::new(client), sent)
}

/// The signed-in library in `crates/fixtures/sync`; `page2` answers the artists' second page.
fn library(page2: Option<Value>) -> impl Fn(&str, &Value) -> Option<Value> + Send + Sync {
    move |endpoint, body| {
        if endpoint == "account/account_menu" {
            return Some(fixture("account_menu"));
        }
        if body["continuation"] == "artists-page-2" {
            return page2.clone();
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
}

// MARK: - Library sections

#[test]
fn artist_sync_upserts_keeps_bookmarks_and_sweeps_missing_ones() {
    let (store, _) = store();
    store
        .save_artist(&artist("UCkept", Some(UNIX_EPOCH)))
        .unwrap();
    store
        .save_artist(&artist("UCgone", Some(UNIX_EPOCH)))
        .unwrap();

    let remote = store
        .apply_artist_sync(&[parsed_artist("UCkept"), parsed_artist("UCnew")])
        .unwrap();

    assert_eq!(remote, HashSet::from(["UCkept".into(), "UCnew".into()]));
    // COALESCE keeps the first bookmark date.
    assert_eq!(bookmark(&store, "UCkept"), Some(UNIX_EPOCH));
    assert_eq!(
        store.fetch_artist("UCkept").unwrap().unwrap().name,
        "UCkept remote"
    );
    assert!(bookmark(&store, "UCnew").is_some());
    assert_eq!(bookmark(&store, "UCgone"), None);
}

#[test]
fn an_empty_remote_list_removes_nothing() {
    let (store, _) = store();
    store
        .save_artist(&artist("UCkept", Some(UNIX_EPOCH)))
        .unwrap();
    store
        .save_playlist(&playlist("VLPLkept", Some("VLPLkept"), true))
        .unwrap();

    assert!(store.apply_artist_sync(&[]).unwrap().is_empty());
    store.apply_album_sync(&[]).unwrap();
    store.apply_playlist_sync(&[]).unwrap();
    store.apply_podcast_sync(&[]).unwrap();

    assert_eq!(bookmark(&store, "UCkept"), Some(UNIX_EPOCH));
    assert!(store.fetch_playlist("VLPLkept").unwrap().is_some());
}

#[tokio::test]
async fn a_failed_page_leaves_the_store_untouched() {
    let (store, _) = store();
    store
        .save_artist(&artist("UCgone", Some(UNIX_EPOCH)))
        .unwrap();
    let store = Arc::new(store);

    // Page 2 answers HTTP 400: the section fails before reaching the store.
    let (client, _) = fake_client(library(None));
    let service = LibrarySyncService::new(client, store.clone());
    assert!(service.sync_subscribed_artists().await.is_err());
    assert_eq!(bookmark(&store, "UCgone"), Some(UNIX_EPOCH));
    assert!(
        store
            .fetch_artist("UCbocTestChannel000000001")
            .unwrap()
            .is_none()
    );

    // Both pages answer: all three artists arrive and the sweep runs.
    let (client, _) = fake_client(library(Some(fixture("library_artists_page2"))));
    let service = LibrarySyncService::new(client, store.clone());
    assert_eq!(service.sync_subscribed_artists().await.unwrap().len(), 3);
    assert_eq!(bookmark(&store, "UCgone"), None);
    assert!(bookmark(&store, "UColafurTestChannel000001").is_some());
}

#[test]
fn playlist_sync_hides_episodes_for_later_skips_detached_and_sweeps_auto_synced() {
    let (store, _) = store();
    store
        .save_playlist(&playlist("VLPLgone", Some("VLPLgone"), true))
        .unwrap();
    store
        .save_playlist(&playlist("VLPLlocal", Some("VLPLlocal"), false))
        .unwrap();
    store
        .save_playlist(&playlist("VLSE", Some("VLSE"), true))
        .unwrap();
    store
        .insert_map_ignoring_conflicts(&SyncPlaylistEntry {
            playlist_id: "VLPLgone".into(),
            song_id: "song0000001".into(),
            position: 0,
            set_video_id: None,
        })
        .unwrap();
    store
        .save_playlist(&playlist("VLPLdetached", None, false))
        .unwrap();

    let remote = store
        .apply_playlist_sync(&[
            ParsedPlaylist {
                browse_id: "VLPLnew".into(),
                title: "New".into(),
                song_count: Some(4),
                thumbnail_url: None,
            },
            ParsedPlaylist {
                browse_id: "VLPLdetached".into(),
                title: "Renamed remotely".into(),
                ..ParsedPlaylist::default()
            },
            ParsedPlaylist {
                browse_id: "VLSE".into(),
                title: "Episodes for Later".into(),
                ..ParsedPlaylist::default()
            },
        ])
        .unwrap();

    assert!(!remote.contains("VLSE"));
    let new = store.fetch_playlist("VLPLnew").unwrap().unwrap();
    assert!(new.is_auto_sync);
    assert_eq!(new.remote_song_count, Some(4));
    assert_eq!(
        store.fetch_playlist("VLPLdetached").unwrap().unwrap().name,
        "VLPLdetached"
    );
    assert!(store.fetch_playlist("VLPLlocal").unwrap().is_some());
    assert!(store.fetch_playlist("VLPLgone").unwrap().is_none());
    assert!(store.fetch_playlist("VLSE").unwrap().is_none());
    assert!(
        store
            .fetch_map_entry("VLPLgone", "song0000001")
            .unwrap()
            .is_none()
    );

    let podcasts = store
        .apply_podcast_sync(&[
            ParsedPodcast {
                browse_id: "SE".into(),
                ..ParsedPodcast::default()
            },
            ParsedPodcast {
                browse_id: "MPSPPLshow".into(),
                name: "Show".into(),
                ..ParsedPodcast::default()
            },
        ])
        .unwrap();
    assert_eq!(podcasts, HashSet::from(["MPSPPLshow".into()]));
    assert!(store.fetch_podcast("SE").unwrap().is_none());
}

#[test]
fn detach_and_reattach_a_playlist() {
    let (store, _) = store();
    store
        .save_playlist(&playlist("PLmine", Some("VLPLmine"), true))
        .unwrap();
    store.detach_playlist("PLmine").unwrap();
    let detached = store.fetch_playlist("PLmine").unwrap().unwrap();
    assert_eq!(detached.browse_id, None);
    assert!(!detached.is_auto_sync && detached.is_editable);
    store.reattach_playlist("PLmine").unwrap();
    assert_eq!(
        store
            .fetch_playlist("PLmine")
            .unwrap()
            .unwrap()
            .browse_id
            .as_deref(),
        Some("VLPLmine")
    );
}

// MARK: - Liked songs

#[test]
fn mirror_liked_songs_staggers_create_dates_in_remote_order() {
    let (store, _) = store();
    for id in ["a", "b", "c"] {
        store.save_song(&SyncSong::skeleton(id, false, "")).unwrap();
    }
    let base = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    store
        .mirror_liked_songs(&["c".into(), "a".into(), "b".into()], base)
        .unwrap();

    let mut liked: Vec<SyncSong> = ["a", "b", "c"]
        .map(|id| store.fetch_song(id).unwrap().unwrap())
        .into_iter()
        .collect();
    assert!(liked.iter().all(|s| s.liked && s.modify_date == base));
    liked.sort_by_key(|s| std::cmp::Reverse(s.create_date));
    let order: Vec<&str> = liked.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(order, ["c", "a", "b"]);
    assert_eq!(liked[2].create_date, base - Duration::from_secs(2));
}

#[test]
fn unmark_stale_liked_songs_spares_recent_rows_and_ignores_an_empty_list() {
    let (store, _) = store();
    for id in ["old", "listed"] {
        let mut song = SyncSong::skeleton(id, true, "");
        song.modify_date = UNIX_EPOCH;
        store.save_song(&song).unwrap();
    }
    store
        .save_song(&SyncSong::skeleton("fresh", true, ""))
        .unwrap();
    let cutoff = SystemTime::now() - Duration::from_secs(60);
    let liked = |id| store.fetch_song(id).unwrap().unwrap().liked;

    store
        .unmark_stale_liked_songs(&HashSet::new(), cutoff)
        .unwrap();
    assert!(liked("old"));

    store
        .unmark_stale_liked_songs(&HashSet::from(["listed".into()]), cutoff)
        .unwrap();
    assert!(!liked("old"));
    assert!(liked("listed"));
    assert!(liked("fresh"));
}

// MARK: - Songs

#[test]
fn save_song_keeps_the_stored_unavailable_flag() {
    let (store, db) = store();
    let mut song = SyncSong::skeleton("gone0000001", false, "");
    store.save_song(&song).unwrap();
    db.write(|db| db.execute("UPDATE song SET is_unavailable = 1", []))
        .unwrap();

    song.title = "Updated".into();
    store.save_song(&song).unwrap();

    let stored: SongEntity = db.fetch_one("gone0000001").unwrap().unwrap();
    assert_eq!(stored.title, "Updated");
    assert!(stored.is_unavailable);
}

#[test]
fn save_song_keeps_play_time_banked_after_the_song_was_read() {
    let (store, db) = store();
    store
        .save_song(&SyncSong::skeleton("play0000001", false, ""))
        .unwrap();

    let mut read_before = store.fetch_song("play0000001").unwrap().unwrap();
    db.increment_total_play_time("play0000001", 120_000)
        .unwrap();
    read_before.liked = true;
    store.save_song(&read_before).unwrap();

    let stored: SongEntity = db.fetch_one("play0000001").unwrap().unwrap();
    assert!(stored.liked);
    assert_eq!(stored.total_play_time, 120_000);
}

#[tokio::test]
async fn failed_like_rollback_keeps_play_time_banked_during_the_request() {
    let (store, db) = store();
    let store = Arc::new(store);
    store
        .save_song(&SyncSong::skeleton("play0000002", false, ""))
        .unwrap();
    let playback = db.clone();
    let banked = std::sync::Once::new();
    let session = Session::mock(move |_| {
        // The song keeps playing while the like request is in flight
        // (banked once: the client retries a 500).
        banked.call_once(|| {
            playback
                .increment_total_play_time("play0000002", 120_000)
                .unwrap();
        });
        HttpResponse {
            status: 500,
            ..HttpResponse::default()
        }
    });
    let client = Arc::new(InnerTubeClient::with_session(
        SwiftyTubeConfig::music(),
        AuthState::guest(),
        session,
    ));
    let _ = MutationService::new(client, store)
        .like_song("play0000002")
        .await;

    let stored: SongEntity = db.fetch_one("play0000002").unwrap().unwrap();
    assert!(!stored.liked);
    assert_eq!(stored.total_play_time, 120_000);
}

// MARK: - LibraryPages

#[test]
fn next_page_token_reads_both_token_shapes_at_any_depth() {
    let shelf = json!({ "contents": [{ "musicShelfRenderer": {
        "continuations": [{ "nextContinuationData": { "continuation": "next" } }],
    } }] });
    assert_eq!(
        LibraryPages::next_page_token(&shelf).as_deref(),
        Some("next")
    );

    let grid = json!({ "items": [{ "continuationItemRenderer": { "continuationEndpoint": {
        "continuationCommand": { "token": "grid-next" },
    } } }] });
    assert_eq!(
        LibraryPages::next_page_token(&grid).as_deref(),
        Some("grid-next")
    );

    assert_eq!(
        LibraryPages::next_page_token(&json!({ "contents": [] })),
        None
    );
    assert_eq!(
        LibraryPages::next_page_token(&fixture("library_artists")).as_deref(),
        Some("artists-page-2")
    );
}

#[tokio::test]
async fn all_artists_follows_every_page_or_returns_none() {
    let (client, _) = fake_client(library(Some(fixture("library_artists_page2"))));
    let first_page =
        LibraryBrowseParser::parse_artists(fixture("library_artists").as_object().unwrap());
    let all = LibraryPages::all_artists(&client, &first_page)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);

    // Fewer than the first page means something was missed.
    let more = [first_page.clone(), first_page.clone()].concat();
    assert!(LibraryPages::all_artists(&client, &more).await.is_none());

    let (client, _) = fake_client(library(None));
    assert!(LibraryPages::all_artists(&client, &[]).await.is_none());
}

// MARK: - SyncBridge

fn bridge(
    route: impl Fn(&str, &Value) -> Option<Value> + Send + Sync + 'static,
) -> (SyncBridge, Arc<DatabaseService>, Sent) {
    let (client, sent) = fake_client(route);
    let db = Arc::new(DatabaseService::open_in_memory());
    let defaults = std::env::temp_dir()
        .join(format!(
            "trop-sync-bridge-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
        .join("sync_defaults.json");
    let _ = std::fs::remove_file(&defaults);
    (SyncBridge::new(client, db.clone(), defaults), db, sent)
}

#[tokio::test]
async fn full_sync_fills_the_database() {
    let (bridge, db, _) = bridge(library(Some(fixture("library_artists_page2"))));
    bridge.force_full_sync().await;

    let count = |sql: &str| -> i64 {
        db.read(|db| db.query_row(sql, [], |row| row.get(0)))
            .unwrap()
    };
    assert_eq!(
        count("SELECT COUNT(*) FROM artist WHERE bookmarked_at IS NOT NULL"),
        3
    );
    assert_eq!(
        count("SELECT COUNT(*) FROM album WHERE bookmarked_at IS NOT NULL"),
        2
    );
    // Liked Music merges into Liked Songs; it never becomes a playlist row.
    assert_eq!(count("SELECT COUNT(*) FROM playlist"), 2);
    assert_eq!(count("SELECT COUNT(*) FROM playlist_song_map"), 0);
    assert_eq!(count("SELECT COUNT(*) FROM song WHERE liked = 1"), 3);
    assert!(bridge.incremental.last_sync_date().is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_callers_share_one_sync() {
    // A guest menu ends each sync right after the session check; the delay keeps it running.
    let (bridge, _, sent) = bridge(|endpoint, _| {
        if endpoint == "account/account_menu" {
            std::thread::sleep(Duration::from_millis(300));
        }
        Some(json!({}))
    });
    let bridge = Arc::new(bridge);
    let menu_requests = || {
        sent.lock()
            .unwrap()
            .iter()
            .filter(|(endpoint, _)| endpoint == "account/account_menu")
            .count()
    };

    let first = tokio::spawn({
        let bridge = bridge.clone();
        async move { bridge.check_and_sync_if_stale().await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    bridge.check_and_sync_if_stale().await;
    first.await.unwrap();
    assert_eq!(menu_requests(), 1);

    // Once it ends, the next call starts a new sync.
    bridge.force_full_sync().await;
    assert_eq!(menu_requests(), 2);
}
