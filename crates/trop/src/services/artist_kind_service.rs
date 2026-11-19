//! `Trop/Services/ArtistKindService.swift`: what kind of act an artist is
//! ("Band", "Singer", "Rapper"…). YouTube Music only says "Artist", so the
//! type comes from MusicBrainz. Each name is looked up once; hits and misses
//! are cached.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Deserialize;

const USER_AGENT: &str = "WoofMusic/1.3 ( https://github.com/LaskyWolf/WoofMusic )";
/// MusicBrainz allows 1 request per second; a small margin keeps under it.
const REQUEST_INTERVAL: Duration = Duration::from_millis(1100);
/// MusicBrainz answers 503 to roughly one request in six even at that pace.
const MAX_ATTEMPTS: usize = 3;
const MINIMUM_SCORE: i64 = 90;

/// Lowercased name → label. An empty label records a miss.
// ponytail: kept for the run, not saved to disk; each name is looked up again after a restart.
static CACHE: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Mutex::default);
static NEXT_REQUEST: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(Mutex::default);

fn cache() -> MutexGuard<'static, HashMap<String, String>> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The artist's kind from the cache alone, so a row can show it at once.
pub fn cached_kind(name: &str) -> Option<String> {
    cache()
        .get(&name.trim().to_lowercase())
        .filter(|label| !label.is_empty())
        .cloned()
}

/// The artist's kind, or `None` when MusicBrainz has no confident match.
pub async fn kind(http: &reqwest::Client, name: &str) -> Option<String> {
    let key = name.trim().to_lowercase();
    if key.is_empty() {
        return None;
    }
    for _ in 0..MAX_ATTEMPTS {
        if let Some(cached) = cache().get(&key) {
            return (!cached.is_empty()).then(|| cached.clone());
        }
        wait_for_turn().await;
        // Another caller may have looked the same name up while this one waited.
        if let Some(cached) = cache().get(&key) {
            return (!cached.is_empty()).then(|| cached.clone());
        }
        // Rate limited or offline: not cached, tried again.
        let Some(artists) = search(http, name).await else {
            continue;
        };
        let label = artists
            .iter()
            .find(|artist| {
                artist.score.unwrap_or(0) >= MINIMUM_SCORE && artist.name.to_lowercase() == key
            })
            .and_then(label)
            .unwrap_or_default();
        cache().insert(key, label.clone());
        return (!label.is_empty()).then_some(label);
    }
    None
}

/// Waits until a request may go out under the rate limit.
async fn wait_for_turn() {
    loop {
        let wait = {
            let mut next = NEXT_REQUEST.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            match *next {
                Some(at) if at > now => at - now,
                _ => {
                    *next = Some(now + REQUEST_INTERVAL);
                    return;
                }
            }
        };
        tokio::time::sleep(wait).await;
    }
}

async fn search(http: &reqwest::Client, name: &str) -> Option<Vec<MbArtist>> {
    let escaped = name.replace('\\', "\\\\").replace('"', "\\\"");
    let url = reqwest::Url::parse_with_params(
        "https://musicbrainz.org/ws/2/artist/",
        [
            ("query", format!("artist:\"{escaped}\"")),
            ("fmt", "json".to_owned()),
            ("limit", "1".to_owned()),
        ],
    )
    .ok()?;
    let response = http
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;
    if response.status() != reqwest::StatusCode::OK {
        return None;
    }
    parse(&response.bytes().await.ok()?)
}

fn parse(body: &[u8]) -> Option<Vec<MbArtist>> {
    serde_json::from_slice::<MbSearch>(body)
        .ok()
        .map(|found| found.artists)
}

fn label(artist: &MbArtist) -> Option<String> {
    let disambiguation = artist
        .disambiguation
        .clone()
        .unwrap_or_default()
        .to_lowercase();
    let tags = artist
        .tags
        .iter()
        .map(|tag| tag.name.as_str())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let label = match artist.kind.as_deref()? {
        // The disambiguation ("Italian rapper", "Scottish DJ and producer")
        // is curated; tags are crowd-sourced, so they only break the tie.
        "Person" => role(&disambiguation)
            .or_else(|| role(&tags))
            .unwrap_or("Solo Artist"),
        "Group" => {
            let hints = format!("{disambiguation} {tags}");
            if ["group", "idol", "k-pop", "kpop"]
                .iter()
                .any(|hint| hints.contains(hint))
            {
                "Group"
            } else {
                "Band"
            }
        }
        "Orchestra" => "Orchestra",
        "Choir" => "Choir",
        "Character" => "Character",
        _ => return None,
    };
    Some(label.to_owned())
}

/// A solo artist's role from free text, or `None` when it names none.
fn role(text: &str) -> Option<&'static str> {
    let words: HashSet<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let dj = words.contains("dj");
    let producer = words.contains("producer");
    if dj {
        return Some(if producer { "DJ/Producer" } else { "DJ" });
    }
    if words.contains("rapper") || words.contains("rap") {
        return Some("Rapper");
    }
    if words.contains("singer") || words.contains("vocalist") {
        return Some("Singer");
    }
    if producer {
        return Some("Producer");
    }
    words.contains("composer").then_some("Composer")
}

#[derive(Deserialize)]
struct MbSearch {
    artists: Vec<MbArtist>,
}

#[derive(Deserialize)]
struct MbArtist {
    name: String,
    #[serde(rename = "type")]
    kind: Option<String>,
    score: Option<i64>,
    disambiguation: Option<String>,
    #[serde(default)]
    tags: Vec<MbTag>,
}

#[derive(Deserialize)]
struct MbTag {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(json: &str) -> Option<String> {
        label(&parse(json.as_bytes()).unwrap()[0])
    }

    #[test]
    fn a_person_is_named_by_the_disambiguation_then_the_tags() {
        let rapper = r#"{"artists":[{"name":"X","type":"Person","score":100,"disambiguation":"Italian rapper"}]}"#;
        assert_eq!(answer(rapper).as_deref(), Some("Rapper"));
        let dj = r#"{"artists":[{"name":"X","type":"Person","disambiguation":"Scottish DJ and producer"}]}"#;
        assert_eq!(answer(dj).as_deref(), Some("DJ/Producer"));
        let tagged = r#"{"artists":[{"name":"X","type":"Person","tags":[{"name":"singer"}]}]}"#;
        assert_eq!(answer(tagged).as_deref(), Some("Singer"));
        let bare = r#"{"artists":[{"name":"X","type":"Person"}]}"#;
        assert_eq!(answer(bare).as_deref(), Some("Solo Artist"));
    }

    #[test]
    fn groups_are_bands_unless_they_say_group_or_idol() {
        let band = r#"{"artists":[{"name":"X","type":"Group"}]}"#;
        assert_eq!(answer(band).as_deref(), Some("Band"));
        let idols = r#"{"artists":[{"name":"X","type":"Group","tags":[{"name":"k-pop"}]}]}"#;
        assert_eq!(answer(idols).as_deref(), Some("Group"));
    }

    #[test]
    fn other_types_and_unknown_types() {
        let orchestra = r#"{"artists":[{"name":"X","type":"Orchestra"}]}"#;
        assert_eq!(answer(orchestra).as_deref(), Some("Orchestra"));
        let other = r#"{"artists":[{"name":"X","type":"Other"}]}"#;
        assert_eq!(answer(other), None);
        let none = r#"{"artists":[{"name":"X"}]}"#;
        assert_eq!(answer(none), None);
    }
}
