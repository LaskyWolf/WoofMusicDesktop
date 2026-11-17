// SwiftyTube/Sync/RegionalCharts.swift.

use crate::networking::inner_tube_client::{BrowseArgs, InnerTubeClient};
use crate::networking::inner_tube_decode::{InnerTubeDecode, JsonDict, as_dict, as_dict_list};
use crate::networking::inner_tube_error::InnerTubeError;
use crate::networking::inner_tube_json::InnerTubeJSON;
use serde_json::json;

/// One playlist entry of the charts page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChartEntry {
    pub id: String,
    pub title: String,
}

/// Country-scoped charts lookups (`FEmusic_charts`).
pub struct RegionalCharts;

impl RegionalCharts {
    /// Playlist id behind the region's Trending shelf: prefers a "Trending N
    /// `Country`" playlist, then the daily chart, then the first playlist
    /// entry found.
    pub async fn trending_playlist_id(
        country: &str,
        client: &InnerTubeClient,
    ) -> Result<String, InnerTubeError> {
        let json = client
            .browse(BrowseArgs {
                form_data: json!({ "selectedValues": [country] }).as_object().cloned(),
                ..BrowseArgs::id("FEmusic_charts")
            })
            .await?;
        Self::select_trending(&Self::playlist_entries(&json)).ok_or(InnerTubeError::InvalidResponse)
    }

    /// Pure selection step, factored out for tests.
    pub fn select_trending(entries: &[ChartEntry]) -> Option<String> {
        for word in ["trending", "daily top"] {
            if let Some(e) = entries
                .iter()
                .find(|e| e.title.to_lowercase().contains(word))
            {
                return Some(e.id.clone());
            }
        }
        entries.first().map(|e| e.id.clone())
    }

    /// `(browseId, title)` of every playlist entry across charts carousels.
    pub fn playlist_entries(json: &JsonDict) -> Vec<ChartEntry> {
        let Some(tabs) = InnerTubeDecode::array(
            &["contents", "singleColumnBrowseResultsRenderer", "tabs"],
            json,
        ) else {
            return Vec::new();
        };
        let Some(first) = tabs.first() else {
            return Vec::new();
        };
        let sections = InnerTubeDecode::array(
            &["tabRenderer", "content", "sectionListRenderer", "contents"],
            first,
        );
        let mut out = Vec::new();
        for section in sections.unwrap_or_default() {
            let inner = as_dict(section.get("itemSectionRenderer"))
                .and_then(|r| as_dict_list(r.get("contents")));
            let unwrapped = inner.and_then(|i| i.first().copied()).unwrap_or(section);
            let items =
                InnerTubeDecode::array(&["musicCarouselShelfRenderer", "contents"], unwrapped);
            for item in items.unwrap_or_default() {
                if let Some(two_row) = as_dict(item.get("musicTwoRowItemRenderer")) {
                    if let (Some(id), Some(title)) = (
                        Self::playlist_id(two_row),
                        InnerTubeJSON::runs_text(as_dict(two_row.get("title"))),
                    ) {
                        out.push(ChartEntry { id, title });
                    }
                } else if let Some(responsive) =
                    as_dict(item.get("musicResponsiveListItemRenderer"))
                    && let (Some(id), Some(title)) = (
                        Self::playlist_id(responsive),
                        Self::first_flex_text(responsive),
                    )
                {
                    out.push(ChartEntry { id, title });
                }
            }
        }
        out
    }

    fn playlist_id(renderer: &JsonDict) -> Option<String> {
        let nav = as_dict(renderer.get("navigationEndpoint"))?;
        if let Some(bid) = InnerTubeDecode::string(&["browseEndpoint", "browseId"], nav)
            && bid.starts_with("VL")
        {
            return Some(bid.to_string());
        }
        InnerTubeDecode::string(&["watchEndpoint", "playlistId"], nav)
            .filter(|pid| pid.starts_with("VL") || pid.starts_with("PL"))
            .map(str::to_string)
    }

    fn first_flex_text(renderer: &JsonDict) -> Option<String> {
        let flex = as_dict_list(renderer.get("flexColumns"))?;
        InnerTubeJSON::runs_text(InnerTubeDecode::dict(
            &["musicResponsiveListItemFlexColumnRenderer", "text"],
            flex.first()?,
        ))
    }
}
