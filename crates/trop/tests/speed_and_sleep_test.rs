use std::time::{Duration, Instant};

use trop::audio::playback_speed::PlaybackSpeed;
use trop::audio::sleep_timer::SleepTimer;

#[test]
fn speed_resets_for_songs_and_remembers_podcasts() {
    let mut speed = PlaybackSpeed::default();
    assert_eq!(speed.select("song", false), 1.0);
    assert_eq!(speed.set(1.26), Some(1.25));
    assert_eq!(speed.set(1.25), None);
    assert_eq!(speed.select("song", false), 1.25, "reloading keeps it");
    assert_eq!(speed.select("other", false), 1.0);

    assert_eq!(speed.select("episode", true), 1.0);
    assert_eq!(speed.set(9.0), Some(3.0));
    assert_eq!(speed.select("song", false), 1.0);
    assert_eq!(speed.select("next episode", true), 3.0);

    assert_eq!(PlaybackSpeed::label(1.0), "1×");
    assert_eq!(PlaybackSpeed::label(1.25), "1.25×");
    assert_eq!(PlaybackSpeed::label(1.5), "1.5×");
}

#[test]
fn speed_stays_in_range_for_bad_values() {
    let mut speed = PlaybackSpeed::default();
    speed.select("song", false);
    assert_eq!(speed.set(f64::NAN), None);
    assert_eq!(speed.rate, 1.0);
    assert_eq!(speed.set(-4.0), Some(0.5));
    assert_eq!(speed.set(f64::INFINITY), Some(3.0));
    // A hand-edited settings file must not start an episode at 0x or 99x.
    for bad in [0.0, 99.0, f64::NAN] {
        let mut speed = PlaybackSpeed::default();
        speed.podcast_rate = bad;
        let rate = speed.select("episode", true);
        assert!((0.5..=3.0).contains(&rate), "{bad} gave {rate}");
    }
}

#[test]
fn sleep_timer_counts_down_once() {
    let now = Instant::now();
    let mut timer = SleepTimer::default();
    timer.start(15, now);
    assert!(timer.is_active());
    assert_eq!(timer.summary(now).as_deref(), Some("15 min left"));
    assert!(!timer.due(now + Duration::from_secs(14 * 60)));
    assert!(timer.due(now + Duration::from_secs(15 * 60)));
    assert!(!timer.is_active());
    assert!(!timer.due(now + Duration::from_secs(16 * 60)));

    timer.stop_at_song_end();
    assert_eq!(timer.summary(now).as_deref(), Some("end of song"));
    assert!(!timer.due(now + Duration::from_secs(3600)));
    assert_eq!(timer.finish_message(), "Good night");
    assert_eq!(timer.finish_message(), "Sleep well");
}
