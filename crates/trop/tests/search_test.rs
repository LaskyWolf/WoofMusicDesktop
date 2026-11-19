//! Port of the Dart `test/trop/search_test.dart`.

mod common;

use common::{expect_well_formed, fixture, fixture_bytes};
use trop::networking::models::yt_item::{SongItem, YTItem};
use trop::search::search_parser::{SearchParser, SearchSection};
use trop::search::search_service::SearchService;
use trop::search::search_view_model::{self as model, SearchSort};

const ORDER: [&str; 7] = [
    "Top result",
    "Songs",
    "Albums",
    "Artists",
    "Playlists",
    "Podcasts",
    "Profiles",
];

fn sections() -> Vec<SearchSection> {
    SearchParser::parse_search_results(&fixture("search_daft_punk"))
}

fn named<'a>(sections: &'a [SearchSection], title: &str) -> &'a SearchSection {
    sections.iter().find(|s| s.title == title).unwrap()
}

#[test]
fn sections_in_app_order_each_with_items() {
    let sections = sections();
    assert_eq!(sections[0].title, "Top result");
    let known: Vec<usize> = sections
        .iter()
        .filter_map(|s| ORDER.iter().position(|t| *t == s.title))
        .collect();
    assert!(known.is_sorted());
    assert!(
        sections
            .iter()
            .all(|s| s.title != "Videos" && s.title != "Episodes")
    );
    for section in &sections {
        assert!(!section.items.is_empty());
        section.items.iter().for_each(expect_well_formed);
    }
}

#[test]
fn top_result_is_the_daft_punk_artist_with_its_audience_line() {
    let sections = sections();
    let top = &sections[0];
    let YTItem::Artist(artist) = &top.items[0] else {
        panic!("top result is not an artist: {:?}", top.items[0]);
    };
    assert_eq!(artist.name, "Daft Punk");
    assert!(artist.browse_id.starts_with("UC"));
    assert!(top.subtitle.as_deref().is_some_and(|s| !s.is_empty()));
}

#[test]
fn top_result_song_reads_artists_and_duration_from_the_card_subtitle() {
    let sections = SearchParser::parse_search_results(&fixture("search_top_result_song"));
    let YTItem::Song(song) = &sections[0].items[0] else {
        panic!("top result is not a song: {:?}", sections[0].items[0]);
    };
    assert_eq!(song.video_id, "5NV6Rdv1a3I");
    assert_eq!(song.artists.len(), 1);
    assert_eq!(song.artists[0].name, "Daft Punk");
    assert_eq!(
        song.artists[0].id.as_deref(),
        Some("UCRr1xG_2WIDs18a6cIiCxeA")
    );
    assert_eq!(song.duration, 249);
    assert_eq!(song.album, None);
}

#[test]
fn songs_albums_artists_playlists_and_podcasts_are_detected() {
    let sections = sections();
    let songs = &named(&sections, "Songs").items;
    assert!(songs.iter().all(|i| matches!(i, YTItem::Song(_))));
    let YTItem::Song(first) = &songs[0] else {
        unreachable!()
    };
    assert!(first.artists.iter().any(|a| a.name == "Daft Punk"));
    assert!(
        named(&sections, "Albums")
            .items
            .iter()
            .all(|i| matches!(i, YTItem::Album(a) if !a.artists.is_empty() && a.year.is_some()))
    );
    let all =
        |title: &str, check: fn(&YTItem) -> bool| named(&sections, title).items.iter().all(check);
    assert!(all("Artists", |i| matches!(i, YTItem::Artist(_))));
    assert!(all("Playlists", |i| matches!(i, YTItem::Playlist(_))));
    assert!(all("Podcasts", |i| matches!(i, YTItem::Podcast(_))));
}

#[test]
fn parse_suggestions_reads_the_text_suggestions() {
    let suggestions = SearchService::parse_suggestions(&fixture_bytes("suggestions_daft_p"));
    assert!(!suggestions.is_empty());
    assert!(suggestions.iter().all(|s| s.starts_with("daft p")));
    assert!(SearchService::parse_suggestions(b"not json").is_empty());
}

#[test]
fn search_playlist_author_is_the_owner_not_the_kind_word() {
    let sections = sections();
    let playlists = &named(&sections, "Playlists").items;
    for item in playlists {
        let YTItem::Playlist(playlist) = item else {
            panic!("not a playlist: {item:?}");
        };
        assert_ne!(
            playlist.author.as_deref(),
            Some("Playlist"),
            "{}",
            playlist.title
        );
    }
    // Row byline: "Playlist • Christian Isbej • 687 views".
    assert!(playlists.iter().any(|p| matches!(p,
        YTItem::Playlist(p) if p.author.as_deref() == Some("Christian Isbej"))));
}

#[test]
fn top_result_playlist_id_loses_only_the_vl_prefix() {
    let response = serde_json::json!({ "contents": { "sectionListRenderer": { "contents": [
        { "musicCardShelfRenderer": {
            "title": { "runs": [{ "text": "Mix" }] },
            "onTap": { "browseEndpoint": {
                "browseId": "VLPLabcVLdef",
                "browseEndpointContextSupportedConfigs": {
                    "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_PLAYLIST" }
                }
            }}
        }}
    ]}}});
    let sections = SearchParser::parse_search_results(response.as_object().unwrap());
    assert_eq!(sections[0].items[0].id(), "PLabcVLdef");
}

#[test]
fn an_artist_query_lifts_the_top_artist_into_the_hero() {
    let mut results = sections();
    let hero = model::extract_hero(&mut results, "daft punk").unwrap();
    assert_eq!(hero.artist.name, "Daft Punk");
    assert!(hero.detail.is_some_and(|d| !d.is_empty()));
    assert!(results.iter().all(|s| s.title != "Top result"));
    assert!(results.iter().all(|s| !s.items.is_empty()));
}

#[test]
fn a_song_query_lists_the_top_artist_first_among_artists() {
    let mut results = sections();
    assert!(model::extract_hero(&mut results, "get lucky").is_none());
    let artists = &named(&results, "Artists").items;
    assert_eq!(artists[0].title(), "Daft Punk");
    let ids: std::collections::HashSet<&str> = artists.iter().map(YTItem::id).collect();
    assert_eq!(ids.len(), artists.len());
}

#[test]
fn display_puts_songs_first_and_filters_follow_it() {
    let mut results = sections();
    model::extract_hero(&mut results, "daft punk");
    let shown = model::display_sections(&results, SearchSort::Relevant, &Default::default());
    assert_eq!(shown[0].title, "Songs");
    assert_eq!(
        model::available_filters(&results)[..2],
        ["Library", "Songs"]
    );
}

#[test]
fn rating_date_and_recent_sorts_reorder_songs_keeping_ties_in_place() {
    let song = |id: &str, views: Option<i64>, year: Option<i64>| {
        YTItem::Song(SongItem {
            video_id: id.into(),
            view_count: views,
            year,
            ..SongItem::default()
        })
    };
    let results = vec![SearchSection::new(
        "Songs",
        vec![
            song("a", Some(5), None),
            song("b", None, Some(2001)),
            song("c", Some(9), Some(2020)),
            song("d", Some(5), None),
        ],
        None,
    )];
    let ids = |sort, recency: &std::collections::HashMap<String, std::time::SystemTime>| {
        model::display_sections(&results, sort, recency)[0]
            .items
            .iter()
            .map(|i| i.id().to_owned())
            .collect::<Vec<_>>()
    };
    let none = Default::default();
    assert_eq!(ids(SearchSort::Relevant, &none), ["a", "b", "c", "d"]);
    assert_eq!(ids(SearchSort::Rating, &none), ["c", "a", "d", "b"]);
    assert_eq!(ids(SearchSort::Date, &none), ["c", "b", "a", "d"]);
    let recency = [("d".to_owned(), std::time::SystemTime::now())].into();
    assert_eq!(ids(SearchSort::Recent, &recency), ["d", "a", "b", "c"]);
}

#[test]
fn local_search_finds_library_songs_artists_and_playlists() {
    use trop::database::database_service::DatabaseService;
    use trop::database::entities::artist_entity::ArtistEntity;
    use trop::database::entities::playlist_entity::PlaylistEntity;
    use trop::database::entities::song_entity::SongEntity;
    let db = DatabaseService::open_in_memory();
    let now = std::time::SystemTime::now();
    db.insert(&SongEntity {
        id: "s1".into(),
        title: "One More Time".into(),
        artist_name: Some("Daft Punk".into()),
        album_name: None,
        duration: 320,
        thumbnail_url: None,
        liked: false,
        total_play_time: 0,
        in_library: None,
        library_add_token: String::new(),
        library_remove_token: String::new(),
        is_episode: false,
        is_uploaded: false,
        is_video: false,
        create_date: now,
        modify_date: now,
        is_unavailable: false,
    })
    .unwrap();
    db.insert(&ArtistEntity {
        id: "UCdp".into(),
        name: "Daft Punk".into(),
        thumbnail_url: None,
        bookmarked_at: Some(now),
        is_podcast_channel: false,
        channel_id: None,
    })
    .unwrap();
    for (id, name) in [("PLdp", "Daft Punk mix"), ("SE", "Daft Punk episodes")] {
        db.insert(&PlaylistEntity {
            id: id.into(),
            browse_id: None,
            name: name.into(),
            thumbnail_url: None,
            is_editable: false,
            bookmarked_at: None,
            remote_song_count: Some(3),
            is_auto_sync: false,
        })
        .unwrap();
    }
    let found = SearchService::local_search(&db, "daft").unwrap();
    let titles: Vec<&str> = found.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Songs", "Artists", "Playlists"]);
    assert_eq!(found[2].items.len(), 1, "the hidden SE playlist stays out");
    assert!(
        SearchService::local_search(&db, "adele")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn songs_under_the_artist_card_carry_that_artist() {
    let sections = sections();
    let top = &sections[0];
    let songs: Vec<&SongItem> = top
        .items
        .iter()
        .filter_map(|item| match item {
            YTItem::Song(song) => Some(song),
            _ => None,
        })
        .collect();
    assert!(!songs.is_empty());
    for song in songs {
        assert_eq!(
            song.artists.first().map(|a| a.name.as_str()),
            Some("Daft Punk"),
            "{} has no artist",
            song.title
        );
        assert!(
            song.artists[0]
                .id
                .as_deref()
                .is_some_and(|id| id.starts_with("UC"))
        );
    }
}
