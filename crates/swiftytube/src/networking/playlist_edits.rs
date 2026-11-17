use serde_json::json;

use super::inner_tube_decode::{JsonDict, as_dict, as_string};

/// `browse/edit_playlist` action dicts. Both apps edit playlists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaylistEditAction {
    AddVideo {
        video_id: String,
        set_video_id: Option<String>,
    },
    RemoveVideo {
        set_video_id: String,
    },
    RenamePlaylist {
        name: String,
    },
}

impl PlaylistEditAction {
    pub fn dictionary(&self) -> JsonDict {
        let value = match self {
            Self::AddVideo {
                video_id,
                set_video_id,
            } => {
                let mut action = json!({ "action": "ACTION_ADD_VIDEO", "addedVideoId": video_id });
                if let Some(set_video_id) = set_video_id {
                    action["setVideoId"] = set_video_id.as_str().into();
                }
                action
            }
            Self::RemoveVideo { set_video_id } => {
                json!({ "action": "ACTION_REMOVE_VIDEO", "setVideoId": set_video_id })
            }
            Self::RenamePlaylist { name } => {
                json!({ "action": "ACTION_SET_PLAYLIST_NAME", "name": name })
            }
        };
        match value {
            serde_json::Value::Object(dict) => dict,
            _ => unreachable!("json! object literal"),
        }
    }

    pub fn dictionaries(actions: &[PlaylistEditAction]) -> Vec<JsonDict> {
        actions.iter().map(Self::dictionary).collect()
    }

    /// Playlist id from a `playlist/create` response (`playlistId` or nested).
    pub fn extract_playlist_id(json: &JsonDict) -> Option<String> {
        as_string(json.get("playlistId"))
            .or_else(|| as_string(as_dict(json.get("response"))?.get("playlistId")))
            .map(str::to_string)
    }
}

/// Apply → remote → rollback + return the error.
pub struct OptimisticMutation;

impl OptimisticMutation {
    pub async fn attempting_remote<E>(
        remote: impl Future<Output = Result<(), E>>,
        rollback: impl Future<Output = ()>,
    ) -> Result<(), E> {
        let result = remote.await;
        if result.is_err() {
            rollback.await;
        }
        result
    }
}
