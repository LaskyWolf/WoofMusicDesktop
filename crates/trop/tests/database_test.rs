use std::collections::BTreeSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use trop::database::database_migrations::columns;
use trop::database::database_service::{
    ConflictResolution, DatabaseService, LibrarySongSort, MIGRATIONS, migrate,
};
use trop::database::entities::album_entity::AlbumEntity;
use trop::database::entities::artist_entity::ArtistEntity;
use trop::database::entities::downloaded_track_entity::DownloadedTrackEntity;
use trop::database::entities::event::Event;
use trop::database::entities::playlist_entity::PlaylistEntity;
use trop::database::entities::playlist_song_map::PlaylistSongMap;
use trop::database::entities::podcast_entity::PodcastEntity;
use trop::database::entities::song_entity::SongEntity;
use trop::database::{DbDate, Record, date_text};
use trop::networking::models::yt_item::{SongItem, YTArtist};

/// 2026-11-17 10:30:00 UTC.
fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_794_911_400)
}

fn minutes(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

fn song(id: &str, title: &str, liked: bool) -> SongEntity {
    SongEntity {
        id: id.into(),
        title: title.into(),
        artist_name: Some("A".into()),
        album_name: None,
        duration: 200,
        thumbnail_url: None,
        liked,
        total_play_time: 0,
        in_library: None,
        library_add_token: String::new(),
        library_remove_token: String::new(),
        is_episode: false,
        is_uploaded: false,
        is_video: false,
        create_date: now(),
        modify_date: now(),
        is_unavailable: false,
    }
}

fn event(song_id: &str, timestamp: SystemTime, play_time: i64) -> Event {
    Event {
        id: None,
        song_id: song_id.into(),
        timestamp,
        play_time,
    }
}

fn strings(db: &DatabaseService, sql: &str) -> Vec<String> {
    db.read(|c| c.prepare(sql)?.query_map([], |r| r.get(0))?.collect())
        .unwrap()
}

/// `PRAGMA table_info` rows as (name, type, notnull, default, pk).
fn table_info(
    db: &DatabaseService,
    table: &str,
) -> Vec<(String, String, bool, Option<String>, i64)> {
    db.read(|c| {
        c.prepare(&format!("PRAGMA table_info(\"{table}\")"))?
            .query_map([], |r| {
                Ok((r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
            })?
            .collect()
    })
    .unwrap()
}

fn temp_db_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("trop-db-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("WoofMusic").join("trop.sqlite")
}

#[test]
fn migrations_create_every_table_and_column_on_an_empty_database() {
    let db = DatabaseService::open_in_memory();
    let tables: BTreeSet<String> = strings(
        &db,
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .into_iter()
    .collect();
    let expected = [
        "grdb_migrations",
        "song",
        "artist",
        "album",
        "playlist",
        "playlist_song_map",
        "event",
        "play_count",
        "format",
        "episode",
        "podcast",
        "podcast_episode_map",
        "downloaded_track",
        "speed_dial",
        "album_song",
    ];
    assert_eq!(tables, expected.iter().map(|t| t.to_string()).collect());

    assert_eq!(
        strings(&db, "SELECT identifier FROM grdb_migrations ORDER BY rowid"),
        MIGRATIONS
            .iter()
            .map(|(id, _)| id.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        strings(
            &db,
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'event_on_timestamp'"
        ),
        ["event_on_timestamp"]
    );

    // Every entity's columns exist in its table.
    fn has_columns<T: Record>(db: &DatabaseService) {
        let names: Vec<String> = table_info(db, T::TABLE).into_iter().map(|c| c.0).collect();
        for column in T::COLUMNS {
            assert!(names.contains(&column.to_string()), "{}.{column}", T::TABLE);
        }
    }
    has_columns::<SongEntity>(&db);
    has_columns::<ArtistEntity>(&db);
    has_columns::<AlbumEntity>(&db);
    has_columns::<PlaylistEntity>(&db);
    has_columns::<PlaylistSongMap>(&db);
    has_columns::<Event>(&db);
    has_columns::<PodcastEntity>(&db);
    has_columns::<DownloadedTrackEntity>(&db);
    has_columns::<trop::database::entities::episode_entity::EpisodeEntity>(&db);
    has_columns::<trop::database::entities::format_entity::FormatEntity>(&db);
    has_columns::<trop::database::entities::speed_dial_entry::SpeedDialEntry>(&db);

    // Types, NOT NULL, defaults and keys as GRDB writes them.
    let song = table_info(&db, "song");
    let col = |name: &str| song.iter().find(|c| c.0 == name).unwrap().clone();
    assert_eq!(col("id"), ("id".into(), "TEXT".into(), false, None, 1));
    assert_eq!(
        col("liked"),
        ("liked".into(), "INTEGER".into(), true, Some("0".into()), 0)
    );
    assert_eq!(col("library_add_token").3, Some("''".into()));
    assert_eq!(
        col("in_library"),
        ("in_library".into(), "TEXT".into(), false, None, 0)
    );
    assert_eq!(
        col("is_unavailable"),
        (
            "is_unavailable".into(),
            "BOOLEAN".into(),
            true,
            Some("0".into()),
            0
        )
    );
    assert_eq!(song.last().unwrap().0, "is_unavailable");

    let speed_dial = table_info(&db, "speed_dial");
    assert_eq!(
        speed_dial[6],
        ("pinned_at".into(), "DATETIME".into(), true, None, 0)
    );
    assert_eq!(table_info(&db, "format")[7].1, "DOUBLE");
    let play_count_pk: Vec<_> = table_info(&db, "play_count")
        .iter()
        .filter(|c| c.4 > 0)
        .map(|c| c.0.clone())
        .collect();
    assert_eq!(play_count_pk, ["song_id", "year", "month"]);
    let album_song_pk: Vec<_> = table_info(&db, "album_song")
        .iter()
        .filter(|c| c.4 > 0)
        .map(|c| c.0.clone())
        .collect();
    assert_eq!(album_song_pk, ["album_id", "song_id"]);
    assert_eq!(
        table_info(&db, "downloaded_track")
            .into_iter()
            .map(|c| c.0)
            .collect::<Vec<_>>(),
        [
            "id",
            "title",
            "artist",
            "duration",
            "thumbnail_url",
            "local_path",
            "downloaded_at"
        ]
    );
    let sql = strings(
        &db,
        "SELECT sql FROM sqlite_master WHERE name = 'playlist_song_map'",
    );
    assert!(sql[0].contains(r#""id" INTEGER PRIMARY KEY AUTOINCREMENT"#));
    assert!(sql[0].contains(r#"UNIQUE ("playlist_id", "song_id")"#));
}

#[test]
fn migrations_are_idempotent_on_reopen() {
    let path = temp_db_path("reopen");
    let first = DatabaseService::open(&path);
    assert!(!first.uses_temporary_storage());
    first.insert(&song("v1", "One", true)).unwrap();
    drop(first);

    let second = DatabaseService::open(&path);
    assert!(!second.uses_temporary_storage());
    assert_eq!(
        strings(&second, "SELECT identifier FROM grdb_migrations").len(),
        MIGRATIONS.len()
    );
    assert_eq!(second.fetch_liked_song_ids().unwrap(), ["v1"]);
    assert_eq!(strings(&second, "PRAGMA journal_mode"), ["wal"]);

    // Running the migrator again changes nothing.
    let mut raw = Connection::open(&path).unwrap();
    migrate(&mut raw).unwrap();
    let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
}

#[test]
fn open_falls_back_to_temporary_storage() {
    let path = temp_db_path("fallback");
    // A directory where the file should be makes the open fail.
    std::fs::create_dir_all(&path).unwrap();
    let db = DatabaseService::open(&path);
    assert!(db.uses_temporary_storage());
    db.insert(&song("v1", "One", false)).unwrap();
    assert!(db.fetch_one::<SongEntity>("v1").unwrap().is_some());

    let other = temp_db_path("fallback-reopen");
    db.reopen(&other);
    assert!(!db.uses_temporary_storage());
    assert!(db.fetch_one::<SongEntity>("v1").unwrap().is_none());
    for p in [path, other] {
        let _ = std::fs::remove_dir_all(p.parent().unwrap().parent().unwrap());
    }
}

#[test]
fn v5_to_v7_repair_a_legacy_downloaded_track_table() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE downloaded_track (id TEXT PRIMARY KEY, title TEXT, duration INTEGER, local_path TEXT, file_size INTEGER);
         INSERT INTO downloaded_track VALUES ('kept', 'Song', 180, '/music/kept.m4a', 1234);
         INSERT INTO downloaded_track VALUES ('dropped', 'No file', 90, NULL, 0);",
    )
    .unwrap();
    migrate(&mut conn).unwrap();

    assert_eq!(
        columns(&conn, "downloaded_track").unwrap(),
        [
            "id",
            "title",
            "artist",
            "duration",
            "thumbnail_url",
            "local_path",
            "downloaded_at"
        ]
    );
    let rows: Vec<DownloadedTrackEntity> = conn
        .prepare("SELECT * FROM downloaded_track")
        .unwrap()
        .query_map([], DownloadedTrackEntity::from_row)
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0].id.as_str(),
            rows[0].artist.as_str(),
            rows[0].duration
        ),
        ("kept", "", 180)
    );
    let raw: String = conn
        .query_row("SELECT downloaded_at FROM downloaded_track", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(raw.ends_with('Z') && raw.contains('T'), "{raw}");
    let age = SystemTime::now()
        .duration_since(rows[0].downloaded_at)
        .unwrap();
    assert!(age < Duration::from_secs(60));
}

#[test]
fn dates_use_grdb_text() {
    assert_eq!(date_text(now()), "2026-11-17 10:30:00.000");
    assert_eq!(
        date_text(now() + Duration::from_millis(45_678)),
        "2026-11-17 10:30:45.678"
    );
    assert_eq!(
        date_text(UNIX_EPOCH + Duration::from_secs(951_825_600)),
        "2000-02-29 12:00:00.000"
    );
    assert_eq!(
        date_text(UNIX_EPOCH - Duration::from_secs(1)),
        "1969-12-31 23:59:59.000"
    );
    assert_eq!(DbDate::parse("2026-11-17 10:30:00.000"), Some(now()));
    assert_eq!(DbDate::parse("2026-11-17T10:30:00Z"), Some(now()));
    assert_eq!(DbDate::parse("2026-11-17 10:30"), Some(now()));
    assert_eq!(
        DbDate::parse("2026-11-17"),
        Some(now() - Duration::from_secs(37_800))
    );
    assert_eq!(DbDate::parse(""), None);
    assert_eq!(DbDate::parse("2026-13-01"), None);
}

/// Dart `database_test.dart`: history, play counts and liked songs round-trip.
#[test]
fn history_play_counts_and_liked_songs_round_trip() {
    let db = DatabaseService::open_in_memory();
    db.insert(&song("v1", "One", true)).unwrap();
    let first = db.insert(&event("v1", now(), 31_000)).unwrap();
    let second = db
        .insert(&event("gone", now() + minutes(5), 40_000))
        .unwrap();
    assert_eq!((first, second), (1, 2));
    db.increment_play_count("v1", 2026, 11).unwrap();
    db.increment_play_count("v1", 2026, 11).unwrap();
    db.increment_total_play_time("v1", 5000).unwrap();

    let history = db.fetch_history(None).unwrap();
    assert_eq!(
        history
            .iter()
            .map(|h| h.event.song_id.as_str())
            .collect::<Vec<_>>(),
        ["gone", "v1"]
    );
    assert_eq!(history[0].song, None);
    assert_eq!(history[1].song.as_ref().unwrap().title, "One");
    assert_eq!(
        history[1].event,
        Event {
            id: Some(1),
            ..event("v1", now(), 31_000)
        }
    );
    assert_eq!(db.fetch_history(Some(1)).unwrap().len(), 1);
    assert_eq!(
        strings(&db, "SELECT CAST(count AS TEXT) FROM play_count"),
        ["2"]
    );
    assert_eq!(
        db.fetch_one::<SongEntity>("v1")
            .unwrap()
            .unwrap()
            .total_play_time,
        5000
    );
    assert_eq!(db.fetch_liked_song_ids().unwrap(), ["v1"]);
    assert_eq!(db.orphan_song_ids().unwrap(), ["gone"]);

    // (song_id, timestamp) is unique.
    assert!(db.insert(&event("v1", now(), 1)).is_err());
}

#[test]
fn song_crud_and_liked_queries() {
    let db = DatabaseService::open_in_memory();
    let mut a = song("a", "Bravo", true);
    a.artist_name = Some("Zed".into());
    a.total_play_time = 50;
    a.in_library = Some(now());
    let mut b = song("b", "Alpha", true);
    b.artist_name = Some("Amy".into());
    b.total_play_time = 30;
    b.create_date = now() + minutes(1);
    let c = song("c", "Charlie", false);
    for s in [&a, &b, &c] {
        db.insert(s).unwrap();
    }
    assert_eq!(db.fetch_one::<SongEntity>("a").unwrap(), Some(a.clone()));
    assert!(db.insert(&a).is_err());
    db.insert_on_conflict(&a, ConflictResolution::Ignore)
        .unwrap();
    assert_eq!(db.fetch_all::<SongEntity>().unwrap().len(), 3);

    let ids = |songs: Vec<SongEntity>| songs.into_iter().map(|s| s.id).collect::<Vec<_>>();
    assert_eq!(ids(db.fetch_all_liked_songs(None).unwrap()), ["b", "a"]);
    assert_eq!(
        ids(db
            .fetch_all_liked_songs(Some(LibrarySongSort::Title))
            .unwrap()),
        ["b", "a"]
    );
    assert_eq!(
        ids(db
            .fetch_all_liked_songs(Some(LibrarySongSort::Artist))
            .unwrap()),
        ["b", "a"]
    );
    assert_eq!(
        ids(db
            .fetch_all_liked_songs(Some(LibrarySongSort::MostPlayed))
            .unwrap()),
        ["a", "b"]
    );
    assert_eq!(ids(db.fetch_liked_songs().unwrap()), ["a", "b"]);
    assert_eq!(db.fetch_all_liked_song_count().unwrap(), 2);
    assert_eq!(ids(db.fetch_top_songs(1).unwrap()), ["a"]);
    assert_eq!(LibrarySongSort::MostPlayed.display_name(), "Most Played");

    // Unlike: update; save inserts a missing row and updates an existing one.
    a.liked = false;
    db.update(&a).unwrap();
    assert_eq!(db.fetch_liked_song_ids().unwrap(), ["b"]);
    assert!(matches!(
        db.update(&song("missing", "?", false)),
        Err(rusqlite::Error::QueryReturnedNoRows)
    ));
    db.save(&song("d", "Delta", true)).unwrap();
    a.title = "Bravo 2".into();
    db.save(&a).unwrap();
    assert_eq!(
        db.fetch_one::<SongEntity>("a").unwrap().unwrap().title,
        "Bravo 2"
    );
    db.insert_or_replace(&song("d", "Delta 2", false)).unwrap();
    assert_eq!(
        db.fetch_one::<SongEntity>("d").unwrap().unwrap().title,
        "Delta 2"
    );
    assert!(db.delete(&c).unwrap());
    assert!(!db.delete(&c).unwrap());

    let found: Vec<SongEntity> = db
        .fetch_all_sql("SELECT * FROM song WHERE title LIKE ? ORDER BY id", ["%a%"])
        .unwrap();
    assert_eq!(ids(found), ["a", "b", "d"]);
}

#[test]
fn recent_top_and_forgotten_songs_follow_events() {
    let db = DatabaseService::open_in_memory();
    let real_now = SystemTime::now();
    let day = Duration::from_secs(86_400);
    for id in ["recent", "older", "forgotten"] {
        db.insert(&song(id, id, true)).unwrap();
    }
    db.insert(&event("older", real_now - 2 * day, 1000))
        .unwrap();
    db.insert(&event("recent", real_now - day, 5000)).unwrap();
    db.insert(&event("forgotten", real_now - 90 * day, 9000))
        .unwrap();

    let ids = |songs: Vec<SongEntity>| songs.into_iter().map(|s| s.id).collect::<Vec<_>>();
    assert_eq!(
        ids(db.fetch_recent_songs(None, None).unwrap()),
        ["recent", "older"]
    );
    assert_eq!(
        ids(db.fetch_recent_songs(Some(100), Some(1)).unwrap()),
        ["recent"]
    );
    assert_eq!(
        ids(db.fetch_forgotten_favorites(None, None).unwrap()),
        ["forgotten"]
    );
    assert_eq!(
        ids(db
            .fetch_top_songs_between(10, real_now - 3 * day, real_now)
            .unwrap()),
        ["recent", "older"]
    );
    assert!(
        db.fetch_top_songs_between(10, real_now, real_now + day)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn playlists_and_song_map_positions() {
    let db = DatabaseService::open_in_memory();
    let playlist = |id: &str, bookmarked: Option<SystemTime>| PlaylistEntity {
        id: id.into(),
        browse_id: Some(format!("VL{id}")),
        name: id.into(),
        thumbnail_url: None,
        is_editable: true,
        bookmarked_at: bookmarked,
        remote_song_count: Some(2),
        is_auto_sync: false,
    };
    db.insert(&playlist("PL1", Some(now()))).unwrap();
    db.insert(&playlist("PL2", Some(now() + minutes(1))))
        .unwrap();
    db.insert(&playlist("SE", Some(now() + minutes(2))))
        .unwrap();
    db.insert(&playlist("PL3", None)).unwrap();
    let ids: Vec<_> = db
        .fetch_playlists(None)
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, ["PL2", "PL1"]);
    assert_eq!(
        db.fetch_one::<PlaylistEntity>("PL1").unwrap(),
        Some(playlist("PL1", Some(now())))
    );

    db.write(|tx| {
        for (position, song_id) in ["s2", "s1", "s3"].iter().enumerate() {
            tx.execute(
                "INSERT INTO playlist_song_map (playlist_id, song_id, position, set_video_id) VALUES ('PL1', ?, ?, NULL)",
                rusqlite::params![song_id, position as i64],
            )?;
        }
        Ok(())
    })
    .unwrap();
    let map = PlaylistSongMap {
        id: None,
        playlist_id: "PL1".into(),
        song_id: "s4".into(),
        position: 3,
        set_video_id: Some("set4".into()),
    };
    assert_eq!(db.insert(&map).unwrap(), 4);
    assert!(db.insert(&map).is_err(), "(playlist_id, song_id) is unique");

    let rows: Vec<PlaylistSongMap> = db
        .fetch_all_sql(
            "SELECT * FROM playlist_song_map WHERE playlist_id = ? ORDER BY position",
            ["PL1"],
        )
        .unwrap();
    assert_eq!(
        rows.iter().map(|r| r.song_id.as_str()).collect::<Vec<_>>(),
        ["s2", "s1", "s3", "s4"]
    );
    assert_eq!(rows[3], PlaylistSongMap { id: Some(4), ..map });

    // A failing write rolls back.
    let failed = db.write(|tx| {
        tx.execute("DELETE FROM playlist_song_map", [])?;
        tx.execute("INSERT INTO nope VALUES (1)", [])
    });
    assert!(failed.is_err());
    assert_eq!(db.fetch_all::<PlaylistSongMap>().unwrap().len(), 4);

    let existing = playlist("PL1", Some(now()));
    let merged =
        PlaylistEntity::merging(Some(&existing), "PL1", None, "Fetched", 9, Some("t".into()));
    assert_eq!(
        (
            merged.name.as_str(),
            merged.is_editable,
            merged.bookmarked_at,
            merged.remote_song_count
        ),
        ("PL1", true, Some(now()), Some(9))
    );
    assert_eq!(merged.thumbnail_url.as_deref(), Some("t"));
    let fresh = PlaylistEntity::merging(None, "PL9", None, "Fetched", 1, None);
    assert_eq!(
        (fresh.name.as_str(), fresh.is_editable, fresh.bookmarked_at),
        ("Fetched", false, None)
    );
}

#[test]
fn album_artist_and_podcast_bookmarks() {
    let db = DatabaseService::open_in_memory();
    let album = |id: &str, title: &str, bookmarked: Option<SystemTime>| AlbumEntity {
        id: id.into(),
        title: title.into(),
        playlist_id: None,
        thumbnail_url: None,
        song_count: 10,
        duration: 2400,
        bookmarked_at: bookmarked,
        is_uploaded: false,
    };
    db.insert(&album("a1", "Zulu", Some(now()))).unwrap();
    db.insert(&album("a2", "Alpha", Some(now() + minutes(1))))
        .unwrap();
    db.insert(&album("a3", "Mike", None)).unwrap();
    let ids = |albums: Vec<AlbumEntity>| albums.into_iter().map(|a| a.id).collect::<Vec<_>>();
    assert_eq!(ids(db.fetch_albums(None).unwrap()), ["a2", "a1"]);
    assert_eq!(ids(db.fetch_albums(Some(1)).unwrap()), ["a2"]);
    assert_eq!(ids(db.fetch_all_albums().unwrap()), ["a2", "a1"]);

    let artist = |id: &str, name: &str, bookmarked: Option<SystemTime>| ArtistEntity {
        id: id.into(),
        name: name.into(),
        thumbnail_url: None,
        bookmarked_at: bookmarked,
        is_podcast_channel: false,
        channel_id: Some(format!("UC{id}")),
    };
    db.insert(&artist("r1", "Bea", Some(now()))).unwrap();
    db.insert(&artist("r2", "Abe", Some(now()))).unwrap();
    db.insert(&artist("r3", "Cal", None)).unwrap();
    let names: Vec<_> = db
        .fetch_artists(None)
        .unwrap()
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(names, ["Abe", "Bea"]);
    // Unbookmark.
    db.save(&artist("r1", "Bea", None)).unwrap();
    assert_eq!(db.fetch_artists(None).unwrap().len(), 1);

    let podcast = |id: &str, name: &str, subscribed: Option<SystemTime>| PodcastEntity {
        id: id.into(),
        name: name.into(),
        thumbnail_url: None,
        subscribed_at: subscribed,
    };
    db.insert(&podcast("p1", "Talk", Some(now()))).unwrap();
    db.insert(&podcast("p2", "News", None)).unwrap();
    assert_eq!(
        db.fetch_all_podcasts().unwrap(),
        [podcast("p1", "Talk", Some(now()))]
    );
}

#[test]
fn speed_dial_pins_round_trip() {
    let db = DatabaseService::open_in_memory();
    let item = SongItem {
        video_id: "vid".into(),
        title: "Song".into(),
        artists: vec![
            YTArtist {
                name: "A".into(),
                id: Some("UC1".into()),
            },
            YTArtist {
                name: "B".into(),
                id: None,
            },
        ],
        album: Some("Album".into()),
        duration: 180,
        thumbnail_url: Some("https://t".into()),
        ..SongItem::default()
    };
    assert!(!db.is_pinned_to_speed_dial("vid").unwrap());
    db.pin_to_speed_dial(&item).unwrap();
    db.pin_to_speed_dial(&item).unwrap();
    assert!(db.is_pinned_to_speed_dial("vid").unwrap());
    let pinned = db.fetch_speed_dial(None).unwrap();
    assert_eq!(pinned.len(), 1);
    assert_eq!(pinned[0].artist, "A, B");
    let back = pinned[0].to_song_item();
    assert_eq!(
        back.artists,
        [YTArtist {
            name: "A, B".into(),
            id: None
        }]
    );
    assert_eq!((back.album.as_deref(), back.duration), (Some("Album"), 180));
    db.remove_from_speed_dial("vid").unwrap();
    assert!(db.fetch_speed_dial(None).unwrap().is_empty());
}

#[test]
fn song_merging_keeps_user_state() {
    let mut stored = song("v", "Old", true);
    stored.total_play_time = 99;
    stored.thumbnail_url = Some("old".into());
    stored.library_add_token = "tok".into();
    let merged = SongEntity::merging(Some(&stored), "v", "New", Some("B".into()), None, 0, None);
    assert_eq!(merged.title, "New");
    assert_eq!(merged.artist_name.as_deref(), Some("A"));
    assert_eq!(
        (merged.duration, merged.thumbnail_url.as_deref()),
        (200, Some("old"))
    );
    assert_eq!(
        (
            merged.liked,
            merged.total_play_time,
            merged.library_add_token.as_str()
        ),
        (true, 99, "tok")
    );
    assert_eq!(merged.create_date, now());
    assert!(merged.modify_date > now());

    let fresh = SongEntity::merging(
        None,
        "w",
        "T",
        Some("B".into()),
        None,
        120,
        Some("t".into()),
    );
    assert_eq!(
        (fresh.artist_name.as_deref(), fresh.duration, fresh.liked),
        (Some("B"), 120, false)
    );
}

#[test]
fn helpers() {
    assert_eq!(DatabaseService::placeholders(3), "?,?,?");
    assert_eq!(DatabaseService::placeholders(0), "");
    let ids: Vec<String> = (0..5).map(|i| i.to_string()).collect();
    assert_eq!(
        DatabaseService::chunked(&ids, 2)
            .iter()
            .map(Vec::len)
            .collect::<Vec<_>>(),
        [2, 2, 1]
    );
    assert!(DatabaseService::chunked(&[], 500).is_empty());
    if let Some(path) = DatabaseService::default_path() {
        assert!(path.ends_with("WoofMusic/trop.sqlite"));
    }
}
