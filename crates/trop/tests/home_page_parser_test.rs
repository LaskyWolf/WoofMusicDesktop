//! Port of the Dart `test/trop/home_page_parser_test.dart`.

mod common;

use common::{expect_well_formed, fixture};
use serde_json::json;
use swiftytube::networking::inner_tube_decode::JsonDict;
use swiftytube::sync::artist_credits::{ArtistCredit, ArtistCredits};
use swiftytube::sync::regional_charts::RegionalCharts;
use trop::explore::explore_view_model::ExploreViewModel;
use trop::explore::mood_card::{MoodFamily, MoodFamilyEntry};
use trop::networking::models::home_page::{ExploreSection, ExploreSectionKind, HomePage, MoodItem};
use trop::networking::models::home_page_parser::HomePageParser;
use trop::networking::models::yt_item::{AlbumItem, PlaylistItem, SongItem, YTItem};

// MARK: - HomePageParser on FEmusic_home

fn home_page() -> HomePage {
    HomePageParser::parse_home_page(&fixture("home")).unwrap()
}

#[test]
fn home_chips_sections_and_continuation() {
    let page = home_page();
    assert!(!page.chips.is_empty());
    assert!(
        page.chips
            .iter()
            .all(|c| !c.title.is_empty() && c.params.is_some())
    );
    assert!(!page.sections.is_empty());
    assert!(page.continuation.is_some());
    for section in &page.sections {
        assert!(!section.title.is_empty());
        assert!(!section.items.is_empty());
        section.items.iter().for_each(expect_well_formed);
    }
}

#[test]
fn home_quick_picks_are_songs_with_artists() {
    let page = home_page();
    let quick_picks = page
        .sections
        .iter()
        .find(|s| s.title.to_lowercase().contains("quick pick"))
        .unwrap();
    assert!(quick_picks.is_songs_only());
    assert!(
        quick_picks
            .items
            .iter()
            .all(|i| matches!(i, YTItem::Song(s) if !s.artists.is_empty()))
    );
}

#[test]
fn home_item_kinds_include_songs_albums_and_playlists() {
    let page = home_page();
    let items: Vec<&YTItem> = page.sections.iter().flat_map(|s| &s.items).collect();
    assert!(items.iter().any(|i| matches!(i, YTItem::Song(_))));
    assert!(items.iter().any(|i| matches!(i, YTItem::Album(_))));
    assert!(items.iter().any(|i| matches!(i, YTItem::Playlist(_))));
}

#[test]
fn home_continuation_page_parses_into_more_sections() {
    let (sections, _) =
        HomePageParser::parse_continuation_sections(&fixture("home_continuation")).unwrap();
    assert!(!sections.is_empty());
    for section in &sections {
        assert!(!section.title.is_empty());
        section.items.iter().for_each(expect_well_formed);
    }
    // A first page is not a continuation answer.
    assert!(HomePageParser::parse_continuation_sections(&fixture("home")).is_none());
}

// MARK: - Explore

fn explore_sections() -> (JsonDict, Vec<ExploreSection>) {
    let json = fixture("explore");
    let sections = HomePageParser::parse_explore_sections(&json);
    (json, sections)
}

fn explore_moods(sections: &[ExploreSection]) -> Vec<MoodItem> {
    sections
        .iter()
        .find(|s| s.kind == ExploreSectionKind::Moods)
        .unwrap()
        .moods
        .clone()
}

#[test]
fn explore_new_releases_moods_and_trending_shelves() {
    let (_, sections) = explore_sections();
    for kind in [
        ExploreSectionKind::Rows,
        ExploreSectionKind::Cards,
        ExploreSectionKind::Moods,
    ] {
        assert!(sections.iter().any(|s| s.kind == kind), "{kind:?}");
    }
    for section in &sections {
        assert!(!section.title.is_empty());
        section.items.iter().for_each(expect_well_formed);
    }
    let moods = explore_moods(&sections);
    assert!(moods.len() > 5);
    assert!(
        moods
            .iter()
            .all(|m| !m.title.is_empty() && m.params.is_some() && m.color.is_some())
    );
    let rows = sections
        .iter()
        .find(|s| s.kind == ExploreSectionKind::Rows)
        .unwrap();
    assert!(rows.items.iter().all(|i| matches!(i, YTItem::Song(_))));
}

#[test]
fn explore_new_release_details_fill_artists_and_kinds() {
    let (json, mut fresh) = explore_sections();
    let releases = ExploreViewModel::apply_new_release_details(&json, &mut fresh).unwrap();
    assert!(!releases.kinds.is_empty());
    assert!(
        releases
            .kinds
            .values()
            .any(|k| ["Single", "Album", "EP"].contains(&k.as_str()))
    );
    let shelf = fresh.iter().find(|s| s.id == releases.section_id).unwrap();
    let albums: Vec<&AlbumItem> = shelf
        .items
        .iter()
        .filter_map(|i| match i {
            YTItem::Album(a) => Some(a),
            _ => None,
        })
        .collect();
    assert!(!albums.is_empty());
    assert!(albums.iter().filter(|a| !a.artists.is_empty()).count() > albums.len() / 2);
}

#[test]
fn explore_mood_category_page_parses_into_card_shelves() {
    let mood = HomePageParser::parse_explore_sections(&fixture("mood_category"));
    assert!(!mood.is_empty());
    for section in &mood {
        assert!(!section.title.is_empty());
        assert!(!section.items.is_empty());
        section.items.iter().for_each(expect_well_formed);
    }
}

#[test]
fn explore_moods_and_genres_grid_is_not_read() {
    // The page is a grid of musicNavigationButtonRenderer; the Swift app
    // never browses it, and parse_explore_sections skips grid buttons.
    assert!(HomePageParser::parse_explore_sections(&fixture("moods_and_genres")).is_empty());
}

#[test]
fn explore_mood_families_group_genres() {
    let (_, sections) = explore_sections();
    let moods = explore_moods(&sections);
    let entries = MoodFamily::entries(&moods);
    assert!(entries.len() <= moods.len());
    assert_eq!(MoodFamily::family("Brazilian Hip-Hop"), Some("Hip-Hop"));
    assert_eq!(MoodFamily::family("Chill"), None);
    let pop = vec![
        MoodItem::new("Pop", None, None),
        MoodItem::new("K-Pop", None, None),
    ];
    assert_eq!(
        MoodFamily::base("Pop", &pop).map(|m| m.title.as_str()),
        Some("Pop")
    );
    let subgenres: Vec<String> = MoodFamily::subgenres("Pop", &pop)
        .into_iter()
        .map(|m| m.title)
        .collect();
    assert_eq!(subgenres, ["K-Pop"]);
    let entries = MoodFamily::entries(&pop);
    assert_eq!(entries.len(), 1);
    assert!(matches!(entries[0], MoodFamilyEntry::Group { .. }));
}

#[test]
fn explore_mood_order_puts_albums_then_rows_then_the_rest() {
    let s = |kind, item| ExploreSection::new("", kind, vec![item], Vec::new());
    let playlist = s(
        ExploreSectionKind::Cards,
        YTItem::Playlist(PlaylistItem {
            id: "VLx".into(),
            title: "p".into(),
            ..PlaylistItem::default()
        }),
    );
    let songs = s(
        ExploreSectionKind::Rows,
        YTItem::Song(SongItem {
            video_id: "v".into(),
            title: "s".into(),
            ..SongItem::default()
        }),
    );
    let albums = s(
        ExploreSectionKind::Cards,
        YTItem::Album(AlbumItem {
            browse_id: "MPREb_x".into(),
            title: "a".into(),
            ..AlbumItem::default()
        }),
    );
    assert_eq!(
        ExploreViewModel::mood_order(vec![playlist.clone(), songs.clone(), albums.clone()]),
        [albums, songs, playlist]
    );
}

// MARK: - RegionalCharts on FEmusic_charts

#[test]
fn regional_charts_picks_the_trending_playlist() {
    let entries = RegionalCharts::playlist_entries(&fixture("charts"));
    assert!(!entries.is_empty());
    assert!(
        entries
            .iter()
            .all(|e| e.id.starts_with("VL") || e.id.starts_with("PL"))
    );
    let id = RegionalCharts::select_trending(&entries).unwrap();
    let entry = entries.iter().find(|e| e.id == id).unwrap();
    assert!(entry.title.to_lowercase().contains("trending"));
}

// MARK: - ArtistCredits

#[test]
fn artist_credits_drop_separators_counts_and_durations_and_keep_channel_ids() {
    let runs = json!([
        {"text": "Song"},
        {"text": " • "},
        {"text": "Daft Punk - Topic", "navigationEndpoint": {"browseEndpoint": {"browseId": "UCabc"}}},
        {"text": " & "},
        {"text": "Pharrell"},
        {"text": " • "},
        {"text": "Random Access Memories", "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_x"}}},
        {"text": "1.2M views"},
        {"text": "3:48"},
    ]);
    let runs: Vec<&JsonDict> = runs
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_object().unwrap())
        .collect();
    assert_eq!(
        ArtistCredits::parse(&runs),
        [
            ArtistCredit {
                name: "Daft Punk".into(),
                browse_id: Some("UCabc".into())
            },
            ArtistCredit {
                name: "Pharrell".into(),
                browse_id: None
            },
        ]
    );
    assert_eq!(ArtistCredits::cleaned_name("Queen (1973)"), "Queen");
}
