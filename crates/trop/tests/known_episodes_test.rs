use trop::database::database_service::DatabaseService;
use trop::networking::models::yt_item::EpisodeItem;
use trop::playback::episode_playback_store::sync_known_episodes;

fn episode(id: &str) -> EpisodeItem {
    EpisodeItem {
        video_id: id.to_owned(),
        title: format!("Episode {id}"),
        ..EpisodeItem::default()
    }
}

#[test]
fn only_episodes_never_seen_are_fresh() {
    let db = DatabaseService::open_in_memory();
    let first = sync_known_episodes(&db, &[episode("a"), episode("b")], "p", "Pod").unwrap();
    assert_eq!(first.len(), 2);
    let second = sync_known_episodes(&db, &[episode("c"), episode("b")], "p", "Pod").unwrap();
    assert_eq!(
        second
            .iter()
            .map(|e| e.video_id.as_str())
            .collect::<Vec<_>>(),
        ["c"]
    );
    assert!(
        sync_known_episodes(&db, &[episode("c")], "p", "Pod")
            .unwrap()
            .is_empty()
    );
}
