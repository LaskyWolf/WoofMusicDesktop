// Swift nests `HomePage.Chip` / `HomePage.Section`; Rust has no nested types,
// so they are `HomePageChip` / `HomePageSection`. Swift's `enum HomeSection`
// keeps its cases and associated values.

use std::sync::atomic::{AtomicU64, Ordering};

use super::yt_item::YTItem;

/// Stands in for Swift's `UUID()`: unique per instance in this process.
pub(crate) fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Debug, PartialEq)]
pub struct HomePage {
    pub chips: Vec<HomePageChip>,
    pub sections: Vec<HomePageSection>,
    pub continuation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HomePageChip {
    pub title: String,
    pub params: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HomePageSection {
    pub title: String,
    pub items: Vec<YTItem>,
}

impl HomePageSection {
    pub fn is_songs_only(&self) -> bool {
        !self.items.is_empty() && self.items.iter().all(|i| matches!(i, YTItem::Song(_)))
    }
}

/// One Explore (`FEmusic_explore`) shelf.
#[derive(Clone, Debug, PartialEq)]
pub struct ExploreSection {
    pub id: u64,
    pub title: String,
    pub kind: ExploreSectionKind,
    pub items: Vec<YTItem>,
    pub moods: Vec<MoodItem>,
}

impl ExploreSection {
    pub fn new(
        title: impl Into<String>,
        kind: ExploreSectionKind,
        items: Vec<YTItem>,
        moods: Vec<MoodItem>,
    ) -> Self {
        Self {
            id: next_id(),
            title: title.into(),
            kind,
            items,
            moods,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExploreSectionKind {
    /// Vertical song/video rows.
    Rows,
    /// Horizontal two-row cards (albums, artists, playlists).
    Cards,
    /// Mood & genre tiles (title + browse params, no artwork).
    Moods,
}

/// Mood/genre shortcut: drills into `FEmusic_moods_and_genres_category`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoodItem {
    pub id: u64,
    pub title: String,
    pub params: Option<String>,
    /// Category color YouTube Music shows next to the button, as 0xRRGGBB.
    pub color: Option<u32>,
}

impl MoodItem {
    pub fn new(title: impl Into<String>, params: Option<String>, color: Option<u32>) -> Self {
        Self {
            id: next_id(),
            title: title.into(),
            params,
            color,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HomeSection {
    QuickPicks(Vec<YTItem>),
    KeepListening(Vec<YTItem>),
    ForgottenFavorites(Vec<YTItem>),
    HomePage(HomePageSection, usize),
    SimilarRecommendation(Vec<YTItem>, String),
    DailyDiscover(Vec<YTItem>),
    FromTheCommunity(Vec<YTItem>),
}

impl HomeSection {
    pub fn id(&self) -> String {
        match self {
            HomeSection::QuickPicks(_) => "quick_picks".into(),
            HomeSection::KeepListening(_) => "keep_listening".into(),
            HomeSection::ForgottenFavorites(_) => "forgotten_favorites".into(),
            HomeSection::HomePage(_, index) => format!("homepage_{index}"),
            HomeSection::SimilarRecommendation(_, title) => format!("similar_{title}"),
            HomeSection::DailyDiscover(_) => "daily_discover".into(),
            HomeSection::FromTheCommunity(_) => "from_the_community".into(),
        }
    }

    pub fn display_title(&self) -> &str {
        match self {
            HomeSection::QuickPicks(_) => "Quick Picks",
            HomeSection::KeepListening(_) => "Keep Listening",
            HomeSection::ForgottenFavorites(_) => "Forgotten Favorites",
            HomeSection::HomePage(section, _) => &section.title,
            HomeSection::SimilarRecommendation(_, title) => title,
            HomeSection::DailyDiscover(_) => "Daily Discover",
            HomeSection::FromTheCommunity(_) => "From the Community",
        }
    }

    pub fn items(&self) -> &[YTItem] {
        match self {
            HomeSection::QuickPicks(items)
            | HomeSection::KeepListening(items)
            | HomeSection::ForgottenFavorites(items)
            | HomeSection::SimilarRecommendation(items, _)
            | HomeSection::DailyDiscover(items)
            | HomeSection::FromTheCommunity(items) => items,
            HomeSection::HomePage(section, _) => &section.items,
        }
    }

    pub fn is_songs_only(&self) -> bool {
        match self {
            HomeSection::HomePage(section, _) => section.is_songs_only(),
            HomeSection::FromTheCommunity(_) => false,
            _ => true,
        }
    }
}
