//! Home's sections built from the library: Quick Picks, Keep Listening,
//! Forgotten Favorites, the mixes, their order among YouTube's shelves, and
//! the saved page shown at launch.

mod common;

use std::time::{Duration, SystemTime};

use common::{fixture, fixture_client};
use trop::database::database_service::DatabaseService;
use trop::database::entities::album_entity::AlbumEntity;
use trop::database::entities::artist_entity::ArtistEntity;
use trop::database::entities::event::Event;
use trop::database::entities::song_entity::SongEntity;
use trop::home::home_view_model::{HomeViewModel, SavedHome};
use trop::home::mixes::{self, MixKind};
use trop::networking::models::home_page::HomeSection;
use trop::networking::models::yt_item::YTItem;
use trop::services::personalization_service as personalization;

const DAY: Duration = Duration::from_secs(86_400);

fn song(id: &str, artist: &str, liked: bool, play_time: i64) -> SongEntity {
    SongEntity {
        id: id.into(),
        title: format!("Song {id}"),
        artist_name: Some(artist.into()),
        album_name: None,
        duration: 200,
        thumbnail_url: Some(format!("https://i.ytimg.com/{id}.jpg")),
        liked,
        total_play_time: play_time,
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

fn play(db: &DatabaseService, id: &str, days_ago: u32) {
    db.insert(&Event {
        id: None,
        song_id: id.into(),
        timestamp: SystemTime::now() - days_ago * DAY - Duration::from_secs(60),
        play_time: 180_000,
    })
    .unwrap();
}

/// 12 liked songs by "Daft Punk" (one played a year ago, the rest never),
/// 6 songs by "Justice" played in the last week, a saved album and artist.
fn library() -> DatabaseService {
    let db = DatabaseService::open_in_memory();
    for i in 0..12 {
        db.save(&song(&format!("dp{i}"), "Daft Punk", true, 1000 + i))
            .unwrap();
    }
    play(&db, "dp0", 200);
    for i in 0..6 {
        let id = format!("ju{i}");
        db.save(&song(&id, "Justice", false, 500)).unwrap();
        play(&db, &id, i);
    }
    db.save(&AlbumEntity {
        id: "MPREb_1".into(),
        title: "Discovery".into(),
        playlist_id: None,
        thumbnail_url: None,
        song_count: 14,
        duration: 0,
        bookmarked_at: Some(SystemTime::now()),
        is_uploaded: false,
    })
    .unwrap();
    db.save(&ArtistEntity {
        id: "UC1".into(),
        name: "Daft Punk".into(),
        thumbnail_url: None,
        bookmarked_at: Some(SystemTime::now()),
        is_podcast_channel: false,
        channel_id: None,
    })
    .unwrap();
    db
}

fn ids(section: &HomeSection) -> Vec<&str> {
    section.items().iter().map(|item| item.id()).collect()
}

#[test]
fn quick_picks_are_liked_songs_up_to_the_limit() {
    let db = library();
    let section = personalization::build_quick_picks(&db, 9);
    assert!(matches!(section, HomeSection::QuickPicks(_)));
    assert_eq!(section.items().len(), 9);
    assert!(ids(&section).iter().all(|id| id.starts_with("dp")));
}

#[test]
fn keep_listening_is_recent_plays_then_albums_then_artists() {
    let db = library();
    let section = personalization::build_keep_listening(&db);
    // Newest play first; dp0 (200 days ago) is within the year but past the six newest.
    assert_eq!(
        ids(&section),
        ["ju0", "ju1", "ju2", "ju3", "ju4", "ju5", "MPREb_1", "UC1"]
    );
    assert!(matches!(section.items()[7], YTItem::Artist(ref a) if a.is_subscribed));
}

#[test]
fn forgotten_favorites_skip_songs_played_in_sixty_days() {
    let db = library();
    play(&db, "dp5", 3);
    let section = personalization::build_forgotten_favorites(&db);
    let ids = ids(&section);
    assert_eq!(ids.len(), 10);
    assert!(!ids.contains(&"dp5"));
    // Least played first.
    assert_eq!(ids[0], "dp0");
}

#[test]
fn mixes_need_five_songs_and_name_the_top_artists() {
    let db = library();
    let mixes = mixes::build(&db, SystemTime::now());
    let titles: Vec<&str> = mixes.iter().map(|m| m.title.as_str()).collect();
    // Rewind needs plays between a year and a month ago: dp0 alone is too few.
    assert_eq!(
        titles,
        ["Flow", "On Repeat", "Daft Punk Mix", "Justice Mix"]
    );
    assert_eq!(mixes[0].kind, MixKind::Flow);
    assert_eq!(mixes[0].songs.len(), 18);
    // On Repeat keeps the most played order.
    assert_eq!(mixes[1].songs.len(), 6);
    assert!(
        mixes[2]
            .songs
            .iter()
            .all(|s| s.artists[0].name == "Daft Punk")
    );

    assert!(mixes::build(&DatabaseService::open_in_memory(), SystemTime::now()).is_empty());
}

#[test]
fn similar_to_reads_the_artist_song_shelf() {
    let items = personalization::related_items(&fixture("artist"));
    assert!(!items.is_empty() && items.len() <= 10);
    assert!(items.iter().all(|item| matches!(item, YTItem::Song(_))));
}

#[tokio::test]
async fn local_sections_sit_around_the_server_shelves() {
    let db = library();
    let (client, _) = fixture_client(|_| "home");
    let mut model = HomeViewModel::new(client);
    model.load_local_sections(&db, 18);
    model.load().await.unwrap();
    let sections = model.home_sections();
    assert!(matches!(sections[0], HomeSection::KeepListening(_)));
    assert!(matches!(sections[1], HomeSection::QuickPicks(_)));
    assert!(
        sections
            .iter()
            .any(|s| matches!(s, HomeSection::ForgottenFavorites(_)))
    );
    assert!(!model.mixes.is_empty());

    // Off: no Quick Picks at all, not even the server's.
    model.show_quick_picks = false;
    model.load_local_sections(&db, 18);
    assert!(
        model
            .home_sections()
            .iter()
            .all(|s| !matches!(s, HomeSection::QuickPicks(_)))
    );
}

#[tokio::test]
async fn the_last_page_shows_until_the_new_one_loads() {
    let path = std::env::temp_dir().join(format!("woof-home-{}.json", std::process::id()));
    SavedHome::clear(&path);
    let (client, _) = fixture_client(|_| "home");

    let mut first = HomeViewModel::new(client.clone());
    first.saved_home = Some(path.clone());
    assert!(!first.show_saved_home());
    first.fetch_home_page().await.unwrap();

    let mut next = HomeViewModel::new(client);
    next.saved_home = Some(path.clone());
    assert!(next.show_saved_home());
    let saved = next.home_page.as_ref().unwrap();
    assert_eq!(
        saved.sections.len(),
        first.home_page.as_ref().unwrap().sections.len()
    );
    // The saved token may have expired.
    assert_eq!(saved.continuation, None);
    // A page on screen is never replaced by the saved one.
    assert!(!next.show_saved_home());
    SavedHome::clear(&path);
}

#[test]
fn an_empty_library_builds_no_sections_and_no_mixes() {
    let db = DatabaseService::open_in_memory();
    assert!(
        personalization::build_quick_picks(&db, 18)
            .items()
            .is_empty()
    );
    assert!(
        personalization::build_keep_listening(&db)
            .items()
            .is_empty()
    );
    assert!(
        personalization::build_forgotten_favorites(&db)
            .items()
            .is_empty()
    );
    assert!(mixes::build(&db, SystemTime::now()).is_empty());
    let mut home = HomeViewModel::new(fixture_client(|_| "home").0);
    home.load_local_sections(&db, 18);
    assert!(home.local_sections.is_empty() && home.mixes.is_empty());
}

#[test]
fn one_play_gives_keep_listening_and_no_mix() {
    let db = DatabaseService::open_in_memory();
    db.save(&song("a", "Solo", false, 5000)).unwrap();
    play(&db, "a", 1);
    assert_eq!(ids(&personalization::build_keep_listening(&db)), ["a"]);
    assert!(mixes::build(&db, SystemTime::now()).is_empty());
}

#[test]
fn songs_without_an_artist_or_title_do_not_break_the_mixes() {
    let db = DatabaseService::open_in_memory();
    for i in 0..8 {
        let mut s = song(&format!("n{i}"), "x", true, 100 + i);
        s.artist_name = None;
        s.thumbnail_url = None;
        if i == 0 {
            s.title = String::new();
        }
        db.save(&s).unwrap();
    }
    let built = mixes::build(&db, SystemTime::now());
    let flow = built.iter().find(|m| m.kind == MixKind::Flow).unwrap();
    assert_eq!(flow.songs.len(), 7);
    assert!(built.iter().all(|m| m.kind != MixKind::Artist));
}

#[test]
fn a_mix_lists_each_song_once_and_never_more_than_thirty() {
    let db = DatabaseService::open_in_memory();
    // Liked and most played at once: it must not show twice.
    for i in 0..45 {
        db.save(&song(&format!("s{i}"), "Same Name", true, 1000 + i))
            .unwrap();
    }
    let built = mixes::build(&db, SystemTime::now());
    for mix in &built {
        let unique: std::collections::HashSet<_> = mix.songs.iter().map(|s| &s.video_id).collect();
        assert_eq!(unique.len(), mix.songs.len(), "{}", mix.id);
        assert!(mix.songs.len() <= 30);
    }
    assert_eq!(
        built.iter().filter(|m| m.kind == MixKind::Artist).count(),
        1
    );
}

#[test]
fn a_corrupt_or_missing_saved_page_shows_nothing() {
    let dir = std::env::temp_dir().join(format!("woof-home-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let client = fixture_client(|_| "home").0;
    for (name, bytes) in [
        ("empty", &b""[..]),
        ("garbage", b"\x00\xff not json"),
        ("array", b"[1,2,3]"),
        ("object", b"{}"),
        ("truncated", b"{\"contents\":{\"singleColumn"),
    ] {
        let path = dir.join(format!("{name}.json"));
        std::fs::write(&path, bytes).unwrap();
        let mut home = HomeViewModel::new(client.clone());
        home.saved_home = Some(path);
        assert!(!home.show_saved_home(), "{name}");
        assert!(home.home_page.is_none(), "{name}");
    }
    let mut home = HomeViewModel::new(client);
    home.saved_home = Some(dir.join("missing.json"));
    assert!(!home.show_saved_home());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_huge_history_still_builds_the_sections() {
    let db = DatabaseService::open_in_memory();
    for i in 0..3000 {
        db.save(&song(
            &format!("h{i}"),
            &format!("Artist {}", i % 7),
            i % 3 == 0,
            i,
        ))
        .unwrap();
        if i % 2 == 0 {
            play(&db, &format!("h{i}"), (i % 400) as u32);
        }
    }
    let mut home = HomeViewModel::new(fixture_client(|_| "home").0);
    home.load_local_sections(&db, 18);
    assert!(home.local_sections.iter().all(|s| s.items().len() <= 18));
    assert!(home.mixes.iter().all(|m| m.songs.len() <= 30));
}
