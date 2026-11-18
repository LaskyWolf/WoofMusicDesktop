//! Port of the Dart `test/trop/yt_item_test.dart`.

mod common;

use common::fixture;
use serde_json::{Value, json};
use swiftytube::networking::inner_tube_decode::JsonDict;
use trop::networking::models::yt_item::{SongItem, YTArtist, YTItem};

/// Every song row (musicResponsiveListItemRenderer) in a fixture.
fn song_rows(json: &Value) -> Vec<SongItem> {
    fn walk(node: &Value, songs: &mut Vec<SongItem>) {
        match node {
            Value::Object(map) => {
                if let Some(Value::Object(renderer)) = map.get("musicResponsiveListItemRenderer")
                    && let Some(song) = SongItem::from(renderer)
                {
                    songs.push(song);
                }
                map.values().for_each(|v| walk(v, songs));
            }
            Value::Array(list) => list.iter().for_each(|v| walk(v, songs)),
            _ => {}
        }
    }
    let mut songs = Vec::new();
    walk(json, &mut songs);
    songs
}

#[test]
fn song_rows_take_the_album_only_from_a_run_that_opens_an_album_page() {
    for name in [
        "album",
        "artist",
        "explore",
        "home",
        "playlist_page1",
        "search_daft_punk",
    ] {
        let songs = song_rows(&Value::Object(fixture(name)));
        assert!(!songs.is_empty(), "{name}");
        for song in &songs {
            // Views ("942K views"), durations ("3:48") and featured artists used to land here.
            if let Some(album) = &song.album {
                assert!(
                    song.album_id
                        .as_deref()
                        .is_some_and(|id| id.starts_with("MPREb_")),
                    "{name}: {} → {album}",
                    song.title
                );
            }
        }
    }
    let home = song_rows(&Value::Object(fixture("home")));
    assert!(
        home.iter().any(|s| s.album_id.is_some()),
        "home song rows link their album"
    );
}

fn run(text: &str) -> Value {
    json!({ "text": text })
}

fn link(text: &str, browse_id: &str, page_type: &str) -> Value {
    json!({ "text": text, "navigationEndpoint": { "browseEndpoint": {
        "browseId": browse_id,
        "browseEndpointContextSupportedConfigs": {
            "browseEndpointContextMusicConfig": { "pageType": page_type }
        }
    }}})
}

fn column(runs: Vec<Value>) -> Value {
    json!({ "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": runs } } })
}

fn song_row(byline: Vec<Value>) -> JsonDict {
    json!({
        "playlistItemData": { "videoId": "abcdefghijk" },
        "flexColumns": [column(vec![run("One More Time")]), column(byline)],
    })
    .as_object()
    .unwrap()
    .clone()
}

fn names(artists: &[YTArtist]) -> Vec<&str> {
    artists.iter().map(|a| a.name.as_str()).collect()
}

#[test]
fn song_row_album_in_the_byline_is_not_an_artist() {
    let row = song_row(vec![
        link(
            "Daft Punk",
            "UCRr1xG_2WIDs18a6cIiCxeA",
            "MUSIC_PAGE_TYPE_ARTIST",
        ),
        run(" • "),
        link("Discovery", "MPREb_7ltM34kr0mH", "MUSIC_PAGE_TYPE_ALBUM"),
        run(" • "),
        run("3:32"),
    ]);
    let Some(YTItem::Song(song)) = YTItem::from_responsive_list_item(&row) else {
        panic!("not a song");
    };
    assert_eq!(song.album.as_deref(), Some("Discovery"));
    assert_eq!(names(&song.artists), ["Daft Punk"]);
}

#[test]
fn artist_named_with_a_number_is_kept_when_it_links_to_an_artist_page() {
    let row = song_row(vec![link(
        "311",
        "UCxxxxxxxxxxxxxxxxxxxxxx",
        "MUSIC_PAGE_TYPE_ARTIST",
    )]);
    let Some(YTItem::Song(song)) = YTItem::from_responsive_list_item(&row) else {
        panic!("not a song");
    };
    assert_eq!(names(&song.artists), ["311"]);
}

#[test]
fn single_kind_word_of_an_album_row_is_not_an_artist() {
    let row = json!({
        "navigationEndpoint": { "browseEndpoint": {
            "browseId": "MPREb_xxxxxxxxxxx",
            "browseEndpointContextSupportedConfigs": {
                "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ALBUM" }
            }
        }},
        "flexColumns": [
            column(vec![run("One More Time")]),
            column(vec![
                run("Single"),
                run(" • "),
                link("Daft Punk", "UCRr1xG_2WIDs18a6cIiCxeA", "MUSIC_PAGE_TYPE_ARTIST"),
                run(" • "),
                run("2000"),
            ]),
        ],
    });
    let Some(YTItem::Album(album)) = YTItem::from_responsive_list_item(row.as_object().unwrap())
    else {
        panic!("not an album");
    };
    assert_eq!(album.year, Some(2000));
    assert_eq!(names(&album.artists), ["Daft Punk"]);
}
