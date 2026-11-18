//! Trop/Activity/ReleaseFeed.swift: new releases from the artists the user follows.
//!
//! YouTube Music's own bell feed is not served to this client, so the list is
//! read from each artist's page: its albums and singles, tagged only with a
//! year. Deezer's public API supplies the exact day, matched by artist name and
//! title; a release Deezer can't date stays out, since a year can't tell
//! January from last week.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use jiff::civil::Date;
use jiff::{Timestamp, ToSpan, tz::TimeZone};
use serde::Deserialize;
use serde_json::{Map, Value};
use swiftytube::networking::inner_tube_client::{BrowseArgs, InnerTubeClient};
use tokio::task::JoinSet;
use tokio::time::Instant;

use super::activity_store::{ActivityItem, ActivityJson};
use crate::database::database_service::DatabaseService;
use crate::services::library_pages::LibraryPages;

pub struct ReleaseFeed;

/// A release and its day, for sorting.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub item: ActivityItem,
    pub date: Date,
}

impl ReleaseFeed {
    /// A guard against an endless list, far above what anyone follows.
    const ARTIST_LIMIT: usize = 500;
    /// Releases kept per artist.
    const PER_ARTIST: usize = 2;
    const PARALLELISM: usize = 5;
    /// Releases older than this many days leave the list.
    const WINDOW_DAYS: i64 = 60;

    /// The followed artists the library sync saved. Blocking: call it off the UI thread.
    pub fn saved_artists(db: &DatabaseService) -> Vec<String> {
        db.read(|conn| {
            conn.prepare(
                "SELECT id FROM artist WHERE bookmarked_at IS NOT NULL AND is_podcast_channel = 0",
            )?
            .query_map([], |row| row.get(0))?
            .collect()
        })
        .unwrap_or_default()
    }

    /// The releases, newest first. `saved` is `saved_artists`; when it is empty
    /// (sync not run yet) the account's list is read instead. `None` when
    /// nothing could be read (offline, signed out), as opposed to no new releases.
    pub async fn items(
        client: Arc<InnerTubeClient>,
        saved: Vec<String>,
        seen: HashSet<String>,
    ) -> Option<Vec<ActivityItem>> {
        let artist_ids: Vec<String> = if saved.is_empty() {
            LibraryPages::all_artists(&client, &[])
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|artist| artist.browse_id)
                .collect()
        } else {
            saved
        };
        if artist_ids.is_empty() {
            return None;
        }
        let today = Timestamp::now().to_zoned(TimeZone::system()).date();
        let seen = Arc::new(seen);
        let mut all: Vec<Release> = Vec::new();
        let mut any_read = false;
        // A few artists at a time, so the app doesn't open dozens of connections
        // at once and Deezer's limit (about 50 requests every 5 seconds) holds.
        for chunk in artist_ids
            .iter()
            .take(Self::ARTIST_LIMIT)
            .collect::<Vec<_>>()
            .chunks(Self::PARALLELISM)
        {
            let mut tasks = JoinSet::new();
            for (index, id) in chunk.iter().enumerate() {
                let (client, seen, id) = (client.clone(), seen.clone(), (*id).clone());
                tasks.spawn(async move {
                    (
                        index,
                        Self::releases_of_artist(&client, &id, &seen, today).await,
                    )
                });
            }
            let mut results = Vec::new();
            while let Some(Ok(result)) = tasks.join_next().await {
                results.push(result);
            }
            any_read |= results.iter().any(|(_, releases)| releases.is_some());
            results.sort_by_key(|(index, _)| *index);
            all.extend(results.into_iter().flat_map(|(_, r)| r.unwrap_or_default()));
        }
        any_read.then(|| newest_first(all))
    }

    /// `None` when the artist's page couldn't be read.
    async fn releases_of_artist(
        client: &InnerTubeClient,
        id: &str,
        seen: &HashSet<String>,
        today: Date,
    ) -> Option<Vec<Release>> {
        let json = Value::Object(client.browse(BrowseArgs::id(id)).await.ok()?);
        // Last year too: a December release is still new in January.
        let years = [today.year().to_string(), (today.year() - 1).to_string()];
        let (artist, candidates) = candidates(&json, &years);
        // Most artists have nothing recent: Deezer is asked only when there is something to date.
        let Some(artist) = artist.filter(|_| !candidates.is_empty()) else {
            return Some(Vec::new());
        };
        let dates = DeezerDates::release_dates(&artist).await;
        Some(pick(&artist, &candidates, &dates, seen, today))
    }
}

/// One album or single of this or last year, in page order.
pub struct Candidate<'a> {
    pub album: String,
    pub title: String,
    pub row: &'a Map<String, Value>,
}

/// The artist's name and this year's and last year's albums and singles.
pub fn candidates<'a>(json: &'a Value, years: &[String]) -> (Option<String>, Vec<Candidate<'a>>) {
    let header = ActivityJson::nodes("musicImmersiveHeaderRenderer", Some(json))
        .into_iter()
        .next()
        .or_else(|| {
            ActivityJson::nodes("musicVisualHeaderRenderer", Some(json))
                .into_iter()
                .next()
        });
    let artist = header.and_then(|header| ActivityJson::text(header.get("title")));
    let found = ActivityJson::nodes("musicCarouselShelfRenderer", Some(json))
        .into_iter()
        .flat_map(|shelf| ActivityJson::nodes("musicTwoRowItemRenderer", shelf.get("contents")))
        .filter_map(|row| {
            let album = ActivityJson::first("browseId", row.get("navigationEndpoint"))?
                .as_str()
                .filter(|id| id.starts_with("MPREb"))?;
            let subtitle = ActivityJson::text(row.get("subtitle"))?;
            if !years.iter().any(|year| subtitle.contains(year.as_str())) {
                return None;
            }
            Some(Candidate {
                album: album.to_owned(),
                title: ActivityJson::text(row.get("title"))?,
                row,
            })
        })
        .collect();
    (artist, found)
}

/// The dated candidates of the last `WINDOW_DAYS`, at most `PER_ARTIST`.
pub fn pick(
    artist: &str,
    candidates: &[Candidate],
    dates: &HashMap<String, Date>,
    seen: &HashSet<String>,
    today: Date,
) -> Vec<Release> {
    let oldest = today
        .checked_sub(ReleaseFeed::WINDOW_DAYS.days())
        .unwrap_or(Date::MIN);
    let mut found = Vec::new();
    // An album and a single often share a title: list it once.
    let mut titles = HashSet::new();
    for candidate in candidates {
        let key = DeezerDates::key(&candidate.title);
        let Some(&date) = dates.get(&key) else {
            continue;
        };
        if date < oldest || !titles.insert(key) {
            continue;
        }
        let id = format!("release-{}", candidate.album);
        let thumbnails = ActivityJson::first("thumbnails", candidate.row.get("thumbnailRenderer"))
            .map(|thumbnails| serde_json::json!({ "thumbnails": thumbnails }));
        found.push(Release {
            item: ActivityItem {
                message: format!("{artist}: {}", candidate.title),
                time: Some(relative_day(date, today)),
                thumbnail: ActivityJson::last_thumbnail(thumbnails.as_ref()),
                album_id: Some(candidate.album.clone()),
                is_read: seen.contains(&id),
                id,
            },
            date,
        });
        if found.len() == ReleaseFeed::PER_ARTIST {
            break;
        }
    }
    found
}

/// Newest first; a collaboration shows on each artist's page: listed once.
fn newest_first(mut all: Vec<Release>) -> Vec<ActivityItem> {
    all.sort_by_key(|release| std::cmp::Reverse(release.date));
    let mut ids = HashSet::new();
    all.into_iter()
        .map(|release| release.item)
        .filter(|item| ids.insert(item.id.clone()))
        .collect()
}

/// "Today", "Yesterday", "3 days ago", "Last week", "5 weeks ago".
// ponytail: whole days only; Swift's `.relative(presentation: .named)` also says "in 2 days" for a future day.
pub fn relative_day(date: Date, today: Date) -> String {
    let days = (today - date).get_days();
    match days {
        ..=0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        2..=6 => format!("{days} days ago"),
        7..=13 => "Last week".to_owned(),
        _ => format!("{} weeks ago", days / 7),
    }
}

/// Release days from Deezer's public API, which needs no key. Only the artist's
/// name leaves the computer. Any failure (no exact name match, rate limit,
/// offline) gives no dates, and those releases stay out of the list.
pub struct DeezerDates;

#[derive(Deserialize)]
struct Search {
    data: Option<Vec<SearchArtist>>,
}

#[derive(Deserialize)]
struct SearchArtist {
    id: i64,
    name: String,
    nb_fan: Option<i64>,
}

#[derive(Deserialize)]
struct Albums {
    data: Option<Vec<Album>>,
}

#[derive(Deserialize)]
struct Album {
    title: String,
    release_date: Option<String>,
}

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("TLS backend initializes")
});

/// When the next Deezer request may leave.
static NEXT: LazyLock<tokio::sync::Mutex<Instant>> =
    LazyLock::new(|| tokio::sync::Mutex::new(Instant::now()));

impl DeezerDates {
    /// Title key → release day.
    pub async fn release_dates(artist: &str) -> HashMap<String, Date> {
        let search: Option<Search> = Self::fetch(HTTP.get(format!(
                "https://api.deezer.com/search/artist?{}",
                form_urlencoded::Serializer::new(String::new())
                    .append_pair("q", artist)
                    .append_pair("limit", "5")
                    .finish()
            )))
        .await;
        // Only the same name, compared by `key` because YouTube pads some names with
        // invisible characters. Several artists can share it: the most followed is
        // almost always the right one.
        let Some(found) = search
            .and_then(|search| search.data)
            .unwrap_or_default()
            .into_iter()
            .filter(|found| Self::key(&found.name) == Self::key(artist))
            .max_by_key(|found| found.nb_fan.unwrap_or(0))
        else {
            return HashMap::new();
        };
        let albums: Option<Albums> = Self::fetch(HTTP.get(format!(
            "https://api.deezer.com/artist/{}/albums?limit=100",
            found.id
        )))
        .await;
        Self::dates(albums.and_then(|albums| albums.data).unwrap_or_default())
    }

    /// The earliest day of each title.
    fn dates(albums: Vec<Album>) -> HashMap<String, Date> {
        let mut dates: HashMap<String, Date> = HashMap::new();
        for album in albums {
            let Some(date) = album
                .release_date
                .and_then(|text| text.parse::<Date>().ok())
            else {
                continue;
            };
            dates
                .entry(Self::key(&album.title))
                .and_modify(|known| *known = (*known).min(date))
                .or_insert(date);
        }
        dates
    }

    /// Lowercased letters and digits only: "From Zero (Deluxe)" and "from zero deluxe" match.
    pub fn key(title: &str) -> String {
        title
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect()
    }

    /// Spaces requests to stay under Deezer's limit of about 50 every 5 seconds;
    /// past it Deezer answers with an error and that artist would get no dates.
    async fn wait_turn() {
        let turn = {
            let mut next = NEXT.lock().await;
            let turn = (*next).max(Instant::now());
            *next = turn + Duration::from_millis(120);
            turn
        };
        tokio::time::sleep_until(turn).await;
    }

    async fn fetch<T: for<'de> Deserialize<'de>>(request: reqwest::RequestBuilder) -> Option<T> {
        Self::wait_turn().await;
        let response = request.send().await.ok()?;
        if response.status().as_u16() != 200 {
            return None;
        }
        serde_json::from_slice(&response.bytes().await.ok()?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i16, m: i8, d: i8) -> Date {
        Date::new(y, m, d).unwrap()
    }

    #[test]
    fn keys_ignore_case_and_punctuation() {
        assert_eq!(
            DeezerDates::key("From Zero (Deluxe)"),
            DeezerDates::key("from zero deluxe")
        );
        let dates = DeezerDates::dates(vec![
            Album {
                title: "Song".to_owned(),
                release_date: Some("2026-10-02".to_owned()),
            },
            Album {
                title: "song".to_owned(),
                release_date: Some("2026-09-30".to_owned()),
            },
            Album {
                title: "Undated".to_owned(),
                release_date: None,
            },
        ]);
        assert_eq!(
            dates,
            HashMap::from([("song".to_owned(), date(2026, 9, 30))])
        );
    }

    #[test]
    fn artist_page_releases_get_deezer_days() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../fixtures/trop/artist.json"
        ))
        .unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        // The fixture's newest releases are from 2023.
        let (artist, found) = candidates(&json, &["2023".to_owned(), "2022".to_owned()]);
        assert_eq!(artist.as_deref(), Some("Daft Punk"));
        assert_eq!(
            found.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            [
                "Random Access Memories (Drumless Edition)",
                "GLBTM (Studio Outtakes)",
                "The Writing of Fragments of Time"
            ]
        );
        let today = date(2023, 6, 1);
        let dates = HashMap::from([
            (
                DeezerDates::key("Random Access Memories (Drumless Edition)"),
                date(2023, 5, 12),
            ),
            // Older than the 60-day window.
            (
                DeezerDates::key("GLBTM (Studio Outtakes)"),
                date(2023, 1, 2),
            ),
            (
                DeezerDates::key("The Writing of Fragments of Time"),
                date(2023, 5, 31),
            ),
        ]);
        let seen = HashSet::from([format!("release-{}", found[0].album)]);
        let releases = pick("Daft Punk", &found, &dates, &seen, today);
        assert_eq!(releases.len(), 2);
        assert!(releases[0].item.is_read && !releases[1].item.is_read);
        assert_eq!(
            releases[0].item.message,
            "Daft Punk: Random Access Memories (Drumless Edition)"
        );
        assert_eq!(releases[0].item.time.as_deref(), Some("2 weeks ago"));
        assert!(releases[0].item.thumbnail.is_some());
        assert!(
            releases[0]
                .item
                .album_id
                .as_deref()
                .unwrap()
                .starts_with("MPREb")
        );
        let items = newest_first(releases);
        assert_eq!(
            items[0].message,
            "Daft Punk: The Writing of Fragments of Time"
        );
    }

    #[test]
    fn relative_days() {
        let today = date(2026, 11, 19);
        assert_eq!(relative_day(today, today), "Today");
        assert_eq!(relative_day(date(2026, 11, 18), today), "Yesterday");
        assert_eq!(relative_day(date(2026, 11, 15), today), "4 days ago");
        assert_eq!(relative_day(date(2026, 11, 10), today), "Last week");
        assert_eq!(relative_day(date(2026, 10, 1), today), "7 weeks ago");
    }
}
