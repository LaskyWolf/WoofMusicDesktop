//! Trop/Services/SyncBridge.swift.

use std::path::PathBuf;
use std::sync::Arc;

use swiftytube::networking::inner_tube_client::InnerTubeClient;
use swiftytube::sync::incremental_sync_service::IncrementalSyncService;
use swiftytube::sync::library_sync_models::LibrarySyncOptions;
use swiftytube::sync::library_sync_service::LibrarySyncService;
use swiftytube::sync::mutation_service::MutationService;
use swiftytube::sync::playlist_detail_service::PlaylistDetailService;
use tokio::sync::Mutex;

use super::trop_library_store::TropLibraryStore;
use crate::database::database_service::DatabaseService;

/// The library store and the SwiftyTube sync services wired to it, plus the
/// one sync allowed to run at a time.
pub struct SyncBridge {
    pub client: Arc<InnerTubeClient>,
    pub store: Arc<TropLibraryStore>,
    pub mutations: MutationService,
    pub incremental: IncrementalSyncService,
    /// Held while a sync runs.
    running: Mutex<()>,
    /// Swift reads the sections to sync from `SettingsStore`; all on by default.
    options: std::sync::Mutex<LibrarySyncOptions>,
}

impl SyncBridge {
    /// `defaults_path` holds the last sync date of the active profile
    /// (`IncrementalSyncService::default_defaults_path()` for the original one).
    pub fn new(
        client: Arc<InnerTubeClient>,
        db: Arc<DatabaseService>,
        defaults_path: PathBuf,
    ) -> Self {
        let store = Arc::new(TropLibraryStore::new(db));
        let library_sync = LibrarySyncService::new(client.clone(), store.clone());
        Self {
            mutations: MutationService::new(client.clone(), store.clone()),
            incremental: IncrementalSyncService::new(client.clone(), library_sync, defaults_path),
            client,
            store,
            running: Mutex::new(()),
            options: std::sync::Mutex::new(LibrarySyncOptions::default()),
        }
    }

    /// Swift `SyncBridge.playlistDetail`; `fetch_playlist` takes `&*self.store`.
    pub fn playlist_detail(&self) -> PlaylistDetailService<'_> {
        PlaylistDetailService::new(&self.client)
    }

    pub fn options(&self) -> LibrarySyncOptions {
        *self.options.lock().unwrap()
    }

    /// The sections later syncs cover.
    pub fn set_options(&self, options: LibrarySyncOptions) {
        *self.options.lock().unwrap() = options;
    }

    // Swift also skips both syncs while the account changes and refreshes
    // `LikeStore` after them; the desktop app has neither yet.
    pub async fn check_and_sync_if_stale(&self) {
        self.share(
            self.incremental
                .check_and_sync_if_stale(self.options(), async {}),
        )
        .await;
    }

    pub async fn force_full_sync(&self) {
        self.share(self.incremental.force_full_sync(self.options(), async {}))
            .await;
    }

    /// Sign-in, Home and Library all ask for a sync at launch; they share the
    /// running one instead of each downloading the whole library.
    // ponytail: no `cancel_running` (Swift stops the sync before a profile
    // switch); add an abort handle when the desktop app can switch profiles.
    async fn share(&self, sync: impl Future<Output = ()>) {
        match self.running.try_lock() {
            Ok(_running) => sync.await,
            Err(_) => drop(self.running.lock().await),
        }
    }
}
