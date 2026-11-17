// Trop's `Log.explore` is not ported: errors go to SwiftyTubeLog, the
// per-section debug traces of the Swift file are left out.

use swiftytube::core::swiftytube_log::SwiftyTubeLog;
use swiftytube::networking::inner_tube_decode::{
    InnerTubeDecode, JsonDict, as_dict, as_dict_list, as_string,
};
use swiftytube::networking::inner_tube_json::InnerTubeJSON;

use super::home_page::{
    ExploreSection, ExploreSectionKind, HomePage, HomePageChip, HomePageSection, MoodItem,
};
use super::yt_item::{
    AlbumItem, ArtistItem, EpisodeItem, PlaylistItem, PodcastItem, SongItem, YTItem,
};
use crate::from_swiftytube::sync::browse_lens::BrowseLens;

fn sorted_keys(dict: &JsonDict) -> Vec<&str> {
    let mut keys: Vec<&str> = dict.keys().map(String::as_str).collect();
    keys.sort();
    keys
}

pub struct HomePageParser;

impl HomePageParser {
    pub const EXPLORE_BROWSE_ID: &str = "FEmusic_explore";
    pub const MOODS_CATEGORY_BROWSE_ID: &str = "FEmusic_moods_and_genres_category";

    pub fn parse_home_page(json: &JsonDict) -> Option<HomePage> {
        let section_list = BrowseLens::section_list(json)?;
        Some(HomePage {
            chips: Self::parse_chips(section_list),
            sections: Self::parse_sections(section_list),
            continuation: Self::extract_continuation(section_list),
        })
    }

    /// Sections and next token of a `continuationContents` answer.
    pub fn parse_continuation_sections(
        json: &JsonDict,
    ) -> Option<(Vec<HomePageSection>, Option<String>)> {
        let section_list =
            InnerTubeDecode::dict(&["continuationContents", "sectionListContinuation"], json)?;
        Some((
            Self::parse_sections(section_list),
            Self::extract_continuation(section_list),
        ))
    }

    // MARK: - Chips

    pub fn parse_chips(section_list: &JsonDict) -> Vec<HomePageChip> {
        InnerTubeDecode::array(&["header", "chipCloudRenderer", "chips"], section_list)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|chip| as_dict(chip.get("chipCloudChipRenderer")))
            .map(|chip| HomePageChip {
                title: InnerTubeJSON::runs_text(as_dict(chip.get("text"))).unwrap_or_default(),
                params: InnerTubeDecode::string(
                    &["navigationEndpoint", "browseEndpoint", "params"],
                    chip,
                )
                .map(str::to_string),
            })
            .collect()
    }

    // MARK: - Sections

    pub fn parse_sections(section_list: &JsonDict) -> Vec<HomePageSection> {
        as_dict_list(section_list.get("contents"))
            .unwrap_or_default()
            .into_iter()
            .filter_map(Self::parse_section)
            .collect()
    }

    pub fn parse_section(content: &JsonDict) -> Option<HomePageSection> {
        Self::parse_carousel_section(as_dict(content.get("musicCarouselShelfRenderer"))?)
    }

    pub fn parse_carousel_section(carousel: &JsonDict) -> Option<HomePageSection> {
        let title = InnerTubeJSON::runs_text(InnerTubeDecode::dict(
            &["header", "musicCarouselShelfBasicHeaderRenderer", "title"],
            carousel,
        ))?;
        let items = Self::parse_items(&as_dict_list(carousel.get("contents")).unwrap_or_default());
        if items.is_empty() {
            return None;
        }
        Some(HomePageSection { title, items })
    }

    pub fn parse_items(contents: &[&JsonDict]) -> Vec<YTItem> {
        let mut out = Vec::new();
        for item in contents {
            if let Some(two_row) = as_dict(item.get("musicTwoRowItemRenderer")) {
                out.extend(Self::parse_two_row_item(two_row));
                continue;
            }
            if let Some(responsive_list) = as_dict(item.get("musicResponsiveListItemRenderer")) {
                out.extend(SongItem::from(responsive_list).map(YTItem::Song));
            }
        }
        out
    }

    pub fn parse_two_row_item(renderer: &JsonDict) -> Option<YTItem> {
        let page_type = Self::extract_page_type(renderer);
        if Self::has_watch_endpoint(renderer) && page_type.is_none() {
            return SongItem::from(renderer).map(YTItem::Song);
        }
        match page_type? {
            "MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK" => {
                AlbumItem::from(renderer).map(YTItem::Album)
            }
            "MUSIC_PAGE_TYPE_ARTIST" => ArtistItem::from(renderer).map(YTItem::Artist),
            "MUSIC_PAGE_TYPE_PLAYLIST" => PlaylistItem::from(renderer).map(YTItem::Playlist),
            "MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE" => {
                PodcastItem::from(renderer).map(YTItem::Podcast)
            }
            "MUSIC_PAGE_TYPE_NON_MUSIC_AUDIO_TRACK_PAGE" => {
                EpisodeItem::from(renderer).map(YTItem::Episode)
            }
            _ => None,
        }
    }

    // MARK: - Explore

    /// Parses `FEmusic_explore` (and mood category pages) into categorized sections.
    pub fn parse_explore_sections(json: &JsonDict) -> Vec<ExploreSection> {
        let Some(contents) = as_dict(json.get("contents")) else {
            SwiftyTubeLog::error(&format!(
                "Explore parse failed: missing contents, top keys={:?}",
                sorted_keys(json)
            ));
            return Vec::new();
        };
        let Some(sections) = BrowseLens::browse_sections(json) else {
            SwiftyTubeLog::error(&format!(
                "Explore parse failed: bad section path, contents keys={:?}",
                sorted_keys(contents)
            ));
            return Vec::new();
        };
        sections
            .into_iter()
            .enumerate()
            .filter_map(|(index, section)| Self::parse_explore_section(section, index))
            .collect()
    }

    fn parse_explore_section(section: &JsonDict, index: usize) -> Option<ExploreSection> {
        if let Some(carousel) = as_dict(section.get("musicCarouselShelfRenderer")) {
            return Self::parse_explore_carousel(carousel, index);
        }

        if let Some(shelf) = as_dict(section.get("musicShelfRenderer")) {
            let title = Self::shelf_title(shelf).unwrap_or_else(|| "Songs".to_string());
            let items = Self::parse_items(&as_dict_list(shelf.get("contents")).unwrap_or_default());
            if items.is_empty() {
                return None;
            }
            return Some(ExploreSection::new(
                title,
                ExploreSectionKind::Rows,
                items,
                Vec::new(),
            ));
        }

        if let Some(immersive) = as_dict(section.get("musicImmersiveCarouselShelfRenderer")) {
            let items =
                Self::parse_items(&as_dict_list(immersive.get("contents")).unwrap_or_default());
            if items.is_empty() {
                return None;
            }
            return Some(ExploreSection::new(
                "Featured",
                ExploreSectionKind::Cards,
                items,
                Vec::new(),
            ));
        }

        if let Some(grid) = as_dict(section.get("gridRenderer")) {
            let items = Self::parse_items(&as_dict_list(grid.get("items")).unwrap_or_default());
            if items.is_empty() {
                return None;
            }
            return Some(ExploreSection::new(
                "More",
                ExploreSectionKind::Cards,
                items,
                Vec::new(),
            ));
        }
        None
    }

    fn parse_explore_carousel(carousel: &JsonDict, index: usize) -> Option<ExploreSection> {
        let basic_header = InnerTubeDecode::dict(
            &["header", "musicCarouselShelfBasicHeaderRenderer"],
            carousel,
        )?;
        let title = InnerTubeJSON::runs_text(as_dict(basic_header.get("title")))?;
        let header_browse_id = Self::explore_header_browse_id(basic_header);
        let raw_items = as_dict_list(carousel.get("contents")).unwrap_or_default();

        // Moods & genres: title + params shortcuts, no playable items.
        if header_browse_id == Some("FEmusic_moods_and_genres") {
            let moods: Vec<MoodItem> = raw_items
                .iter()
                .filter_map(|item| Self::parse_mood(item, index))
                .collect();
            if moods.is_empty() {
                return None;
            }
            return Some(ExploreSection::new(
                title,
                ExploreSectionKind::Moods,
                Vec::new(),
                moods,
            ));
        }

        // Video shelf: videos aren't supported.
        if header_browse_id == Some("FEmusic_new_releases_videos") {
            return None;
        }

        let items = Self::parse_items(&raw_items);
        if items.is_empty() {
            if let Some(first) = raw_items.first() {
                SwiftyTubeLog::debug(&format!(
                    "Explore section[{index}] dropped: {}",
                    Self::describe_unparsed(first)
                ));
            }
            return None;
        }

        // Chart shelves have machine-readable header browseIds;
        // anything song-only renders as rows, mixed content as cards.
        let is_chart =
            header_browse_id.is_some_and(|id| id.starts_with("VLPL") || id.starts_with("VLOLA"));
        let kind = if is_chart || items.iter().all(|i| matches!(i, YTItem::Song(_))) {
            ExploreSectionKind::Rows
        } else {
            ExploreSectionKind::Cards
        };
        Some(ExploreSection::new(title, kind, items, Vec::new()))
    }

    /// Header identity per ytmusic-rs: the title runs' navigation endpoint,
    /// falling back to the more-content button.
    pub fn explore_header_browse_id(basic_header: &JsonDict) -> Option<&str> {
        if let Some(runs) = InnerTubeDecode::array(&["title", "runs"], basic_header)
            && let Some(first) = runs.first()
            && let Some(bid) = InnerTubeDecode::string(
                &["navigationEndpoint", "browseEndpoint", "browseId"],
                first,
            )
        {
            return Some(bid);
        }
        Self::extract_browse_endpoint(basic_header).map(|(browse_id, _)| browse_id)
    }

    fn shelf_title(shelf: &JsonDict) -> Option<String> {
        let basic_header = InnerTubeDecode::dict(&["header", "musicShelfHeaderRenderer"], shelf)?;
        InnerTubeJSON::runs_text(as_dict(basic_header.get("title")))
    }

    /// Best-effort mood shortcut parsing (title + category params).
    fn parse_mood(item: &JsonDict, section_index: usize) -> Option<MoodItem> {
        if let Some(two_row) = as_dict(item.get("musicTwoRowItemRenderer"))
            && let Some(title) = InnerTubeJSON::runs_text(as_dict(two_row.get("title")))
        {
            let params = Self::mood_params(as_dict(two_row.get("navigationEndpoint")));
            return Some(MoodItem::new(title, params, None));
        }
        if let Some(button) = as_dict(item.get("musicNavigationButtonRenderer"))
            && let Some(title) = InnerTubeJSON::runs_text(as_dict(button.get("buttonText")))
        {
            let stripe = as_dict(button.get("solid")).and_then(|s| s.get("leftStripeColor"));
            let color = stripe
                .and_then(|s| s.as_i64().or_else(|| s.as_f64().map(|f| f as i64)))
                .map(|c| (c & 0xFF_FFFF) as u32);
            let params = Self::mood_params(as_dict(button.get("clickCommand")));
            return Some(MoodItem::new(title, params, color));
        }
        SwiftyTubeLog::debug(&format!(
            "Explore section[{section_index}] unparsed mood keys={:?}",
            sorted_keys(item)
        ));
        None
    }

    fn mood_params(endpoint: Option<&JsonDict>) -> Option<String> {
        let browse = as_dict(endpoint?.get("browseEndpoint"))?;
        if as_string(browse.get("browseId")) != Some(Self::MOODS_CATEGORY_BROWSE_ID) {
            return None;
        }
        as_string(browse.get("params")).map(str::to_string)
    }

    /// One-line fingerprint of an unparseable item for log mapping.
    fn describe_unparsed(item: &JsonDict) -> String {
        let Some(inner) = as_dict(item.get("musicTwoRowItemRenderer"))
            .or_else(|| as_dict(item.get("musicResponsiveListItemRenderer")))
        else {
            return format!("keys={:?}", sorted_keys(item));
        };
        format!(
            "renderer={:?} pageType={} watch={} browse={}",
            sorted_keys(item),
            Self::extract_page_type(inner).unwrap_or("none"),
            InnerTubeDecode::string(&["navigationEndpoint", "watchEndpoint", "videoId"], inner)
                .unwrap_or("none"),
            InnerTubeDecode::string(&["navigationEndpoint", "browseEndpoint", "browseId"], inner)
                .unwrap_or("none"),
        )
    }

    // MARK: - Extract Helpers

    pub fn extract_page_type(renderer: &JsonDict) -> Option<&str> {
        InnerTubeDecode::string(
            &[
                "navigationEndpoint",
                "browseEndpoint",
                "browseEndpointContextSupportedConfigs",
                "browseEndpointContextMusicConfig",
                "pageType",
            ],
            renderer,
        )
    }

    pub fn has_watch_endpoint(renderer: &JsonDict) -> bool {
        as_dict(renderer.get("navigationEndpoint"))
            .and_then(|nav| nav.get("watchEndpoint"))
            .is_some_and(|w| !w.is_null())
    }

    /// `(browseId, params)` of a header's "More" button.
    pub fn extract_browse_endpoint(header: &JsonDict) -> Option<(&str, Option<&str>)> {
        let browse = InnerTubeDecode::dict(
            &[
                "moreContentButton",
                "buttonRenderer",
                "navigationEndpoint",
                "browseEndpoint",
            ],
            header,
        )?;
        Some((
            as_string(browse.get("browseId"))?,
            as_string(browse.get("params")),
        ))
    }

    pub fn extract_continuation(section_list: &JsonDict) -> Option<String> {
        InnerTubeDecode::continuation_token(Some(section_list)).map(str::to_string)
    }
}
