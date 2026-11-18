// SwiftyTube/Sync/IncrementalSyncService.swift. `UserDefaults` becomes a
// JSON object file (`defaults_path`), keyed like the Swift defaults.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::library_sync_models::LibrarySyncOptions;
use super::library_sync_service::LibrarySyncService;
use super::library_sync_store::JsonLibrarySyncStore;
use crate::core::swiftytube_log::SwiftyTubeLog;
use crate::networking::inner_tube_client::InnerTubeClient;
use crate::networking::inner_tube_decode::JsonDict;

pub struct IncrementalSyncService {
    client: Arc<InnerTubeClient>,
    library_sync: LibrarySyncService,
    defaults_path: PathBuf,
}

impl IncrementalSyncService {
    const LAST_SYNC_KEY: &str = "lastSyncTimestamp";
    const SYNC_THRESHOLD: Duration = Duration::from_secs(30 * 60);

    pub fn new(
        client: Arc<InnerTubeClient>,
        library_sync: LibrarySyncService,
        defaults_path: PathBuf,
    ) -> Self {
        Self {
            client,
            library_sync,
            defaults_path,
        }
    }

    /// Swift `UserDefaults.standard`: `sync_defaults.json` in the app data directory.
    pub fn default_defaults_path() -> Option<PathBuf> {
        Some(JsonLibrarySyncStore::default_directory()?.join("sync_defaults.json"))
    }

    /// Seconds since 1970, stored as a JSON number.
    pub fn last_sync_date(&self) -> Option<SystemTime> {
        let seconds = self.defaults().get(Self::LAST_SYNC_KEY)?.as_f64()?;
        UNIX_EPOCH.checked_add(Duration::try_from_secs_f64(seconds).ok()?)
    }

    /// Swift defaults: `LibrarySyncOptions::default()`, no likes callback (`async {}`).
    pub async fn check_and_sync_if_stale(
        &self,
        options: LibrarySyncOptions,
        on_likes_refresh: impl Future<Output = ()>,
    ) {
        let last_sync = self.last_sync_date().unwrap_or(UNIX_EPOCH);
        let elapsed = SystemTime::now()
            .duration_since(last_sync)
            .unwrap_or_default();
        if elapsed < Self::SYNC_THRESHOLD {
            return;
        }
        self.force_full_sync(options, on_likes_refresh).await;
    }

    pub async fn force_full_sync(
        &self,
        options: LibrarySyncOptions,
        on_likes_refresh: impl Future<Output = ()>,
    ) {
        let login_verified = match self.client.verify_session(None, None).await {
            Ok(verified) => verified,
            Err(error) => {
                SwiftyTubeLog::error(&format!(
                    "Library sync skipped: session check failed: {error:?} ({error})"
                ));
                false
            }
        };
        if !login_verified {
            SwiftyTubeLog::error("Library sync skipped: not signed in");
            return;
        }

        let result = self.library_sync.sync_all(options, on_likes_refresh).await;
        if options.enabled_count() == 0 || result.completed_sections < options.enabled_count() {
            return;
        }
        self.set_last_sync_date(SystemTime::now());
    }

    /// The defaults file; missing or unreadable reads as empty, like an unset key.
    fn defaults(&self) -> JsonDict {
        std::fs::read(&self.defaults_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn set_last_sync_date(&self, date: SystemTime) {
        let seconds = date.duration_since(UNIX_EPOCH).unwrap_or_default();
        let mut defaults = self.defaults();
        defaults.insert(Self::LAST_SYNC_KEY.into(), seconds.as_secs_f64().into());
        let saved = self
            .defaults_path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&self.defaults_path, serde_json::to_vec(&defaults)?));
        if let Err(error) = saved {
            SwiftyTubeLog::error(&format!("Saving the last sync date failed: {error}"));
        }
    }
}
