use serde_json::Value;
use swiftytube::formatting::duration_format::DurationFormat;
use swiftytube::networking::inner_tube_decode::{
    InnerTubeDecode, JsonDict, as_dict, as_dict_list, as_string,
};
use swiftytube::networking::inner_tube_json::InnerTubeJSON;

use crate::networking::models::home_page::next_id;
use crate::networking::models::yt_item::{
    AlbumItem, ArtistItem, PlaylistItem, PodcastItem, SongItem, YTItem,
};

#[derive(Clone, Debug, PartialEq)]
pub struct SearchSection {
    /// Stands in for Swift's `UUID()`: unique per instance.
    pub id: u64,
    pub title: String,
    pub items: Vec<YTItem>,
    /// The top result card's subtitle after its kind word, such as
    /// "12.3M monthly audience" for an artist.
    pub subtitle: Option<String>,
}

impl SearchSection {
    pub fn new(title: impl Into<String>, items: Vec<YTItem>, subtitle: Option<String>) -> Self {
        Self {
            id: next_id(),
            title: title.into(),
            items,
            subtitle,
        }
    }
}

/// Shelves the app can't play or open; their results are dropped.
const UNSUPPORTED_SECTIONS: &[&str] = &["Videos", "Episodes"];

const SECTION_ORDER: &[&str] = &[
    "Top result",
    "Songs",
    "Albums",
    "Artists",
    "Playlists",
    "Podcasts",
    "Profiles",
];

fn responsive_items(contents: Option<&Value>) -> Vec<YTItem> {
    as_dict_list(contents)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|dict| as_dict(dict.get("musicResponsiveListItemRenderer")))
        .filter_map(YTItem::from_responsive_list_item)
        .collect()
}

pub struct SearchParser;

impl SearchParser {
    pub fn parse_search_results(json: &JsonDict) -> Vec<SearchSection> {
        let Some(contents) = as_dict(json.get("contents")) else {
            return Vec::new();
        };

        let tabbed = InnerTubeDecode::array(&["tabbedSearchResultsRenderer", "tabs"], contents)
            .and_then(|tabs| tabs.first().copied())
            .and_then(|tab| {
                InnerTubeDecode::dict(&["tabRenderer", "content", "sectionListRenderer"], tab)
            });
        let section_list = match tabbed {
            Some(tabbed) => as_dict_list(tabbed.get("contents")),
            None => as_dict(contents.get("sectionListRenderer"))
                .and_then(|slr| as_dict_list(slr.get("contents"))),
        };
        let Some(section_list) = section_list else {
            return Vec::new();
        };

        let mut all_sections = Vec::new();
        for section in section_list {
            // MARK: Top result
            if let Some(card_shelf) = as_dict(section.get("musicCardShelfRenderer")) {
                let basic_header = InnerTubeDecode::dict(
                    &["header", "musicCardShelfHeaderBasicRenderer"],
                    card_shelf,
                );
                let title = Self::runs_text(basic_header.and_then(|h| h.get("title")))
                    .unwrap_or("Top result");
                let mut items: Vec<YTItem> = Vec::new();
                items.extend(Self::parse_top_result(card_shelf));
                for item in responsive_items(card_shelf.get("contents")) {
                    if items.first().is_none_or(|first| first.id() != item.id()) {
                        items.push(item);
                    }
                }
                if !items.is_empty() {
                    all_sections.push(SearchSection::new(
                        title,
                        items,
                        Self::top_result_detail(card_shelf),
                    ));
                }
            }

            // MARK: Shelf
            if let Some(shelf) = as_dict(section.get("musicShelfRenderer")) {
                let items = responsive_items(shelf.get("contents"));
                if items.is_empty() {
                    continue;
                }
                match Self::runs_text(shelf.get("title")) {
                    Some(title) => all_sections.push(SearchSection::new(title, items, None)),
                    None => all_sections.extend(Self::group_items_by_type(items)),
                }
            }

            // MARK: Fallback section
            if let Some(item_section) = as_dict(section.get("itemSectionRenderer")) {
                let items = responsive_items(item_section.get("contents"));
                if !items.is_empty() {
                    all_sections.extend(Self::group_items_by_type(items));
                }
            }
        }

        // Swift groups through a Dictionary (unordered) and then sorts by the
        // fixed order; titles outside it keep first-seen order here.
        let mut grouped: Vec<(String, Vec<SearchSection>)> = Vec::new();
        for section in all_sections
            .into_iter()
            .filter(|s| !UNSUPPORTED_SECTIONS.contains(&s.title.as_str()))
        {
            match grouped
                .iter_mut()
                .find(|(title, _)| *title == section.title)
            {
                Some((_, group)) => group.push(section),
                None => grouped.push((section.title.clone(), vec![section])),
            }
        }
        let mut merged: Vec<SearchSection> = grouped
            .into_iter()
            .map(|(title, group)| {
                let subtitle = group[0].subtitle.clone();
                let items = group.into_iter().flat_map(|s| s.items).collect();
                SearchSection::new(title, items, subtitle)
            })
            .collect();
        // Stable sort: ties keep first-seen order.
        merged.sort_by_key(|s| {
            SECTION_ORDER
                .iter()
                .position(|t| *t == s.title)
                .unwrap_or(999)
        });
        merged
    }

    // MARK: - Private

    fn group_items_by_type(items: Vec<YTItem>) -> Vec<SearchSection> {
        const NAMES: [&str; 5] = ["Songs", "Albums", "Artists", "Playlists", "Podcasts"];
        let mut groups: Vec<Vec<YTItem>> = vec![Vec::new(); NAMES.len()];
        for item in items {
            let key = match &item {
                YTItem::Song(s) if s.is_music_video() => continue, // "Videos"
                YTItem::Song(_) => 0,
                YTItem::Album(_) => 1,
                YTItem::Artist(_) => 2,
                YTItem::Playlist(_) => 3,
                YTItem::Podcast(_) => 4,
                YTItem::Episode(_) => continue, // "Episodes"
            };
            groups[key].push(item);
        }
        NAMES
            .iter()
            .zip(groups)
            .filter(|(_, items)| !items.is_empty())
            .map(|(name, items)| SearchSection::new(*name, items, None))
            .collect()
    }

    fn parse_top_result(card_shelf: &JsonDict) -> Option<YTItem> {
        let nav = as_dict(card_shelf.get("onTap"))
            .or_else(|| as_dict(card_shelf.get("navigationEndpoint")));
        let title = Self::runs_text(card_shelf.get("title"))
            .unwrap_or("Unknown")
            .to_string();
        let thumbnail_url = InnerTubeJSON::music_thumbnail_url(card_shelf);

        let watch = nav.and_then(|n| as_dict(n.get("watchEndpoint")));
        if let Some(video_id) = as_string(watch.and_then(|w| w.get("videoId"))) {
            // Subtitle runs: "Song"/"Video" • artists • album • stats • duration.
            let song = SongItem::from_two_row_item(
                card_shelf,
                video_id.to_string(),
                DurationFormat::parse_renderer_duration(card_shelf),
                as_string(watch.and_then(|w| w.get("playlistId"))).map(str::to_string),
            );
            return Some(YTItem::Song(SongItem {
                title,
                thumbnail_url,
                ..song
            }));
        }

        let browse = nav.and_then(|n| as_dict(n.get("browseEndpoint")))?;
        let browse_id = as_string(browse.get("browseId"))?.to_string();
        let page_type = InnerTubeDecode::string(
            &[
                "browseEndpointContextSupportedConfigs",
                "browseEndpointContextMusicConfig",
                "pageType",
            ],
            browse,
        );

        let album = |browse_id: String, title: String, thumbnail_url: Option<String>| {
            YTItem::Album(AlbumItem {
                browse_id,
                title,
                thumbnail_url,
                ..AlbumItem::default()
            })
        };
        let artist = |browse_id: String, name: String, thumbnail_url: Option<String>| {
            YTItem::Artist(ArtistItem {
                browse_id,
                name,
                thumbnail_url,
                is_subscribed: false,
            })
        };
        let playlist = |browse_id: String, title: String, thumbnail_url: Option<String>| {
            YTItem::Playlist(PlaylistItem {
                id: browse_id.replace("VL", ""),
                title,
                thumbnail_url,
                ..PlaylistItem::default()
            })
        };

        match page_type {
            Some("MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK") => {
                Some(album(browse_id, title, thumbnail_url))
            }
            Some("MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL") => {
                Some(artist(browse_id, title, thumbnail_url))
            }
            Some("MUSIC_PAGE_TYPE_PLAYLIST") => Some(playlist(browse_id, title, thumbnail_url)),
            Some("MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE") => {
                Some(YTItem::Podcast(PodcastItem {
                    browse_id,
                    title,
                    author: None,
                    thumbnail_url,
                }))
            }
            _ if browse_id.starts_with("VL") => Some(playlist(browse_id, title, thumbnail_url)),
            _ if browse_id.starts_with("UC") || browse_id.starts_with("MPCH") => {
                Some(artist(browse_id, title, thumbnail_url))
            }
            _ if browse_id.starts_with("MPREb_") => Some(album(browse_id, title, thumbnail_url)),
            _ => None,
        }
    }

    /// The card's subtitle runs ("Artist", " • ", "12.3M monthly audience")
    /// without the leading kind word and the separators.
    fn top_result_detail(card_shelf: &JsonDict) -> Option<String> {
        let runs = InnerTubeDecode::array(&["subtitle", "runs"], card_shelf).unwrap_or_default();
        let details: Vec<&str> = runs
            .iter()
            .skip(1)
            .filter_map(|r| as_string(r.get("text")))
            .filter(|t| t.trim() != "•")
            .collect();
        (!details.is_empty()).then(|| details.join(" • "))
    }

    /// First run's text, not the joined runs.
    fn runs_text(value: Option<&Value>) -> Option<&str> {
        let runs = as_dict_list(as_dict(value)?.get("runs"))?;
        as_string(runs.first()?.get("text"))
    }
}
