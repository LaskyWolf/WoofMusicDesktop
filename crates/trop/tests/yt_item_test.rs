//! Port of the Dart `test/trop/yt_item_test.dart`.

mod common;

use common::fixture;
use serde_json::Value;
use trop::networking::models::yt_item::SongItem;

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
