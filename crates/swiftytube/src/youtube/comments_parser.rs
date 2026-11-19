//! `YouTube/CommentsParser.swift`: the comments of a WEB `/next` response,
//! from its continuation items and the `frameworkUpdates` entities.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::LazyLock;

use percent_encoding::percent_decode_str;
use regex::Regex;
use serde_json::Value;

use crate::networking::inner_tube_decode::{JsonDict, as_dict, as_dict_list, as_string};
use crate::youtube::comment_models::{
    CommentAuthor, CommentItem, CommentRepliesPage, CommentSort, CommentTextRun, CommentVoteState,
    CommentsPage,
};

/// Swift `enum CommentsParser` namespace.
pub struct CommentsParser;

struct Entities<'a> {
    comments: BTreeMap<String, &'a JsonDict>,
    toolbar_states: HashMap<String, &'a JsonDict>,
    toolbar_surfaces: HashMap<String, &'a JsonDict>,
}

static COMPACT_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([\d.,\s\x{00a0}]+)\s*([kKmM])?").expect("valid pattern"));

impl CommentsParser {
    // MARK: - Token discovery

    /// Pulls the first comments-section continuation from a WEB `/next` watch response.
    pub fn find_initial_comments_token(json: &JsonDict) -> Option<String> {
        let contents = dictionary(
            json,
            &[
                "contents",
                "twoColumnWatchNextResults",
                "results",
                "results",
            ],
        )
        .and_then(|results| as_dict_list(results.get("contents")))
        .unwrap_or_default();

        for item in contents {
            let Some(section) = as_dict(item.get("itemSectionRenderer")) else {
                continue;
            };
            let identifier = as_string(section.get("sectionIdentifier"))
                .or_else(|| as_string(section.get("targetId")))
                .unwrap_or("");
            if identifier != "comment-item-section" && identifier != "comments-section" {
                continue;
            }
            if let Some(token) = continuation_token(section.get("contents")) {
                return Some(token);
            }
        }

        // Music-host / alternate layouts: walk the tree for a comments section id.
        deep_find_in_dict(json, 0)
    }

    // MARK: - Pages

    pub fn parse_comments_page(json: &JsonDict) -> CommentsPage {
        let entities = build_entities(json);
        let mut page = CommentsPage::default();

        for action in collect_actions(json) {
            if let Some(thread) = as_dict(action.get("commentThreadRenderer")) {
                let view_model = view_model(thread);
                let comment_id = view_model.and_then(|vm| as_string(vm.get("commentId")));
                let entity = comment_id.and_then(|id| entities.comments.get(id).copied());
                let Some(mut item) = make_comment(entity, view_model, &entities, true) else {
                    continue;
                };

                let reply_continuation = dictionary(thread, &["replies", "commentRepliesRenderer"])
                    .and_then(|replies| as_dict_list(replies.get("contents")))
                    .and_then(|contents| contents.into_iter().find_map(continuation_token_dict));

                let pinned = view_model.and_then(|vm| as_string(vm.get("pinnedText")));
                item.is_pinned = pinned.is_some_and(|text| !text.is_empty());
                item.pinned_text = pinned.map(str::to_owned);
                let toolbar = entity.and_then(|e| as_dict(e.get("toolbar")));
                let reply_count = toolbar.and_then(|t| t.get("replyCount"));
                item.reply_count = parse_compact_number(reply_count);
                item.reply_count_text = string_value(reply_count).unwrap_or_default();
                item.reply_continuation = reply_continuation;
                page.items.push(item);
            } else if let Some(token) = continuation_token_dict(action) {
                page.next_page_token = Some(token);
            } else if let Some(header) = as_dict(action.get("commentsHeaderRenderer")) {
                page.total_count_text = extract_count_text(header).or(page.total_count_text);
                page.create_comment_params =
                    simplebox_create_params(header).or(page.create_comment_params);
                if let Some(sub_items) =
                    dictionary(header, &["sortMenu", "sortFilterSubMenuRenderer"])
                        .and_then(|menu| as_dict_list(menu.get("subMenuItems")))
                {
                    if let Some(top) = sub_items
                        .first()
                        .and_then(|i| continuation_command_token(i))
                    {
                        page.sort_tokens.insert(CommentSort::Top, top);
                    }
                    if let Some(newest) =
                        sub_items.get(1).and_then(|i| continuation_command_token(i))
                    {
                        page.sort_tokens.insert(CommentSort::Newest, newest);
                    }
                }
            }
        }
        page
    }

    pub fn parse_replies_page(json: &JsonDict) -> CommentRepliesPage {
        let entities = build_entities(json);
        let mut items: Vec<CommentItem> = Vec::new();
        let mut seen = HashSet::new();
        let mut next_page_token = None;

        let mut push = |view_model: Option<&JsonDict>, items: &mut Vec<CommentItem>| {
            let Some(id) = view_model.and_then(|vm| as_string(vm.get("commentId"))) else {
                return;
            };
            if seen.contains(id) {
                return;
            }
            let entity = entities.comments.get(id).copied();
            let Some(mut reply) = make_comment(entity, view_model, &entities, false) else {
                return;
            };
            reply.reply_count = 0;
            reply.reply_count_text = String::new();
            reply.reply_continuation = None;
            seen.insert(id.to_owned());
            items.push(reply);
        };

        for action in collect_actions(json) {
            if action.get("commentViewModel").is_some() {
                push(view_model(action), &mut items);
            } else if let Some(thread) = as_dict(action.get("commentThreadRenderer")) {
                push(view_model(thread), &mut items);
            } else if let Some(token) = continuation_token_dict(action) {
                next_page_token = Some(token);
            }
        }

        if items.is_empty() {
            for entity in entities.comments.values() {
                let props = as_dict(entity.get("properties"));
                let level = props
                    .and_then(|p| p.get("replyLevel"))
                    .and_then(|level| {
                        level
                            .as_i64()
                            .or_else(|| string_value(Some(level))?.parse().ok())
                    })
                    .unwrap_or(0);
                let Some(id) = props.and_then(|p| as_string(p.get("commentId"))) else {
                    continue;
                };
                if level < 1 {
                    continue;
                }
                let mut stub = JsonDict::new();
                stub.insert("commentId".into(), id.into());
                push(Some(&stub), &mut items);
            }
        }

        CommentRepliesPage {
            items,
            next_page_token,
        }
    }

    pub fn plain_text(runs: &[CommentTextRun]) -> String {
        runs.iter()
            .map(|run| match run {
                CommentTextRun::Text(text)
                | CommentTextRun::Link { text, .. }
                | CommentTextRun::Timestamp { text, .. } => text.as_str(),
            })
            .collect()
    }
}

// MARK: - Entities

fn build_entities(json: &JsonDict) -> Entities<'_> {
    let mutations = dictionary(json, &["frameworkUpdates", "entityBatchUpdate"])
        .and_then(|batch| as_dict_list(batch.get("mutations")))
        .unwrap_or_default();
    let mut entities = Entities {
        comments: BTreeMap::new(),
        toolbar_states: HashMap::new(),
        toolbar_surfaces: HashMap::new(),
    };
    for mutation in mutations {
        let Some(payload) = as_dict(mutation.get("payload")) else {
            continue;
        };
        let key = |entity: &JsonDict| {
            as_string(entity.get("key"))
                .or_else(|| as_string(mutation.get("entityKey")))
                .map(str::to_owned)
        };
        if let Some(comment) = as_dict(payload.get("commentEntityPayload"))
            && let Some(id) = dictionary(comment, &["properties"])
                .and_then(|props| as_string(props.get("commentId")))
        {
            entities.comments.insert(id.to_owned(), comment);
        }
        if let Some(toolbar) = as_dict(payload.get("engagementToolbarStateEntityPayload"))
            && let Some(key) = key(toolbar)
        {
            entities.toolbar_states.insert(key, toolbar);
        }
        if let Some(surface) = as_dict(payload.get("engagementToolbarSurfaceEntityPayload"))
            && let Some(key) = key(surface)
        {
            entities.toolbar_surfaces.insert(key, surface);
        }
    }
    entities
}

fn collect_actions(json: &JsonDict) -> Vec<&JsonDict> {
    let endpoints = as_dict_list(json.get("onResponseReceivedEndpoints")).unwrap_or_default();
    let mut out = Vec::new();
    for endpoint in endpoints {
        for key in [
            "appendContinuationItemsAction",
            "reloadContinuationItemsCommand",
        ] {
            if let Some(items) = dictionary(endpoint, &[key])
                .and_then(|action| as_dict_list(action.get("continuationItems")))
            {
                out.extend(items);
            }
        }
    }
    out
}

fn view_model(node: &JsonDict) -> Option<&JsonDict> {
    dictionary(node, &["commentViewModel", "commentViewModel"])
        .or_else(|| as_dict(node.get("commentViewModel")))
}

fn make_comment(
    entity: Option<&JsonDict>,
    view_model: Option<&JsonDict>,
    entities: &Entities,
    is_thread: bool,
) -> Option<CommentItem> {
    let entity = entity?;
    let props = as_dict(entity.get("properties"))?;
    let id = as_string(props.get("commentId"))?;
    let content = as_dict(props.get("content"))?;
    as_string(content.get("content"))?;

    let empty = JsonDict::new();
    let toolbar = as_dict(entity.get("toolbar")).unwrap_or(&empty);
    let state_key = view_model
        .and_then(|vm| as_string(vm.get("toolbarStateKey")))
        .or_else(|| as_string(props.get("toolbarStateKey")));
    let state = state_key.and_then(|key| entities.toolbar_states.get(key).copied());
    let vote_state = vote_state(state.and_then(|s| as_string(s.get("likeState"))));
    let like_text = string_value(if vote_state == CommentVoteState::Liked {
        toolbar.get("likeCountLiked")
    } else {
        toolbar.get("likeCountNotliked")
    })
    .or_else(|| string_value(toolbar.get("likeCountNotliked")))
    .or_else(|| string_value(toolbar.get("likeCountLiked")))
    .unwrap_or_default();

    let surface = view_model
        .and_then(|vm| as_string(vm.get("toolbarSurfaceKey")))
        .and_then(|key| entities.toolbar_surfaces.get(key).copied());

    Some(CommentItem {
        id: id.to_owned(),
        author: make_author(entity),
        runs: to_runs(content),
        published_time: string_value(props.get("publishedTime")).unwrap_or_default(),
        like_count_text: like_text,
        is_hearted: state.and_then(|s| as_string(s.get("heartState")))
            == Some("TOOLBAR_HEART_STATE_HEARTED"),
        is_pinned: false,
        pinned_text: None,
        reply_count: if is_thread {
            parse_compact_number(toolbar.get("replyCount"))
        } else {
            0
        },
        reply_count_text: if is_thread {
            string_value(toolbar.get("replyCount")).unwrap_or_default()
        } else {
            String::new()
        },
        reply_continuation: None,
        vote_state,
        like_action: action_token(surface, "likeCommand"),
        unlike_action: action_token(surface, "unlikeCommand"),
        dislike_action: action_token(surface, "dislikeCommand"),
        undislike_action: action_token(surface, "undislikeCommand"),
        delete_action: delete_action(surface, entity),
        edit_params: edit_params(surface, entity),
    })
}

fn vote_state(like_state: Option<&str>) -> CommentVoteState {
    match like_state {
        Some("TOOLBAR_LIKE_STATE_LIKED") => CommentVoteState::Liked,
        Some("TOOLBAR_LIKE_STATE_DISLIKED") => CommentVoteState::Disliked,
        _ => CommentVoteState::Indifferent,
    }
}

fn action_token(surface: Option<&JsonDict>, command: &str) -> Option<String> {
    dictionary(
        surface?,
        &[command, "innertubeCommand", "performCommentActionEndpoint"],
    )
    .and_then(|endpoint| as_string(endpoint.get("action")))
    .map(str::to_owned)
}

/// The menu items of the toolbar surface, then of the entity's own menus.
fn menu_item_lists<'a>(
    surface: Option<&'a JsonDict>,
    entity: &'a JsonDict,
) -> Vec<Vec<&'a JsonDict>> {
    let mut lists = Vec::new();
    if let Some(items) = surface
        .and_then(|s| {
            dictionary(
                s,
                &[
                    "menuCommand",
                    "innertubeCommand",
                    "menuEndpoint",
                    "menu",
                    "menuRenderer",
                ],
            )
        })
        .and_then(|menu| as_dict_list(menu.get("items")))
    {
        lists.push(items);
    }
    for key in ["actionMenu", "menu"] {
        if let Some(items) = dictionary(entity, &[key, "menuRenderer"])
            .and_then(|menu| as_dict_list(menu.get("items")))
        {
            lists.push(items);
        }
    }
    lists
}

fn delete_action(surface: Option<&JsonDict>, entity: &JsonDict) -> Option<String> {
    menu_item_lists(surface, entity)
        .iter()
        .find_map(|items| delete_token(items))
}

fn edit_params(surface: Option<&JsonDict>, entity: &JsonDict) -> Option<String> {
    menu_item_lists(surface, entity)
        .iter()
        .find_map(|items| edit_token(items))
}

fn edit_token(items: &[&JsonDict]) -> Option<String> {
    items.iter().find_map(|item| {
        let renderer = as_dict(item.get("menuNavigationItemRenderer"))
            .or_else(|| as_dict(item.get("menuServiceItemRenderer")))?;
        let endpoint = as_dict(renderer.get("navigationEndpoint"))
            .or_else(|| as_dict(renderer.get("serviceEndpoint")))?;
        let dialog = dictionary(
            endpoint,
            &[
                "updateCommentDialogEndpoint",
                "dialog",
                "commentDialogRenderer",
            ],
        )?;
        let submit = dictionary(
            dialog,
            &["submitButton", "buttonRenderer", "serviceEndpoint"],
        )?;
        dictionary(submit, &["updateCommentEndpoint"])
            .and_then(|update| as_string(update.get("updateCommentParams")))
            .map(str::to_owned)
    })
}

fn delete_token(items: &[&JsonDict]) -> Option<String> {
    let mut fallback = None;
    for item in items {
        let Some(renderer) = as_dict(item.get("menuServiceItemRenderer"))
            .or_else(|| as_dict(item.get("menuNavigationItemRenderer")))
        else {
            continue;
        };
        let Some(endpoint) = as_dict(renderer.get("serviceEndpoint"))
            .or_else(|| as_dict(renderer.get("navigationEndpoint")))
        else {
            continue;
        };
        let Some(token) = menu_action_token(endpoint) else {
            continue;
        };
        let icon = dictionary(renderer, &["icon"]).and_then(|i| as_string(i.get("iconType")));
        let title: String = dictionary(renderer, &["text"])
            .and_then(|t| as_dict_list(t.get("runs")))
            .map(|runs| {
                runs.iter()
                    .filter_map(|r| as_string(r.get("text")))
                    .collect()
            })
            .unwrap_or_default();
        if icon == Some("DELETE") || title == "Delete" {
            return Some(token);
        }
        fallback.get_or_insert(token);
    }
    fallback
}

fn menu_action_token(endpoint: &JsonDict) -> Option<String> {
    if let Some(action) = dictionary(endpoint, &["performCommentActionEndpoint"])
        .and_then(|e| as_string(e.get("action")))
    {
        return Some(action.to_owned());
    }
    let confirm = as_dict(endpoint.get("confirmDialogEndpoint"))?;
    let dialog = dictionary(confirm, &["content", "confirmDialogRenderer"])
        .or_else(|| dictionary(confirm, &["dialog", "confirmDialogRenderer"]))?;
    let confirm_button = as_dict(dialog.get("confirmButton"))?;
    let target = as_dict(confirm_button.get("serviceEndpoint"))
        .or_else(|| dictionary(confirm_button, &["buttonRenderer", "serviceEndpoint"]))?;
    dictionary(target, &["performCommentActionEndpoint"])
        .and_then(|e| as_string(e.get("action")))
        .map(str::to_owned)
}

fn make_author(entity: &JsonDict) -> CommentAuthor {
    let empty = JsonDict::new();
    let author = as_dict(entity.get("author")).unwrap_or(&empty);
    let channel_path = match as_string(author.get("channelId")) {
        Some(id) if !id.is_empty() => Some(format!("/channel/{id}")),
        _ => dictionary(
            author,
            &[
                "channelCommand",
                "innertubeCommand",
                "commandMetadata",
                "webCommandMetadata",
            ],
        )
        .and_then(|meta| as_string(meta.get("url")))
        .map(str::to_owned),
    };
    let flag = |key: &str| author.get(key).and_then(Value::as_bool).unwrap_or(false);
    CommentAuthor {
        name: string_value(author.get("displayName")).unwrap_or_default(),
        avatar_url: string_value(author.get("avatarThumbnailUrl")),
        channel_url: channel_path.map(|path| absolute_youtube_url(&path)),
        is_creator: flag("isCreator"),
        is_verified: flag("isVerified"),
        is_artist: flag("isArtist"),
    }
}

// MARK: - Text runs

fn to_runs(content: &JsonDict) -> Vec<CommentTextRun> {
    let text = string_value(content.get("content")).unwrap_or_default();
    let commands = as_dict_list(content.get("commandRuns")).unwrap_or_default();
    if commands.is_empty() {
        return if text.is_empty() {
            Vec::new()
        } else {
            vec![CommentTextRun::Text(text)]
        };
    }

    let mut sorted: Vec<(i64, i64, &JsonDict)> = commands
        .into_iter()
        .filter_map(|command| {
            let start = command.get("startIndex")?.as_i64()?;
            let length = command.get("length")?.as_i64()?;
            Some((start, length, command))
        })
        .collect();
    sorted.sort_by_key(|(start, _, _)| *start);

    // Indexes count UTF-16 units, as in JavaScript.
    let utf16: Vec<u16> = text.encode_utf16().collect();
    let count = utf16.len() as i64;
    let mut runs = Vec::new();
    let mut cursor = 0_i64;
    for (start, length, command) in sorted {
        if start < cursor || start > count || length < 0 || length > count - start {
            continue;
        }
        if start > cursor {
            runs.push(CommentTextRun::Text(utf16_string(
                &utf16[cursor as usize..start as usize],
            )));
        }
        let end = (start + length) as usize;
        let slice = utf16_string(&utf16[start as usize..end]);
        let innertube = dictionary(command, &["onTap", "innertubeCommand"]);
        runs.push(run_from_command(slice, innertube));
        cursor = start + length;
    }
    if cursor < count {
        runs.push(CommentTextRun::Text(utf16_string(
            &utf16[cursor as usize..],
        )));
    }
    runs.retain(|run| !matches!(run, CommentTextRun::Text(text) if text.is_empty()));
    runs
}

fn run_from_command(text: String, command: Option<&JsonDict>) -> CommentTextRun {
    if let Some(watch) = command.and_then(|c| as_dict(c.get("watchEndpoint")))
        && let Some(seconds) = watch.get("startTimeSeconds").and_then(Value::as_f64)
    {
        return CommentTextRun::Timestamp {
            text,
            seconds,
            video_id: as_string(watch.get("videoId")).map(str::to_owned),
        };
    }
    let url = command.and_then(|c| {
        dictionary(c, &["urlEndpoint"])
            .and_then(|e| as_string(e.get("url")))
            .or_else(|| {
                dictionary(c, &["commandMetadata", "webCommandMetadata"])
                    .and_then(|m| as_string(m.get("url")))
            })
    });
    match url {
        Some(url) => CommentTextRun::Link {
            text,
            url: unwrap_redirect(&absolute_youtube_url(url)),
        },
        None => CommentTextRun::Text(text),
    }
}

// MARK: - Helpers

fn continuation_token_dict(dict: &JsonDict) -> Option<String> {
    dictionary(
        dict,
        &[
            "continuationItemRenderer",
            "continuationEndpoint",
            "continuationCommand",
        ],
    )
    .or_else(|| {
        dictionary(
            dict,
            &[
                "continuationItemRenderer",
                "button",
                "buttonRenderer",
                "command",
                "continuationCommand",
            ],
        )
    })
    .and_then(|command| as_string(command.get("token")))
    .map(str::to_owned)
}

/// A dict's token, or the first one in an array of dicts.
fn continuation_token(node: Option<&Value>) -> Option<String> {
    match node? {
        Value::Object(dict) => continuation_token_dict(dict),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_object)
            .find_map(continuation_token_dict),
        _ => None,
    }
}

fn continuation_command_token(item: &JsonDict) -> Option<String> {
    dictionary(item, &["serviceEndpoint", "continuationCommand"])
        .and_then(|command| as_string(command.get("token")))
        .map(str::to_owned)
}

fn simplebox_create_params(header: &JsonDict) -> Option<String> {
    dictionary(
        header,
        &[
            "createRenderer",
            "commentSimpleboxRenderer",
            "submitButton",
            "buttonRenderer",
            "serviceEndpoint",
            "createCommentEndpoint",
        ],
    )
    .and_then(|endpoint| as_string(endpoint.get("createCommentParams")))
    .map(str::to_owned)
}

fn extract_count_text(header: &JsonDict) -> Option<String> {
    if let Some(runs) = dictionary(header, &["countText"]).and_then(|c| as_dict_list(c.get("runs")))
        && let Some(found) = runs
            .iter()
            .filter_map(|run| string_value(run.get("text")))
            .map(|text| text.trim().to_owned())
            .find(|text| text.chars().any(|c| c.is_numeric()))
    {
        return Some(found);
    }
    text_of(header.get("commentsCount")).or_else(|| text_of(header.get("countText")))
}

fn text_of(node: Option<&Value>) -> Option<String> {
    let dict = as_dict(node)?;
    if let Some(simple) = as_string(dict.get("simpleText")) {
        return Some(simple.to_owned());
    }
    let runs = as_dict_list(dict.get("runs"))?;
    Some(
        runs.iter()
            .filter_map(|run| string_value(run.get("text")))
            .collect(),
    )
}

/// "3" → 3, "1.2K" → 1200, "2,661" → 2661.
pub(crate) fn parse_compact_number(value: Option<&Value>) -> i64 {
    let raw = string_value(value).unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return 0;
    }
    let Some(captures) = COMPACT_NUMBER.captures(raw) else {
        return raw
            .chars()
            .filter(|c| c.is_numeric())
            .collect::<String>()
            .parse()
            .unwrap_or(0);
    };
    let digits: String = captures[1]
        .chars()
        .filter(|c| *c != '\u{00a0}' && *c != ' ')
        .collect();
    let suffix = captures.get(2).map(|m| m.as_str().to_ascii_lowercase());
    let number: f64 = if suffix.is_some() {
        digits.replace(',', ".").parse().unwrap_or(0.0)
    } else {
        digits.replace([',', '.'], "").parse().unwrap_or(0.0)
    };
    if !number.is_finite() {
        return 0;
    }
    let multiplier = match suffix.as_deref() {
        Some("k") => 1_000.0,
        Some("m") => 1_000_000.0,
        _ => 1.0,
    };
    (number * multiplier).round() as i64
}

fn absolute_youtube_url(path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_owned();
    }
    let slash = if path.starts_with('/') { "" } else { "/" };
    format!("https://www.youtube.com{slash}{path}")
}

/// `https://www.youtube.com/redirect?q=<target>` → the target.
fn unwrap_redirect(url: &str) -> String {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    let Some(rest) = rest else {
        return url.to_owned();
    };
    let (host, path_and_query) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let (path, query) = path_and_query
        .split_once('?')
        .unwrap_or((path_and_query, ""));
    if !host.ends_with("youtube.com") || path != "/redirect" {
        return url.to_owned();
    }
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("q="))
        .map(|target| {
            percent_decode_str(&target.replace('+', " "))
                .decode_utf8_lossy()
                .into_owned()
        })
        .unwrap_or_else(|| url.to_owned())
}

fn deep_find_comments_token(value: &Value, depth: usize) -> Option<String> {
    if depth >= 12 {
        return None;
    }
    match value {
        Value::Object(dict) => deep_find_in_dict(dict, depth),
        Value::Array(items) => items
            .iter()
            .find_map(|nested| deep_find_comments_token(nested, depth + 1)),
        _ => None,
    }
}

fn deep_find_in_dict(dict: &JsonDict, depth: usize) -> Option<String> {
    let identifier =
        as_string(dict.get("sectionIdentifier")).or_else(|| as_string(dict.get("targetId")));
    if matches!(
        identifier,
        Some("comment-item-section" | "comments-section")
    ) && let Some(token) = continuation_token(dict.get("contents"))
    {
        return Some(token);
    }
    dict.values()
        .find_map(|nested| deep_find_comments_token(nested, depth + 1))
}

fn dictionary<'a>(root: &'a JsonDict, path: &[&str]) -> Option<&'a JsonDict> {
    let mut current = root;
    for key in path {
        current = as_dict(current.get(*key))?;
    }
    Some(current)
}

/// Swift `stringValue`: a string, or a number written out.
fn string_value(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(string) => Some(string.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn utf16_string(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}
