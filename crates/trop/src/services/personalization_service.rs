//! `Trop/Services/PersonalizationService.swift` and `RadioOptions.swift`:
//! Home's sections built from the library (Quick Picks, Keep Listening,
//! Forgotten Favorites) and from the account (Daily Discover, "Similar to",
//! community playlists), and the radio: a queue of songs like a given one,
//! from the `next` endpoint's watch playlist.

use std::collections::HashSet;

use serde_json::Value;
use swiftytube::formatting::duration_format::DurationFormat;
use swiftytube::networking::inner_tube_client::{BrowseArgs, InnerTubeClient, NextArgs};
use swiftytube::networking::inner_tube_decode::{
    InnerTubeDecode, JsonDict, as_dict, as_dict_list, as_string,
};
use swiftytube::networking::inner_tube_json::InnerTubeJSON;
use swiftytube::streaming::video_availability::VideoAvailability;
use swiftytube::sync::artist_credits::ArtistCredits;
use swiftytube::sync::browse_lens::BrowseLens;
use swiftytube::sync::library_browse_parser::LibraryBrowseParser;

use crate::database::database_service::DatabaseService;
use crate::database::entities::song_entity::SongEntity;
use crate::networking::models::home_page::HomeSection;
use crate::networking::models::yt_item::{
    AlbumItem, ArtistItem, PlaylistItem, SongItem, YTArtist, YTItem, has_explicit_badge,
};

/// The music video type the player gives podcast episodes.
pub const PODCAST_EPISODE: &str = "MUSIC_VIDEO_TYPE_PODCAST_EPISODE";

// MARK: - Phase 1: sections from the library

/// Swift `SongItem(entity:)`.
pub fn song_item(entity: &SongEntity) -> SongItem {
    SongItem {
        video_id: entity.id.clone(),
        title: if entity.title.is_empty() {
            "Unknown".to_owned()
        } else {
            entity.title.clone()
        },
        artists: entity
            .artist_name
            .iter()
            .map(|name| YTArtist::named(ArtistCredits::cleaned_name(name)))
            .collect(),
        album: entity.album_name.clone(),
        duration: entity.duration,
        thumbnail_url: entity.thumbnail_url.clone(),
        music_video_type: entity.is_episode.then(|| PODCAST_EPISODE.to_owned()),
        is_unavailable: entity.is_unavailable.then_some(true),
        ..SongItem::default()
    }
}

fn song_items(entities: &[SongEntity]) -> Vec<YTItem> {
    entities
        .iter()
        .map(|e| YTItem::Song(song_item(e)))
        .collect()
}

/// A failed read leaves that part out and goes to the log.
fn rows<T>(name: &str, result: rusqlite::Result<Vec<T>>) -> Vec<T> {
    result.unwrap_or_else(|error| {
        log::error!("Home: reading {name} failed: {error}");
        Vec::new()
    })
}

/// Liked songs in a random order, at most `limit`.
// ponytail: Swift also fetches the title of rows saved without one
// (`enrichLikedSongs`); here they read "Unknown" until a sync fills them.
pub fn build_quick_picks(db: &DatabaseService, limit: usize) -> HomeSection {
    let mut liked = rows("liked songs", db.fetch_liked_songs());
    fastrand::shuffle(&mut liked);
    liked.truncate(limit.max(1));
    HomeSection::QuickPicks(song_items(&liked))
}

/// The newest plays of the last year, then saved albums and artists.
pub fn build_keep_listening(db: &DatabaseService) -> HomeSection {
    let mut items = song_items(&rows(
        "recent songs",
        db.fetch_recent_songs(Some(365), Some(6)),
    ));
    items.extend(
        rows("albums", db.fetch_albums(Some(4)))
            .into_iter()
            .map(|album| {
                YTItem::Album(AlbumItem {
                    browse_id: album.id,
                    title: album.title,
                    artists: Vec::new(),
                    year: None,
                    thumbnail_url: album.thumbnail_url,
                    playlist_id: album.playlist_id,
                    is_explicit: false,
                })
            }),
    );
    items.extend(
        rows("artists", db.fetch_artists(Some(4)))
            .into_iter()
            .map(|artist| {
                YTItem::Artist(ArtistItem {
                    is_subscribed: artist.bookmarked_at.is_some(),
                    browse_id: artist.id,
                    name: artist.name,
                    thumbnail_url: artist.thumbnail_url,
                })
            }),
    );
    HomeSection::KeepListening(items)
}

/// Liked songs not played in 60 days.
pub fn build_forgotten_favorites(db: &DatabaseService) -> HomeSection {
    HomeSection::ForgottenFavorites(song_items(&rows(
        "forgotten favorites",
        db.fetch_forgotten_favorites(Some(60), Some(10)),
    )))
}

// MARK: - Phase 2: sections from the account

pub fn is_logged_in(client: &InnerTubeClient) -> bool {
    client.current_auth().sapisid.is_some()
}

/// Daily Discover, the community playlists, then a "Similar to" section per
/// top artist. Empty for a guest.
pub async fn build_phase2_sections(
    client: &InnerTubeClient,
    db: &DatabaseService,
) -> Vec<HomeSection> {
    if !is_logged_in(client) {
        return Vec::new();
    }
    // "Your Playlists" is not built: it repeats the Library tab.
    let (daily, (similar, community)) = tokio::join!(
        build_daily_discover(client, db),
        build_artist_sections(client, db)
    );
    [daily, community]
        .into_iter()
        .chain(similar)
        .filter(|section| !section.items().is_empty())
        .collect()
}

/// Up to 10 songs from the radios of three random liked songs.
pub async fn build_daily_discover(client: &InnerTubeClient, db: &DatabaseService) -> HomeSection {
    let mut liked = rows("liked songs", db.fetch_liked_songs());
    fastrand::shuffle(&mut liked);
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for seed in liked.iter().take(3) {
        if found.len() >= 10 {
            break;
        }
        // One radio fills the 10 items; the automix call would only add more.
        let Ok(json) = next(client, Some(seed.id.clone()), format!("RDAMVM{}", seed.id)).await
        else {
            continue;
        };
        let Some(panel) = playlist_panel(&json) else {
            continue;
        };
        for song in panel_songs(&panel).0 {
            if found.len() >= 10 {
                break;
            }
            if seen.insert(song.video_id.clone()) {
                found.push(YTItem::Song(song));
            }
        }
    }
    HomeSection::DailyDiscover(found)
}

/// "Similar to <artist>" sections and community playlists, both parsed
/// from one browse of each of three saved artists.
pub async fn build_artist_sections(
    client: &InnerTubeClient,
    db: &DatabaseService,
) -> (Vec<HomeSection>, HomeSection) {
    let mut sections = Vec::new();
    let mut seen = HashSet::new();
    let mut community = Vec::new();
    for artist in rows("artists", db.fetch_artists(Some(3))) {
        let Ok(json) = client.browse(BrowseArgs::id(artist.id.clone())).await else {
            continue;
        };
        let related = related_items(&json);
        if !related.is_empty() {
            sections.push(HomeSection::SimilarRecommendation(
                related,
                format!("Similar to {}", artist.name),
            ));
        }
        for playlist in LibraryBrowseParser::extract_community_playlists(&json) {
            if community.len() >= 5 {
                break;
            }
            let id = LibraryBrowseParser::normalize_playlist_id(&playlist.browse_id).to_owned();
            if seen.insert(id.clone()) {
                community.push(YTItem::Playlist(PlaylistItem {
                    id,
                    title: playlist.title,
                    author: None,
                    thumbnail_url: playlist.thumbnail_url,
                    song_count: playlist.song_count,
                }));
            }
        }
    }
    (sections, HomeSection::FromTheCommunity(community))
}

/// The songs of an artist page's song shelves, at most 10.
pub fn related_items(json: &JsonDict) -> Vec<YTItem> {
    BrowseLens::browse_sections(json)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|section| InnerTubeDecode::array(&["musicShelfRenderer", "contents"], section))
        .flatten()
        .filter_map(|entry| as_dict(entry.get("musicResponsiveListItemRenderer")))
        .filter_map(SongItem::from)
        .map(YTItem::Song)
        .take(10)
        .collect()
}

// MARK: - Radio

/// Swift caps each panel at 50 songs.
const PANEL_LIMIT: usize = 50;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RadioStyle {
    #[default]
    Similar,
    Variety,
    DeepCuts,
}

impl RadioStyle {
    pub const ALL: [Self; 3] = [Self::Similar, Self::Variety, Self::DeepCuts];

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Similar => "Similar",
            Self::Variety => "Variety",
            Self::DeepCuts => "Deep Cuts",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Similar => "Closest matches to this song",
            Self::Variety => "Wider mix around this song",
            Self::DeepCuts => "Lesser-known related tracks",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RadioOptions {
    pub style: RadioStyle,
    pub limit: usize,
    pub allow_explicit: bool,
}

impl Default for RadioOptions {
    fn default() -> Self {
        Self {
            style: RadioStyle::Similar,
            limit: 25,
            allow_explicit: true,
        }
    }
}

/// The radio for `video_id` and the index of that song in it.
pub async fn fetch_radio(
    client: &InnerTubeClient,
    video_id: &str,
    options: RadioOptions,
) -> Result<(Vec<SongItem>, usize), String> {
    let json = next(
        client,
        Some(video_id.to_owned()),
        format!("RDAMVM{video_id}"),
    )
    .await?;
    let panel = playlist_panel(&json).ok_or("The radio answer has no song list.")?;
    let (mut songs, current) = panel_songs(&panel);
    if songs.is_empty() {
        return Err("The radio has no songs.".into());
    }
    if let Some((auto_id, playlist_id)) = automix_endpoint(&panel)
        && let Ok(json) = next(client, auto_id, playlist_id).await
        && let Some(panel) = playlist_panel(&json)
    {
        let (more, _) = panel_songs(&panel);
        let more: Vec<SongItem> = more
            .into_iter()
            .filter(|song| !songs.iter().any(|s| s.video_id == song.video_id))
            .collect();
        songs.extend(more);
    }
    shape(songs, current, options).ok_or_else(|| "The radio has no songs.".into())
}

async fn next(
    client: &InnerTubeClient,
    video_id: Option<String>,
    playlist_id: String,
) -> Result<JsonDict, String> {
    let args = NextArgs {
        video_id,
        playlist_id: Some(playlist_id),
        ..Default::default()
    };
    client.next(args).await.map_err(|error| error.to_string())
}

/// Applies the radio style, the explicit filter and the length limit.
pub fn shape(
    mut songs: Vec<SongItem>,
    current: usize,
    options: RadioOptions,
) -> Option<(Vec<SongItem>, usize)> {
    match options.style {
        RadioStyle::Similar => {}
        RadioStyle::Variety if songs.len() > 2 => fastrand::shuffle(&mut songs[2..]),
        RadioStyle::Variety => {}
        RadioStyle::DeepCuts if songs.len() > 5 => {
            songs.drain(..3);
        }
        RadioStyle::DeepCuts => {}
    }
    if !options.allow_explicit {
        songs.retain(|song| !song.is_explicit);
    }
    songs.truncate(options.limit.max(1));
    if songs.is_empty() {
        return None;
    }
    let current = current.min(songs.len() - 1);
    Some((songs, current))
}

/// `contents.singleColumnMusicWatchNextResultsRenderer…playlistPanelRenderer.contents`.
pub fn playlist_panel(json: &JsonDict) -> Option<Vec<&JsonDict>> {
    let path = [
        "contents",
        "singleColumnMusicWatchNextResultsRenderer",
        "tabbedRenderer",
        "watchNextTabbedResultsRenderer",
    ];
    let mut dict = json;
    for key in path {
        dict = as_dict(dict.get(key))?;
    }
    let tab = as_dict_list(dict.get("tabs"))?.into_iter().next()?;
    let mut dict = as_dict(tab.get("tabRenderer"))?;
    for key in [
        "content",
        "musicQueueRenderer",
        "content",
        "playlistPanelRenderer",
    ] {
        dict = as_dict(dict.get(key))?;
    }
    as_dict_list(dict.get("contents"))
}

/// The panel's songs and the index of the selected one.
pub fn panel_songs(panel: &[&JsonDict]) -> (Vec<SongItem>, usize) {
    let mut songs = Vec::new();
    let mut current = None;
    for entry in panel {
        if songs.len() >= PANEL_LIMIT {
            break;
        }
        let Some(renderer) = as_dict(entry.get("playlistPanelVideoRenderer")) else {
            continue;
        };
        let Some(song) = panel_song(renderer) else {
            continue;
        };
        if current.is_none() && renderer.get("selected").and_then(Value::as_bool) == Some(true) {
            current = Some(songs.len());
        }
        songs.push(song);
    }
    (songs, current.unwrap_or(0))
}

/// The automix playlist the last panel entry points to: (video id, playlist id).
fn automix_endpoint(panel: &[&JsonDict]) -> Option<(Option<String>, String)> {
    let mut dict = *panel.last()?;
    for key in [
        "automixPreviewVideoRenderer",
        "content",
        "automixPlaylistVideoRenderer",
        "navigationEndpoint",
        "watchPlaylistEndpoint",
    ] {
        dict = as_dict(dict.get(key))?;
    }
    let playlist_id = as_string(dict.get("playlistId"))?.to_owned();
    Some((
        as_string(dict.get("videoId")).map(str::to_owned),
        playlist_id,
    ))
}

fn panel_song(renderer: &JsonDict) -> Option<SongItem> {
    let video_id = as_string(renderer.get("videoId"))?.to_owned();
    let title = InnerTubeJSON::runs_text(as_dict(renderer.get("title")))
        .unwrap_or_else(|| "Unknown".into());
    let byline = as_dict(renderer.get("longBylineText"))
        .or_else(|| as_dict(renderer.get("shortBylineText")));
    let artists = ArtistCredits::parse(&InnerTubeJSON::raw_runs(byline))
        .into_iter()
        .map(|credit| YTArtist {
            name: credit.name,
            id: credit.browse_id,
        })
        .collect();
    Some(SongItem {
        video_id,
        title,
        artists,
        duration: DurationFormat::parse_renderer_duration(renderer),
        thumbnail_url: InnerTubeJSON::nested_thumbnail_url(Some(renderer)),
        is_explicit: has_explicit_badge(renderer),
        music_video_type: VideoAvailability::row_music_video_type(renderer),
        ..SongItem::default()
    })
}
