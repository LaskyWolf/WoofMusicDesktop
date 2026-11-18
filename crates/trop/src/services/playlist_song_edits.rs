//! Trop/Services/PlaylistSongEdits.swift, plus the desktop's rename and
//! reorder (Rust-only, marked below).
//!
//! `browse/edit_playlist` wants the bare playlist id, but library rows store
//! the browse id (VL…), and `MutationService` forwards that id unchanged.
//! Synced playlists therefore go straight to `edit_playlist` with the prefix
//! stripped; local playlists keep using `MutationService`. Callers reload the
//! playlist afterwards.

use serde_json::json;
use swiftytube::networking::inner_tube_decode::JsonDict;
use swiftytube::networking::playlist_edits::PlaylistEditAction;
use swiftytube::sync::library_sync_models::SyncError;
use swiftytube::sync::library_sync_store::LibrarySyncStore;

use super::sync_bridge::SyncBridge;

pub struct PlaylistSongEdits;

impl PlaylistSongEdits {
    pub async fn add(
        bridge: &SyncBridge,
        song_id: &str,
        playlist_id: &str,
    ) -> Result<(), SyncError> {
        if !Self::is_synced(bridge, playlist_id)? {
            return bridge
                .mutations
                .add_to_playlist(playlist_id, song_id, None)
                .await;
        }
        let action = PlaylistEditAction::AddVideo {
            video_id: song_id.to_owned(),
            set_video_id: None,
        };
        Self::edit(bridge, playlist_id, vec![action.dictionary()]).await
    }

    pub async fn remove(
        bridge: &SyncBridge,
        song_id: &str,
        set_video_id: &str,
        playlist_id: &str,
    ) -> Result<(), SyncError> {
        if !Self::is_synced(bridge, playlist_id)? {
            return bridge
                .mutations
                .remove_from_playlist(playlist_id, song_id, set_video_id)
                .await;
        }
        let action = PlaylistEditAction::RemoveVideo {
            set_video_id: set_video_id.to_owned(),
        };
        Self::edit(bridge, playlist_id, vec![action.dictionary()]).await?;
        bridge.store.delete_map_entry(playlist_id, song_id)?;
        Ok(())
    }

    /// Rust-only: Swift calls `MutationService.renamePlaylist`, which sends the
    /// stored id as is; this sends the bare id like the song edits do.
    pub async fn rename(
        bridge: &SyncBridge,
        playlist_id: &str,
        name: &str,
    ) -> Result<(), SyncError> {
        let saved = bridge.store.fetch_playlist(playlist_id)?;
        if saved.as_ref().is_none_or(|p| p.browse_id.is_some()) {
            let action = PlaylistEditAction::RenamePlaylist {
                name: name.to_owned(),
            };
            Self::edit(bridge, playlist_id, vec![action.dictionary()]).await?;
        }
        if let Some(mut saved) = saved {
            saved.name = name.to_owned();
            bridge.store.save_playlist(&saved)?;
        }
        Ok(())
    }

    /// Rust-only: moves the row at `from` one place up or down, by its
    /// `playlistSetVideoId`. Synced playlists only: local ones keep no order.
    pub async fn move_song(
        bridge: &SyncBridge,
        playlist_id: &str,
        set_video_ids: &[Option<String>],
        from: usize,
        up: bool,
    ) -> Result<(), SyncError> {
        let Some(action) = Self::move_action(set_video_ids, from, up) else {
            return Ok(());
        };
        Self::edit(bridge, playlist_id, vec![action]).await
    }

    /// `ACTION_MOVE_VIDEO_BEFORE` (ytmusicapi `moveItem`): the row goes right
    /// before its new successor, or to the end without one. `None` when the
    /// row cannot move that way or a needed row has no set id.
    pub fn move_action(
        set_video_ids: &[Option<String>],
        from: usize,
        up: bool,
    ) -> Option<JsonDict> {
        let moved = set_video_ids.get(from)?.as_deref()?;
        let successor = if up {
            Some(set_video_ids.get(from.checked_sub(1)?)?.as_deref()?)
        } else if from + 1 >= set_video_ids.len() {
            return None;
        } else {
            match set_video_ids.get(from + 2) {
                None => None,
                Some(id) => Some(id.as_deref()?),
            }
        };
        let mut action = json!({ "action": "ACTION_MOVE_VIDEO_BEFORE", "setVideoId": moved });
        if let Some(successor) = successor {
            action["movedSetVideoIdSuccessor"] = successor.into();
        }
        match action {
            serde_json::Value::Object(dict) => Some(dict),
            _ => None,
        }
    }

    pub fn bare_id(playlist_id: &str) -> &str {
        playlist_id.strip_prefix("VL").unwrap_or(playlist_id)
    }

    /// Same rule as the detail page: only a saved row without a browse id is local.
    pub fn is_synced(bridge: &SyncBridge, playlist_id: &str) -> Result<bool, SyncError> {
        let saved = bridge.store.fetch_playlist(playlist_id)?;
        Ok(saved.is_none_or(|p| p.browse_id.is_some()))
    }

    async fn edit(
        bridge: &SyncBridge,
        playlist_id: &str,
        actions: Vec<JsonDict>,
    ) -> Result<(), SyncError> {
        bridge
            .client
            .edit_playlist(Self::bare_id(playlist_id), actions, None, None)
            .await?;
        Ok(())
    }
}
