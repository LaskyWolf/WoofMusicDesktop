use serde_json::json;
use trop::services::personalization_service::{
    RadioOptions, RadioStyle, panel_songs, playlist_panel, shape,
};

fn row(id: &str, selected: bool, explicit: bool) -> serde_json::Value {
    let badges = if explicit {
        json!([{ "musicInlineBadgeRenderer": { "icon": { "iconType": "MUSIC_EXPLICIT_BADGE" } } }])
    } else {
        json!([])
    };
    json!({ "playlistPanelVideoRenderer": {
        "videoId": id,
        "selected": selected,
        "title": { "runs": [{ "text": format!("Title {id}") }] },
        "longBylineText": { "runs": [
            { "text": "Artist", "navigationEndpoint": { "browseEndpoint": { "browseId": "UC1" } } },
            { "text": " • " },
            { "text": "Album" }
        ] },
        "lengthText": { "runs": [{ "text": "3:05" }] },
        "badges": badges
    } })
}

fn next_answer(rows: Vec<serde_json::Value>) -> serde_json::Map<String, serde_json::Value> {
    json!({ "contents": { "singleColumnMusicWatchNextResultsRenderer": { "tabbedRenderer": {
        "watchNextTabbedResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
            "musicQueueRenderer": { "content": { "playlistPanelRenderer": { "contents": rows } } }
        } } }] }
    } } } })
    .as_object()
    .unwrap()
    .clone()
}

#[test]
fn reads_the_radio_panel() {
    let json = next_answer(vec![
        row("a", false, false),
        row("b", true, true),
        row("c", false, false),
    ]);
    let panel = playlist_panel(&json).unwrap();
    let (songs, current) = panel_songs(&panel);
    assert_eq!(
        songs
            .iter()
            .map(|s| s.video_id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert_eq!(current, 1);
    assert_eq!(songs[0].title, "Title a");
    assert_eq!(songs[0].artists[0].name, "Artist");
    assert_eq!(songs[0].artists[0].id.as_deref(), Some("UC1"));
    assert_eq!(songs[0].duration, 185);
    assert!(songs[1].is_explicit && !songs[0].is_explicit);
}

#[test]
fn radio_options_shape_the_queue() {
    let json = next_answer(
        (0..10)
            .map(|i| row(&i.to_string(), i == 0, i % 2 == 1))
            .collect(),
    );
    let (songs, _) = panel_songs(&playlist_panel(&json).unwrap());
    let options = RadioOptions {
        style: RadioStyle::DeepCuts,
        limit: 3,
        allow_explicit: false,
    };
    let (shaped, current) = shape(songs.clone(), 0, options).unwrap();
    assert_eq!(
        shaped
            .iter()
            .map(|s| s.video_id.as_str())
            .collect::<Vec<_>>(),
        ["4", "6", "8"]
    );
    assert_eq!(current, 0);
    let (variety, _) = shape(
        songs,
        0,
        RadioOptions {
            style: RadioStyle::Variety,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(&variety[0].video_id, "0");
    assert_eq!(variety.len(), 10);
}
