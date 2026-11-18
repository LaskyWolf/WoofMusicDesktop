//! Trop/Services/PlaylistCoverUploader.swift, synced playlists only.
//!
//! Sets a custom cover on a YouTube Music playlist, the same way the
//! music.youtube.com editor does: a resumable image upload returns a blob id,
//! then `browse/edit_playlist` attaches it with ACTION_SET_CUSTOM_THUMBNAIL.
//! This is an undocumented web flow; if YouTube changes it the upload fails
//! with an error and nothing else is touched.
//!
//! The caller passes the square JPEG (Swift `squareJPEG`): Trop does not decode
//! images. Local covers (`LocalPlaylistCover`) are not ported: the desktop app
//! has no local playlists yet.

use serde_json::{Value, json};
use swiftytube::auth::sapisid_auth::SAPISIDAuth;
use swiftytube::networking::inner_tube_decode::{InnerTubeDecode, JsonDict, as_string};
use swiftytube::networking::inner_tube_json::InnerTubeJSON;
use swiftytube::sync::library_sync_store::LibrarySyncStore;

use super::playlist_song_edits::PlaylistSongEdits;
use super::sync_bridge::SyncBridge;

#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error("Sign in to YouTube Music to change playlist covers.")]
    NotSignedIn,
    #[error("YouTube Music refused the cover ({0}). Try again later.")]
    Rejected(&'static str),
}

const ORIGIN: &str = "https://music.youtube.com";
const UPLOAD_URL: &str =
    "https://music.youtube.com/playlist_image_upload/playlist_custom_thumbnail";

pub struct PlaylistCoverUploader;

impl PlaylistCoverUploader {
    /// Square side sent to YouTube; it serves the cover back at up to 544px.
    pub const COVER_SIDE: u32 = 1024;

    /// Uploads `jpeg` as the cover of `playlist_id`, saves the new cover URL
    /// locally and returns it.
    pub async fn set_cover(
        bridge: &SyncBridge,
        jpeg: Vec<u8>,
        playlist_id: &str,
    ) -> Result<Option<String>, UploadError> {
        let auth = bridge.client.current_auth();
        let Some(sapisid) = auth.sapisid.filter(|_| !auth.cookies.is_empty()) else {
            return Err(UploadError::NotSignedIn);
        };
        let cookie = auth
            .cookies
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        let http = reqwest::Client::new();
        let signed = |url: &str| {
            http.post(url)
                .header("Cookie", &cookie)
                .header(
                    "Authorization",
                    SAPISIDAuth::authorization_header(&sapisid, ORIGIN),
                )
                .header("Origin", ORIGIN)
                .header("X-Origin", ORIGIN)
                .header("Referer", format!("{ORIGIN}/"))
                .header("X-Goog-AuthUser", "0")
                .header(
                    "Content-Type",
                    "application/x-www-form-urlencoded;charset=utf-8",
                )
        };

        // 1. Open an upload session.
        let start = signed(UPLOAD_URL)
            .header("X-Goog-Upload-Protocol", "resumable")
            .header("X-Goog-Upload-Command", "start")
            .header(
                "X-Goog-Upload-Header-Content-Length",
                jpeg.len().to_string(),
            )
            .send()
            .await
            .ok()
            .filter(|response| response.status() == 200)
            .ok_or(UploadError::Rejected("upload start"))?;
        let session_url = start
            .headers()
            .get("x-goog-upload-url")
            .and_then(|value| value.to_str().ok())
            .ok_or(UploadError::Rejected("upload start"))?
            .to_owned();

        // 2. Send the image; the reply carries the blob id.
        let sent = signed(&session_url)
            .header("X-Goog-Upload-Command", "upload, finalize")
            .header("X-Goog-Upload-Offset", "0")
            .body(jpeg)
            .send()
            .await
            .ok()
            .filter(|response| response.status() == 200)
            .ok_or(UploadError::Rejected("image upload"))?;
        let reply: Value = sent
            .bytes()
            .await
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .ok_or(UploadError::Rejected("image upload"))?;
        let blob_id = reply
            .get("encryptedBlobId")
            .and_then(Value::as_str)
            .ok_or(UploadError::Rejected("image upload"))?;

        // 3. Attach the uploaded image to the playlist.
        let response = bridge
            .client
            .edit_playlist(
                PlaylistSongEdits::bare_id(playlist_id),
                vec![Self::set_thumbnail_action(blob_id)],
                None,
                None,
            )
            .await
            .map_err(|_| UploadError::Rejected("playlist update"))?;
        if as_string(response.get("status")) != Some("STATUS_SUCCEEDED") {
            return Err(UploadError::Rejected("playlist update"));
        }
        let new_url = Self::new_cover_url(&response);
        if let Some(url) = &new_url
            && let Ok(Some(mut saved)) = bridge.store.fetch_playlist(playlist_id)
        {
            saved.thumbnail_url = Some(url.clone());
            let _ = bridge.store.save_playlist(&saved);
        }
        Ok(new_url)
    }

    fn set_thumbnail_action(blob_id: &str) -> JsonDict {
        let action = json!({
            "action": "ACTION_SET_CUSTOM_THUMBNAIL",
            "addedCustomThumbnail": {
                "imageKey": {
                    "type": "PLAYLIST_IMAGE_TYPE_CUSTOM_THUMBNAIL",
                    "name": "studio_square_thumbnail"
                },
                "playlistScottyEncryptedBlobId": blob_id
            }
        });
        match action {
            Value::Object(dict) => dict,
            _ => unreachable!("json! object literal"),
        }
    }

    /// The cover URL in the `edit_playlist` reply's new header.
    pub fn new_cover_url(response: &JsonDict) -> Option<String> {
        InnerTubeDecode::dict(
            &[
                "newHeader",
                "musicEditablePlaylistDetailHeaderRenderer",
                "header",
                "musicResponsiveHeaderRenderer",
            ],
            response,
        )
        .and_then(InnerTubeJSON::music_thumbnail_url)
    }
}
