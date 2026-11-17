// SwiftyTube/Sync/PlaylistDetailService.swift: the page walk that reads a
// playlist's rows. `fetchPlaylist` (saves rows in the sync store) comes with
// R5, as does `fetchPlaylistRows`' `onPage` progress callback (no caller yet).
// `dumpDiagnosticJSON` is not ported; the diagnostic line is still logged.

use std::collections::HashSet;

use serde_json::Value;
use swiftytube::core::swiftytube_log::SwiftyTubeLog;
use swiftytube::networking::inner_tube_client::{BrowseArgs, InnerTubeClient};
use swiftytube::networking::inner_tube_decode::{
    InnerTubeDecode, JsonDict, as_dict, as_dict_list, as_string,
};
use swiftytube::networking::inner_tube_error::InnerTubeError;
use swiftytube::networking::inner_tube_json::InnerTubeJSON;

use super::browse_lens::BrowseLens;

/// All raw track rows of a playlist, plus the first page for header parsing.
#[derive(Clone, Debug, Default)]
pub struct PlaylistRows {
    pub first_page: JsonDict,
    pub rows: Vec<JsonDict>,
}

pub struct PlaylistDetailService<'a> {
    pub client: &'a InnerTubeClient,
}

impl<'a> PlaylistDetailService<'a> {
    pub fn new(client: &'a InnerTubeClient) -> Self {
        Self { client }
    }

    /// All raw track rows across every page, plus the first page's JSON for
    /// header parsing. Markers are stripped; each row carries a
    /// `musicResponsiveListItemRenderer`. With `throw_on_truncation`, a repeat
    /// token or page cap fails instead of returning partial rows.
    /// Swift defaults: `max_pages: 100`, `throw_on_truncation: false`.
    pub async fn fetch_playlist_rows(
        &self,
        playlist_id: &str,
        max_pages: usize,
        throw_on_truncation: bool,
    ) -> Result<PlaylistRows, InnerTubeError> {
        // Stored ids may already carry the prefix; blindly prepending yields
        // VLVL… and the server answers with a content-less response.
        let browse_id = Self::request_browse_id(playlist_id);
        let mut all_items = Vec::new();
        let mut token: Option<String> = None;
        let mut seen_tokens = HashSet::new();
        let mut pages = 0;
        let mut had_shelf = false;
        let mut first_page: Option<JsonDict> = None;
        loop {
            // Continuation requests carry only the token; resending browseId
            // makes the server ignore it and return page one again.
            let args = match token.take() {
                Some(continuation) => BrowseArgs {
                    continuation: Some(continuation),
                    ..BrowseArgs::default()
                },
                None => BrowseArgs::id(browse_id.clone()),
            };
            let json = self.client.browse(args).await?;
            if let Some(items) = Self::extract_playlist_items(&json) {
                had_shelf = true;
                all_items.extend(items.into_iter().cloned());
            }
            token = Self::extract_playlist_continuation(&json).map(str::to_string);
            if first_page.is_none() {
                first_page = Some(json);
            }
            pages += 1;
            if let Some(t) = &token
                && !seen_tokens.insert(t.clone())
            {
                let message = format!("fetchPlaylist {playlist_id}: repeated continuation token");
                if throw_on_truncation {
                    return Err(InnerTubeError::TruncatedResponse(message));
                }
                SwiftyTubeLog::error(&format!("{message}, stopping"));
                break;
            }
            if pages >= max_pages {
                let message = format!("fetchPlaylist {playlist_id}: hit {max_pages}-page cap");
                if throw_on_truncation {
                    return Err(InnerTubeError::TruncatedResponse(message));
                }
                SwiftyTubeLog::error(&format!("{message}, stopping"));
                break;
            }
            if token.is_none() {
                break;
            }
        }

        let first_page = first_page.unwrap_or_default();
        if !had_shelf {
            let detail = Self::diagnose_no_shelf(&first_page);
            SwiftyTubeLog::error(&format!("fetchPlaylist {playlist_id}: {detail}"));
            if let Some(prompt) = Self::sign_in_prompt_text(&first_page) {
                SwiftyTubeLog::error(&format!(
                    "fetchPlaylist {playlist_id}: server sent a sign-in prompt ({prompt}) — session not accepted"
                ));
                return Err(InnerTubeError::SessionExpired);
            }
            return Err(InnerTubeError::InvalidResponseContent(format!(
                "no playlist shelf in browse response ({detail})"
            )));
        }
        Ok(PlaylistRows {
            first_page,
            rows: all_items,
        })
    }

    pub fn diagnose_no_shelf(json: &JsonDict) -> String {
        fn sorted_keys(dict: &JsonDict) -> String {
            let mut keys: Vec<&str> = dict.keys().map(String::as_str).collect();
            keys.sort();
            keys.join(",")
        }
        let mut parts = vec![format!("keys={}", sorted_keys(json))];
        if let Some(contents) = as_dict(json.get("contents")) {
            parts.push(format!("contentsKeys={}", sorted_keys(contents)));
        }
        if let Some(alert) = Self::alert_renderer_text(json) {
            parts.push(format!("alertText={alert}"));
        }
        if let Some(sign_in) = Self::sign_in_prompt_text(json) {
            parts.push(format!("signInMessage={sign_in}"));
        }
        if let Some(response_context) = as_dict(json.get("responseContext")) {
            let yes = response_context.contains_key("visitorData");
            parts.push(format!(
                "responseVisitorData={}",
                if yes { "yes" } else { "no" }
            ));
        }
        parts.join(" ")
    }

    pub fn alert_renderer_text(json: &JsonDict) -> Option<String> {
        as_dict_list(json.get("alerts"))
            .unwrap_or_default()
            .into_iter()
            .find_map(|alert| {
                InnerTubeJSON::runs_text(InnerTubeDecode::dict(&["alertRenderer", "text"], alert))
                    .or_else(|| {
                        InnerTubeJSON::runs_text(InnerTubeDecode::dict(
                            &["alertWithButtonRenderer", "text"],
                            alert,
                        ))
                    })
            })
    }

    pub fn sign_in_prompt_text(json: &JsonDict) -> Option<String> {
        struct Walk {
            message: Option<String>,
            link: bool,
        }
        impl Walk {
            fn map(&mut self, map: &JsonDict, depth: usize) {
                if self.message.is_none()
                    && let Some(renderer) = as_dict(map.get("messageRenderer"))
                {
                    self.message = InnerTubeJSON::runs_text(as_dict(renderer.get("text")))
                        .or_else(|| InnerTubeJSON::runs_text(as_dict(renderer.get("subtext"))));
                }
                if map.get("signInEndpoint").is_some_and(|v| !v.is_null()) {
                    self.link = true;
                }
                for value in map.values() {
                    self.value(value, depth + 1);
                }
            }
            fn value(&mut self, node: &Value, depth: usize) {
                if depth >= 24 {
                    return;
                }
                match node {
                    Value::Object(map) => self.map(map, depth),
                    Value::Array(list) => list.iter().for_each(|v| self.value(v, depth + 1)),
                    _ => {}
                }
            }
        }
        let mut walk = Walk {
            message: None,
            link: false,
        };
        walk.map(json, 0);
        walk.link
            .then(|| walk.message.unwrap_or_else(|| "sign-in prompt".to_string()))
    }

    /// Endpoint browse id for a playlist, tolerating ids that already carry
    /// the VL prefix (library rows store the endpoint's id verbatim).
    pub fn request_browse_id(playlist_id: &str) -> String {
        if playlist_id.starts_with("VL") {
            playlist_id.to_string()
        } else {
            format!("VL{playlist_id}")
        }
    }

    pub fn extract_playlist_items(json: &JsonDict) -> Option<Vec<&JsonDict>> {
        let shelves = Self::playlist_shelves(json);
        if !shelves.is_empty() {
            return Some(
                shelves
                    .into_iter()
                    .flat_map(|shelf| {
                        Self::track_rows(as_dict_list(shelf.get("contents")).unwrap_or_default())
                    })
                    .collect(),
            );
        }
        // Continuation pages for large/private playlists arrive with no shelf
        // at all: rows live in
        // onResponseReceivedActions[].appendContinuationItemsAction.continuationItems.
        let action_items = Self::continuation_action_items(json);
        if !action_items.is_empty() {
            return Some(Self::track_rows(action_items));
        }
        None
    }

    pub fn extract_playlist_continuation(json: &JsonDict) -> Option<&str> {
        // Shelf-level tokens first: a sectionList-level token points at the
        // next *section* (a carousel), not at more tracks.
        if let Some(token) = Self::shelf_continuation_token(&Self::playlist_shelves(json)) {
            return Some(token);
        }
        // Next-page token carried by a trailing continuationItemRenderer.
        let action_items = Self::continuation_action_items(json);
        if let Some(token) = Self::trailing_continuation_token(&action_items) {
            return Some(token);
        }
        // Last resort: sectionList-level token. This usually addresses the
        // next section rather than more tracks.
        InnerTubeDecode::continuation_token(InnerTubeDecode::dict(
            &[
                "contents",
                "twoColumnBrowseResultsRenderer",
                "secondaryContents",
                "sectionListRenderer",
            ],
            json,
        ))
    }

    fn shelf_in(section: &JsonDict) -> Option<&JsonDict> {
        let inner = as_dict(section.get("itemSectionRenderer"))
            .and_then(|r| as_dict_list(r.get("contents")));
        let unwrapped = inner.and_then(|i| i.first().copied()).unwrap_or(section);
        as_dict(unwrapped.get("musicPlaylistShelfRenderer"))
            .or_else(|| as_dict(unwrapped.get("musicShelfRenderer")))
    }

    /// Every candidate track shelf in a playlist response, in priority order:
    /// browse sections (tabbed single/two-column), two-column secondary
    /// shelves, classic continuations.
    fn playlist_shelves(json: &JsonDict) -> Vec<&JsonDict> {
        let mut shelves: Vec<&JsonDict> = BrowseLens::browse_sections(json)
            .unwrap_or_default()
            .into_iter()
            .filter_map(Self::shelf_in)
            .collect();
        shelves.extend(Self::two_column_shelves(json));
        if let Some(continuation_contents) = as_dict(json.get("continuationContents")) {
            for key in ["musicPlaylistShelfContinuation", "musicShelfContinuation"] {
                if let Some(shelf) = as_dict(continuation_contents.get(key)) {
                    shelves.push(shelf);
                }
            }
        }
        shelves
    }

    /// First shelf-level continuation token, preferring `continuations[]`
    /// over trailing markers within each shelf.
    fn shelf_continuation_token<'j>(shelves: &[&'j JsonDict]) -> Option<&'j str> {
        shelves.iter().find_map(|shelf| {
            InnerTubeDecode::continuation_token(Some(shelf)).or_else(|| {
                Self::trailing_continuation_token(
                    &as_dict_list(shelf.get("contents")).unwrap_or_default(),
                )
            })
        })
    }

    /// Every song shelf from a twoColumnBrowseResultsRenderer response
    /// (secondaryContents → sectionListRenderer).
    fn two_column_shelves(json: &JsonDict) -> Vec<&JsonDict> {
        InnerTubeDecode::array(
            &[
                "contents",
                "twoColumnBrowseResultsRenderer",
                "secondaryContents",
                "sectionListRenderer",
                "contents",
            ],
            json,
        )
        .unwrap_or_default()
        .into_iter()
        .filter_map(Self::shelf_in)
        .collect()
    }

    /// Rows delivered outside any shelf: later continuation pages arrive as
    /// `onResponseReceivedActions[].appendContinuationItemsAction.continuationItems`.
    fn continuation_action_items(json: &JsonDict) -> Vec<&JsonDict> {
        let mut actions = as_dict_list(json.get("onResponseReceivedActions")).unwrap_or_default();
        actions.extend(as_dict_list(json.get("onResponseReceivedEndpoints")).unwrap_or_default());
        actions
            .into_iter()
            .flat_map(|action| {
                InnerTubeDecode::array(
                    &["appendContinuationItemsAction", "continuationItems"],
                    action,
                )
                .unwrap_or_default()
            })
            .collect()
    }

    /// Next-page token carried by a trailing `continuationItemRenderer`
    /// (`continuationEndpoint.continuationCommand.token`, occasionally nested
    /// in `commandExecutorCommand.commands[]`).
    fn trailing_continuation_token<'j>(items: &[&'j JsonDict]) -> Option<&'j str> {
        for item in items {
            let Some(endpoint) =
                InnerTubeDecode::dict(&["continuationItemRenderer", "continuationEndpoint"], item)
            else {
                continue;
            };
            if let Some(token) =
                InnerTubeDecode::string(&["continuationCommand", "token"], endpoint)
                && !token.is_empty()
            {
                return Some(token);
            }
            for command in InnerTubeDecode::array(&["commandExecutorCommand", "commands"], endpoint)
                .unwrap_or_default()
            {
                let continuation = as_dict(command.get("continuationCommand"));
                let token = as_string(continuation.and_then(|c| c.get("token")));
                let is_browse = as_string(continuation.and_then(|c| c.get("request")))
                    == Some("CONTINUATION_REQUEST_TYPE_BROWSE");
                if is_browse && let Some(token) = token.filter(|t| !t.is_empty()) {
                    return Some(token);
                }
            }
        }
        None
    }

    /// Drops `continuationItemRenderer` entries (page markers, not tracks).
    fn track_rows(items: Vec<&JsonDict>) -> Vec<&JsonDict> {
        items
            .into_iter()
            .filter(|i| i.get("continuationItemRenderer").is_none_or(Value::is_null))
            .collect()
    }
}
