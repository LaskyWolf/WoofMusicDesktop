use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use trop::database::database_service::DatabaseService;
use trop::database::entities::downloaded_track_entity::DownloadedTrackEntity;
use trop::networking::models::yt_item::{SongItem, YTArtist};
use trop::services::download_manager::{DownloadManager, DownloadState, Resolved};

#[test]
fn file_names_follow_swift() {
    assert_eq!(
        DownloadManager::file_name("Song", "A, B", "abc123", "m4a"),
        "A, B - Song [abc123].m4a"
    );
    assert_eq!(
        DownloadManager::file_name("", "A", "abc123", "webm"),
        "abc123 [abc123].webm"
    );
    assert_eq!(
        DownloadManager::sanitize_file_name(" AC/DC: Live?. "),
        "AC-DC- Live-"
    );
    assert_eq!(DownloadManager::sanitize_file_name("..."), "Untitled");
    assert_eq!(
        DownloadManager::extension("audio/webm; codecs=\"opus\""),
        "webm"
    );
    assert_eq!(
        DownloadManager::extension("audio/mp4; codecs=\"mp4a.40.2\""),
        "m4a"
    );
}

/// Serves `body` once over HTTP on a local port and returns its URL.
fn serve_once(body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/audio", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        let _ = stream.read(&mut request);
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    });
    url
}

fn resolved(url: String, mime_type: &str) -> Resolved {
    Resolved {
        url,
        mime_type: mime_type.to_owned(),
        duration: Some(1),
    }
}

#[tokio::test]
async fn download_records_the_row_and_delete_removes_it() {
    let dir = std::env::temp_dir().join(format!("woof-downloads-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = Arc::new(DatabaseService::open_in_memory());
    let manager = DownloadManager::new(db.clone(), dir.clone());
    let song = SongItem {
        video_id: "vid_123456".into(),
        title: "Song".into(),
        artists: vec![YTArtist {
            name: "Artist".into(),
            id: None,
        }],
        duration: 180,
        ..SongItem::default()
    };
    let url = serve_once(vec![7; 20_000]);
    let saved = manager
        .download(song.clone(), async { Ok(resolved(url, "audio/webm")) })
        .await;
    assert_eq!(saved, Ok(true));
    assert!(manager.downloads().is_empty());

    let row = db
        .fetch_one::<DownloadedTrackEntity>("vid_123456")
        .unwrap()
        .unwrap();
    assert_eq!(row.local_path, "Artist - Song [vid_123456].webm");
    assert_eq!(row.artist, "Artist");
    let path = manager.local_path("vid_123456").unwrap();
    assert_eq!(path, dir.join(&row.local_path));
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 20_000);
    assert_eq!(manager.tracks().unwrap(), vec![row]);

    // A row whose file is gone leaves the list.
    std::fs::rename(&path, dir.join("moved")).unwrap();
    assert!(manager.tracks().unwrap().is_empty());
    assert!(
        db.fetch_one::<DownloadedTrackEntity>("vid_123456")
            .unwrap()
            .is_none()
    );

    // A failed resolve shows as a failed download until cancelled.
    let failed = manager
        .download(song.clone(), async { Err("no stream".to_owned()) })
        .await;
    assert!(failed.is_err());
    assert_eq!(manager.downloads().len(), 1);
    manager.cancel("vid_123456");
    assert!(manager.downloads().is_empty());

    // Delete removes the file and the row.
    let url = serve_once(vec![7; 20_000]);
    let saved = manager
        .download(song, async { Ok(resolved(url, "audio/mp4")) })
        .await;
    assert_eq!(saved, Ok(true));
    let path = manager.local_path("vid_123456").unwrap();
    assert!(path.ends_with("Artist - Song [vid_123456].m4a"));
    manager.delete("vid_123456").unwrap();
    assert!(!path.exists());
    assert!(manager.local_path("vid_123456").is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// One HTTP 200 answer: the first half of `len` bytes, the rest once `rest` fires.
fn serve_in_two_halves(len: usize) -> (String, mpsc::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/audio", listener.local_addr().unwrap());
    let (rest, wait) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 2048];
        let _ = stream.read(&mut request);
        let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n");
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&vec![7; len / 2]).unwrap();
        let _ = wait.recv();
        let _ = stream.write_all(&vec![7; len - len / 2]);
    });
    (url, rest)
}

/// Waits until the download of `video_id` has saved some bytes.
async fn wait_for_bytes(manager: &DownloadManager, video_id: &str) {
    for _ in 0..500 {
        let saving = manager.downloads().iter().any(|d| {
            d.song.video_id == video_id
                && matches!(d.state, DownloadState::Downloading(p) if p > 0.05)
        });
        if saving {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{video_id} saved nothing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn download_again_after_cancel_survives_the_cancelled_run() {
    let dir = std::env::temp_dir().join(format!("woof-downloads-retry-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let manager = Arc::new(DownloadManager::new(
        Arc::new(DatabaseService::open_in_memory()),
        dir.clone(),
    ));
    let song = SongItem {
        video_id: "vid_retry01".into(),
        title: "Song".into(),
        duration: 180,
        ..SongItem::default()
    };
    let start = |url: String| {
        let (manager, song) = (manager.clone(), song.clone());
        tokio::spawn(async move {
            manager
                .download(song, async { Ok(resolved(url, "audio/webm")) })
                .await
        })
    };

    let (url, first_rest) = serve_in_two_halves(200_000);
    let first = start(url);
    wait_for_bytes(&manager, "vid_retry01").await;
    manager.cancel("vid_retry01");
    let (url, second_rest) = serve_in_two_halves(200_000);
    let second = start(url);
    wait_for_bytes(&manager, "vid_retry01").await;

    // The cancelled run gets more bytes, stops and cleans up its own file.
    first_rest.send(()).unwrap();
    assert_eq!(first.await.unwrap(), Ok(false));
    second_rest.send(()).unwrap();
    assert_eq!(second.await.unwrap(), Ok(true));
    let path = manager.local_path("vid_retry01").unwrap();
    assert_eq!(std::fs::metadata(path).unwrap().len(), 200_000);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn a_batch_counts_saved_and_failed_songs_and_delete_all_empties_it() {
    let dir = std::env::temp_dir().join(format!("woof-downloads-batch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = Arc::new(DatabaseService::open_in_memory());
    let manager = DownloadManager::new(db.clone(), dir.clone());
    let song = |id: &str| SongItem {
        video_id: id.into(),
        title: id.into(),
        duration: 1,
        ..SongItem::default()
    };
    // One song alone is not a batch: its own row shows enough.
    manager.join_batch(&["vid_batch_a".into()]);
    assert_eq!(manager.batch_progress(), None);
    // A page's songs join together before the first one starts.
    manager.join_batch(&["vid_batch_a".into(), "vid_batch_b".into()]);

    let url = serve_once(vec![7; 20_000]);
    assert_eq!(
        manager
            .download(song("vid_batch_a"), async {
                Ok(resolved(url, "audio/webm"))
            })
            .await,
        Ok(true)
    );
    let failed = manager
        .download(song("vid_batch_b"), async { Err("no stream".to_owned()) })
        .await;
    assert!(failed.is_err());

    let batch = manager.batch_progress().unwrap();
    assert_eq!((batch.total, batch.completed, batch.failed), (2, 1, 1));
    assert!(batch.is_done());
    assert_eq!(batch.fraction, 1.0);
    assert_eq!(
        batch.failure_reason.as_deref(),
        Some("This song has no stream to save. Try again later.")
    );
    assert_eq!(manager.total_storage_bytes(), 20_000);

    // Closing the card ends the run; a new download starts a fresh one.
    manager.clear_finished_batch();
    assert_eq!(manager.batch_progress(), None);

    manager.delete_all().unwrap();
    assert!(manager.tracks().unwrap().is_empty());
    assert!(manager.downloads().is_empty());
    assert_eq!(manager.total_storage_bytes(), 0);
    std::fs::remove_dir_all(&dir).unwrap();
}
