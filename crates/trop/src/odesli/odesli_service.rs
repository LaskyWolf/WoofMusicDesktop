// Trop/Odesli/OdesliService.swift: network calls to the Odesli API.

use reqwest::Url;
use swiftytube::networking::http_client::{HttpClient, Session};

use super::odesli_defaults::OdesliDefaults;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum OdesliError {
    #[error(
        "To open Spotify and Apple Music links, add an Odesli API key in Settings > Integrations."
    )]
    NotConnected,
    #[error("Odesli didn't accept this API key. Check it and try again.")]
    InvalidKey,
    #[error("Odesli is receiving too many requests from this key. Wait a minute and try again.")]
    RateLimited,
    #[error("Odesli couldn't find this on YouTube Music.")]
    NotFound,
    #[error("Couldn't reach Odesli. Check your connection and try again.")]
    Unreachable,
}

pub struct OdesliService;

impl OdesliService {
    /// Fails only when the key is refused or Odesli can't be reached: a rate
    /// limit or a missing test song says nothing against the key.
    pub async fn validate(session: &Session, key: &str) -> Result<(), OdesliError> {
        match Self::links(session, OdesliDefaults::VALIDATION_LINK, key).await {
            Ok(_) | Err(OdesliError::RateLimited | OdesliError::NotFound) => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Returns the JSON listing `link` on every platform.
    pub async fn links(session: &Session, link: &str, key: &str) -> Result<Vec<u8>, OdesliError> {
        let response = HttpClient::data_from(Self::links_url(link, key), session)
            .await
            .map_err(|_| OdesliError::Unreachable)?;
        match response.status {
            200 => Ok(response.data),
            401 | 403 => Err(OdesliError::InvalidKey),
            429 => Err(OdesliError::RateLimited),
            _ => Err(OdesliError::NotFound),
        }
    }

    /// Form encoding writes "+" as "%2B", so the server never reads it as a space.
    pub fn links_url(link: &str, key: &str) -> Url {
        Url::parse_with_params(
            OdesliDefaults::API_URL,
            [("url", link), ("songIfSingle", "true"), ("key", key)],
        )
        .expect("API_URL is a valid URL")
    }
}
