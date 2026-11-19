// Trop/Odesli/Odesli.swift, plus the two Odesli helpers of
// Trop/Utilities/MusicLink.swift (`isConvertible`, `init(odesliResponse:)`).

use std::collections::HashMap;

use reqwest::Url;
use serde::Deserialize;
use swiftytube::auth::secure_store::{SecureStore, SecureStoreError};
use swiftytube::networking::http_client::Session;

use super::odesli_defaults::OdesliDefaults;
use super::odesli_service::{OdesliError, OdesliService};
use crate::utilities::music_link::MusicLink;

#[derive(Deserialize)]
struct OdesliResponse {
    #[serde(rename = "linksByPlatform")]
    links_by_platform: HashMap<String, Platform>,
}

#[derive(Deserialize)]
struct Platform {
    url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OdesliConnectError {
    #[error(transparent)]
    Service(#[from] OdesliError),
    #[error(transparent)]
    Store(#[from] SecureStoreError),
}

/// Converts Spotify and Apple Music links to YouTube Music with Odesli, the
/// service behind song.link. The API key lives in the keyring.
pub struct Odesli {
    pub session: Session,
    store: SecureStore,
    key: Option<String>,
}

impl Odesli {
    pub fn new(session: Session) -> Self {
        let mut odesli = Self {
            session,
            store: SecureStore::new(OdesliDefaults::KEYCHAIN_SERVICE),
            key: None,
        };
        odesli.reload_for_active_profile();
        odesli
    }

    pub fn is_connected(&self) -> bool {
        self.key.is_some()
    }

    /// Reads the key from the keyring again.
    pub fn reload_for_active_profile(&mut self) {
        self.key = self
            .store
            .load(OdesliDefaults::KEY_ACCOUNT)
            .ok()
            .and_then(|data| String::from_utf8(data).ok());
    }

    /// Checks `raw_key` with Odesli, then saves it.
    pub async fn connect(&mut self, raw_key: &str) -> Result<(), OdesliConnectError> {
        let key = raw_key.trim();
        OdesliService::validate(&self.session, key).await?;
        self.store
            .save(key.as_bytes(), OdesliDefaults::KEY_ACCOUNT)?;
        self.key = Some(key.to_string());
        Ok(())
    }

    pub fn disconnect(&mut self) {
        if let Err(error) = self.store.delete(OdesliDefaults::KEY_ACCOUNT) {
            log::error!("Could not delete the Odesli key: {error}");
        }
        self.key = None;
    }

    /// Sends `link` to Odesli and returns the same music on YouTube.
    pub async fn convert(&self, link: &str) -> Result<MusicLink, OdesliError> {
        let key = self.key.as_deref().ok_or(OdesliError::NotConnected)?;
        let response = OdesliService::links(&self.session, link, key).await?;
        Self::youtube_link(&response)
            .and_then(|url| MusicLink::parse(&url))
            .ok_or(OdesliError::NotFound)
    }

    /// True for Spotify and Apple Music links. Only these may go to Odesli, so
    /// other links the user shares stay on the computer.
    pub fn is_convertible(link: &str) -> bool {
        let Ok(url) = Url::parse(link) else {
            return false;
        };
        ["http", "https"].contains(&url.scheme())
            && url.host_str().is_some_and(|host| {
                ["open.spotify.com", "music.apple.com", "geo.music.apple.com"].contains(&host)
            })
    }

    /// The YouTube link in an Odesli reply, preferring YouTube Music.
    pub fn youtube_link(odesli_response: &[u8]) -> Option<String> {
        let response: OdesliResponse = serde_json::from_slice(odesli_response).ok()?;
        ["youtubeMusic", "youtube"]
            .into_iter()
            .filter_map(|platform| response.links_by_platform.get(platform))
            .map(|platform| platform.url.clone())
            .find(|url| Url::parse(url).is_ok())
    }
}
