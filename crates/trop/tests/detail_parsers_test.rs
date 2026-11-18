//! Port of the Dart `test/trop/detail_parsers_test.dart`.

mod common;

use common::{expect_well_formed, fixture};
use swiftytube::networking::inner_tube_decode::JsonDict;
use swiftytube::sync::playlist_detail_service::PlaylistDetailService;
use trop::detail::album_detail_parser::AlbumDetailParser;
use trop::detail::artist_detail_parser::ArtistDetailParser;
use trop::detail::playlist_detail_parser::PlaylistDetailParser;
use trop::detail::podcast_detail_parser::PodcastDetailParser;
use trop::networking::models::yt_item::YTItem;

#[test]
fn album_header_playlist_id_and_tracks() {
    let album = AlbumDetailParser::parse_album_detail(&fixture("album"), "MPREb_test");
    assert_ne!(album.title, "Unknown Album");
    assert!(!album.artists.is_empty());
    assert!(album.artists[0].id.as_deref().unwrap().starts_with("UC"));
    assert!(album.thumbnail_url.as_deref().unwrap().starts_with("http"));
    assert!(
        album
            .playlist_id
            .as_deref()
            .unwrap()
            .starts_with("OLAK5uy_")
    );
    assert!(!album.songs.is_empty());
    assert_eq!(album.song_count, album.songs.len() as i64);
    for song in &album.songs {
        expect_well_formed(&YTItem::Song(song.clone()));
        assert!(song.duration > 0);
    }
}

#[test]
fn artist_header_and_every_shelf_kind() {
    let artist = ArtistDetailParser::parse_artist_detail(&fixture("artist"), "UCtest");
    assert_ne!(artist.name, "Unknown Artist");
    assert!(artist.thumbnail_url.as_deref().unwrap().starts_with("http"));
    assert!(!artist.subscriber_count_text.as_deref().unwrap().is_empty());
    assert!(!artist.songs.is_empty());
    assert!(!artist.albums.is_empty());
    assert!(!artist.singles.is_empty());
    assert!(!artist.related_artists.is_empty());
    assert!(artist.has_music());
    let items = artist
        .songs
        .iter()
        .chain(&artist.videos)
        .cloned()
        .map(YTItem::Song)
        .chain(
            artist
                .albums
                .iter()
                .chain(&artist.singles)
                .cloned()
                .map(YTItem::Album),
        )
        .chain(artist.playlists.iter().cloned().map(YTItem::Playlist))
        .chain(artist.related_artists.iter().cloned().map(YTItem::Artist));
    for item in items {
        expect_well_formed(&item);
    }
}

#[test]
fn playlist_rows_from_the_page_walk_header_and_author() {
    let page = fixture("playlist_page1");
    let rows = PlaylistDetailService::extract_playlist_items(&page).unwrap();
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|r| r.contains_key("musicResponsiveListItemRenderer"))
    );

    let playlist = PlaylistDetailParser::parse_playlist_detail(&page, &rows, "PLtest");
    assert_ne!(playlist.title, "Unknown Playlist");
    assert!(!playlist.author_name.as_deref().unwrap().is_empty());
    assert!(
        playlist
            .thumbnail_url
            .as_deref()
            .unwrap()
            .starts_with("http")
    );
    assert_eq!(playlist.songs.len(), rows.len());
    assert!(playlist.duration > 0);
    for song in &playlist.songs {
        expect_well_formed(&YTItem::Song(song.clone()));
    }
    let set_video_ids = PlaylistDetailParser::set_video_ids(&rows);
    assert!(
        playlist
            .songs
            .iter()
            .all(|s| set_video_ids.contains_key(&s.video_id))
    );
    assert_eq!(PlaylistDetailService::request_browse_id("PLx"), "VLPLx");
    assert_eq!(PlaylistDetailService::request_browse_id("VLPLx"), "VLPLx");
}

#[test]
fn podcast_show_header_and_episodes_stamped_with_the_show_name() {
    let podcast = PodcastDetailParser::parse_podcast_detail(&fixture("podcast"), "MPSPtest");
    assert_ne!(podcast.title, "Unknown Podcast");
    let author = podcast.author.as_deref().unwrap();
    assert!(!author.is_empty());
    assert!(!podcast.description_text.as_deref().unwrap().is_empty());
    assert!(
        podcast
            .thumbnail_url
            .as_deref()
            .unwrap()
            .starts_with("http")
    );
    assert!(!podcast.episodes.is_empty());
    for episode in &podcast.episodes {
        expect_well_formed(&YTItem::Episode(episode.clone()));
        assert!(episode.duration > 0);
        assert!(!episode.publish_date.as_deref().unwrap().is_empty());
        assert_eq!(episode.artists.len(), 1);
        assert_eq!(episode.artists[0].name, author);
    }
}

// MARK: - Header drift

#[test]
fn album_year_from_subtitle_and_spoken_total_length_from_second_subtitle() {
    // Header runs: subtitle "Album • 2013", secondSubtitle "13 songs • 1 hour, 14 minutes".
    let album = AlbumDetailParser::parse_album_detail(&fixture("album"), "MPREb_test");
    assert_eq!(album.year, Some(2013));
    assert_eq!(album.duration, 74 * 60);
    assert_eq!(album.song_count, 13);
}

#[test]
fn playlist_spoken_length_with_hours_and_minutes() {
    let page = fixture("playlist_page1");
    let header = serde_json::to_string(&page).unwrap().replacen(
        "\"30 minutes\"",
        "\"1 hour, 14 minutes\"",
        1,
    );
    let edited: JsonDict = serde_json::from_str(&header).unwrap();
    // No rows: the header length is used only when the rows carry no durations.
    assert_eq!(
        PlaylistDetailParser::parse_playlist_detail(&edited, &[], "PLtest").duration,
        74 * 60
    );
    assert_eq!(
        PlaylistDetailParser::parse_playlist_detail(&page, &[], "PLtest").duration,
        30 * 60
    );
}

#[test]
fn artist_description_from_the_about_shelf() {
    let artist = ArtistDetailParser::parse_artist_detail(&fixture("artist"), "UCtest");
    assert!(
        artist
            .description_text
            .as_deref()
            .unwrap()
            .starts_with("Daft Punk were a French electronic music duo")
    );
}

#[test]
fn artist_page_album_card_reads_its_year_from_the_subtitle() {
    let artist = ArtistDetailParser::parse_artist_detail(&fixture("artist"), "UCtest");
    let album = artist
        .albums
        .iter()
        .find(|a| a.title == "Random Access Memories")
        .unwrap();
    // Card subtitle: "2013".
    assert_eq!(album.year, Some(2013));
}
