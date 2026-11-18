//! Trop/History/HistoryViewModel.swift, plus the search filter of
//! HistoryView.swift and `RecapService.shelfDate`.
//!
//! The History page merges two lists: plays from this app (database events,
//! which can be deleted) and YouTube Music's own history (plays on other
//! devices). Days come from the local time zone, like Swift's `Calendar.current`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::SystemTime;

use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use serde_json::Value;
use swiftytube::networking::artwork_urls::ArtworkURLs;
use swiftytube::networking::inner_tube_client::{BrowseArgs, InnerTubeClient};
use swiftytube::networking::inner_tube_decode::JsonDict;
use swiftytube::sync::browse_lens::BrowseLens;

use super::shorts::Shorts;
use crate::database::database_service::{DatabaseService, HistoryEntry};
use crate::database::entities::event::Event;
use crate::networking::models::yt_item::{SongItem, YTArtist};

/// One song in the merged history: a play from this app (it has an `event`,
/// so it can be deleted) or one only YouTube Music knows about.
#[derive(Clone, Debug, PartialEq)]
pub struct HistoryRow {
    pub song: SongItem,
    pub event: Option<Event>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HistoryDay {
    pub title: String,
    pub rows: Vec<HistoryRow>,
}

/// One shelf of YouTube Music's history: its title ("Today", "Last week") and songs.
pub type RemoteSection = (String, Vec<SongItem>);

#[derive(Debug)]
pub struct HistoryViewModel {
    pub days: Vec<HistoryDay>,
    pub is_loading: bool,
    local: Vec<HistoryEntry>,
    /// Plays read from the database; `load_more_local` raises it by `PAGE_SIZE`.
    local_limit: i64,
    /// The last read filled the limit, so older plays may be waiting.
    pub has_more_local: bool,
    remote: Vec<RemoteSection>,
}

impl Default for HistoryViewModel {
    fn default() -> Self {
        Self {
            days: Vec::new(),
            is_loading: true,
            local: Vec::new(),
            local_limit: Self::PAGE_SIZE,
            has_more_local: false,
            remote: Vec::new(),
        }
    }
}

impl HistoryViewModel {
    const PAGE_SIZE: i64 = 100;

    /// How many plays the next local read asks for.
    pub fn local_limit(&self) -> i64 {
        self.local_limit
    }

    /// Reads the newest `limit` plays. Blocking: call it off the UI thread.
    pub fn read_local(db: &DatabaseService, limit: i64) -> rusqlite::Result<Vec<HistoryEntry>> {
        db.fetch_history(Some(limit))
    }

    pub fn set_local(&mut self, entries: Vec<HistoryEntry>) {
        self.has_more_local = entries.len() as i64 == self.local_limit;
        self.local = entries;
        self.is_loading = false;
        self.rebuild();
    }

    /// The limit for the next `PAGE_SIZE` older plays, or `None` when there are none.
    pub fn load_more_local(&mut self) -> Option<i64> {
        if !self.has_more_local {
            return None;
        }
        self.has_more_local = false;
        self.local_limit += Self::PAGE_SIZE;
        Some(self.local_limit)
    }

    /// YouTube Music's history, without Shorts. Fails when signed out or offline.
    pub async fn read_remote(client: Arc<InnerTubeClient>) -> Result<Vec<RemoteSection>, String> {
        let json = client
            .browse(BrowseArgs::id("FEmusic_history"))
            .await
            .map_err(|error| error.to_string())?;
        let mut sections = Self::parse_remote_history(&json);
        for (_, songs) in &mut sections {
            *songs = Shorts::removing(std::mem::take(songs)).await;
        }
        sections.retain(|(_, songs)| !songs.is_empty());
        Ok(sections)
    }

    pub fn set_remote(&mut self, sections: Vec<RemoteSection>) {
        self.remote = sections;
        self.rebuild();
    }

    /// Deletes plays from this app. Blocking: call it off the UI thread.
    pub fn delete_events(db: &DatabaseService, ids: &[i64]) -> rusqlite::Result<()> {
        db.write(|tx| {
            for id in ids {
                tx.execute("DELETE FROM event WHERE id = ?", [id])?;
            }
            Ok(())
        })
    }

    fn rebuild(&mut self) {
        let tz = TimeZone::system();
        let today = Timestamp::now().to_zoned(tz.clone()).date();
        self.days = merge(&self.local, &self.remote, &tz, today);
    }

    /// Swift `HistoryScreenView.displayableDays`: the days with only the rows
    /// whose title or artists hold every word of `query`.
    pub fn filtered(&self, query: &str) -> Vec<HistoryDay> {
        self.days
            .iter()
            .filter_map(|day| {
                let rows: Vec<HistoryRow> = day
                    .rows
                    .iter()
                    .filter(|row| matches(&row.song, query))
                    .cloned()
                    .collect();
                (!rows.is_empty()).then(|| HistoryDay {
                    title: day.title.clone(),
                    rows,
                })
            })
            .collect()
    }

    /// Swift `parseRemoteHistory`: each `musicShelfRenderer` of the history
    /// browse, as its title and songs.
    pub fn parse_remote_history(json: &JsonDict) -> Vec<RemoteSection> {
        let has_single_column = json
            .get("contents")
            .and_then(|contents| contents.get("singleColumnBrowseResultsRenderer"))
            .is_some_and(Value::is_object);
        if !has_single_column {
            return Vec::new();
        }
        let Some(shelves) = BrowseLens::browse_sections(json) else {
            return Vec::new();
        };
        let mut sections = Vec::new();
        for shelf in shelves {
            let Some(shelf) = shelf.get("musicShelfRenderer").and_then(Value::as_object) else {
                continue;
            };
            let title = shelf
                .get("title")
                .and_then(|title| title.get("runs"))
                .and_then(|runs| runs.get(0))
                .and_then(|run| run.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("Unknown")
                .to_owned();
            let Some(items) = shelf.get("contents").and_then(Value::as_array) else {
                continue;
            };
            let songs: Vec<SongItem> = items
                .iter()
                .filter_map(|item| item.get("musicResponsiveListItemRenderer"))
                .filter_map(Value::as_object)
                .filter_map(SongItem::from)
                .collect();
            if !songs.is_empty() {
                sections.push((title, songs));
            }
        }
        sections
    }
}

/// Swift `rebuild`: plays from this app by day, then what YouTube Music lists
/// for that day and this app does not. A play from here that already reached
/// YouTube Music shows once.
pub fn merge(
    local: &[HistoryEntry],
    remote: &[RemoteSection],
    tz: &TimeZone,
    today: Date,
) -> Vec<HistoryDay> {
    let mut by_day: HashMap<Date, Vec<HistoryRow>> = HashMap::new();
    let mut sorted: Vec<&HistoryEntry> = local.iter().collect();
    sorted.sort_by_key(|entry| std::cmp::Reverse(entry.event.timestamp));
    for entry in sorted {
        by_day
            .entry(local_day(entry.event.timestamp, tz))
            .or_default()
            .push(HistoryRow {
                song: song_from_entry(entry),
                event: Some(entry.event.clone()),
            });
    }
    // Shelves covering a period ("Last week") have no single day: they keep their
    // title and sit where that period starts, holding only songs not listed elsewhere.
    let mut periods: Vec<(Date, HistoryDay)> = Vec::new();
    let mut everywhere: HashSet<String> = local.iter().map(|e| e.event.song_id.clone()).collect();
    for (title, songs) in remote {
        if let Some(day) = day_for_title(title, today) {
            let rows = by_day.entry(day).or_default();
            let mut known: HashSet<String> = rows.iter().map(|r| r.song.video_id.clone()).collect();
            for song in songs {
                if known.insert(song.video_id.clone()) {
                    everywhere.insert(song.video_id.clone());
                    rows.push(HistoryRow {
                        song: song.clone(),
                        event: None,
                    });
                }
            }
            continue;
        }
        let rows: Vec<HistoryRow> = songs
            .iter()
            .filter(|song| everywhere.insert(song.video_id.clone()))
            .map(|song| HistoryRow {
                song: song.clone(),
                event: None,
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        let date = shelf_date(title, today).unwrap_or(Date::MIN);
        periods.push((
            date,
            HistoryDay {
                title: title.clone(),
                rows,
            },
        ));
    }
    let mut dated: Vec<(Date, HistoryDay)> = by_day
        .into_iter()
        .filter(|(_, rows)| !rows.is_empty())
        .map(|(date, rows)| {
            let title = day_title(date, today);
            (date, HistoryDay { title, rows })
        })
        .collect();
    // Days are unique; sort them so the merge below is stable.
    dated.sort_by_key(|(date, _)| std::cmp::Reverse(*date));
    dated.extend(periods);
    // A stable sort: for equal dates, a day stays before the period that starts on it.
    dated.sort_by_key(|(date, _)| std::cmp::Reverse(*date));
    dated.into_iter().map(|(_, day)| day).collect()
}

fn local_day(time: SystemTime, tz: &TimeZone) -> Date {
    Timestamp::try_from(time)
        .map(|ts| ts.to_zoned(tz.clone()).date())
        .unwrap_or(Date::MIN)
}

/// "Today", "Yesterday", or a date like "Nov 12, 2026".
fn day_title(day: Date, today: Date) -> String {
    if day == today {
        "Today".to_owned()
    } else if day == yesterday(today) {
        "Yesterday".to_owned()
    } else {
        day.strftime("%b %-d, %Y").to_string()
    }
}

fn yesterday(today: Date) -> Date {
    today.yesterday().unwrap_or(today)
}

fn days_before(today: Date, days: i64) -> Option<Date> {
    today.checked_sub(days.days()).ok()
}

/// Swift `day(forTitle:)`: the day of one of YouTube's section titles
/// ("Today", "Monday", "November 12"), which the app asks for in English.
/// `None` for a title in another form.
pub fn day_for_title(title: &str, today: Date) -> Option<Date> {
    let text = title.trim().to_lowercase();
    match text.as_str() {
        "today" => return Some(today),
        "yesterday" => return Some(yesterday(today)),
        _ => {}
    }
    for offset in 2..=7 {
        let day = days_before(today, offset)?;
        if weekday_name(day.weekday()) == text {
            return Some(day);
        }
    }
    // "November 12" or "Nov 12", with an optional ", 2025".
    let (month_day, year) = match text.split_once(',') {
        Some((month_day, year)) => (month_day, Some(year.trim().parse::<i16>().ok()?)),
        None => (text.as_str(), None),
    };
    let (month, day) = month_day.trim().split_once(' ')?;
    let month = month_number(month)?;
    let day: i8 = day.trim().parse().ok()?;
    match year {
        Some(year) => Date::new(year, month, day).ok(),
        None => {
            let date = Date::new(today.year(), month, day).ok()?;
            // No year in the title means the last 12 months.
            if date > today {
                date.checked_sub(1.year()).ok()
            } else {
                Some(date)
            }
        }
    }
}

/// Swift `RecapService.shelfDate`: where a period shelf ("Last week",
/// "September 2025") starts, for sorting it among the days.
pub fn shelf_date(title: &str, today: Date) -> Option<Date> {
    let text = title.trim().to_lowercase();
    match text.as_str() {
        "today" => return Some(today),
        "yesterday" => return days_before(today, 1),
        "this week" => return days_before(today, 2),
        "last week" => return days_before(today, 7),
        "this month" => return days_before(today, 14),
        "last month" => return today.checked_sub(1.month()).ok(),
        _ => {}
    }
    let (month, year) = match text.split_once(' ') {
        Some((month, year)) => (month_number(month)?, year.trim().parse::<i16>().ok()?),
        None => {
            let month = month_number(&text)?;
            // "September" alone is the most recent September.
            let year = if month > today.month() {
                today.year() - 1
            } else {
                today.year()
            };
            (month, year)
        }
    };
    Date::new(year, month, 15).ok()
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// 1-12 for an English month name or its three-letter short form.
fn month_number(name: &str) -> Option<i8> {
    let name = name.trim().trim_end_matches('.');
    MONTHS
        .iter()
        .position(|month| *month == name || (name.len() == 3 && month.starts_with(name)))
        .map(|index| index as i8 + 1)
}

fn weekday_name(weekday: Weekday) -> &'static str {
    match weekday {
        Weekday::Monday => "monday",
        Weekday::Tuesday => "tuesday",
        Weekday::Wednesday => "wednesday",
        Weekday::Thursday => "thursday",
        Weekday::Friday => "friday",
        Weekday::Saturday => "saturday",
        Weekday::Sunday => "sunday",
    }
}

/// Swift `HistoryScreenView.matches`: every word of `query` is in the title or an artist's name.
pub fn matches(song: &SongItem, query: &str) -> bool {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let text = std::iter::once(song.title.as_str())
        .chain(song.artists.iter().map(|artist| artist.name.as_str()))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    words.iter().all(|word| text.contains(word.as_str()))
}

/// Swift `SongItem(historyEntry:)`: the saved song, or a placeholder named
/// after its id when the song row is gone.
pub fn song_from_entry(entry: &HistoryEntry) -> SongItem {
    match &entry.song {
        Some(song) => SongItem {
            video_id: song.id.clone(),
            title: song.title.clone(),
            artists: song
                .artist_name
                .iter()
                .map(|name| YTArtist {
                    name: name.clone(),
                    id: None,
                })
                .collect(),
            album: song.album_name.clone(),
            duration: song.duration,
            thumbnail_url: song.thumbnail_url.clone(),
            ..SongItem::default()
        },
        None => SongItem {
            video_id: entry.event.song_id.clone(),
            title: entry.event.song_id.clone(),
            thumbnail_url: Some(ArtworkURLs::fallback(&entry.event.song_id)),
            ..SongItem::default()
        },
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    fn date(y: i16, m: i8, d: i8) -> Date {
        Date::new(y, m, d).unwrap()
    }

    fn song(id: &str) -> SongItem {
        SongItem {
            video_id: id.to_owned(),
            title: id.to_owned(),
            ..SongItem::default()
        }
    }

    fn entry(id: i64, song_id: &str, at: Timestamp) -> HistoryEntry {
        HistoryEntry {
            event: Event {
                id: Some(id),
                song_id: song_id.to_owned(),
                timestamp: SystemTime::UNIX_EPOCH + Duration::from_secs(at.as_second() as u64),
                play_time: 60_000,
            },
            song: None,
        }
    }

    #[test]
    fn titles_map_to_days() {
        // A Thursday.
        let today = date(2026, 11, 19);
        assert_eq!(day_for_title("Today", today), Some(today));
        assert_eq!(day_for_title("yesterday", today), Some(date(2026, 11, 18)));
        assert_eq!(day_for_title("Monday", today), Some(date(2026, 11, 16)));
        // Seven days back: last Thursday, not today.
        assert_eq!(day_for_title("Thursday", today), Some(date(2026, 11, 12)));
        assert_eq!(day_for_title("November 2", today), Some(date(2026, 11, 2)));
        // A later day of the year is last year's.
        assert_eq!(day_for_title("Dec 24", today), Some(date(2025, 12, 24)));
        assert_eq!(
            day_for_title("March 3, 2024", today),
            Some(date(2024, 3, 3))
        );
        assert_eq!(day_for_title("Last week", today), None);
    }

    #[test]
    fn period_shelves_have_a_start() {
        let today = date(2026, 11, 19);
        assert_eq!(shelf_date("Last week", today), Some(date(2026, 11, 12)));
        assert_eq!(shelf_date("Last month", today), Some(date(2026, 10, 19)));
        assert_eq!(shelf_date("September", today), Some(date(2026, 9, 15)));
        assert_eq!(shelf_date("December", today), Some(date(2025, 12, 15)));
        assert_eq!(shelf_date("September 2024", today), Some(date(2024, 9, 15)));
        assert_eq!(shelf_date("Something", today), None);
    }

    #[test]
    fn merge_dedups_and_orders() {
        let tz = TimeZone::UTC;
        let today = date(2026, 11, 19);
        let noon = |d: Date| d.at(12, 0, 0, 0).to_zoned(tz.clone()).unwrap().timestamp();
        let local = [
            entry(1, "a", noon(today)),
            entry(2, "b", noon(date(2026, 11, 18))),
        ];
        let remote = vec![
            // "a" already played here today: shows once.
            ("Today".to_owned(), vec![song("a"), song("c")]),
            ("Last week".to_owned(), vec![song("b"), song("d")]),
            ("Tuesday".to_owned(), vec![song("e")]),
        ];
        let days = merge(&local, &remote, &tz, today);
        let ids = |day: &HistoryDay| {
            day.rows
                .iter()
                .map(|r| r.song.video_id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            days.iter().map(|d| d.title.as_str()).collect::<Vec<_>>(),
            ["Today", "Yesterday", "Nov 17, 2026", "Last week"]
        );
        assert_eq!(ids(&days[0]), ["a", "c"]);
        assert!(days[0].rows[0].event.is_some() && days[0].rows[1].event.is_none());
        assert_eq!(ids(&days[2]), ["e"]);
        // "b" is listed on a day already.
        assert_eq!(ids(&days[3]), ["d"]);
    }

    #[test]
    fn older_days_get_a_date_title() {
        assert_eq!(
            day_title(date(2026, 3, 4), date(2026, 11, 19)),
            "Mar 4, 2026"
        );
    }

    #[test]
    fn search_needs_every_word() {
        let mut s = song("x");
        s.title = "Get Lucky".to_owned();
        s.artists = vec![YTArtist {
            name: "Daft Punk".to_owned(),
            id: None,
        }];
        assert!(matches(&s, "lucky daft"));
        assert!(matches(&s, "  "));
        assert!(!matches(&s, "lucky queen"));
    }

    #[test]
    fn parses_history_shelves() {
        let row = |id: &str| {
            json!({ "musicResponsiveListItemRenderer": {
                "playlistItemData": { "videoId": id },
                "flexColumns": [
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": format!("Song {id}") }] } } },
                    { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Artist" }] } } }
                ]
            }})
        };
        let json = json!({ "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": { "sectionListRenderer": { "contents": [
            { "musicShelfRenderer": { "title": { "runs": [{ "text": "Today" }] }, "contents": [row("v1"), row("v2")] } },
            { "musicCarouselShelfRenderer": {} },
            { "musicShelfRenderer": { "title": { "runs": [{ "text": "Empty" }] }, "contents": [] } }
        ] } } } }] } } });
        let sections = HistoryViewModel::parse_remote_history(json.as_object().unwrap());
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, "Today");
        assert_eq!(
            sections[0]
                .1
                .iter()
                .map(|s| s.video_id.as_str())
                .collect::<Vec<_>>(),
            ["v1", "v2"]
        );
    }
}
