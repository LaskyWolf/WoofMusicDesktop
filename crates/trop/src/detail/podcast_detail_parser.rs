// Parsing half of Trop/Detail/PodcastDetailView.swift. The Swift file mixes
// the SwiftUI view, its view model and this parser; only the parser and the
// browse call of `PodcastDetailViewModel.load()` are translated. Subscribing,
// known-episode tracking and auto-downloads land with R5.

use serde_json::Value;
use swiftytube::formatting::duration_format::DurationFormat;
use swiftytube::networking::inner_tube_client::{BrowseArgs, InnerTubeClient};
use swiftytube::networking::inner_tube_decode::{
    InnerTubeDecode, JsonDict, as_dict, as_dict_list, as_string,
};
use swiftytube::networking::inner_tube_error::InnerTubeError;
use swiftytube::networking::inner_tube_json::InnerTubeJSON;

use super::detail_models::PodcastDetailInfo;
use crate::from_swiftytube::sync::browse_lens::BrowseLens;
use crate::networking::models::yt_item::{EpisodeItem, YTArtist};

/// Header fields extracted from the podcast browse response.
struct ParsedPodcastHeader {
    title: String,
    author: Option<String>,
    description_text: Option<String>,
    thumbnail_url: Option<String>,
    is_subscribed: bool,
}

/// Subtitle metadata for one episode row.
#[derive(Default)]
struct ParsedEpisodeMeta {
    publish_date: Option<String>,
    views_text: Option<String>,
    duration: i64,
}

fn clock_or_spoken(text: &str) -> Option<i64> {
    DurationFormat::parse_clock(text).or_else(|| DurationFormat::parse_spoken(text))
}

pub struct PodcastDetailParser;

impl PodcastDetailParser {
    /// `PodcastDetailViewModel.load()` without the view state: browse + parse.
    pub async fn load(
        inner_tube: &InnerTubeClient,
        browse_id: &str,
    ) -> Result<PodcastDetailInfo, InnerTubeError> {
        let json = inner_tube.browse(BrowseArgs::id(browse_id)).await?;
        Ok(Self::parse_podcast_detail(&json, browse_id))
    }

    pub fn parse_podcast_detail(json: &JsonDict, browse_id: &str) -> PodcastDetailInfo {
        let header = Self::parse_podcast_header(json);
        let mut episodes = Self::collect_podcast_episodes(json);
        // Episodes carry no artist runs; stamp the show name so queue rows,
        // the mini player and menus show it (matching the big player author).
        let show_name = header.author.as_deref().unwrap_or(&header.title);
        for episode in &mut episodes {
            if episode.artists.is_empty() {
                episode.artists = vec![YTArtist::named(show_name)];
            }
        }
        PodcastDetailInfo {
            title: header.title,
            author: header.author,
            description_text: header.description_text,
            thumbnail_url: header.thumbnail_url,
            browse_id: browse_id.to_string(),
            is_subscribed: header.is_subscribed,
            episodes,
        }
    }

    // MARK: - Podcast header parsing

    /// Reads title/author/description/artwork/subscribed state from the
    /// immersive header, responsive header and microformat fallbacks.
    fn parse_podcast_header(json: &JsonDict) -> ParsedPodcastHeader {
        let mut header = ParsedPodcastHeader {
            title: "Unknown Podcast".to_string(),
            author: None,
            description_text: None,
            thumbnail_url: None,
            is_subscribed: false,
        };
        let first = BrowseLens::first_section_item(json);
        let wrapped = first
            .and_then(|f| as_dict(f.get("itemSectionRenderer")))
            .and_then(|r| as_dict_list(r.get("contents")));
        let first_tab_section = wrapped.and_then(|w| w.first().copied()).or(first);

        // Podcast show pages render an immersive header at the top level
        // (title, show description, artwork) — this is where the full
        // show blurb lives.
        let top_header = as_dict(json.get("header"));
        if let Some(immersive) =
            top_header.and_then(|h| as_dict(h.get("musicImmersiveHeaderRenderer")))
        {
            Self::apply_immersive_header(immersive, &mut header);
        }

        let renderer = first_tab_section
            .and_then(|s| as_dict(s.get("musicResponsiveHeaderRenderer")))
            .or_else(|| {
                top_header.and_then(|h| {
                    as_dict(h.get("musicDetailHeaderRenderer"))
                        .or_else(|| as_dict(h.get("musicResponsiveHeaderRenderer")))
                })
            });
        if let Some(renderer) = renderer {
            Self::apply_responsive_header(renderer, &mut header);
        }

        // Last resort: microformat description (always present on browse pages).
        if header.description_text.as_deref().is_none_or(str::is_empty) {
            header.description_text = InnerTubeDecode::string(
                &["microformat", "microformatDataRenderer", "description"],
                json,
            )
            .map(str::to_string);
        }
        header
    }

    fn apply_immersive_header(immersive: &JsonDict, header: &mut ParsedPodcastHeader) {
        if let Some(title) = InnerTubeJSON::runs_text(as_dict(immersive.get("title"))) {
            header.title = title;
        }
        if header.thumbnail_url.is_none() {
            header.thumbnail_url = InnerTubeJSON::music_thumbnail_url(immersive);
        }
        if let Some(desc_dict) = as_dict(immersive.get("description")) {
            let desc = InnerTubeJSON::runs_text(InnerTubeDecode::dict(
                &["musicDescriptionShelfRenderer", "description"],
                desc_dict,
            ))
            .or_else(|| InnerTubeJSON::runs_text(Some(desc_dict)));
            if let Some(desc) = desc.filter(|d| !d.is_empty()) {
                header.description_text = Some(desc);
            }
        }
        if header.author.is_none() {
            header.author = InnerTubeJSON::runs_texts(as_dict(immersive.get("subtitle")))
                .into_iter()
                .next()
                .or_else(|| {
                    InnerTubeJSON::runs_texts(as_dict(immersive.get("straplineTextOne")))
                        .into_iter()
                        .next()
                });
        }
    }

    fn apply_responsive_header(renderer: &JsonDict, header: &mut ParsedPodcastHeader) {
        if let Some(title) = InnerTubeJSON::runs_text(as_dict(renderer.get("title"))) {
            header.title = title;
        }
        if header.thumbnail_url.is_none() {
            header.thumbnail_url = InnerTubeJSON::music_thumbnail_url(renderer);
        }
        // The show blurb is usually nested one level deeper:
        // description.musicDescriptionShelfRenderer.description.runs.
        // Never clobber the immersive-header description already found.
        if header.description_text.is_none() {
            header.description_text = InnerTubeJSON::runs_text(InnerTubeDecode::dict(
                &[
                    "description",
                    "musicDescriptionShelfRenderer",
                    "description",
                ],
                renderer,
            ))
            .or_else(|| InnerTubeJSON::runs_text(as_dict(renderer.get("description"))))
            .or_else(|| InnerTubeJSON::runs_text(as_dict(renderer.get("descriptionText"))))
            .or_else(|| as_string(renderer.get("description")).map(str::to_string));
        }
        Self::apply_header_buttons(renderer, header);
        Self::apply_header_subtitle(renderer, header);
    }

    /// Subscribed state from the header action buttons, when present.
    fn apply_header_buttons(renderer: &JsonDict, header: &mut ParsedPodcastHeader) {
        for button in as_dict_list(renderer.get("buttons")).unwrap_or_default() {
            if InnerTubeDecode::value(&["toggleButtonRenderer", "isToggled"], button)
                == Some(&Value::Bool(true))
            {
                header.is_subscribed = true;
            }
            if let Some(subscribed) =
                InnerTubeDecode::value(&["subscribeButtonRenderer", "subscribed"], button)
                    .and_then(Value::as_bool)
            {
                header.is_subscribed = subscribed;
            }
        }
    }

    fn apply_header_subtitle(renderer: &JsonDict, header: &mut ParsedPodcastHeader) {
        if header.author.is_none()
            && let Some(first) = InnerTubeDecode::array(&["straplineTextOne", "runs"], renderer)
                .and_then(|s| s.first().copied())
        {
            header.author = as_string(first.get("text")).map(str::to_string);
        }
        if (header.author.is_none() || header.description_text.is_none())
            && let Some(subtitle) = InnerTubeDecode::array(&["subtitle", "runs"], renderer)
        {
            for run in subtitle {
                let Some(text) = as_string(run.get("text")) else {
                    continue;
                };
                let trimmed = text.trim();
                if trimmed.is_empty() || trimmed == "•" {
                    continue;
                }
                if header.author.is_none() {
                    header.author = Some(trimmed.to_string());
                } else if header.description_text.is_none() && trimmed.chars().count() > 3 {
                    // Second distinct subtitle run is often the show blurb.
                    header.description_text = Some(trimmed.to_string());
                }
            }
        }
        if header.description_text.is_none()
            && let Some(text) = InnerTubeJSON::runs_text(as_dict(renderer.get("secondSubtitle")))
            && !text.is_empty()
            && !text.to_lowercase().contains("episode")
        {
            header.description_text = Some(text);
        }
    }

    // MARK: - Podcast episode parsing

    /// Podcast episodes use musicMultiRowListItemRenderer inside
    /// secondaryContents.sectionListRenderer.contents[].musicShelfRenderer.contents
    /// or musicPlaylistShelfRenderer.contents.
    pub fn collect_podcast_episodes(json: &JsonDict) -> Vec<EpisodeItem> {
        fn from_sections(sections: Vec<&JsonDict>) -> Vec<EpisodeItem> {
            let mut episodes = Vec::new();
            for section in sections {
                match as_dict(section.get("itemSectionRenderer"))
                    .and_then(|r| as_dict_list(r.get("contents")))
                {
                    Some(inner) => {
                        for shelf in inner {
                            episodes.extend(PodcastDetailParser::parse_shelf_contents(shelf));
                        }
                    }
                    None => episodes.extend(PodcastDetailParser::parse_shelf_contents(section)),
                }
            }
            episodes
        }

        // Primary: twoColumnBrowseResultsRenderer.secondaryContents
        let mut episodes = InnerTubeDecode::array(
            &[
                "contents",
                "twoColumnBrowseResultsRenderer",
                "secondaryContents",
                "sectionListRenderer",
                "contents",
            ],
            json,
        )
        .map(from_sections)
        .unwrap_or_default();

        // Fallback: singleColumnBrowseResultsRenderer
        if episodes.is_empty()
            && let Some(sections) = BrowseLens::browse_sections(json)
        {
            episodes = from_sections(sections);
        }
        episodes
    }

    fn parse_shelf_contents(shelf: &JsonDict) -> Vec<EpisodeItem> {
        InnerTubeDecode::array(&["musicShelfRenderer", "contents"], shelf)
            .or_else(|| InnerTubeDecode::array(&["musicPlaylistShelfRenderer", "contents"], shelf))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|item| {
                if let Some(multi_row) = as_dict(item.get("musicMultiRowListItemRenderer")) {
                    Self::parse_multi_row_episode(multi_row)
                } else {
                    as_dict(item.get("musicResponsiveListItemRenderer")).and_then(EpisodeItem::from)
                }
            })
            .collect()
    }

    pub fn parse_multi_row_episode(renderer: &JsonDict) -> Option<EpisodeItem> {
        let video_id = InnerTubeDecode::string(&["onTap", "watchEndpoint", "videoId"], renderer)?;
        let mut duration = Self::progress_renderer_duration(renderer);
        let meta = Self::episode_subtitle_meta(renderer, duration == 0);
        if duration == 0 {
            duration = meta.duration;
        }
        Some(EpisodeItem {
            video_id: video_id.to_string(),
            title: InnerTubeJSON::runs_text(as_dict(renderer.get("title")))
                .unwrap_or_else(|| "Unknown".to_string()),
            artists: Vec::new(),
            duration,
            thumbnail_url: InnerTubeJSON::last_thumbnail_url(
                InnerTubeDecode::array(
                    &[
                        "thumbnail",
                        "musicThumbnailRenderer",
                        "thumbnail",
                        "thumbnails",
                    ],
                    renderer,
                )
                .as_deref(),
            ),
            publish_date: meta.publish_date,
            // Per-episode description snippet (shown under the title like YTM).
            description_text: InnerTubeJSON::runs_text(as_dict(renderer.get("description")))
                .or_else(|| as_string(renderer.get("description")).map(str::to_string)),
            views_text: meta.views_text,
        })
    }

    /// Most reliable duration source: the playback progress renderer.
    fn progress_renderer_duration(renderer: &JsonDict) -> i64 {
        InnerTubeJSON::runs_text(InnerTubeDecode::dict(
            &[
                "playbackProgress",
                "musicPlaybackProgressRenderer",
                "durationText",
            ],
            renderer,
        ))
        .filter(|t| !t.is_empty())
        .and_then(|t| clock_or_spoken(&t))
        .unwrap_or(0)
    }

    /// Subtitle format: "17k views • 4 hr ago • 1 hr 23 mins".
    fn episode_subtitle_meta(renderer: &JsonDict, fallback_duration: bool) -> ParsedEpisodeMeta {
        let mut meta = ParsedEpisodeMeta::default();
        let Some(runs) = InnerTubeDecode::array(&["subtitle", "runs"], renderer) else {
            return meta;
        };
        for text in runs.iter().filter_map(|r| as_string(r.get("text"))) {
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed == "•" {
                continue;
            }
            let lower = trimmed.to_lowercase();
            let spoken = DurationFormat::parse_spoken(trimmed);
            if lower.contains("ago") {
                // Relative date ("4 hr ago") — never a duration.
                meta.publish_date.get_or_insert_with(|| trimmed.to_string());
            } else if trimmed.contains(':') {
                if fallback_duration && meta.duration == 0 {
                    meta.duration = DurationFormat::parse_clock(trimmed).unwrap_or(0);
                }
            } else if let Some(spoken) = spoken.filter(|_| fallback_duration && meta.duration == 0)
            {
                meta.duration = spoken;
            } else if lower.contains("view")
                || lower.contains("listening")
                || lower.contains("download")
            {
                meta.views_text = Some(trimmed.to_string());
            } else {
                meta.publish_date.get_or_insert_with(|| trimmed.to_string());
            }
        }
        meta
    }
}
