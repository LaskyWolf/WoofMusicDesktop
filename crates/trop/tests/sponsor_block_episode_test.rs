use trop::audio::sponsor_block::{Segment, SponsorBlock, hash_prefix, parse};
use trop::playback::episode_playback_store::EpisodePlaybackStore;

#[test]
fn sponsor_block_keeps_only_skippable_music_offtopic_of_the_song() {
    let body = br#"[
        {"videoID":"other","segments":[{"category":"music_offtopic","actionType":"skip","segment":[1,2]}]},
        {"videoID":"abc","segments":[
            {"category":"music_offtopic","actionType":"skip","segment":[200,230]},
            {"category":"sponsor","actionType":"skip","segment":[50,60]},
            {"category":"music_offtopic","actionType":"mute","segment":[70,80]},
            {"category":"music_offtopic","segment":[0,12.5]}
        ]}
    ]"#;
    assert_eq!(
        parse(body, "abc"),
        [
            Segment {
                start: 0.0,
                end: 12.5
            },
            Segment {
                start: 200.0,
                end: 230.0
            }
        ]
    );
    assert!(parse(b"not json", "abc").is_empty());
    // SHA-256 of "abc" starts with ba7816bf.
    assert_eq!(hash_prefix("abc"), "ba78");
}

#[test]
fn sponsor_block_skips_each_segment_once() {
    let mut block = SponsorBlock::default();
    assert!(block.track("abc"));
    assert!(!block.track("abc"));
    block.found(
        "abc",
        vec![Segment {
            start: 0.0,
            end: 12.5,
        }],
    );
    block.found(
        "old",
        vec![Segment {
            start: 50.0,
            end: 90.0,
        }],
    );
    assert_eq!(block.skip_to(0.3), Some(12.5));
    assert_eq!(block.skip_to(0.3), None, "seeking back plays it");
    assert_eq!(block.skip_to(60.0), None);
    let mut fresh = SponsorBlock::default();
    fresh.track("x");
    fresh.found(
        "x",
        vec![Segment {
            start: 0.0,
            end: 10.0,
        }],
    );
    assert_eq!(
        fresh.skip_to(9.5),
        None,
        "the last second is not worth a jump"
    );
}

#[test]
fn episodes_resume_until_nearly_finished() {
    let mut store = EpisodePlaybackStore::default();
    assert!(store.save_position("ep", 600.0, 3600.0, 1000.0));
    assert!(
        !store.save_position("ep", 602.0, 3600.0, 1002.0),
        "throttled"
    );
    assert!(store.save_position("ep", 610.0, 3600.0, 1006.0));
    assert_eq!(store.position("ep").unwrap().resume_at(), Some(610.0));
    assert!(
        store.save_position("ep", 3580.0, 3600.0, 1007.0),
        "finishing writes at once"
    );
    assert_eq!(store.position("ep").unwrap().resume_at(), None);
    assert!(store.save_position("short", 5.0, 3600.0, 1000.0));
    assert_eq!(
        store.position("short").unwrap().resume_at(),
        None,
        "under 10 s starts over"
    );
    assert!(store.clear("ep"));
    assert!(store.position("ep").is_none());
}
