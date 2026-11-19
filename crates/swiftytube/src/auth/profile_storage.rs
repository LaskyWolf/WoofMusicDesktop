// Trop/Auth/ProfileStorage.swift. Here and not in trop: the sync state
// (`IncrementalSyncService`) lives in this crate and moves with the profile.
//
// Where each profile keeps its database, downloads and sync state. The first
// profile keeps the app's original folder, so its data never moves. The
// active profile is read once per run: switching profiles restarts the app.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
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
            .map(|path| Self::load_from(&path))
            .unwrap_or_default()
    }

    /// A file that doesn't parse is moved to `profiles.json.bak` before the
    /// empty registry replaces it on the next save: the account-to-folder
    /// map in it is the only thing that finds each account's library again.
    fn load_from(path: &Path) -> Self {
        let Ok(bytes) = std::fs::read(path) else {
            return Self::default();
        };
        let mut registry: Self = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            log::error!("profiles.json is unreadable, kept as profiles.json.bak: {error}");
            let _ = std::fs::rename(path, path.with_extension("json.bak"));
            Self::default()
        });
        // An edited file can list an account twice; the list shows it once.
        let mut seen = std::collections::HashSet::new();
        registry.profiles.retain(|p| seen.insert(p.id.clone()));
        registry
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = registry_path().ok_or_else(|| std::io::Error::other("no data folder"))?;
        self.save_to(&path)
    }

    /// Written to a file of its own and renamed over `path`, so a crash or a
    /// second process never leaves half a file.
    fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension(format!(
            "json.{}.{:?}.tmp",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&temp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&temp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&temp);
        })
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
    Some(storage_dir(&base_dir()?, active()))
}

/// `storage` comes from a file anyone can edit: only a plain folder name may
/// go below `Profiles`. `..`, `/x` and `a/b` fall back to the original folder.
fn storage_dir(base: &Path, storage: &str) -> PathBuf {
    let mut parts = Path::new(storage).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(name)), None) => base.join("Profiles").join(name),
        _ => base.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("woof-profiles-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn storage_names_cannot_leave_the_profiles_folder() {
        let base = Path::new("/data/WoofMusic");
        assert_eq!(storage_dir(base, ""), base);
        assert_eq!(storage_dir(base, "abc-1"), base.join("Profiles/abc-1"));
        for bad in ["..", "../x", "/etc", "a/b", "../../.ssh", "."] {
            assert_eq!(storage_dir(base, bad), base, "{bad}");
        }
    }

    #[test]
    fn unreadable_registry_is_kept_aside_and_loads_empty() {
        let dir = scratch("corrupt");
        let path = dir.join("profiles.json");
        std::fs::write(&path, b"{ not json").unwrap();
        assert_eq!(Registry::load_from(&path), Registry::default());
        assert_eq!(
            std::fs::read(dir.join("profiles.json.bak")).unwrap(),
            b"{ not json"
        );
        assert_eq!(
            Registry::load_from(&dir.join("missing.json")),
            Registry::default()
        );
    }

    #[test]
    fn save_replaces_the_file_whole_and_leaves_no_temp_files() {
        let dir = scratch("save");
        let path = dir.join("profiles.json");
        let mut registry = Registry::default();
        registry.claim("müller 日本 ✓");
        registry.save_to(&path).unwrap();
        assert_eq!(Registry::load_from(&path), registry);
        let files = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(files, 1);
    }

    #[test]
    fn an_account_listed_twice_loads_once() {
        let dir = scratch("dupes");
        let path = dir.join("profiles.json");
        let entry = r#"{"id":"a","name":"A"}"#;
        let json = format!(r#"{{"profiles":[{entry},{entry},{{"id":"b","name":"B"}}]}}"#);
        std::fs::write(&path, json).unwrap();
        let ids: Vec<_> = Registry::load_from(&path)
            .profiles
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["a", "b"]);
    }

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
