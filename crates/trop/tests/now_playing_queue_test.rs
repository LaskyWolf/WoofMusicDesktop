use trop::audio::now_playing::{Queue, SavedQueue};
use trop::networking::models::yt_item::SongItem;

fn song(id: &str) -> SongItem {
    SongItem {
        video_id: id.into(),
        title: id.into(),
        ..Default::default()
    }
}

fn unavailable(id: &str) -> SongItem {
    SongItem {
        is_unavailable: Some(true),
        ..song(id)
    }
}

fn ids(queue: &Queue) -> Vec<&str> {
    queue.songs.iter().map(|s| s.video_id.as_str()).collect()
}

fn queue(list: &[&str], index: usize) -> Queue {
    let mut queue = Queue::default();
    queue.set(&list.iter().map(|id| song(id)).collect::<Vec<_>>(), index);
    queue
}

#[test]
fn set_drops_unavailable_songs_and_keeps_the_start() {
    let songs = [song("a"), unavailable("x"), song("b"), unavailable("y")];
    let mut queue = Queue::default();
    for (start, expected) in [(0, 0), (1, 1), (2, 1), (3, 1)] {
        queue.set(&songs, start);
        assert_eq!(ids(&queue), ["a", "b"]);
        assert_eq!(queue.index, expected, "start {start}");
    }
    queue.set(&[unavailable("x")], 0);
    assert!(queue.songs.is_empty());
}

#[test]
fn play_next_songs_line_up_after_the_current_one() {
    let mut queue = queue(&["a", "b", "c"], 0);
    queue.enqueue_next(song("n1"));
    queue.enqueue_next(song("n2"));
    assert_eq!(ids(&queue), ["a", "n1", "n2", "b", "c"]);
}

#[test]
fn radio_keeps_play_next_songs() {
    let mut queue = queue(&["a", "b"], 0);
    queue.enqueue_next(song("n1"));
    queue.replace_with_radio(&[song("a"), song("r1"), song("r2")], 0);
    assert_eq!(ids(&queue), ["a", "n1", "r1", "r2"]);
}

#[test]
fn shuffle_keeps_the_current_song_and_unshuffle_restores_the_order() {
    let list: Vec<String> = (0..30).map(|i| i.to_string()).collect();
    let list: Vec<&str> = list.iter().map(String::as_str).collect();
    let mut queue = queue(&list, 5);
    queue.shuffle();
    assert!(queue.is_shuffle_on);
    assert_eq!(queue.index, 5);
    assert_eq!(&ids(&queue)[..6], &list[..6]);
    let mut sorted = ids(&queue);
    sorted.sort();
    let mut expected = list.clone();
    expected.sort();
    assert_eq!(sorted, expected);

    queue.move_to(9);
    let current = queue.song().unwrap().video_id.clone();
    queue.append(&[song("late")]);
    queue.unshuffle();
    assert!(!queue.is_shuffle_on);
    assert_eq!(queue.song().unwrap().video_id, current);
    assert_eq!(&ids(&queue)[..30], &list[..]);
    assert_eq!(ids(&queue)[30], "late");
}

#[test]
fn remove_and_move_keep_the_current_song() {
    let mut queue = queue(&["a", "b", "c", "d"], 2);
    queue.remove(0);
    assert_eq!((ids(&queue), queue.index), (vec!["b", "c", "d"], 1));
    queue.remove(1); // the current song stays
    assert_eq!(queue.songs.len(), 3);
    queue.move_song(2, 0);
    assert_eq!((ids(&queue), queue.index), (vec!["d", "b", "c"], 2));
    queue.move_song(2, 0);
    assert_eq!((ids(&queue), queue.index), (vec!["c", "d", "b"], 0));
}

#[test]
fn a_saved_queue_comes_back_with_its_song_play_next_and_shuffle() {
    let mut queue = Queue::default();
    queue.set(&[song("a"), song("b"), song("c")], 0);
    queue.enqueue_next(song("n"));
    queue.is_shuffle_on = true;
    let saved = queue.saved();
    assert!(queue.is_saved_as(&saved));

    let path = std::env::temp_dir().join(format!("woof-queue-{}.json", std::process::id()));
    saved.save(&path).unwrap();
    let restored = Queue::restored(&SavedQueue::load(&path)).unwrap();
    assert!(restored.is_saved_as(&saved));
    assert_eq!(restored.next_song().map(|s| s.video_id.as_str()), Some("n"));

    // An empty queue removes the file, like Swift removing its keys.
    SavedQueue::default().save(&path).unwrap();
    assert!(!path.exists());
    assert!(Queue::restored(&SavedQueue::load(&path)).is_none());
}

#[test]
fn a_restored_index_is_clamped_and_a_stale_play_next_dropped() {
    let saved = SavedQueue {
        songs: vec![song("a"), song("b")],
        index: 9,
        play_next_end: Some(1),
        is_shuffle_on: false,
    };
    let queue = Queue::restored(&saved).unwrap();
    assert_eq!(queue.index, 1);
    assert_eq!(queue.saved().play_next_end, None);
}

#[test]
fn a_song_removed_while_shuffled_stays_removed_after_unshuffle() {
    let mut queue = queue(&["a", "b", "c", "d", "e"], 0);
    queue.shuffle();
    let removed = ids(&queue)[2].to_owned();
    queue.remove(2);
    queue.unshuffle();
    assert_eq!(queue.songs.len(), 4);
    assert!(!ids(&queue).contains(&removed.as_str()));
}

#[test]
fn queue_edge_cases_do_not_panic() {
    let mut empty = Queue::default();
    empty.move_song(0, 0);
    empty.remove(0);
    empty.shuffle();
    empty.unshuffle();
    assert!(empty.song().is_none() && !empty.has_next() && !empty.has_previous());

    let mut one = queue(&["a"], 0);
    one.shuffle();
    one.move_song(0, 0);
    one.move_song(0, 5);
    one.remove(0);
    one.unshuffle();
    assert_eq!((ids(&one), one.index), (vec!["a"], 0));
}
