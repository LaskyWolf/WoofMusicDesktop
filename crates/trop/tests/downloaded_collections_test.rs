use std::time::SystemTime;

use trop::database::database_service::DatabaseService;
use trop::database::entities::downloaded_track_entity::DownloadedTrackEntity;
use trop::database::entities::song_entity::SongEntity;
use trop::services::downloaded_collections::DownloadedCollections;

fn song(id: &str) -> SongEntity {
    SongEntity {
        id: id.into(),
        title: id.into(),
        artist_name: None,
        album_name: None,
        duration: 1,
        thumbnail_url: None,
        liked: false,
        total_play_time: 0,
        in_library: None,
        library_add_token: String::new(),
        library_remove_token: String::new(),
        is_episode: false,
        is_uploaded: false,
        is_video: false,
        create_date: SystemTime::now(),
        modify_date: SystemTime::now(),
        is_unavailable: false,
    }
}

fn saved(id: &str) -> DownloadedTrackEntity {
    DownloadedTrackEntity {
        id: id.into(),
        title: id.into(),
        artist: String::new(),
        duration: 1,
        thumbnail_url: None,
        local_path: format!("{id}.m4a"),
        downloaded_at: SystemTime::now(),
    }
}

#[test]
fn collections_with_every_song_saved_are_complete() {
    let db = DatabaseService::open_in_memory();
    for id in ["a", "b", "c"] {
        db.insert(&song(id)).unwrap();
    }
    // A playlist under its VL id, and an album saved by `save_album_songs`.
    for song_id in ["a", "b"] {
        db.write(|tx| {
            tx.execute(
                "INSERT INTO playlist_song_map (playlist_id, song_id, position) VALUES ('VLPLx', ?, 0)",
                [song_id],
            )
        })
        .unwrap();
    }
    DownloadedCollections::save_album_songs(&db, "MPREb_1", &["b".into(), "c".into()]).unwrap();

    assert_eq!(
        DownloadedCollections::read(&db).unwrap(),
        DownloadedCollections::default()
    );

    db.insert(&saved("a")).unwrap();
    db.insert(&saved("b")).unwrap();
    let collections = DownloadedCollections::read(&db).unwrap();
    // The playlist answers to its bare id and its VL id.
    assert!(collections.is_complete("PLx") && collections.is_complete("VLPLx"));
    assert!(collections.has_downloads("PLx"));
    // The album has "b" saved but not "c".
    assert!(!collections.is_complete("MPREb_1"));
    assert!(collections.has_downloads("MPREb_1"));

    db.insert(&saved("c")).unwrap();
    assert!(
        DownloadedCollections::read(&db)
            .unwrap()
            .is_complete("MPREb_1")
    );

    // Saving again replaces the album's songs.
    DownloadedCollections::save_album_songs(&db, "MPREb_1", &["c".into()]).unwrap();
    let count: i64 = db
        .read(|c| c.query_row("SELECT COUNT(*) FROM album_song", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(count, 1);
}
