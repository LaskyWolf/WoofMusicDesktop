use trop::audio::now_playing::Queue;
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
