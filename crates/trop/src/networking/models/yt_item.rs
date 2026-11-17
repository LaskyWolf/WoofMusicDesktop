// Swift's `enum YTItem { case song(SongItem) ... }` stays an enum with one
// struct per case. The `init(entity:)` conversions land with the database
// port (R5).

use swiftytube::formatting::count_format::CountFormat;
use swiftytube::formatting::duration_format::{DurationFormat, DurationFormatInt};
use swiftytube::networking::inner_tube_decode::{
    InnerTubeDecode, JsonDict, as_dict, as_dict_list, as_string,
};
use swiftytube::networking::inner_tube_json::InnerTubeJSON;

use super::home_page_parser::HomePageParser;
use crate::from_swiftytube::streaming::video_availability::VideoAvailability;
use crate::from_swiftytube::sync::artist_credits::{ArtistCredit, ArtistCredits};

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct YTArtist {
    pub name: String,
    pub id: Option<String>,
}

impl YTArtist {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            id: None,
        }
    }
}

impl From<ArtistCredit> for YTArtist {
    fn from(credit: ArtistCredit) -> Self {
        Self {
            name: credit.name,
            id: credit.browse_id,
        }
    }
}

fn credits_to_artists(runs: &[&JsonDict]) -> Vec<YTArtist> {
    ArtistCredits::parse(runs)
        .into_iter()
        .map(YTArtist::from)
        .collect()
}

/// Bare 4-digit release year segment.
fn parse_year(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    if trimmed.len() != 4 {
        return None;
    }
    trimmed
        .parse()
        .ok()
        .filter(|year| *year > 1900 && *year < 2100)
}

/// Subtitle stats scraped from a result's runs.
#[derive(Default)]
struct ItemStats {
    view_count: Option<i64>,
    year: Option<i64>,
    starts_with_video: bool,
}

/// Scans subtitle segments for view counts and years.
fn extract_item_stats(segments: &[Vec<String>]) -> ItemStats {
    let mut stats = ItemStats::default();
    for text in segments.iter().flatten() {
        stats.view_count = stats
            .view_count
            .or_else(|| CountFormat::parse_compact_count(text));
        stats.year = stats.year.or_else(|| parse_year(text));
    }
    stats.starts_with_video = segments
        .first()
        .and_then(|seg| seg.first())
        .is_some_and(|t| t.trim().to_lowercase() == "video");
    stats
}

#[derive(Clone, Debug, PartialEq)]
pub enum YTItem {
    Song(SongItem),
    Album(AlbumItem),
    Artist(ArtistItem),
    Playlist(PlaylistItem),
    Podcast(PodcastItem),
    Episode(EpisodeItem),
}

impl YTItem {
    pub fn id(&self) -> &str {
        match self {
            YTItem::Song(s) => &s.video_id,
            YTItem::Album(a) => &a.browse_id,
            YTItem::Artist(a) => &a.browse_id,
            YTItem::Playlist(p) => &p.id,
            YTItem::Podcast(p) => &p.browse_id,
            YTItem::Episode(e) => &e.video_id,
        }
    }

    pub fn title(&self) -> &str {
        match self {
            YTItem::Song(s) => &s.title,
            YTItem::Album(a) => &a.title,
            YTItem::Artist(a) => &a.name,
            YTItem::Playlist(p) => &p.title,
            YTItem::Podcast(p) => &p.title,
            YTItem::Episode(e) => &e.title,
        }
    }

    pub fn thumbnail_url(&self) -> Option<&str> {
        match self {
            YTItem::Song(s) => s.thumbnail_url.as_deref(),
            YTItem::Album(a) => a.thumbnail_url.as_deref(),
            YTItem::Artist(a) => a.thumbnail_url.as_deref(),
            YTItem::Playlist(p) => p.thumbnail_url.as_deref(),
            YTItem::Podcast(p) => p.thumbnail_url.as_deref(),
            YTItem::Episode(e) => e.thumbnail_url.as_deref(),
        }
    }

    pub fn video_id(&self) -> Option<&str> {
        match self {
            YTItem::Song(s) => Some(&s.video_id),
            YTItem::Episode(e) => Some(&e.video_id),
            _ => None,
        }
    }

    pub fn first_album_browse_id(&self) -> Option<&str> {
        match self {
            YTItem::Song(s) => s.album_id.as_deref(),
            _ => None,
        }
    }

    pub fn web_url(&self) -> Option<String> {
        match self {
            YTItem::Song(s) => Some(s.web_url()),
            YTItem::Album(a) => a.web_url(),
            YTItem::Artist(a) => Some(a.web_url()),
            YTItem::Playlist(p) => Some(p.web_url()),
            YTItem::Podcast(_) => None,
            YTItem::Episode(e) => Some(e.web_url()),
        }
    }

    pub fn from_responsive_list_item(renderer: &JsonDict) -> Option<YTItem> {
        let flex_columns = as_dict_list(renderer.get("flexColumns")).filter(|c| !c.is_empty())?;

        let title = flex_text(&flex_columns, 0).unwrap_or("Unknown").to_string();
        let thumbnail_url = extract_responsive_thumbnail(renderer);
        let page_type = HomePageParser::extract_page_type(renderer);
        let has_watch = HomePageParser::has_watch_endpoint(renderer);

        // Determine isEpisode
        let is_non_music_audio_track =
            page_type == Some("MUSIC_PAGE_TYPE_NON_MUSIC_AUDIO_TRACK_PAGE");
        let subtitle_runs = flex_text_runs(&flex_columns, 1);
        let is_first_subtitle_episode = subtitle_runs
            .first()
            .and_then(|r| as_string(r.get("text")))
            .is_some_and(|t| t.trim() == "Episode");
        let has_podcast_link = subtitle_runs.iter().any(|r| {
            HomePageParser::extract_page_type(r) == Some("MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE")
        });
        let is_episode = is_non_music_audio_track || is_first_subtitle_episode || has_podcast_link;

        if is_episode {
            let video_id = extract_video_id(renderer).unwrap_or_default();
            if !video_id.is_empty() {
                let segments = split_runs_by_separator(&subtitle_runs);
                let podcast_name = segments.last().map(|seg| seg.join(" "));
                return Some(YTItem::Episode(EpisodeItem {
                    video_id: video_id.to_string(),
                    title,
                    artists: podcast_name
                        .map(|name| vec![YTArtist::named(ArtistCredits::cleaned_name(&name))])
                        .unwrap_or_default(),
                    duration: DurationFormat::parse_renderer_duration(renderer),
                    thumbnail_url,
                    ..EpisodeItem::default()
                }));
            }
        }

        if (has_watch || page_type.is_none())
            && let Some(song) = SongItem::from(renderer)
        {
            return Some(YTItem::Song(song));
        }

        let Some(browse_id) = extract_two_row_browse_id(renderer) else {
            return if has_watch {
                SongItem::from(renderer).map(YTItem::Song)
            } else {
                None
            };
        };
        let browse_id = browse_id.to_string();

        match page_type {
            Some("MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK") => {
                let runs = flex_text_runs(&flex_columns, 1);
                let mut year = None;
                for seg in split_runs_by_separator(&runs) {
                    if let Some(y) = seg.first().and_then(|t| t.parse::<i64>().ok())
                        && y > 1900
                        && y < 2100
                    {
                        year = Some(y);
                    }
                }
                Some(YTItem::Album(AlbumItem {
                    browse_id,
                    title,
                    artists: credits_to_artists(&runs),
                    year,
                    thumbnail_url,
                    playlist_id: extract_playlist_id(renderer).map(str::to_string),
                    is_explicit: false,
                }))
            }
            Some("MUSIC_PAGE_TYPE_ARTIST") => Some(YTItem::Artist(ArtistItem {
                browse_id,
                name: title,
                thumbnail_url,
                is_subscribed: false,
            })),
            Some("MUSIC_PAGE_TYPE_PLAYLIST") => {
                let segments = split_runs_by_separator(&flex_text_runs(&flex_columns, 1));
                Some(YTItem::Playlist(PlaylistItem {
                    id: browse_id,
                    title,
                    author: segments.first().map(|seg| seg.join(" ")),
                    thumbnail_url,
                    song_count: None,
                }))
            }
            Some("MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE") => {
                Some(YTItem::Podcast(PodcastItem {
                    browse_id,
                    title,
                    author: None,
                    thumbnail_url,
                }))
            }
            Some("MUSIC_PAGE_TYPE_NON_MUSIC_AUDIO_TRACK_PAGE") => {
                EpisodeItem::from(renderer).map(YTItem::Episode)
            }
            _ => {
                if browse_id.starts_with("FElike_") || browse_id.starts_with("VL") {
                    Some(YTItem::Playlist(PlaylistItem {
                        id: browse_id,
                        title,
                        thumbnail_url,
                        ..PlaylistItem::default()
                    }))
                } else if browse_id.starts_with("UC") {
                    Some(YTItem::Artist(ArtistItem {
                        browse_id,
                        name: title,
                        thumbnail_url,
                        is_subscribed: false,
                    }))
                } else if browse_id.starts_with("MPREb_") {
                    Some(YTItem::Album(AlbumItem {
                        browse_id,
                        title,
                        thumbnail_url,
                        ..AlbumItem::default()
                    }))
                } else {
                    None
                }
            }
        }
    }
}

/// Who added a song to a shared playlist.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlaylistContributor {
    pub name: String,
    pub is_owner: bool,
    pub avatar_url: Option<String>,
}

impl PlaylistContributor {
    /// `contributorsAvatars` of a playlist row. The owner is the first avatar of
    /// the playlist header; avatars are matched on the image URL without its
    /// size suffix, which is the same in the header and in the rows.
    pub fn from(renderer: &JsonDict, owner_avatar_url: Option<&str>) -> Option<Self> {
        let avatar = InnerTubeDecode::array(
            &["contributorsAvatars", "avatarStackViewModel", "avatars"],
            renderer,
        )
        .and_then(|avatars| avatars.first().copied())
        .and_then(|first| as_dict(first.get("avatarViewModel")));
        let name = as_string(avatar?.get("accessibilityText")).filter(|n| !n.is_empty())?;
        let url = InnerTubeDecode::array(&["image", "sources"], avatar?)
            .and_then(|sources| sources.first().copied())
            .and_then(|source| as_string(source.get("url")));
        let is_owner = matches!((owner_avatar_url, url), (Some(owner), Some(url)) if Self::image_key(url) == Self::image_key(owner));
        Some(Self {
            name: name.to_string(),
            is_owner,
            avatar_url: url.map(str::to_string),
        })
    }

    pub fn image_key(url: &str) -> &str {
        url.split_once('=').map_or(url, |(key, _)| key)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SongItem {
    pub video_id: String,
    pub title: String,
    pub artists: Vec<YTArtist>,
    pub album: Option<String>,
    pub album_id: Option<String>,
    pub duration: i64,
    pub thumbnail_url: Option<String>,
    pub is_explicit: bool,
    pub playlist_id: Option<String>,
    pub like_status: Option<String>,

    /// Parsed subtitle stats (search results only; `None` when absent).
    pub view_count: Option<i64>,
    pub year: Option<i64>,
    pub is_video: Option<bool>,

    /// Watch-endpoint music type (`MUSIC_VIDEO_TYPE_ATV` for auto-generated
    /// art-tracks), when the row carried one.
    pub music_video_type: Option<String>,

    /// Set on rows of a shared playlist.
    pub added_by: Option<PlaylistContributor>,

    /// True when YouTube Music greys the row out.
    pub is_unavailable: Option<bool>,
}

impl SongItem {
    /// True for music-video results (subtitle starts with "Video").
    pub fn is_music_video(&self) -> bool {
        self.is_video == Some(true)
    }

    /// Unavailable songs show greyed out and never enter the queue or the download counts.
    pub fn is_available(&self) -> bool {
        self.is_unavailable != Some(true)
    }

    pub fn web_url(&self) -> String {
        format!("https://music.youtube.com/watch?v={}", self.video_id)
    }

    pub fn artist_names_display(&self) -> String {
        self.artists
            .iter()
            .filter(|a| !a.name.is_empty())
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// "Artists • m:ss" subtitle line used by song rows.
    pub fn subtitle_line(&self, duration: i64) -> String {
        let duration_str = duration.formatted_duration();
        let artists = self.artist_names_display();
        if artists.is_empty() {
            return duration_str;
        }
        if duration_str.is_empty() {
            return artists;
        }
        format!("{artists} • {duration_str}")
    }

    pub fn from(renderer: &JsonDict) -> Option<SongItem> {
        let video_id = extract_video_id(renderer)?.to_string();
        let duration = DurationFormat::parse_renderer_duration(renderer);
        let playlist_id = extract_playlist_id(renderer).map(str::to_string);

        let mut song = match as_dict_list(renderer.get("flexColumns")) {
            Some(flex_columns) => Self::from_responsive_list_item(
                renderer,
                &flex_columns,
                video_id,
                duration,
                playlist_id,
            ),
            None => Self::from_two_row_item(renderer, video_id, duration, playlist_id),
        };
        if as_string(renderer.get("musicItemRendererDisplayPolicy"))
            == Some("MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT")
        {
            song.is_unavailable = Some(true);
        }
        Some(song)
    }

    fn from_responsive_list_item(
        renderer: &JsonDict,
        flex_columns: &[&JsonDict],
        video_id: String,
        duration: i64,
        playlist_id: Option<String>,
    ) -> SongItem {
        let title = flex_text(flex_columns, 0).unwrap_or("Unknown").to_string();
        let runs = flex_text_runs(flex_columns, 1);
        let segments = split_runs_by_separator(&runs);

        // Album = first run in any subtitle column that opens an album page. Position
        // is unreliable: those slots also hold views, durations and featured artists.
        let album_run = (1..flex_columns.len())
            .flat_map(|i| flex_text_runs(flex_columns, i))
            .find_map(|run| album_browse_id(run).map(|id| (run, id)));
        let album = album_run
            .and_then(|(run, _)| as_string(run.get("text")))
            .map(|t| t.trim().to_string());
        let album_id = album_run.map(|(_, id)| id.to_string());

        let artists = if segments.is_empty() {
            Vec::new()
        } else {
            let credits = ArtistCredits::parse(&runs);
            let to_artist = |raw: &String| {
                let name = ArtistCredits::cleaned_name(raw);
                let id = credits
                    .iter()
                    .find(|c| c.name == name)
                    .and_then(|c| c.browse_id.clone());
                YTArtist { name, id }
            };
            let artist_segments: Vec<&Vec<String>> = segments
                .iter()
                .skip_while(|seg| seg.iter().all(|t| !ArtistCredits::is_artist_name(t)))
                .collect();
            let chosen: Vec<&Vec<String>> = if artist_segments.is_empty() {
                vec![&segments[0]]
            } else {
                artist_segments
            };
            chosen
                .into_iter()
                .flatten()
                .filter(|t| ArtistCredits::is_artist_name(t))
                .map(to_artist)
                .collect()
        };
        let stats = extract_item_stats(&segments);
        SongItem {
            video_id,
            title,
            artists,
            album,
            album_id,
            duration,
            thumbnail_url: extract_responsive_thumbnail(renderer),
            playlist_id,
            view_count: stats.view_count,
            year: stats.year,
            is_video: stats.starts_with_video.then_some(true),
            music_video_type: VideoAvailability::row_music_video_type(renderer),
            ..SongItem::default()
        }
    }

    fn from_two_row_item(
        renderer: &JsonDict,
        video_id: String,
        duration: i64,
        playlist_id: Option<String>,
    ) -> SongItem {
        let title = InnerTubeJSON::runs_text(as_dict(renderer.get("title")))
            .unwrap_or_else(|| "Unknown".to_string());
        let mut artists = Vec::new();
        let mut album = None;
        let mut album_id = None;
        let runs = InnerTubeDecode::array(&["subtitle", "runs"], renderer);
        if let Some(runs) = &runs {
            for run in runs {
                let Some(text) = as_string(run.get("text")) else {
                    continue;
                };
                let trimmed = text.trim();
                if trimmed.is_empty() || trimmed == "•" {
                    continue;
                }
                let bid = InnerTubeDecode::string(
                    &["navigationEndpoint", "browseEndpoint", "browseId"],
                    run,
                );
                if let Some(bid) = bid.filter(|b| b.starts_with("MPREb_")) {
                    album = Some(trimmed.to_string());
                    album_id = Some(bid.to_string());
                }
            }
            artists = credits_to_artists(runs);
        }
        let mut sub_segments: Vec<Vec<String>> = vec![Vec::new()];
        for text in runs
            .iter()
            .flatten()
            .filter_map(|r| as_string(r.get("text")))
            .map(str::trim)
        {
            if text == "•" || text == "·" {
                sub_segments.push(Vec::new());
            } else if !text.is_empty() {
                sub_segments.last_mut().unwrap().push(text.to_string());
            }
        }
        let stats = extract_item_stats(&sub_segments);
        SongItem {
            video_id,
            title,
            artists,
            album,
            album_id,
            duration,
            thumbnail_url: extract_two_row_thumbnail(renderer),
            playlist_id,
            view_count: stats.view_count,
            year: stats.year,
            is_video: stats.starts_with_video.then_some(true),
            music_video_type: VideoAvailability::row_music_video_type(renderer),
            ..SongItem::default()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AlbumItem {
    pub browse_id: String,
    pub title: String,
    pub artists: Vec<YTArtist>,
    pub year: Option<i64>,
    pub thumbnail_url: Option<String>,
    pub playlist_id: Option<String>,
    pub is_explicit: bool,
}

impl AlbumItem {
    pub fn web_url(&self) -> Option<String> {
        self.playlist_id
            .as_ref()
            .map(|id| format!("https://music.youtube.com/playlist?list={id}"))
    }

    pub fn from(renderer: &JsonDict) -> Option<AlbumItem> {
        Some(AlbumItem {
            browse_id: extract_two_row_browse_id(renderer)?.to_string(),
            title: two_row_title(renderer),
            thumbnail_url: extract_two_row_thumbnail(renderer),
            playlist_id: extract_playlist_id(renderer).map(str::to_string),
            ..AlbumItem::default()
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ArtistItem {
    pub browse_id: String,
    pub name: String,
    pub thumbnail_url: Option<String>,
    pub is_subscribed: bool,
}

impl ArtistItem {
    pub fn web_url(&self) -> String {
        format!("https://music.youtube.com/channel/{}", self.browse_id)
    }

    pub fn from(renderer: &JsonDict) -> Option<ArtistItem> {
        Some(ArtistItem {
            browse_id: extract_two_row_browse_id(renderer)?.to_string(),
            name: two_row_title(renderer),
            thumbnail_url: extract_two_row_thumbnail(renderer),
            is_subscribed: false,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistItem {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub thumbnail_url: Option<String>,
    pub song_count: Option<i64>,
}

impl PlaylistItem {
    pub fn web_url(&self) -> String {
        format!("https://music.youtube.com/playlist?list={}", self.id)
    }

    pub fn from(renderer: &JsonDict) -> Option<PlaylistItem> {
        Some(PlaylistItem {
            id: extract_two_row_browse_id(renderer)?.to_string(),
            title: two_row_title(renderer),
            author: InnerTubeJSON::runs_texts(as_dict(renderer.get("subtitle")))
                .into_iter()
                .next(),
            thumbnail_url: extract_two_row_thumbnail(renderer),
            song_count: None,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PodcastItem {
    pub browse_id: String,
    pub title: String,
    pub author: Option<String>,
    pub thumbnail_url: Option<String>,
}

impl PodcastItem {
    pub fn from(renderer: &JsonDict) -> Option<PodcastItem> {
        Some(PodcastItem {
            browse_id: extract_two_row_browse_id(renderer)?.to_string(),
            title: two_row_title(renderer),
            author: None,
            thumbnail_url: extract_two_row_thumbnail(renderer),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EpisodeItem {
    pub video_id: String,
    pub title: String,
    pub artists: Vec<YTArtist>,
    pub duration: i64,
    pub thumbnail_url: Option<String>,
    pub publish_date: Option<String>,
    pub description_text: Option<String>,
    pub views_text: Option<String>,
}

impl EpisodeItem {
    pub fn web_url(&self) -> String {
        format!("https://music.youtube.com/watch?v={}", self.video_id)
    }

    pub fn from(renderer: &JsonDict) -> Option<EpisodeItem> {
        let video_id = extract_video_id(renderer)?.to_string();
        let mut duration = DurationFormat::parse_renderer_duration(renderer);
        if duration == 0
            && let Some(duration_text) = InnerTubeJSON::runs_text(InnerTubeDecode::dict(
                &[
                    "playbackProgress",
                    "musicPlaybackProgressRenderer",
                    "durationText",
                ],
                renderer,
            ))
            .filter(|t| !t.is_empty())
        {
            duration = DurationFormat::parse_clock(&duration_text)
                .or_else(|| DurationFormat::parse_spoken(&duration_text))
                .unwrap_or(0);
        }
        // Subtitle runs carry "views • date" alongside the duration.
        let mut publish_date = None;
        let mut views_text = None;
        if let Some(flex_columns) = as_dict_list(renderer.get("flexColumns")) {
            for run in flex_text_runs(&flex_columns, 1) {
                let Some(text) = as_string(run.get("text")) else {
                    continue;
                };
                let trimmed = text.trim();
                if trimmed.is_empty() || trimmed == "•" || trimmed.contains(':') {
                    continue;
                }
                let lower = trimmed.to_lowercase();
                if lower.contains("view")
                    || lower.contains("listening")
                    || lower.contains("download")
                {
                    views_text.get_or_insert_with(|| trimmed.to_string());
                } else {
                    publish_date.get_or_insert_with(|| trimmed.to_string());
                }
            }
        }
        Some(EpisodeItem {
            video_id,
            title: two_row_title(renderer),
            artists: Vec::new(),
            duration,
            thumbnail_url: extract_two_row_thumbnail(renderer),
            publish_date,
            description_text: InnerTubeJSON::runs_text(as_dict(renderer.get("description"))),
            views_text,
        })
    }

    pub fn to_song_item(&self) -> SongItem {
        SongItem {
            video_id: self.video_id.clone(),
            title: self.title.clone(),
            artists: self.artists.clone(),
            duration: self.duration,
            thumbnail_url: self.thumbnail_url.clone(),
            // The player response rarely names the type: the first clients
            // tried are not YouTube Music ones. The queue item carries it instead.
            music_video_type: Some("MUSIC_VIDEO_TYPE_PODCAST_EPISODE".to_string()),
            ..SongItem::default()
        }
    }
}

// MARK: - Extract Helpers

fn two_row_title(renderer: &JsonDict) -> String {
    InnerTubeJSON::runs_text(as_dict(renderer.get("title")))
        .unwrap_or_else(|| "Unknown".to_string())
}

fn extract_video_id(renderer: &JsonDict) -> Option<&str> {
    // 1. Top-level videoId (rare, search result items)
    if let Some(top) = as_string(renderer.get("videoId")) {
        return Some(top);
    }

    // 2. playlistItemData.videoId — primary for playlist / album track rows
    if let Some(from_item_data) =
        InnerTubeDecode::string(&["playlistItemData", "videoId"], renderer)
    {
        return Some(from_item_data);
    }

    // 3. flexColumns[0] run navigationEndpoint.watchEndpoint.videoId
    if let Some(flex_columns) = as_dict_list(renderer.get("flexColumns"))
        && let Some(first) = flex_columns.first()
        && let Some(runs) = InnerTubeDecode::array(
            &["musicResponsiveListItemFlexColumnRenderer", "text", "runs"],
            first,
        )
        && let Some(first_run) = runs.first()
        && let Some(id) = InnerTubeDecode::string(
            &["navigationEndpoint", "watchEndpoint", "videoId"],
            first_run,
        )
    {
        return Some(id);
    }

    // 4. overlay musicPlayButtonRenderer.playNavigationEndpoint.watchEndpoint.videoId
    if let Some(overlay_id) = InnerTubeDecode::string(
        &[
            "overlay",
            "musicItemThumbnailOverlayRenderer",
            "content",
            "musicPlayButtonRenderer",
            "playNavigationEndpoint",
            "watchEndpoint",
            "videoId",
        ],
        renderer,
    ) {
        return Some(overlay_id);
    }

    // 5. navigationEndpoint.watchEndpoint.videoId (legacy)
    InnerTubeDecode::string(
        &["navigationEndpoint", "watchEndpoint", "videoId"],
        renderer,
    )
}

fn extract_two_row_browse_id(renderer: &JsonDict) -> Option<&str> {
    InnerTubeDecode::string(
        &["navigationEndpoint", "browseEndpoint", "browseId"],
        renderer,
    )
}

fn extract_playlist_id(renderer: &JsonDict) -> Option<&str> {
    InnerTubeDecode::string(
        &["navigationEndpoint", "watchEndpoint", "playlistId"],
        renderer,
    )
}

fn extract_two_row_thumbnail(renderer: &JsonDict) -> Option<String> {
    InnerTubeJSON::nested_thumbnail_url(Some(renderer)).or_else(|| {
        InnerTubeJSON::nested_thumbnail_url(InnerTubeDecode::dict(
            &["thumbnailRenderer", "musicThumbnailRenderer"],
            renderer,
        ))
    })
}

// MARK: - Responsive List Item Helpers

fn flex_renderer<'a>(flex_columns: &[&'a JsonDict], index: usize) -> Option<&'a JsonDict> {
    let col = flex_columns.get(index)?;
    as_dict(col.get("musicResponsiveListItemFlexColumnRenderer"))
        .or_else(|| as_dict(col.get("musicResponsiveListItemColumnRenderer")))
}

fn flex_text<'a>(flex_columns: &[&'a JsonDict], index: usize) -> Option<&'a str> {
    as_string(flex_text_runs(flex_columns, index).first()?.get("text"))
}

/// Browse id of a run that opens an album page (`MPREb_` id or album page type).
fn album_browse_id(run: &JsonDict) -> Option<&str> {
    let browse = InnerTubeDecode::dict(&["navigationEndpoint", "browseEndpoint"], run)?;
    let bid = as_string(browse.get("browseId"))?;
    let page_type = InnerTubeDecode::string(
        &[
            "browseEndpointContextSupportedConfigs",
            "browseEndpointContextMusicConfig",
            "pageType",
        ],
        browse,
    );
    (bid.starts_with("MPREb_") || page_type == Some("MUSIC_PAGE_TYPE_ALBUM")).then_some(bid)
}

fn flex_text_runs<'a>(flex_columns: &[&'a JsonDict], index: usize) -> Vec<&'a JsonDict> {
    flex_renderer(flex_columns, index)
        .and_then(|r| InnerTubeDecode::array(&["text", "runs"], r))
        .unwrap_or_default()
}

const SEPARATORS: &[&str] = &["•", "·", ",", "&", "and", "feat.", "ft.", "featuring"];

fn split_runs_by_separator(runs: &[&JsonDict]) -> Vec<Vec<String>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    for run in runs {
        let Some(text) = as_string(run.get("text")) else {
            continue;
        };
        let trimmed = text.trim();
        if SEPARATORS.contains(&trimmed.to_lowercase().as_str()) {
            if !current.is_empty() {
                segments.push(std::mem::take(&mut current));
            }
        } else if !trimmed.is_empty() {
            current.push(trimmed.to_string());
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn extract_responsive_thumbnail(renderer: &JsonDict) -> Option<String> {
    let thumbnail = as_dict(renderer.get("thumbnail"))?;
    InnerTubeJSON::nested_thumbnail_url(as_dict(thumbnail.get("musicThumbnailRenderer"))).or_else(
        || InnerTubeJSON::last_thumbnail_url(as_dict_list(thumbnail.get("thumbnails")).as_deref()),
    )
}
