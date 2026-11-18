// Data half of Trop/Explore/ExploreViewModel.swift: the explore browse,
// regional trending swap, new-release details and mood pages. SwiftUI state
// (phase, error) and `MoodGenreIndex` (listening personalization, R5) stay out.
//
// Methods take `&mut self`, so two loads never interleave and the Swift
// generation guards against stale answers are not needed.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use swiftytube::core::swiftytube_log::SwiftyTubeLog;
use swiftytube::networking::inner_tube_client::{BrowseArgs, InnerTubeClient};
use swiftytube::networking::inner_tube_decode::{
    InnerTubeDecode, JsonDict, as_dict, as_dict_list, as_string,
};
use swiftytube::networking::inner_tube_error::InnerTubeError;
use swiftytube::networking::inner_tube_json::InnerTubeJSON;

use super::mood_card::MoodFamily;
use crate::networking::models::home_page::{ExploreSection, ExploreSectionKind, MoodItem};
use crate::networking::models::home_page_parser::HomePageParser;
use crate::networking::models::yt_item::{SongItem, YTItem};
use swiftytube::sync::browse_lens::BrowseLens;
use swiftytube::sync::playlist_detail_service::PlaylistDetailService;
use swiftytube::sync::regional_charts::RegionalCharts;

/// The "New albums & singles" shelf, which Explore draws as its own section.
#[derive(Clone, Debug, PartialEq)]
pub struct NewReleases {
    pub section_id: u64,
    /// Release kind per album browseId ("Single", "Album", "EP").
    pub kinds: HashMap<String, String>,
}

pub struct ExploreViewModel {
    pub inner_tube: Arc<InnerTubeClient>,

    /// `SettingsStore.contentCountry`.
    pub region: String,

    pub sections: Vec<ExploreSection>,

    /// `None` when Explore didn't send the new releases shelf.
    pub new_releases: Option<NewReleases>,

    /// The chosen family chip (Pop): its genres show as cards under the chips.
    pub selected_family: Option<String>,

    /// The chosen mood: its page replaces the feed.
    pub selected_mood: Option<MoodItem>,

    /// The chosen mood's sections, `None` while they load.
    pub mood_sections: Option<Vec<ExploreSection>>,
}

impl ExploreViewModel {
    /// Swift default `region`: "US".
    pub fn new(inner_tube: Arc<InnerTubeClient>, region: impl Into<String>) -> Self {
        Self {
            inner_tube,
            region: region.into(),
            sections: Vec::new(),
            new_releases: None,
            selected_family: None,
            selected_mood: None,
            mood_sections: None,
        }
    }

    /// Explore's moods & genres, shown as the tab's filter chips.
    pub fn moods(&self) -> &[MoodItem] {
        self.sections
            .iter()
            .find(|s| s.kind == ExploreSectionKind::Moods)
            .map_or(&[], |s| &s.moods)
    }

    // MARK: - Loading

    /// Swift `fetch()`. Fails on network failure.
    pub async fn load(&mut self) -> Result<(), InnerTubeError> {
        let json = self
            .inner_tube
            .browse(BrowseArgs {
                form_data: json!({ "selectedValues": [self.region] })
                    .as_object()
                    .cloned(),
                ..BrowseArgs::id(HomePageParser::EXPLORE_BROWSE_ID)
            })
            .await?;
        self.sections = HomePageParser::parse_explore_sections(&json);
        self.new_releases = Self::apply_new_release_details(&json, &mut self.sections);
        self.apply_regional_trending().await;
        if let Some(chosen) = self.selected_mood.take() {
            self.selected_mood = self
                .moods()
                .iter()
                .find(|m| m.title == chosen.title)
                .cloned();
            if self.selected_mood.is_none() {
                self.mood_sections = None;
            }
        }
        if let Some(family) = &self.selected_family
            && !self
                .moods()
                .iter()
                .any(|m| MoodFamily::family(&m.title) == Some(family.as_str()))
        {
            self.selected_family = None;
        }
        Ok(())
    }

    /// Swaps the Trending shelf for the region's chart tracks. The explore
    /// carousel follows the IP/account region, which the region setting
    /// cannot move — but per-country charts can, via the country selector.
    /// Falls back to explore's own items on any failure.
    async fn apply_regional_trending(&mut self) {
        let Some(index) = self
            .sections
            .iter()
            .position(|s| s.title.to_lowercase().contains("trending"))
        else {
            return;
        };
        let rows = match self.regional_trending_rows().await {
            Ok(rows) => rows,
            Err(error) => {
                SwiftyTubeLog::notice(&format!("Explore regional trending failed: {error}"));
                return;
            }
        };
        let songs: Vec<YTItem> = rows
            .iter()
            .filter_map(|row| as_dict(row.get("musicResponsiveListItemRenderer")))
            .filter_map(SongItem::from)
            .map(YTItem::Song)
            .collect();
        if songs.is_empty() {
            return;
        }
        self.sections[index].items = songs;
        self.sections[index].kind = ExploreSectionKind::Rows;
    }

    async fn regional_trending_rows(&self) -> Result<Vec<JsonDict>, InnerTubeError> {
        let playlist_id =
            RegionalCharts::trending_playlist_id(&self.region, &self.inner_tube).await?;
        let result = PlaylistDetailService::new(&self.inner_tube)
            .fetch_playlist_rows(&playlist_id, 100, false)
            .await?;
        Ok(result.rows)
    }

    // MARK: - New releases

    /// Finds the new releases shelf (header browseId `FEmusic_new_releases_albums`)
    /// and reads each card's kind from its subtitle ("Single • Artist",
    /// "Album • Artist • 2026"). Artists and year come from `AlbumItem::from`.
    pub fn apply_new_release_details(
        json: &JsonDict,
        sections: &mut [ExploreSection],
    ) -> Option<NewReleases> {
        let raw = BrowseLens::browse_sections(json)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| as_dict(s.get("musicCarouselShelfRenderer")))
            .find(|c| Self::header_browse_id(c) == Some("FEmusic_new_releases_albums"))?;
        let title = InnerTubeJSON::runs_text(InnerTubeDecode::dict(
            &["header", "musicCarouselShelfBasicHeaderRenderer", "title"],
            raw,
        ))?;
        let index = sections
            .iter()
            .position(|s| s.kind == ExploreSectionKind::Cards && s.title == title)?;

        let mut kinds = HashMap::new();
        for content in as_dict_list(raw.get("contents")).unwrap_or_default() {
            let Some(two_row) = as_dict(content.get("musicTwoRowItemRenderer")) else {
                continue;
            };
            let browse_id = InnerTubeDecode::string(
                &["navigationEndpoint", "browseEndpoint", "browseId"],
                two_row,
            );
            let first_run = InnerTubeDecode::array(&["subtitle", "runs"], two_row)
                .and_then(|runs| runs.first().copied());
            // A first run without a link is the kind, not an artist.
            if let (Some(browse_id), Some(first_run)) = (browse_id, first_run)
                && first_run
                    .get("navigationEndpoint")
                    .is_none_or(|v| v.is_null())
                && let Some(kind) = as_string(first_run.get("text"))
            {
                kinds.insert(browse_id.to_string(), kind.trim().to_string());
            }
        }

        Some(NewReleases {
            section_id: sections[index].id,
            kinds,
        })
    }

    /// The shelf header's browseId: the title link, else the "More" button.
    fn header_browse_id(carousel: &JsonDict) -> Option<&str> {
        let header = InnerTubeDecode::dict(
            &["header", "musicCarouselShelfBasicHeaderRenderer"],
            carousel,
        )?;
        HomePageParser::explore_header_browse_id(header)
    }

    // MARK: - Moods

    /// Picks a family chip: the family's own genre ("Pop") fills the feed, or
    /// the normal feed stays when YouTube lists none. `None` clears everything.
    pub async fn select_family(&mut self, family: Option<String>) {
        self.selected_family.clone_from(&family);
        let Some(family) = family else {
            return self.select(None).await;
        };
        let members: Vec<MoodItem> = self
            .moods()
            .iter()
            .filter(|m| MoodFamily::family(&m.title) == Some(family.as_str()))
            .cloned()
            .collect();
        let base = MoodFamily::base(&family, &members).cloned();
        self.select(base).await;
    }

    /// Picks a plain genre chip, or a sub-genre card of the chosen family.
    pub async fn select_genre(&mut self, mood: MoodItem) {
        if MoodFamily::family(&mood.title) != self.selected_family.as_deref() {
            self.selected_family = None;
        }
        self.select(Some(mood)).await;
    }

    /// Shows a mood's page, from the cache at once when it has one, and
    /// refreshes it when the cached copy is old. `None` goes back to the feed.
    pub async fn select(&mut self, mood: Option<MoodItem>) {
        self.selected_mood.clone_from(&mood);
        let Some(mood) = mood else {
            self.mood_sections = None;
            return;
        };
        self.mood_sections = MoodDetailCache::sections(&mood).map(Self::mood_order);
        if MoodDetailCache::is_fresh(&mood) {
            return;
        }
        let loaded = Self::load_mood_detail(&self.inner_tube, &mood).await;
        // A failed request returns nothing: keep the cached page.
        if loaded.is_empty() {
            self.mood_sections.get_or_insert_with(Vec::new);
            return;
        }
        MoodDetailCache::store(loaded.clone(), &mood);
        self.mood_sections = Some(Self::mood_order(loaded));
    }

    /// New albums & singles first, then song shelves (the mood's trending),
    /// then the rest. By content, not title: YouTube localises titles.
    pub fn mood_order(mut sections: Vec<ExploreSection>) -> Vec<ExploreSection> {
        // Stable sort: ties keep their order.
        sections.sort_by_key(|section| {
            if section.kind == ExploreSectionKind::Cards
                && section.items.iter().any(|i| matches!(i, YTItem::Album(_)))
            {
                0
            } else if section.kind == ExploreSectionKind::Rows {
                1
            } else {
                2
            }
        });
        sections
    }

    /// Full category page (songs, playlists, videos, albums shelves). Empty on failure.
    pub async fn load_mood_detail(
        inner_tube: &InnerTubeClient,
        mood: &MoodItem,
    ) -> Vec<ExploreSection> {
        let result = inner_tube
            .browse(BrowseArgs {
                params: mood.params.clone(),
                ..BrowseArgs::id(HomePageParser::MOODS_CATEGORY_BROWSE_ID)
            })
            .await;
        match result {
            Ok(json) => HomePageParser::parse_explore_sections(&json),
            Err(error) => {
                SwiftyTubeLog::notice(&format!("Explore mood '{}' failed: {error}", mood.title));
                Vec::new()
            }
        }
    }
}

type MoodCacheEntry = (Vec<ExploreSection>, Instant);

static MOOD_CACHE: LazyLock<Mutex<HashMap<String, MoodCacheEntry>>> = LazyLock::new(Mutex::default);

/// Mood pages kept in memory, so reopening one shows it at once instead of reloading.
pub struct MoodDetailCache;

impl MoodDetailCache {
    /// A cached page older than this is refreshed quietly in the background.
    pub const MAX_AGE: Duration = Duration::from_secs(30 * 60);

    fn key(mood: &MoodItem) -> String {
        mood.params.clone().unwrap_or_else(|| mood.title.clone())
    }

    fn cache() -> std::sync::MutexGuard<'static, HashMap<String, MoodCacheEntry>> {
        MOOD_CACHE.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn sections(mood: &MoodItem) -> Option<Vec<ExploreSection>> {
        Self::cache().get(&Self::key(mood)).map(|(s, _)| s.clone())
    }

    pub fn is_fresh(mood: &MoodItem) -> bool {
        Self::cache()
            .get(&Self::key(mood))
            .is_some_and(|(_, date)| date.elapsed() < Self::MAX_AGE)
    }

    pub fn store(sections: Vec<ExploreSection>, mood: &MoodItem) {
        Self::cache().insert(Self::key(mood), (sections, Instant::now()));
    }
}
