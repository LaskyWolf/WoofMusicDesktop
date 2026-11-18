use std::time::{Duration, Instant};

use trop::database::database_service::DatabaseService;
use trop::networking::models::yt_item::SongItem;
use trop::playback::playback_state_service::{PlayRecord, PlaybackStateService, save};

fn secs(start: Instant, s: u64) -> Instant {
    start + Duration::from_secs(s)
}

#[test]
fn banks_chunks_and_records_history_once() {
    let t = Instant::now();
    let mut state = PlaybackStateService::default();
    state.start_tracking("a");
    // Loading time before audio runs does not count.
    assert!(state.tick(secs(t, 10)).is_empty());
    state.set_playing(true, secs(t, 10));
    assert!(state.tick(secs(t, 14)).is_empty());
    assert_eq!(
        state.tick(secs(t, 16)),
        [PlayRecord::PlayTime {
            video_id: "a".into(),
            ms: 6_000
        }]
    );
    // Paused time does not count either.
    state.set_playing(false, secs(t, 20));
    state.set_playing(true, secs(t, 100));
    // 10 s before the pause and 26 s after it, 6 s already banked.
    let records = state.tick(secs(t, 126));
    assert_eq!(
        records,
        [
            PlayRecord::PlayTime {
                video_id: "a".into(),
                ms: 36_000 - 6_000
            },
            PlayRecord::Playback {
                video_id: "a".into(),
                ms: 36_000
            },
        ]
    );
    // History is recorded once; stop banks the rest.
    assert_eq!(
        state.stop_tracking(secs(t, 128)),
        [PlayRecord::PlayTime {
            video_id: "a".into(),
            ms: 2_000
        }]
    );
    assert!(state.tick(secs(t, 200)).is_empty());
}

#[test]
fn stop_counts_short_plays_and_history_past_the_threshold() {
    let t = Instant::now();
    let mut state = PlaybackStateService::default();
    state.start_tracking("short");
    state.set_playing(true, t);
    assert_eq!(
        state.stop_tracking(secs(t, 3)),
        [PlayRecord::PlayTime {
            video_id: "short".into(),
            ms: 3_000
        }]
    );
    state.start_tracking("long");
    state.set_playing(true, t);
    // No tick ran: stop banks everything and records history.
    assert_eq!(
        state.stop_tracking(secs(t, 31)),
        [
            PlayRecord::PlayTime {
                video_id: "long".into(),
                ms: 31_000
            },
            PlayRecord::Playback {
                video_id: "long".into(),
                ms: 31_000
            },
        ]
    );
}

#[test]
fn save_creates_the_song_and_fills_top_songs() {
    let db = DatabaseService::open_in_memory();
    let song = SongItem {
        video_id: "v1".into(),
        title: "Song".into(),
        duration: 200,
        ..SongItem::default()
    };
    let records = [
        PlayRecord::PlayTime {
            video_id: "v1".into(),
            ms: 40_000,
        },
        PlayRecord::Playback {
            video_id: "v1".into(),
            ms: 30_000,
        },
    ];
    save(&db, Some(&song), &records).unwrap();
    save(
        &db,
        None,
        &[PlayRecord::PlayTime {
            video_id: "v1".into(),
            ms: 5_000,
        }],
    )
    .unwrap();
    let top = db.fetch_top_songs(100).unwrap();
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].title, "Song");
    assert_eq!(top[0].total_play_time, 45_000);
    assert_eq!(db.fetch_history(None).unwrap().len(), 1);
}
