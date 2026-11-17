// SwiftyTube/Sync/ArtistCredits.swift.

use std::sync::LazyLock;

use crate::networking::inner_tube_decode::{JsonDict, as_dict, as_string};
use regex::Regex;

/// One credited artist: display name plus the artist page id when the run
/// actually links one (`None` for plain-text credits and topic channels).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ArtistCredit {
    pub name: String,
    pub browse_id: Option<String>,
}

/// Runs that separate credits rather than naming anyone.
const SEPARATORS: &[&str] = &["•", "·", ",", "&", "and", "feat.", "ft.", "featuring"];

/// Runs that label the row instead of crediting anyone.
const JUNK_LABELS: &[&str] = &[
    "song",
    "video",
    "track",
    "music",
    "podcast",
    "episode",
    "album",
    "playlist",
    "released",
    "year",
    "plays",
    "views",
    "downloads",
    "listeners",
    "subscribers",
    "watchers",
    "likes",
];

static JUNK_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"^[\d,\.]+[KMBT]?\s*(views|downloads|listeners|subscribers|plays|watchers|likes?)?$",
        r"^\d{4}$",
        r"^[\d,\.]+[KMBT]?$",
        r"^[\d,\.]+[KMBT]?\s+likes?$",
        // Clock durations ("1:33:21", "3:21"): some rows carry the length in
        // the byline, and without this they parse as an artist credit.
        r"^\d{1,3}[:.,]\d{2}(?:[:.,]\d{2})?$",
    ]
    .iter()
    .map(|p| Regex::new(&format!("(?i){p}")).unwrap())
    .collect()
});

static YEAR_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*[\(\[]\d{4}[\)\]]\s*$").unwrap());

pub struct ArtistCredits;

impl ArtistCredits {
    /// True when raw run text can name an artist (not a separator, label or
    /// count). Structural callers (shelf segment splitting) use this.
    pub fn is_artist_name(text: &str) -> bool {
        let lower = text.trim().to_lowercase();
        if lower.is_empty()
            || SEPARATORS.contains(&lower.as_str())
            || JUNK_LABELS.contains(&lower.as_str())
        {
            return false;
        }
        !JUNK_PATTERNS.iter().any(|p| p.is_match(&lower))
    }

    /// Display form of a raw credit name: topic suffix and bare years off.
    pub fn cleaned_name(name: &str) -> String {
        let mut s = name.to_string();
        let lower = s.to_lowercase();
        for suffix in [" - topic", " – topic", " — topic"] {
            if lower.ends_with(suffix) {
                s.truncate(s.len() - suffix.len());
            }
        }
        YEAR_SUFFIX.replace_all(&s, "").trim().to_string()
    }

    /// Credits from raw byline/subtitle/strapline runs. Separators split,
    /// junk is dropped, album/playlist-linked runs are skipped (they name a
    /// release, not a performer), names are cleaned, empties dropped and
    /// repeats merged (keeping the copy that carries a page id).
    pub fn parse(runs: &[&JsonDict]) -> Vec<ArtistCredit> {
        let mut credits: Vec<ArtistCredit> = Vec::new();
        for run in runs {
            let Some(text) = as_string(run.get("text")) else {
                continue;
            };
            let trimmed = text.trim();
            if !Self::is_artist_name(trimmed) {
                continue;
            }
            let browse_id = Self::browse_id(run);
            if let Some(id) = browse_id
                && Self::is_album_or_playlist_link(id, run)
            {
                continue;
            }
            // Channel ids stay attached (topic channels included): a channel
            // page for the right person beats a name-search lottery that can
            // land on a different person with a similar name.
            let name = Self::cleaned_name(trimmed);
            if name.is_empty() {
                continue;
            }
            let lower = name.to_lowercase();
            if let Some(existing) = credits.iter_mut().find(|c| c.name.to_lowercase() == lower) {
                if existing.browse_id.is_none() && browse_id.is_some() {
                    existing.browse_id = browse_id.map(str::to_string);
                }
                continue;
            }
            credits.push(ArtistCredit {
                name,
                browse_id: browse_id.map(str::to_string),
            });
        }
        credits
    }

    // MARK: - Link policy

    fn browse_id(run: &JsonDict) -> Option<&str> {
        let nav = as_dict(run.get("navigationEndpoint"))?;
        as_string(as_dict(nav.get("browseEndpoint"))?.get("browseId"))
    }

    fn music_page_type(run: &JsonDict) -> Option<&str> {
        // Shelf rows nest the config inside browseEndpoint; watch panels
        // hoist it to navigationEndpoint level. Check both.
        let nav = as_dict(run.get("navigationEndpoint"));
        let browse = as_dict(nav.and_then(|n| n.get("browseEndpoint")));
        let supported = as_dict(
            browse.and_then(|b| b.get("browseEndpointContextSupportedConfigs")),
        )
        .or_else(|| as_dict(nav.and_then(|n| n.get("browseEndpointContextSupportedConfigs"))));
        as_string(as_dict(supported?.get("browseEndpointContextMusicConfig"))?.get("pageType"))
    }

    /// Album/playlist links name a release, never a performer.
    fn is_album_or_playlist_link(browse_id: &str, run: &JsonDict) -> bool {
        if browse_id.starts_with("MPREb_")
            || browse_id.starts_with("VL")
            || browse_id.starts_with("PL")
        {
            return true;
        }
        Self::music_page_type(run)
            .is_some_and(|t| t != "MUSIC_PAGE_TYPE_ARTIST" && t != "MUSIC_PAGE_TYPE_USER_CHANNEL")
    }
}
