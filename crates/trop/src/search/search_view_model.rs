//! The logic of Trop/Search/SearchViewModel.swift: sorting, filters, the
//! artist hero and the recent searches. The app keeps the state.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::database::database_service::HistoryEntry;
use crate::networking::models::yt_item::{ArtistItem, YTItem};
use crate::search::search_parser::SearchSection;

/// Result ordering for the search screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchSort {
    #[default]
    Relevant,
    Date,
    Recent,
    Rating,
}

impl SearchSort {
    pub const ALL: [Self; 4] = [Self::Relevant, Self::Date, Self::Recent, Self::Rating];

    pub fn title(self) -> &'static str {
        match self {
            Self::Relevant => "Most Relevant",
            Self::Date => "Date Released",
            Self::Recent => "Recently Played",
            Self::Rating => "Rating",
        }
    }
}

impl std::fmt::Display for SearchSort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.title())
    }
}

/// Canonical display order: songs first, then albums, artists, playlists, podcasts.
pub const DISPLAY_ORDER: [&str; 5] = ["Songs", "Albums", "Artists", "Playlists", "Podcasts"];
pub const MAX_HISTORY_ENTRIES: usize = 20;
const MAX_MATCHING_HISTORY_ENTRIES: usize = 3;

/// The artist lifted into the hero card, with its audience line.
#[derive(Clone, Debug, PartialEq)]
pub struct Hero {
    pub artist: ArtistItem,
    /// Such as "12.3M monthly audience".
    pub detail: Option<String>,
}

/// Artist-first layout: when the query names the first artist in the results,
/// lift it into the hero card. Otherwise it is listed with the other artists.
/// A top result card's songs move to the top of Songs; the API often repeats
/// them there, so a song already in Songs is not added twice.
pub fn extract_hero(results: &mut Vec<SearchSection>, query: &str) -> Option<Hero> {
    let index = results
        .iter()
        .position(|section| matches!(section.items.first(), Some(YTItem::Artist(_))))?;
    let Some(YTItem::Artist(artist)) = results[index].items.first().cloned() else {
        return None;
    };
    let is_artist_search = is_search_for_artist(query, &artist.name);
    let hero = is_artist_search.then(|| Hero {
        artist: artist.clone(),
        detail: results[index].subtitle.clone(),
    });
    let card_songs: Vec<YTItem> = results[index]
        .items
        .iter()
        .filter(|item| matches!(item, YTItem::Song(_)))
        .cloned()
        .collect();
    results[index].items.retain(|item| {
        !matches!(item, YTItem::Song(_))
            && !(matches!(item, YTItem::Artist(_)) && item.id() == artist.browse_id)
    });
    if !is_artist_search {
        prepend(results, "Artists", vec![YTItem::Artist(artist)]);
    }
    if !card_songs.is_empty() {
        prepend(results, "Songs", card_songs);
    }
    results.retain(|section| !section.items.is_empty());
    hero
}

/// Puts `items` first in the section titled `title` (added when missing),
/// without listing an item twice.
fn prepend(results: &mut Vec<SearchSection>, title: &str, items: Vec<YTItem>) {
    match results.iter_mut().find(|section| section.title == title) {
        Some(section) => {
            let mut seen = HashSet::new();
            let merged = items.into_iter().chain(section.items.drain(..));
            section.items = merged
                .filter(|item| seen.insert(item.id().to_owned()))
                .collect();
        }
        None => results.push(SearchSection::new(title, items, None)),
    }
}

/// True when the query is the artist's name or a part of it, ignoring case,
/// accents and punctuation: "weeknd" and "the weeknd", not "blinding lights".
pub fn is_search_for_artist(query: &str, name: &str) -> bool {
    let query = comparable(query);
    !query.is_empty() && comparable(name).contains(&query)
}

fn comparable(text: &str) -> String {
    fold(text).chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Lowercase without accents, like Swift's case- and diacritic-insensitive folding.
// ponytail: folds Latin accents only; add unicode-normalization if other
// scripts' marks need folding.
fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'à'..='å' | 'ā' | 'ă' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'è'..='ë' | 'ē' | 'ę' | 'ě' => 'e',
            'ì'..='ï' | 'ī' => 'i',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò'..='ö' | 'ø' | 'ō' | 'ő' => 'o',
            'ù'..='ü' | 'ū' | 'ů' | 'ű' => 'u',
            'ý' | 'ÿ' => 'y',
            'ś' | 'š' => 's',
            'ź' | 'ż' | 'ž' => 'z',
            'ł' => 'l',
            c => c,
        })
        .collect()
}

/// The chips after "All Results": Library, then the sections present.
pub fn available_filters(results: &[SearchSection]) -> Vec<&'static str> {
    std::iter::once("Library")
        .chain(
            DISPLAY_ORDER
                .into_iter()
                .filter(|title| results.iter().any(|s| s.title == *title)),
        )
        .collect()
}

/// Results ordered for display, with the active sort applied to Songs (and
/// Albums for Date Released). `recency`: latest play per video id.
pub fn display_sections(
    results: &[SearchSection],
    sort: SearchSort,
    recency: &HashMap<String, SystemTime>,
) -> Vec<SearchSection> {
    let known = DISPLAY_ORDER
        .iter()
        .filter_map(|title| results.iter().find(|s| s.title == *title));
    let others = results
        .iter()
        .filter(|s| !DISPLAY_ORDER.contains(&s.title.as_str()));
    known
        .chain(others)
        .map(|section| SearchSection {
            items: sorted_items(section, sort, recency),
            ..section.clone()
        })
        .collect()
}

fn sorted_items(
    section: &SearchSection,
    sort: SearchSort,
    recency: &HashMap<String, SystemTime>,
) -> Vec<YTItem> {
    let mut items = section.items.clone();
    // `sort_by_key` is stable: ties keep their relevance order. Keys are
    // reversed for newest / most first.
    match (section.title.as_str(), sort) {
        (_, SearchSort::Relevant) => {}
        ("Songs" | "Albums", SearchSort::Date) => {
            items.sort_by_key(|item| std::cmp::Reverse(year(item)))
        }
        ("Songs", SearchSort::Rating) => {
            // Search results expose view counts (not likes): most-watched first.
            items.sort_by_key(|item| std::cmp::Reverse(views(item)))
        }
        ("Songs", SearchSort::Recent) => items.sort_by_key(|item| {
            std::cmp::Reverse(item.video_id().and_then(|id| recency.get(id)).copied())
        }),
        _ => {}
    }
    items
}

fn year(item: &YTItem) -> i64 {
    match item {
        YTItem::Song(song) => song.year.unwrap_or(-1),
        YTItem::Album(album) => album.year.unwrap_or(-1),
        _ => -1,
    }
}

fn views(item: &YTItem) -> i64 {
    match item {
        YTItem::Song(song) => song.view_count.unwrap_or(-1),
        _ => -1,
    }
}

/// Latest play time per video id, for Recently Played.
pub fn recency_map(entries: &[HistoryEntry]) -> HashMap<String, SystemTime> {
    let mut map: HashMap<String, SystemTime> = HashMap::new();
    for entry in entries {
        let latest = map
            .entry(entry.event.song_id.clone())
            .or_insert(entry.event.timestamp);
        *latest = (*latest).max(entry.event.timestamp);
    }
    map
}

/// Swift `updateHistory`: `query` moves to the front, the list keeps 20.
pub fn remember(history: &mut Vec<String>, query: &str) {
    history.retain(|entry| entry != query);
    history.insert(0, query.to_owned());
    history.truncate(MAX_HISTORY_ENTRIES);
}

/// Earlier searches that contain what is being typed.
pub fn matching_history<'a>(history: &'a [String], typed: &str) -> Vec<&'a str> {
    let typed = fold(typed.trim());
    history
        .iter()
        .filter(|entry| fold(entry).contains(&typed))
        .take(MAX_MATCHING_HISTORY_ENTRIES)
        .map(String::as_str)
        .collect()
}

/// Suggestions not already listed as earlier searches.
pub fn new_suggestions<'a>(suggestions: &'a [String], matching: &[&str]) -> Vec<&'a str> {
    let earlier: HashSet<String> = matching.iter().map(|q| q.to_lowercase()).collect();
    suggestions
        .iter()
        .filter(|s| !earlier.contains(&s.to_lowercase()))
        .map(String::as_str)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artist_search_ignores_case_accents_and_punctuation() {
        assert!(is_search_for_artist("adele", "Adele"));
        assert!(is_search_for_artist("weeknd", "The Weeknd"));
        assert!(is_search_for_artist("ac dc", "AC/DC"));
        assert!(is_search_for_artist("beyonce", "Beyoncé"));
        assert!(!is_search_for_artist("blinding lights", "The Weeknd"));
        assert!(!is_search_for_artist(
            "blinding lights the weeknd",
            "The Weeknd"
        ));
        assert!(!is_search_for_artist("!!!", "Adele"));
    }

    #[test]
    fn history_keeps_twenty_newest_first_without_repeats() {
        let mut history = Vec::new();
        for n in 0..25 {
            remember(&mut history, &format!("q{n}"));
        }
        remember(&mut history, "q10");
        assert_eq!(history.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(history[0], "q10");
        assert_eq!(history[1], "q24");
        assert_eq!(history.iter().filter(|q| *q == "q10").count(), 1);
    }

    #[test]
    fn typing_lists_three_matching_searches_then_the_new_suggestions() {
        let history: Vec<String> = [
            "Daft Punk",
            "daft punk live",
            "Adele",
            "Daftendirekt",
            "dàft",
        ]
        .map(String::from)
        .to_vec();
        let matching = matching_history(&history, " daft ");
        assert_eq!(matching, ["Daft Punk", "daft punk live", "Daftendirekt"]);
        let suggestions = [
            "daft punk".to_owned(),
            "daft punk around the world".to_owned(),
        ];
        assert_eq!(
            new_suggestions(&suggestions, &matching),
            ["daft punk around the world"]
        );
    }
}
