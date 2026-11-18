//! Port of the Dart `test/trop/search_test.dart`.

mod common;

use common::{expect_well_formed, fixture, fixture_bytes};
use trop::networking::models::yt_item::YTItem;
use trop::search::search_parser::{SearchParser, SearchSection};
use trop::search::search_service::SearchService;

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
