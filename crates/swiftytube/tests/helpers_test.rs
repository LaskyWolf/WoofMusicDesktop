//! Port of the Dart `test/swiftytube/helpers_test.dart`.

use serde_json::{Value, json};
use swiftytube::formatting::count_format::{CountFormat, CountFormatInt};
use swiftytube::formatting::duration_format::{DurationFormat, DurationFormatInt};
use swiftytube::networking::artwork_urls::ArtworkURLs;
use swiftytube::networking::inner_tube_decode::{InnerTubeDecode, JsonDict};
use swiftytube::networking::inner_tube_json::InnerTubeJSON;
use swiftytube::networking::search_suggestions::SearchSuggestions;

fn dict(value: Value) -> JsonDict {
    match value {
        Value::Object(dict) => dict,
        _ => panic!("not an object"),
    }
}

#[test]
fn runs_text_joins_runs_falls_back_to_simple_text() {
    let joined = dict(json!({ "runs": [{ "text": "A" }, { "text": "B" }] }));
    assert_eq!(
        InnerTubeJSON::runs_text(Some(&joined)).as_deref(),
        Some("AB")
    );
    let simple = dict(json!({ "simpleText": "S" }));
    assert_eq!(
        InnerTubeJSON::runs_text(Some(&simple)).as_deref(),
        Some("S")
    );
    assert_eq!(
        InnerTubeJSON::runs_text(Some(&dict(json!({ "runs": [] })))),
        None
    );
    assert_eq!(InnerTubeJSON::runs_text(None), None);
}

#[test]
fn runs_texts_drops_bullets_and_blanks() {
    let runs = dict(json!({
        "runs": [{ "text": "Song" }, { "text": " • " }, { "text": "  " }, { "text": "Artist" }],
    }));
    assert_eq!(InnerTubeJSON::runs_texts(Some(&runs)), ["Song", "Artist"]);
}

#[test]
fn raw_runs_and_thumbnails() {
    assert!(InnerTubeJSON::raw_runs(Some(&dict(json!({ "runs": "nope" })))).is_empty());
    let nested = dict(json!({ "thumbnail": { "thumbnails": [{ "url": "a" }, { "url": "b" }] } }));
    assert_eq!(
        InnerTubeJSON::nested_thumbnail_url(Some(&nested)).as_deref(),
        Some("b")
    );
    let music = dict(json!({
        "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [{ "url": "m" }] } } },
    }));
    assert_eq!(
        InnerTubeJSON::music_thumbnail_url(&music).as_deref(),
        Some("m")
    );
    let cropped = dict(json!({ "croppedSquareThumbnail": { "thumbnails": [{ "url": "c" }] } }));
    assert_eq!(
        InnerTubeJSON::music_thumbnail_url(&cropped).as_deref(),
        Some("c")
    );
    assert_eq!(
        InnerTubeJSON::music_thumbnail_url(&dict(json!({ "other": 1 }))),
        None
    );
}

fn decode_json() -> JsonDict {
    dict(json!({
        "a": {
            "b": { "c": "leaf" },
            "list": [{ "x": 1 }],
            "mixed": [{ "x": 1 }, 2],
        },
        "continuations": [{ "nextContinuationData": { "continuation": "TOKEN" } }],
    }))
}

#[test]
fn path_walking() {
    let json = decode_json();
    assert_eq!(
        InnerTubeDecode::string(&["a", "b", "c"], &json),
        Some("leaf")
    );
    assert_eq!(
        InnerTubeDecode::dict(&["a", "b"], &json),
        Some(&dict(json!({ "c": "leaf" })))
    );
    assert_eq!(
        InnerTubeDecode::array(&["a", "list"], &json).map(|a| a.len()),
        Some(1)
    );
    assert!(InnerTubeDecode::array(&["a", "mixed"], &json).is_none());
    assert!(InnerTubeDecode::value(&["a", "b", "c", "d"], &json).is_none());
    assert_eq!(
        InnerTubeDecode::continuation_token(Some(&json)),
        Some("TOKEN")
    );
}

#[test]
fn int_from_string_or_int_and_runs_text_tolerant() {
    assert_eq!(
        InnerTubeDecode::int_from_string_or_int(Some(&json!("42"))),
        Some(42)
    );
    assert_eq!(
        InnerTubeDecode::int_from_string_or_int(Some(&json!(42))),
        Some(42)
    );
    assert_eq!(
        InnerTubeDecode::int_from_string_or_int(Some(&json!("4.2"))),
        None
    );
    assert_eq!(
        InnerTubeDecode::runs_text_tolerant(Some(&json!("bare"))).as_deref(),
        Some("bare")
    );
    assert_eq!(
        InnerTubeDecode::runs_text_tolerant(Some(&json!({ "text": "T" }))).as_deref(),
        Some("T")
    );
}

#[test]
fn count_format() {
    assert_eq!(
        CountFormat::parse_compact_count("1.2M views"),
        Some(1_200_000)
    );
    assert_eq!(
        CountFormat::parse_compact_count("500k views"),
        Some(500_000)
    );
    // "views" is required so years and bare numbers in subtitles never read as counts.
    assert_eq!(CountFormat::parse_compact_count("500K"), None);
    assert_eq!(
        CountFormat::parse_compact_count("12,345 views"),
        Some(12_345)
    );
    assert_eq!(CountFormat::parse_compact_count("views"), None);
    assert_eq!(1_234_567_i64.formatted_views(" views"), "1,234,567 views");
}

#[test]
fn duration_format() {
    assert_eq!(DurationFormat::parse_clock("3:05"), Some(185));
    assert_eq!(DurationFormat::parse_clock("1:02:03"), Some(3723));
    assert_eq!(DurationFormat::parse_clock("1,234"), None);
    assert_eq!(DurationFormat::parse_spoken("1 hr 23 mins"), Some(4980));
    assert_eq!(DurationFormat::playback_time(-5.0), "0:00");
    assert_eq!(DurationFormat::playback_time(3725.9), "1:02:05");
    assert_eq!(DurationFormat::short_duration(125), "2m 5s");
    assert_eq!(3723_i64.formatted_duration(), "1:02:03");
    assert_eq!(3600_i64.words_duration(), "1 h");
    let renderer = dict(json!({
        "flexColumns": [
            {},
            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Artist" }, { "text": " 4:10 " }] } } },
        ],
    }));
    assert_eq!(DurationFormat::parse_renderer_duration(&renderer), 250);
}

#[test]
fn artwork_urls() {
    assert_eq!(
        ArtworkURLs::artwork(
            Some("https://lh3.googleusercontent.com/abc=w60-h60-l90-rj"),
            480
        )
        .as_deref(),
        Some("https://lh3.googleusercontent.com/abc=w480-h480-l90-rj")
    );
    assert_eq!(
        ArtworkURLs::artwork(Some("https://lh3.googleusercontent.com/abc=s120-c"), 160).as_deref(),
        Some("https://lh3.googleusercontent.com/abc=w160-h160")
    );
    assert_eq!(ArtworkURLs::rung(200.0), ArtworkURLs::CARD_PX);
    assert_eq!(
        ArtworkURLs::sd_variant(Some("https://i.ytimg.com/vi/ID/hqdefault.jpg?x=1")).as_deref(),
        Some("https://i.ytimg.com/vi/ID/sddefault.jpg")
    );
}

#[test]
fn search_suggestions_parse_handles_jsonp() {
    let data = br#"window.google.ac.h(["q",[["one",0],["two",0]],{}])"#;
    assert_eq!(SearchSuggestions::parse(data), ["one", "two"]);
    assert!(SearchSuggestions::parse(b"garbage").is_empty());
}
