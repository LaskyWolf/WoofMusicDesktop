//! Trop/Services/LibraryPages.swift.

use std::collections::HashSet;

use serde_json::Value;
use swiftytube::networking::inner_tube_client::{BrowseArgs, InnerTubeClient};
use swiftytube::networking::inner_tube_decode::JsonDict;
use swiftytube::sync::library_browse_parser::LibraryBrowseParser;
use swiftytube::sync::library_parsed_models::{ParsedAlbum, ParsedArtist, ParsedPlaylist};

/// Every page of a library list. YouTube Music sends 25 entries per page and
/// puts the next page's token inside the shelf or grid.
pub struct LibraryPages;

impl LibraryPages {
    /// A guard against a token that never ends.
    const MAX_PAGES: usize = 40;

    /// `None` when a page failed: the list would be incomplete. Swift default
    /// for `first_page`: empty.
    pub async fn all_artists(
        client: &InnerTubeClient,
        first_page: &[ParsedArtist],
    ) -> Option<Vec<ParsedArtist>> {
        Self::all(
            client,
            "FEmusic_library_corpus_artists",
            first_page.len(),
            LibraryBrowseParser::parse_artists,
            |artist| &artist.browse_id,
        )
        .await
    }

    pub async fn all_albums(
        client: &InnerTubeClient,
        first_page: &[ParsedAlbum],
    ) -> Option<Vec<ParsedAlbum>> {
        Self::all(
            client,
            "FEmusic_liked_albums",
            first_page.len(),
            LibraryBrowseParser::parse_albums,
            |album| &album.browse_id,
        )
        .await
    }

    pub async fn all_playlists(
        client: &InnerTubeClient,
        first_page: &[ParsedPlaylist],
    ) -> Option<Vec<ParsedPlaylist>> {
        Self::all(
            client,
            "FEmusic_liked_playlists",
            first_page.len(),
            LibraryBrowseParser::parse_playlists,
            |playlist| &playlist.browse_id,
        )
        .await
    }

    /// The whole list, or `None` when a page fails. Callers then keep their
    /// first page but remove nothing: a partial list would drop every entry it misses.
    async fn all<T>(
        client: &InnerTubeClient,
        browse_id: &str,
        first_page_count: usize,
        parse: fn(&JsonDict) -> Vec<T>,
        id: fn(&T) -> &String,
    ) -> Option<Vec<T>> {
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        let mut request = BrowseArgs::id(browse_id);
        for _ in 0..Self::MAX_PAGES {
            let json = client.browse(request).await.ok()?;
            let page: Vec<T> = parse(&json)
                .into_iter()
                .filter(|item| seen.insert(id(item).clone()))
                .collect();
            let page_is_empty = page.is_empty();
            items.extend(page);
            let token = Self::next_page_token(&Value::Object(json));
            let Some(token) = token.filter(|_| !page_is_empty) else {
                break;
            };
            request = BrowseArgs {
                continuation: Some(token),
                ..BrowseArgs::default()
            };
        }
        // Fewer than SwiftyTube's own first page means something was missed.
        (items.len() >= first_page_count).then_some(items)
    }

    pub fn next_page_token(json: &Value) -> Option<String> {
        let token = |key, field| {
            first(key, json)?
                .as_object()?
                .get(field)?
                .as_str()
                .map(str::to_string)
        };
        token("nextContinuationData", "continuation")
            .or_else(|| token("continuationCommand", "token"))
    }
}

/// Swift `ActivityJSON.first` (`Activity/ActivityView.swift`): the first value
/// stored under `key`, at any depth.
// ponytail: private copy; moves to `activity/` when that folder is translated.
fn first<'a>(key: &str, node: &'a Value) -> Option<&'a Value> {
    match node {
        Value::Object(dict) => dict
            .get(key)
            .or_else(|| dict.values().find_map(|value| first(key, value))),
        Value::Array(list) => list.iter().find_map(|value| first(key, value)),
        _ => None,
    }
}
