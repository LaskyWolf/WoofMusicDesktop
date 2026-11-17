//! Port of the Dart `test/trop/view_models_test.dart`.

mod common;

use common::{fixture, fixture_client};
use serde_json::{Value, json};
use swiftytube::core::youtube_client::YouTubeClient;
use swiftytube::sync::playlist_detail_service::PlaylistDetailService;
use trop::detail::playlist_detail_parser::PlaylistDetailParser;
use trop::explore::explore_view_model::ExploreViewModel;
use trop::home::home_view_model::HomeViewModel;
use trop::networking::models::home_page::HomeSection;
use trop::networking::models::yt_item::YTItem;
use trop::search::search_parser::SearchParser;
use trop::search::search_service::SearchService;

// MARK: - HomeViewModel

#[tokio::test]
async fn home_load_continuation_and_section_order() {
    let (client, sent) = fixture_client(|body| {
        if body.get("continuation").is_some() {
            "home_continuation"
        } else {
            "home"
        }
    });
    let mut model = HomeViewModel::new(client);
    model.load().await.unwrap();
    {
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["browseId"], "FEmusic_home");
    }
    let first_count = model.home_page.as_ref().unwrap().sections.len();

    let sections = model.home_sections();
    assert!(matches!(sections[0], HomeSection::QuickPicks(_)));
    // "New releases" repeats Explore and is hidden on Home.
    assert!(
        sections
            .iter()
            .all(|s| s.display_title().to_lowercase() != "new releases")
    );

    assert!(
        model
            .load_more_if_needed(first_count - 1, first_count)
            .await
    );
    assert!(
        sent.lock()
            .unwrap()
            .last()
            .unwrap()
            .get("continuation")
            .is_some()
    );
    assert!(model.home_page.as_ref().unwrap().sections.len() > first_count);
}

#[tokio::test]
async fn home_chip_swaps_in_its_feed_and_closing_it_restores_the_page() {
    let (client, sent) = fixture_client(|_| "home");
    let mut model = HomeViewModel::new(client);
    model.load().await.unwrap();
    let original = model.home_page.clone().unwrap();
    let chip = original.chips[0].clone();

    model.toggle_chip(Some(chip.clone())).await;
    assert_eq!(
        sent.lock().unwrap().last().unwrap()["params"],
        json!(chip.params)
    );
    assert_eq!(model.selected_chip.as_ref(), Some(&chip));

    model.toggle_chip(Some(chip)).await;
    assert!(model.selected_chip.is_none());
    assert_eq!(model.home_page.as_ref(), Some(&original));
}

#[tokio::test]
async fn home_hide_explicit_drops_explicit_songs() {
    let (client, _) = fixture_client(|_| "home");
    let mut model = HomeViewModel::new(client);
    model.load().await.unwrap();
    let first = &mut model.home_page.as_mut().unwrap().sections[0].items[0];
    let YTItem::Song(song) = first else {
        panic!("Quick picks start with a song");
    };
    song.is_explicit = true;
    let explicit = first.clone();
    model.hide_explicit = true;
    assert!(!model.home_sections()[0].items().contains(&explicit));
}

// MARK: - ExploreViewModel

#[tokio::test]
async fn explore_loads_swaps_trending_for_the_regional_chart_and_opens_a_mood() {
    let (client, sent) =
        fixture_client(|body| match body.get("browseId").and_then(Value::as_str) {
            // The playlist's next page holds a related shelf, not tracks:
            // any page without a track shelf or token ends the walk.
            None => "moods_and_genres",
            Some("FEmusic_explore") => "explore",
            Some("FEmusic_charts") => "charts",
            Some("FEmusic_moods_and_genres_category") => "mood_category",
            Some(_) => "playlist_page1",
        });
    let mut model = ExploreViewModel::new(client, "US");
    model.load().await.unwrap();
    assert_eq!(
        sent.lock().unwrap()[0]["formData"],
        json!({ "selectedValues": ["US"] })
    );
    assert!(model.new_releases.is_some());
    assert!(!model.moods().is_empty());
    let trending = model
        .sections
        .iter()
        .find(|s| s.title.to_lowercase().contains("trending"))
        .unwrap();
    let page = fixture("playlist_page1");
    let chart_rows = PlaylistDetailService::extract_playlist_items(&page).unwrap();
    let expected: Vec<String> =
        PlaylistDetailParser::parse_playlist_detail(&Default::default(), &chart_rows, "")
            .songs
            .into_iter()
            .map(|s| s.video_id)
            .collect();
    let ids: Vec<&str> = trending.items.iter().map(YTItem::id).collect();
    assert_eq!(ids, expected);

    let mood = model.moods()[0].clone();
    model.select(Some(mood.clone())).await;
    assert_eq!(
        sent.lock().unwrap().last().unwrap()["params"],
        json!(mood.params)
    );
    assert!(model.mood_sections.as_ref().is_some_and(|s| !s.is_empty()));
    model.select(None).await;
    assert!(model.mood_sections.is_none());
}

// MARK: - SearchService

#[tokio::test]
async fn search_service_returns_the_raw_response_for_search_parser() {
    let (client, _) = fixture_client(|_| "search_daft_punk");
    let service = SearchService::new(client);
    let json = service
        .search("daft punk", None, YouTubeClient::WEB_REMIX)
        .await
        .unwrap();
    assert!(!SearchParser::parse_search_results(&json).is_empty());
}
