//! Trop/Audio/Lyrics/Providers/GeniusProvider.swift.
//! https://github.com/spicetify/cli/blob/main/CustomApps/lyrics-plus/ProviderGenius.js

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use swiftytube::networking::http_client::Session;

use super::lrclib_provider::LyricsError;
use super::lyrics_http::{LyricsHTTP, LyricsText};
use super::lyrics_provider::{LyricsFuture, LyricsProvider, LyricsQuery};
use crate::audio::lyrics::lyrics_service::LyricLine;
use crate::utilities::string_app::StringApp;

static FEAT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s*(feat\.?|ft\.?|featuring).*$").unwrap());
static PARENS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(.*?\)").unwrap());
static BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[.*?\]").unwrap());
static BR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<br\s*/?>").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

const HEADERS: [(&str, &str); 3] = [
    (
        "User-Agent",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
    ),
    ("Accept-Language", "en-US,en;q=0.9"),
    (
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
    ),
];

pub struct GeniusProvider;

impl GeniusProvider {
    const SEARCH_BASE: &str = "https://genius.com/api/search/song";

    async fn fetch_lines(
        query: &LyricsQuery,
        session: &Session,
    ) -> Result<Vec<LyricLine>, LyricsError> {
        // A network error is passed on, so being offline is not taken for "no lyrics".
        let mut network_error = None;
        for title in Self::title_variants(&query.title) {
            match Self::fetch_once(&query.artist, &title, session).await {
                Ok(lines) if !lines.is_empty() => return Ok(lines),
                Ok(_) => {}
                Err(error) if error.is_answer() => {}
                Err(error) => network_error = Some(error),
            }
        }
        Err(network_error.unwrap_or(LyricsError::NotFound))
    }

    // Search + scrape.

    async fn fetch_once(
        artist: &str,
        title: &str,
        session: &Session,
    ) -> Result<Vec<LyricLine>, LyricsError> {
        let q = format!("{artist} {title}");
        let request = LyricsHTTP::request(
            Self::SEARCH_BASE,
            &[("per_page", "20"), ("q", &q)],
            &HEADERS,
        )?;
        let json = LyricsHTTP::get_json(request, session).await?;
        let sections = json
            .get("response")
            .and_then(|r| r.get("sections"))
            .and_then(Value::as_array)
            .ok_or(LyricsError::DecodingFailed)?;
        let url = sections
            .iter()
            .filter_map(|s| s.get("hits").and_then(Value::as_array))
            .flatten()
            .next()
            .ok_or(LyricsError::NotFound)?
            .get("result")
            .and_then(|r| r.get("url"))
            .and_then(Value::as_str)
            .ok_or(LyricsError::DecodingFailed)?;
        Self::fetch_lyrics_page(url, session).await
    }

    async fn fetch_lyrics_page(
        url: &str,
        session: &Session,
    ) -> Result<Vec<LyricLine>, LyricsError> {
        let data = LyricsHTTP::get(LyricsHTTP::request(url, &[], &HEADERS)?, session).await?;
        let html = String::from_utf8(data).map_err(|_| LyricsError::DecodingFailed)?;
        let containers = Self::extract_lyrics_containers(&html);
        if containers.is_empty() {
            return Err(LyricsError::NotFound);
        }
        let lines = LyricsText::plain_lines(&Self::html_to_text(&containers.join("<br>")));
        if lines.is_empty() {
            return Err(LyricsError::NotFound);
        }
        Ok(lines)
    }

    // Title variants.

    fn title_variants(title: &str) -> Vec<String> {
        let mut variants: Vec<String> = Vec::new();
        let candidates = [
            title.to_string(),
            // Strip parenthetical / bracketed extra info: (feat. X), [Remastered], etc.
            // Swift's pattern `\(.*?)` does not compile, so Swift strips only the brackets.
            BRACKETS
                .replace_all(&PARENS.replace_all(title, ""), "")
                .into_owned(),
            // Strip trailing "feat./ft./featuring" clauses.
            FEAT.replace_all(title, "").into_owned(),
        ];
        for value in candidates {
            let v = value.trim().to_string();
            if !v.is_empty() && !variants.contains(&v) {
                variants.push(v);
            }
        }
        variants
    }

    // HTML extraction.

    /// The inner HTML of each `data-lyrics-container="true"` element, matching nested `<div>`s.
    fn extract_lyrics_containers(html: &str) -> Vec<String> {
        const MARKER: &str = "data-lyrics-container=\"true\"";
        let mut results = Vec::new();
        let mut scan = 0;
        while let Some(found) = html[scan..].find(MARKER) {
            let after_marker = scan + found + MARKER.len();
            // Find the closing '>' of the opening tag.
            let Some(tag_close) = html[after_marker..].find('>') else {
                break;
            };
            let inner_start = after_marker + tag_close + 1;
            let mut pos = inner_start;
            let mut depth = 1;
            while pos < html.len() && depth > 0 {
                let rest = &html[pos..];
                if rest.starts_with("<div") {
                    depth += 1;
                    let Some(gt) = rest.find('>') else { break };
                    pos += gt + 1;
                } else if rest.starts_with("</div>") {
                    depth -= 1;
                    // Swift's slice also keeps "</div" of the closing tag.
                    if depth == 0 {
                        results.push(html[inner_start..pos].to_string());
                    }
                    pos += "</div>".len();
                } else {
                    pos += rest.chars().next().map_or(1, char::len_utf8);
                }
            }
            scan = pos;
        }
        results
    }

    /// Converts a fragment of Genius lyrics HTML into plain text.
    fn html_to_text(html: &str) -> String {
        let s = BR.replace_all(html, "\n").replace("</p>", "\n");
        TAG.replace_all(&s, "").decoding_html_entities()
    }
}

impl LyricsProvider for GeniusProvider {
    fn id(&self) -> &'static str {
        "genius"
    }

    fn name(&self) -> &'static str {
        "Genius"
    }

    fn fetch<'a>(&'a self, query: &'a LyricsQuery, session: &'a Session) -> LyricsFuture<'a> {
        Box::pin(Self::fetch_lines(query, session))
    }
}
