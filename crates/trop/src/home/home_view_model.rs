// Data half of Trop/Home/HomeViewModel.swift: the browse calls, chips,
// continuations and section ordering. SwiftUI state (loading flags, error,
// notification observers), the personalization sections (Quick Picks from
// history, Keep Listening, Daily Discover...) and `SavedHome` (the last page
// cached on disk) land with the library port (R5). Until then `local_sections`
// and `phase2_sections` stay empty.
//
// Methods take `&mut self`, so two loads never interleave: the Swift
// generation guards and `isLoadingMore` are not needed. Every method
// returns when the state below is updated; the UI reads `home_sections`.

use std::sync::Arc;

use swiftytube::core::swiftytube_log::SwiftyTubeLog;
use swiftytube::networking::inner_tube_client::{BrowseArgs, InnerTubeClient};
use swiftytube::networking::inner_tube_error::InnerTubeError;

use crate::networking::models::home_page::{HomePage, HomePageChip, HomePageSection, HomeSection};
use crate::networking::models::home_page_parser::HomePageParser;
use crate::networking::models::yt_item::YTItem;

/// Lowercased fragments of YouTube shelf titles Home doesn't show.
/// "daily discover" only matches YouTube's shelf: the app's own
/// Daily Discover is a `HomeSection::DailyDiscover`, not a server shelf.
const HIDDEN_SERVER_SHELF_FRAGMENTS: &[&str] = &[
    "music video",
    "from your library",
    "your playlist",
    "daily discover",
    "shorts",
    "new release",
];

pub struct HomeViewModel {
    pub inner_tube: Arc<InnerTubeClient>,

    /// `SettingsStore.showQuickPicks` / `hideExplicit`.
    pub show_quick_picks: bool,
    pub hide_explicit: bool,

    pub home_page: Option<HomePage>,
    pub selected_chip: Option<HomePageChip>,

    /// Sections built from the local library (R5).
    pub local_sections: Vec<HomeSection>,

    /// Account-only sections built after sign-in (R5).
    pub phase2_sections: Vec<HomeSection>,

    previous_home_page: Option<HomePage>,
}

impl HomeViewModel {
    /// Swift defaults: `show_quick_picks: true`, `hide_explicit: false`.
    pub fn new(inner_tube: Arc<InnerTubeClient>) -> Self {
        Self {
            inner_tube,
            show_quick_picks: true,
            hide_explicit: false,
            home_page: None,
            selected_chip: None,
            local_sections: Vec::new(),
            phase2_sections: Vec::new(),
            previous_home_page: None,
        }
    }

    // MARK: - Home Data Loading

    /// First page of `FEmusic_home`, then up to three continuations until a
    /// Quick Picks shelf shows up. Fails on network or parse failure.
    pub async fn load(&mut self) -> Result<(), InnerTubeError> {
        self.fetch_home_page().await?;
        self.ensure_quick_picks_available().await
    }

    /// Browses `FEmusic_home`. While a mood chip is selected the page is kept
    /// for when the chip closes.
    pub async fn fetch_home_page(&mut self) -> Result<(), InnerTubeError> {
        let json = self
            .inner_tube
            .browse(BrowseArgs::id("FEmusic_home"))
            .await?;
        let page = HomePageParser::parse_home_page(&json).ok_or(InnerTubeError::DecodingFailed)?;
        // A mood chip's feed stays on screen. This page comes back when it closes.
        if self.selected_chip.is_none() {
            self.home_page = Some(page);
        } else {
            self.previous_home_page = Some(page);
        }
        Ok(())
    }

    pub async fn ensure_quick_picks_available(&mut self) -> Result<(), InnerTubeError> {
        if !self.show_quick_picks {
            return Ok(());
        }
        for _ in 0..3 {
            if self.has_quick_picks_server_section() {
                break;
            }
            let Some(continuation) = self.home_page.as_ref().and_then(|p| p.continuation.clone())
            else {
                break;
            };
            if !self.append_continuation(continuation).await? {
                break;
            }
        }
        Ok(())
    }

    fn has_quick_picks_server_section(&self) -> bool {
        self.home_page.as_ref().is_some_and(|page| {
            page.sections
                .iter()
                .any(|s| matches!(self.map_server_section(s, 0), HomeSection::QuickPicks(_)))
        })
    }

    /// Swift `loadMoreIfNeeded(currentIndex:total:)`: call it when the list is
    /// within three sections of its end. False when nothing was loaded.
    pub async fn load_more_if_needed(&mut self, current_index: usize, total: usize) -> bool {
        let Some(continuation) = self.home_page.as_ref().and_then(|p| p.continuation.clone())
        else {
            return false;
        };
        if total.saturating_sub(current_index) > 3 {
            return false;
        }
        match self.append_continuation(continuation).await {
            Ok(loaded) => loaded,
            Err(error) => {
                SwiftyTubeLog::notice(&format!("Continuation error: {error}"));
                false
            }
        }
    }

    async fn append_continuation(&mut self, continuation: String) -> Result<bool, InnerTubeError> {
        let json = self
            .inner_tube
            .browse(BrowseArgs {
                continuation: Some(continuation),
                ..BrowseArgs::default()
            })
            .await?;
        let Some((sections, continuation)) = HomePageParser::parse_continuation_sections(&json)
        else {
            return Ok(false);
        };
        if let Some(page) = &mut self.home_page {
            page.sections.extend(sections);
            page.continuation = continuation;
        }
        Ok(true)
    }

    /// Selects a mood chip (its filtered feed replaces the sections), or
    /// closes it when `chip` is `None` or already selected. A failed request
    /// restores the unfiltered page.
    pub async fn toggle_chip(&mut self, chip: Option<HomePageChip>) {
        let chip = match chip {
            Some(chip) if self.selected_chip.as_ref().map(|c| &c.title) != Some(&chip.title) => {
                chip
            }
            _ => {
                self.close_chip();
                return;
            }
        };
        if self.selected_chip.is_none() {
            self.previous_home_page = self.home_page.clone();
        }
        let params = chip.params.clone();
        self.selected_chip = Some(chip);
        let result = self
            .inner_tube
            .browse(BrowseArgs {
                params,
                ..BrowseArgs::id("FEmusic_home")
            })
            .await;
        match result {
            Ok(json) => {
                if let Some(page) = HomePageParser::parse_home_page(&json)
                    && let Some(home_page) = &mut self.home_page
                {
                    home_page.sections = page.sections;
                    // The filtered feed carries its own continuation; reusing
                    // the unfiltered one would page wrong sections in below.
                    home_page.continuation = page.continuation;
                }
            }
            Err(_) => self.close_chip(),
        }
    }

    fn close_chip(&mut self) {
        self.home_page = self.previous_home_page.take();
        self.selected_chip = None;
    }

    // MARK: - Section Management

    /// Swift `mergeSections()`: what Home shows, in order.
    pub fn home_sections(&self) -> Vec<HomeSection> {
        let server_sections = self
            .home_page
            .iter()
            .flat_map(|p| p.sections.iter().enumerate())
            .map(|(i, s)| self.map_server_section(s, i));
        // A mood chip shows only YouTube's filtered feed; mixing the local
        // sections back in pushed the filtered results below the fold.
        let all: Vec<HomeSection> = if self.selected_chip.is_some() {
            server_sections.collect()
        } else {
            self.local_sections
                .iter()
                .cloned()
                .chain(server_sections)
                .chain(self.phase2_sections.iter().cloned())
                .collect()
        };
        self.order_sections(all)
    }

    fn map_server_section(&self, section: &HomePageSection, index: usize) -> HomeSection {
        let title = section.title.to_lowercase();
        if title == "quick picks" || title.contains("quick pick") {
            return HomeSection::QuickPicks(if self.show_quick_picks {
                section.items.clone()
            } else {
                Vec::new()
            });
        }
        if title.contains("listen again") || title.contains("keep listening") {
            return HomeSection::KeepListening(section.items.clone());
        }
        if title.contains("forgotten") || title.contains("favorite") {
            return HomeSection::ForgottenFavorites(section.items.clone());
        }
        HomeSection::HomePage(section.clone(), index)
    }

    fn order_sections(&self, sections: Vec<HomeSection>) -> Vec<HomeSection> {
        let mut filtered = Vec::new();
        for section in sections {
            // Videos aren't supported, so YouTube's video shelves are dropped,
            // along with shelves that repeat Library, Explore or local sections.
            if let HomeSection::HomePage(page_section, _) = &section
                && Self::is_hidden_server_shelf(&page_section.title)
            {
                continue;
            }
            let f = self.apply_filters(section);
            // Kept while empty: pinned songs fill it. Off hides pinned songs too.
            if matches!(f, HomeSection::QuickPicks(_)) {
                if self.show_quick_picks {
                    filtered.push(f);
                }
                continue;
            }
            if !f.items().is_empty() {
                filtered.push(f);
            }
        }

        // Keeps first-seen order; a later section with the same id and more
        // items replaces the earlier one in place.
        let mut by_id: Vec<(String, HomeSection)> = Vec::new();
        for section in filtered {
            let id = section.id();
            match by_id.iter_mut().find(|(existing, _)| *existing == id) {
                Some((_, existing)) => {
                    if section.items().len() > existing.items().len() {
                        *existing = section;
                    }
                }
                None => by_id.push((id, section)),
            }
        }
        let mut ordered: Vec<HomeSection> = by_id.into_iter().map(|(_, s)| s).collect();
        // Stable sort: ties on weight keep first-seen order.
        ordered.sort_by_key(|s| std::cmp::Reverse(Self::section_weight(s)));
        ordered
    }

    pub fn is_hidden_server_shelf(title: &str) -> bool {
        let lower = title.to_lowercase();
        HIDDEN_SERVER_SHELF_FRAGMENTS
            .iter()
            .any(|fragment| lower.contains(fragment))
    }

    fn apply_filters(&self, section: HomeSection) -> HomeSection {
        let hide_explicit = self.hide_explicit;
        let keep = |item: &YTItem| {
            !hide_explicit
                || match item {
                    YTItem::Song(s) => !s.is_explicit,
                    YTItem::Album(a) => !a.is_explicit,
                    _ => true,
                }
        };
        let filter = |items: Vec<YTItem>| items.into_iter().filter(|i| keep(i)).collect();
        match section {
            HomeSection::QuickPicks(items) => HomeSection::QuickPicks(filter(items)),
            HomeSection::KeepListening(items) => HomeSection::KeepListening(filter(items)),
            HomeSection::ForgottenFavorites(items) => {
                HomeSection::ForgottenFavorites(filter(items))
            }
            HomeSection::HomePage(page_section, index) => HomeSection::HomePage(
                HomePageSection {
                    title: page_section.title,
                    items: filter(page_section.items),
                },
                index,
            ),
            HomeSection::SimilarRecommendation(items, title) => {
                HomeSection::SimilarRecommendation(filter(items), title)
            }
            HomeSection::DailyDiscover(items) => HomeSection::DailyDiscover(filter(items)),
            HomeSection::FromTheCommunity(items) => HomeSection::FromTheCommunity(filter(items)),
        }
    }

    fn section_weight(section: &HomeSection) -> i64 {
        match section {
            // Keep Listening is a small strip that sits directly above Quick Picks.
            HomeSection::KeepListening(_) => 110,
            HomeSection::QuickPicks(_) => 100,
            HomeSection::ForgottenFavorites(_) => 60,
            HomeSection::HomePage(_, index) => 40 - *index as i64,
            HomeSection::DailyDiscover(_) => 70,
            HomeSection::SimilarRecommendation(..) => 35,
            HomeSection::FromTheCommunity(_) => 25,
        }
    }
}
