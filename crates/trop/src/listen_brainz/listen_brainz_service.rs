// Trop/ListenBrainz/ListenBrainzService.swift: network calls to the ListenBrainz API.
// Swift's snake_case key strategies become snake_case field names.

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use reqwest::{Method, Request, Url};
use serde::{Deserialize, Serialize};
use swiftytube::networking::http_client::{HttpClient, Session};

use super::listen_brainz_defaults::ListenBrainzDefaults;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ListenBrainzError {
    #[error(
        "ListenBrainz didn't accept this token. Copy it again from your ListenBrainz settings."
    )]
    InvalidToken,
    #[error("Couldn't reach ListenBrainz. Check your connection and try again.")]
    Unreachable,
}

#[derive(Deserialize)]
struct ValidateResponse {
    valid: bool,
    user_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ListenBrainzListen {
    /// Seconds since 1970; absent for `playing_now`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listened_at: Option<i64>,
    pub track_metadata: TrackMetadata,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TrackMetadata {
    pub artist_name: String,
    pub track_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_name: Option<String>,
    pub additional_info: AdditionalInfo,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AdditionalInfo {
    pub media_player: String,
    pub submission_client: String,
    pub music_service: String,
    pub origin_url: String,
    pub duration_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListenType {
    PlayingNow,
    Single,
}

impl ListenType {
    pub fn raw_value(self) -> &'static str {
        match self {
            ListenType::PlayingNow => "playing_now",
            ListenType::Single => "single",
        }
    }
}

#[derive(Serialize)]
struct SubmitBody<'a> {
    listen_type: &'a str,
    payload: [&'a ListenBrainzListen; 1],
}

pub struct ListenBrainzService;

impl ListenBrainzService {
    /// Returns the user name the token belongs to.
    pub async fn validate(session: &Session, token: &str) -> Result<String, ListenBrainzError> {
        let mut request = Request::new(Method::GET, Self::api_url("validate-token"));
        Self::authorize(&mut request, token)?;
        let response = HttpClient::data(request, session)
            .await
            .map_err(|_| ListenBrainzError::Unreachable)?;
        serde_json::from_slice::<ValidateResponse>(&response.data)
            .ok()
            .filter(|reply| reply.valid)
            .and_then(|reply| reply.user_name)
            .ok_or(ListenBrainzError::InvalidToken)
    }

    /// Returns the HTTP status code, `None` when the server was not reached.
    pub async fn submit(
        session: &Session,
        listen: &ListenBrainzListen,
        listen_type: ListenType,
        token: &str,
    ) -> Option<u16> {
        let mut request = Request::new(Method::POST, Self::api_url("submit-listens"));
        Self::authorize(&mut request, token).ok()?;
        request
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        *request.body_mut() = Some(Self::submit_body(listen, listen_type).into());
        let reply = HttpClient::data(request, session).await.ok()?;
        Some(reply.status)
    }

    /// `{"listen_type": ..., "payload": [listen]}`.
    pub fn submit_body(listen: &ListenBrainzListen, listen_type: ListenType) -> Vec<u8> {
        serde_json::to_vec(&SubmitBody {
            listen_type: listen_type.raw_value(),
            payload: [listen],
        })
        .expect("a listen always serializes")
    }

    /// A token with characters a header can't hold is one ListenBrainz would refuse.
    fn authorize(request: &mut Request, token: &str) -> Result<(), ListenBrainzError> {
        let value = HeaderValue::from_str(&format!("Token {token}"))
            .map_err(|_| ListenBrainzError::InvalidToken)?;
        request.headers_mut().insert(AUTHORIZATION, value);
        Ok(())
    }

    fn api_url(path: &str) -> Url {
        Url::parse(&format!("{}{path}", ListenBrainzDefaults::API_BASE))
            .expect("API_BASE is a valid URL")
    }
}
