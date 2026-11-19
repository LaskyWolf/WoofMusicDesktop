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
