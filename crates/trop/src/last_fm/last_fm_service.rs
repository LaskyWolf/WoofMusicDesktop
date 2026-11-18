// Trop/LastFM/LastFMService.swift: signing and network calls to Last.fm.
// The service owns the keys and the token store that Swift reads from statics.

use std::collections::BTreeMap;
use std::time::Duration;

use md5::{Digest, Md5};
use reqwest::header::{CONTENT_TYPE, HeaderValue, USER_AGENT};
use reqwest::{Method, Request, Url};
use serde::Deserialize;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::Value;
use swiftytube::networking::http_client::{HttpClient, Session};

use super::last_fm_defaults::LastFMDefaults;
use super::last_fm_token_store::LastFMTokenStore;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct LastFMSession {
    pub name: String,
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct LastFMAuthResponse {
    pub session: LastFMSession,
}

#[derive(Deserialize)]
pub struct LastFMTokenResponse {
    pub token: String,
}

#[derive(Deserialize)]
pub struct LastFMErrorResponse {
    pub error: i64,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LastFMError {
    #[error("Last.fm API keys not configured")]
    NotConfigured,
    /// Swift wraps the transport or decoding `Error`; this keeps its message.
    #[error("{0}")]
    Network(String),
    #[error("HTTP {0}: {1}")]
    Http(u16, String),
    #[error("Last.fm error {0}: {1}")]
    Api(i64, String),
    #[error("Missing session key")]
    MissingSession,
    #[error("Token not yet authorized — please approve on Last.fm and tap Done")]
    UnauthorizedToken,
    #[error("Invalid response")]
    InvalidResponse,
}

pub struct LastFMService {
    pub session: Session,
    /// `LastFMDefaults::api_key()` unless set otherwise.
    pub api_key: String,
    /// `LastFMDefaults::secret()` unless set otherwise.
    pub secret: String,
    pub token_store: LastFMTokenStore,
}

impl LastFMService {
    const USER_AGENT: &str = "Trop/1.0 (https://github.com/686udjie/Trop)";
    const SENSITIVE_PARAM_KEYS: [&str; 4] = ["password", "token", "sk", "api_sig"];

    pub fn new(session: Session, token_store: LastFMTokenStore) -> Self {
        Self {
            session,
            api_key: LastFMDefaults::api_key(),
            secret: LastFMDefaults::secret(),
            token_store,
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty() && !self.secret.is_empty()
    }

    // MARK: - Signature

    /// md5 of every `key + value` sorted by key, then the secret.
    pub fn api_sig(params: &BTreeMap<String, String>, secret: &str) -> String {
        let mut text: String = params.iter().map(|(k, v)| format!("{k}{v}")).collect();
        text.push_str(secret);
        Md5::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// The form body: `params`, their `api_sig`, then `format=json` (not signed).
    pub fn form_body(params: &BTreeMap<String, String>, secret: &str) -> String {
        let mut all = params.clone();
        all.insert("api_sig".into(), Self::api_sig(params, secret));
        all.insert("format".into(), "json".into());
        form_urlencoded::Serializer::new(String::new())
            .extend_pairs(&all)
            .finish()
    }

    fn make_request(&self, params: &BTreeMap<String, String>) -> Result<Request, LastFMError> {
        if !self.is_configured() {
            return Err(LastFMError::NotConfigured);
        }
        let url = Url::parse(LastFMDefaults::BASE_URL).map_err(|_| LastFMError::InvalidResponse)?;
        let mut request = Request::new(Method::POST, url);
        let headers = request.headers_mut();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        headers.insert(USER_AGENT, HeaderValue::from_static(Self::USER_AGENT));
        *request.timeout_mut() = Some(Duration::from_secs(15));
        *request.body_mut() = Some(Self::form_body(params, &self.secret).into());
        Ok(request)
    }

    async fn perform<T: DeserializeOwned>(
        &self,
        params: BTreeMap<String, String>,
    ) -> Result<T, LastFMError> {
        let method = params.get("method").map_or("?", String::as_str);
        log::debug!("LastFM {method} params={}", Self::redacted_params(&params));
        let request = self.make_request(&params)?;
        let response = HttpClient::data(request, &self.session)
            .await
            .map_err(|error| {
                // Usually offline.
                log::warn!("LastFM {method} transport error: {error}");
                LastFMError::Network(error.to_string())
            })?;

        // Last.fm reports API errors inside a 200 response too.
        if let Ok(error) = serde_json::from_slice::<LastFMErrorResponse>(&response.data)
            && error.error != 0
        {
            // 14: the web flow's token is not approved yet.
            if error.error == 14 {
                return Err(LastFMError::UnauthorizedToken);
            }
            log::error!(
                "LastFM {method} apiError={} msg={}",
                error.error,
                error.message
            );
            return Err(LastFMError::Api(error.error, error.message));
        }

        if !(200..=299).contains(&response.status) {
            log::error!("LastFM {method} http={}", response.status);
            return Err(LastFMError::Http(
                response.status,
                "Response body omitted".into(),
            ));
        }

        serde_json::from_slice(&response.data).map_err(|error| {
            log::error!("LastFM {method} response could not be decoded");
            LastFMError::Network(error.to_string())
        })
    }

    fn redacted_params(params: &BTreeMap<String, String>) -> String {
        params
            .iter()
            .map(|(key, value)| {
                if Self::SENSITIVE_PARAM_KEYS.contains(&key.as_str()) {
                    format!("{key}=<redacted>")
                } else {
                    format!("{key}={value}")
                }
            })
            .collect::<Vec<_>>()
            .join("&")
    }

    fn params<const N: usize>(pairs: [(&str, &str); N]) -> BTreeMap<String, String> {
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn session_key(&self) -> Result<String, LastFMError> {
        self.token_store
            .retrieve_session_key()
            .filter(|key| !key.is_empty())
            .ok_or(LastFMError::MissingSession)
    }

    // MARK: - Public API

    pub async fn get_token(&self) -> Result<String, LastFMError> {
        let params = Self::params([("method", "auth.getToken"), ("api_key", &self.api_key)]);
        let response: LastFMTokenResponse = self.perform(params).await?;
        Ok(response.token)
    }

    pub async fn get_session(&self, token: &str) -> Result<LastFMAuthResponse, LastFMError> {
        let params = Self::params([
            ("method", "auth.getSession"),
            ("api_key", &self.api_key),
            ("token", token),
        ]);
        self.perform(params).await
    }

    pub async fn update_now_playing(
        &self,
        artist: &str,
        track: &str,
        album: Option<&str>,
        track_number: Option<i64>,
        duration: Option<i64>,
    ) -> Result<(), LastFMError> {
        let sk = self.session_key()?;
        let mut params = Self::params([
            ("method", "track.updateNowPlaying"),
            ("api_key", &self.api_key),
            ("artist", artist),
            ("track", track),
            ("sk", &sk),
        ]);
        if let Some(album) = album.filter(|a| !a.is_empty()) {
            params.insert("album".into(), album.into());
        }
        if let Some(n) = track_number {
            params.insert("trackNumber".into(), n.to_string());
        }
        if let Some(d) = duration {
            params.insert("duration".into(), d.to_string());
        }
        self.perform::<IgnoredAny>(params).await.map(drop)
    }

    /// `timestamp`: seconds since 1970 when the song started.
    pub async fn scrobble(
        &self,
        artist: &str,
        track: &str,
        timestamp: i64,
        album: Option<&str>,
        track_number: Option<i64>,
        duration: Option<i64>,
    ) -> Result<(), LastFMError> {
        let sk = self.session_key()?;
        let timestamp = timestamp.to_string();
        let mut params = Self::params([
            ("method", "track.scrobble"),
            ("api_key", &self.api_key),
            ("sk", &sk),
            ("artist[0]", artist),
            ("track[0]", track),
            ("timestamp[0]", &timestamp),
        ]);
        if let Some(album) = album.filter(|a| !a.is_empty()) {
            params.insert("album[0]".into(), album.into());
        }
        if let Some(n) = track_number {
            params.insert("trackNumber[0]".into(), n.to_string());
        }
        if let Some(d) = duration {
            params.insert("duration[0]".into(), d.to_string());
        }
        self.perform::<IgnoredAny>(params).await.map(drop)
    }

    pub async fn set_love_status(
        &self,
        artist: &str,
        track: &str,
        love: bool,
    ) -> Result<(), LastFMError> {
        let sk = self.session_key()?;
        let method = if love { "track.love" } else { "track.unlove" };
        let params = Self::params([
            ("method", method),
            ("api_key", &self.api_key),
            ("sk", &sk),
            ("artist", artist),
            ("track", track),
        ]);
        self.perform::<IgnoredAny>(params).await.map(drop)
    }

    // MARK: - Avatar

    /// The user's picture via `user.getInfo` (no signature needed), largest first.
    pub async fn fetch_avatar_url(&self, username: &str) -> Option<String> {
        let trimmed = username.trim();
        if trimmed.is_empty() {
            log::warn!("fetchAvatarURL: empty username");
            return None;
        }
        if !self.is_configured() {
            log::warn!("fetchAvatarURL: API keys not configured");
            return None;
        }
        let url = Url::parse_with_params(
            LastFMDefaults::BASE_URL,
            [
                ("method", "user.getInfo"),
                ("user", trimmed),
                ("api_key", &self.api_key),
                ("format", "json"),
            ],
        )
        .ok()?;
        let mut request = Request::new(Method::GET, url);
        request
            .headers_mut()
            .insert(USER_AGENT, HeaderValue::from_static(Self::USER_AGENT));
        *request.timeout_mut() = Some(Duration::from_secs(10));
        let response = match HttpClient::data(request, &self.session).await {
            Ok(response) => response,
            Err(error) => {
                // Optional data, usually missing because the device is offline.
                log::warn!("fetchAvatarURL failed: {error}");
                return None;
            }
        };
        if !(200..=299).contains(&response.status) {
            log::warn!("fetchAvatarURL: http={} user={trimmed}", response.status);
            return None;
        }
        let json: Value = serde_json::from_slice(&response.data).ok()?;
        if let Some(error) = json["error"].as_i64().filter(|e| *e != 0) {
            log::warn!("fetchAvatarURL: apiError={error} msg={}", json["message"]);
            return None;
        }
        // Each image is `{"size": ..., "#text": url}` or a bare url string.
        let images: Vec<(Option<&str>, &str)> = json["user"]["image"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|image| match image {
                Value::String(text) => Some((None, text.as_str())),
                _ => Some((image["size"].as_str(), image["#text"].as_str()?)),
            })
            .filter(|(_, text)| !text.is_empty())
            .collect();
        let preferred = ["extralarge", "large", "medium", "small"]
            .into_iter()
            .filter_map(|size| images.iter().find(|(s, _)| *s == Some(size)));
        let url = preferred
            .chain(images.first())
            .find_map(|(_, text)| Self::valid_avatar_url(text));
        if url.is_none() {
            log::debug!("fetchAvatarURL: no image for user={trimmed}");
        }
        url
    }

    /// Last.fm still returns http URLs in some profiles; the CDN supports HTTPS.
    fn valid_avatar_url(raw: &str) -> Option<String> {
        let mut url = Url::parse(raw.trim()).ok()?;
        if !["http", "https"].contains(&url.scheme()) || url.host().is_none() {
            return None;
        }
        url.set_scheme("https").ok()?;
        Some(url.into())
    }
}
