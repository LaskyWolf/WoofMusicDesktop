//! Trop/Services/PlaylistRemoval.swift.
//!
//! Takes a playlist out of the library. A synced playlist is deleted on
//! YouTube Music when you own it and un-saved when someone else does, then
//! forgotten locally. A local playlist is only forgotten locally.

use swiftytube::networking::inner_tube_error::InnerTubeError;
use swiftytube::sync::library_sync_models::SyncError;
use swiftytube::sync::library_sync_store::LibrarySyncStore;

use super::playlist_song_edits::PlaylistSongEdits;
use super::sync_bridge::SyncBridge;

#[derive(Debug, thiserror::Error)]
pub enum RemovalError {
    #[error("Sign in to remove a synced playlist.")]
    NotSignedIn,
    #[error(transparent)]
    Sync(#[from] SyncError),
}

#[derive(Clone, Copy)]
enum Call {
    Delete,
    Unsave,
}

pub struct PlaylistRemoval;

impl PlaylistRemoval {
    pub async fn remove(
        bridge: &SyncBridge,
        playlist_id: &str,
        owned: bool,
    ) -> Result<(), RemovalError> {
        let saved = bridge.store.fetch_playlist(playlist_id).ok().flatten();
        if saved.is_some_and(|p| p.browse_id.is_some()) {
            if bridge.client.current_auth().sapisid.is_none() {
                return Err(RemovalError::NotSignedIn);
            }
            // The library keeps the browse id (VL…); the endpoints want the bare playlist id.
            let bare_id = PlaylistSongEdits::bare_id(playlist_id);
            // The ownership flag can be stale, so a rejected call falls back to the other one.
            let calls = if owned {
                [Call::Delete, Call::Unsave]
            } else {
                [Call::Unsave, Call::Delete]
            };
            Self::send(bridge, bare_id, calls).await?;
        }
        bridge
            .store
            .delete_playlist_rows(playlist_id)
            .map_err(SyncError::from)?;
        Ok(())
    }

    async fn send(bridge: &SyncBridge, bare_id: &str, calls: [Call; 2]) -> Result<(), SyncError> {
        let mut first_rejection = None;
        for call in calls {
            let result = match call {
                Call::Delete => bridge.client.delete_playlist(bare_id, None, None).await,
                Call::Unsave => {
                    bridge
                        .client
                        .rate_playlist(bare_id, "INDIFFERENT", None, None)
                        .await
                }
            };
            match result {
                Ok(_) => return Ok(()),
                Err(
                    error @ InnerTubeError::HttpError {
                        status_code: 400..500,
                        ..
                    },
                ) => {
                    first_rejection.get_or_insert(error);
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(first_rejection
            .unwrap_or(InnerTubeError::InvalidResponse)
            .into())
    }
}
