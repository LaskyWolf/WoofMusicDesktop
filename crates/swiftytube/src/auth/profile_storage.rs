// Trop/Auth/ProfileStorage.swift. Here and not in trop: the sync state
// (`IncrementalSyncService`) lives in this crate and moves with the profile.
//
// Where each profile keeps its database, downloads and sync state. The first
// profile keeps the app's original folder, so its data never moves. The
// active profile is read once per run: switching profiles restarts the app.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// The original folder. Other profiles use a random id.
pub const ORIGINAL: &str = "";

/// `profiles.json` in the app's data folder; Swift keeps it in `UserDefaults`.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Registry {
    /// The storage the next launch opens.
    pub active: String,
    /// Account id → storage. A storage stays reserved after a sign-out, so
    /// signing back in finds the same library and downloads.
    pub owners: BTreeMap<String, String>,
    /// The account signed in now (Swift `AccountState.currentProfileId`).
    pub current: Option<String>,
    /// Accounts to switch back to, in the order they were added.
    pub profiles: Vec<SavedProfile>,
}

/// A Google sign-in kept on this computer. Its session sits in the keyring
/// under `profile.<id>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedProfile {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
}

/// `$XDG_DATA_HOME` (or `~/.local/share`) on Linux, `%APPDATA%` on Windows,
/// then `WoofMusic`.
pub fn base_dir() -> Option<PathBuf> {
    let env = |name| {
        std::env::var_os(name)
            .filter(|v: &std::ffi::OsString| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = env("XDG_DATA_HOME")
        .or_else(|| env("APPDATA"))
        .or_else(|| env("HOME").map(|home| home.join(".local").join("share")))?;
    Some(base.join("WoofMusic"))
}

fn registry_path() -> Option<PathBuf> {
    base_dir().map(|dir| dir.join("profiles.json"))
}

impl Registry {
    /// Empty when the file is missing or unreadable.
    pub fn load() -> Self {
        registry_path()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = registry_path().ok_or_else(|| std::io::Error::other("no data folder"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)
    }

    /// The account's storage, claimed for it if new.
    pub fn claim(&mut self, account_id: &str) -> String {
        let storage = storage_for(account_id, &self.owners, &self.active, new_storage_id());
        self.owners.insert(account_id.to_owned(), storage.clone());
        storage
    }

    /// The account seen before under any of `ids` (Google doesn't always send
    /// the same identifier).
    pub fn claimed<'a>(&self, ids: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
        ids.into_iter().find(|id| self.owners.contains_key(*id))
    }
}

/// A folder name no other profile has: the time in nanoseconds and the process.
fn new_storage_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{nanos:x}-{:x}", std::process::id())
}

/// A new account takes the active storage if no account owns it: that is the
/// data of the app before its first sign-in.
pub fn storage_for(
    account_id: &str,
    owners: &BTreeMap<String, String>,
    active: &str,
    new_storage: String,
) -> String {
    if let Some(owned) = owners.get(account_id) {
        return owned.clone();
    }
    if owners.values().any(|storage| storage == active) {
        new_storage
    } else {
        active.to_owned()
    }
}

/// The storage this run uses, read once.
pub fn active() -> &'static str {
    static ACTIVE: OnceLock<String> = OnceLock::new();
    ACTIVE.get_or_init(|| Registry::load().active)
}

/// The active profile's folder: the app's folder for the original profile,
/// `Profiles/<id>` inside it for the others.
pub fn dir() -> Option<PathBuf> {
    let base = base_dir()?;
    Some(match active() {
        ORIGINAL => base,
        storage => base.join("Profiles").join(storage),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_account_takes_the_original_data_then_others_get_new_folders() {
        let none = BTreeMap::new();
        assert_eq!(storage_for("a", &none, ORIGINAL, "new".into()), ORIGINAL);
        let a = BTreeMap::from([("a".to_owned(), ORIGINAL.to_owned())]);
        assert_eq!(storage_for("b", &a, ORIGINAL, "new".into()), "new");
        let both = BTreeMap::from([
            ("a".to_owned(), ORIGINAL.to_owned()),
            ("b".to_owned(), "new".to_owned()),
        ]);
        assert_eq!(storage_for("a", &both, "new", "other".into()), ORIGINAL);
    }

    #[test]
    fn claim_remembers_and_finds_accounts() {
        let mut registry = Registry::default();
        assert_eq!(registry.claim("a"), ORIGINAL);
        let second = registry.claim("b");
        assert_ne!(second, ORIGINAL);
        assert_eq!(registry.claim("b"), second);
        assert_eq!(registry.claimed(["x", "b"]), Some("b"));
        assert_eq!(registry.claimed(["x"]), None);
    }
}
