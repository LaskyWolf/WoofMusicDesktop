//! The radio part of `Trop/Services/PersonalizationService.swift` and
//! `RadioOptions.swift`: a queue of songs like a given one, from the
//! `next` endpoint's watch playlist.

use serde_json::Value;
use swiftytube::formatting::duration_format::DurationFormat;
use swiftytube::networking::inner_tube_client::{InnerTubeClient, NextArgs};
use swiftytube::networking::inner_tube_decode::{JsonDict, as_dict, as_dict_list, as_string};
use swiftytube::networking::inner_tube_json::InnerTubeJSON;
use swiftytube::streaming::video_availability::VideoAvailability;
use swiftytube::sync::artist_credits::ArtistCredits;

use crate::networking::models::yt_item::{SongItem, YTArtist, has_explicit_badge};

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
