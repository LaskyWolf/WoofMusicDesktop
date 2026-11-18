//! Trop/Activity/ActivityView.swift, the data half: `ActivityItem`,
//! `ActivityJSON` and the seen list of `ActivityStore`.
//!
//! Swift keeps the seen ids in `UserDefaults`; here they are a JSON array in
//! `activity_defaults.json`, next to the other defaults files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use swiftytube::sync::incremental_sync_service::IncrementalSyncService;

/// One entry of Activity: a new release from a followed artist.
#[derive(Clone, Debug, PartialEq)]
pub struct ActivityItem {
    pub id: String,
    pub message: String,
    pub time: Option<String>,
    pub thumbnail: Option<String>,
    /// The album page it opens (Swift `DetailRoute.album`).
    pub album_id: Option<String>,
    pub is_read: bool,
}

/// Unread entries, for the sidebar badge (Swift `ActivityStore.unseen`).
pub fn unseen(items: &[ActivityItem]) -> usize {
    items.iter().filter(|item| !item.is_read).count()
}

/// The ids already opened or shown, newest last.
pub struct SeenIds;

impl SeenIds {
    const LIMIT: usize = 500;

    pub fn default_path() -> Option<PathBuf> {
        IncrementalSyncService::default_defaults_path()
            .map(|path| path.with_file_name("activity_defaults.json"))
    }

    /// Empty when the file is missing or unreadable.
    pub fn load(path: &Path) -> Vec<String> {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Adds `ids` to the saved list, keeping the newest `LIMIT`. Blocking.
    pub fn save(path: &Path, ids: &HashSet<String>) {
        if ids.is_empty() {
            return;
        }
        let mut saved = Self::load(path);
        saved.retain(|id| !ids.contains(id));
        let mut added: Vec<String> = ids.iter().cloned().collect();
        added.sort();
        saved.extend(added);
        let start = saved.len().saturating_sub(Self::LIMIT);
        let written = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(path, serde_json::to_vec(&saved[start..])?));
        if let Err(error) = written {
            log::warn!("Activity seen list not saved: {error}");
        }
    }
}

/// Swift `ActivityJSON`: reading what YouTube answers, which nests the same
/// few shapes at different depths.
pub struct ActivityJson;

impl ActivityJson {
    pub fn text(node: Option<&Value>) -> Option<String> {
        let dict = node?.as_object()?;
        if let Some(simple) = dict.get("simpleText").and_then(Value::as_str) {
            return Some(simple.to_owned());
        }
        let runs = dict.get("runs")?.as_array()?;
        Some(
            runs.iter()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .collect(),
        )
    }

    pub fn last_thumbnail(node: Option<&Value>) -> Option<String> {
        node?
            .get("thumbnails")?
            .as_array()?
            .last()?
            .get("url")?
            .as_str()
            .map(str::to_owned)
    }

    /// The first value stored under `key`, at any depth.
    pub fn first<'a>(key: &str, node: Option<&'a Value>) -> Option<&'a Value> {
        match node? {
            Value::Object(dict) => dict.get(key).or_else(|| {
                dict.values()
                    .find_map(|value| Self::first(key, Some(value)))
            }),
            Value::Array(array) => array.iter().find_map(|value| Self::first(key, Some(value))),
            _ => None,
        }
    }

    /// Every object stored under `key`, at any depth (not inside a match).
    pub fn nodes<'a>(key: &str, node: Option<&'a Value>) -> Vec<&'a Map<String, Value>> {
        match node {
            Some(Value::Object(dict)) => match dict.get(key).and_then(Value::as_object) {
                Some(found) => vec![found],
                None => dict
                    .values()
                    .flat_map(|value| Self::nodes(key, Some(value)))
                    .collect(),
            },
            Some(Value::Array(array)) => array
                .iter()
                .flat_map(|value| Self::nodes(key, Some(value)))
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn json_helpers_read_nested_shapes() {
        let json = json!({ "a": [{ "b": { "title": { "runs": [{ "text": "Da" }, { "text": "ft" }] } } }] });
        let b = ActivityJson::nodes("b", Some(&json));
        assert_eq!(b.len(), 1);
        assert_eq!(
            ActivityJson::text(b[0].get("title")).as_deref(),
            Some("Daft")
        );
        assert_eq!(ActivityJson::first("text", Some(&json)), Some(&json!("Da")));
        let thumbs = json!({ "thumbnails": [{ "url": "small" }, { "url": "big" }] });
        assert_eq!(
            ActivityJson::last_thumbnail(Some(&thumbs)).as_deref(),
            Some("big")
        );
    }

    #[test]
    fn seen_ids_are_saved() {
        let dir = std::env::temp_dir().join(format!("woof-activity-{}", std::process::id()));
        let path = dir.join("activity_defaults.json");
        SeenIds::save(&path, &HashSet::from(["a".to_owned()]));
        SeenIds::save(&path, &HashSet::from(["b".to_owned()]));
        SeenIds::save(&path, &HashSet::from(["a".to_owned()]));
        // Seen again: moves to the end, once.
        assert_eq!(SeenIds::load(&path), ["b", "a"]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
