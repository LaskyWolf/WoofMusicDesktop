// Minimal transport. Returns raw JSON - apps parse
// music vs. video schemas themselves.

use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::http_client::{HttpClient, Session};
use super::inner_tube_decode::JsonDict;
use super::inner_tube_error::InnerTubeError;
use super::request_builder::RequestBuilder;
use super::retry_policy::{RetryPolicy, random_u64};
use crate::auth::auth_state::AuthState;
use crate::core::swiftytube_config::SwiftyTubeConfig;
use crate::core::swiftytube_log::SwiftyTubeLog;
use crate::core::youtube_client::YouTubeClient;
use crate::core::youtube_locale::YouTubeLocale;
use crate::streaming::player_request_options::PlayerRequestOptions;

/// `browse` arguments (Swift default parameters).
#[derive(Clone, Debug, Default)]
pub struct BrowseArgs {
    pub browse_id: Option<String>,
    pub params: Option<String>,
    pub continuation: Option<String>,
    pub form_data: Option<JsonDict>,
    pub client: Option<YouTubeClient>,
    pub locale: Option<YouTubeLocale>,
    pub po_token: Option<String>,
}

impl BrowseArgs {
    /// The common case: one `browseId`, everything else default.
    pub fn id(browse_id: impl Into<String>) -> Self {
        Self {
            browse_id: Some(browse_id.into()),
            ..Self::default()
        }
    }
}

/// `search` arguments (Swift default parameters).
#[derive(Clone, Debug, Default)]
pub struct SearchArgs {
    pub query: String,
    pub params: Option<String>,
    pub continuation: Option<String>,
    pub client: Option<YouTubeClient>,
    pub locale: Option<YouTubeLocale>,
}

/// `next` arguments (Swift default parameters).
#[derive(Clone, Debug, Default)]
pub struct NextArgs {
    pub video_id: Option<String>,
    pub playlist_id: Option<String>,
    pub index: Option<i64>,
    pub continuation: Option<String>,
    pub client: Option<YouTubeClient>,
    pub locale: Option<YouTubeLocale>,
}

/// `player` arguments (Swift default parameters).
#[derive(Clone, Debug, Default)]
pub struct PlayerArgs {
    pub video_id: String,
    pub playlist_id: Option<String>,
    pub client: Option<YouTubeClient>,
    pub locale: Option<YouTubeLocale>,
    pub options: PlayerRequestOptions,
}

/// Shared InnerTube transport. Swift `actor`: methods take `&self` and state
/// sits behind short-lived locks that are never held across an `.await`.
pub struct InnerTubeClient {
    config: Mutex<SwiftyTubeConfig>,
    session: Session,
    auth: Mutex<AuthState>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

impl InnerTubeClient {
    pub fn new(config: SwiftyTubeConfig, auth: AuthState) -> Self {
        let session = reqwest::Client::builder()
            .timeout(Duration::from_secs_f64(config.timeout_seconds))
            .build()
            .expect("TLS backend initializes");
        Self::with_session(config, auth, Session::Live(session))
    }

    /// Custom session (Swift tests inject a stubbed `URLSession`).
    pub fn with_session(config: SwiftyTubeConfig, auth: AuthState, session: Session) -> Self {
        Self {
            config: Mutex::new(config),
            session,
            auth: Mutex::new(auth),
        }
    }

    pub fn config(&self) -> SwiftyTubeConfig {
        lock(&self.config).clone()
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    // MARK: - Auth

    pub fn update_auth(&self, auth: AuthState) {
        *lock(&self.auth) = auth;
    }

    pub fn current_auth(&self) -> AuthState {
        lock(&self.auth).clone()
    }

    /// New default locale (and Accept-Language) for future requests.
    pub fn update_locale(&self, locale: YouTubeLocale) {
        let mut config = lock(&self.config);
        config.accept_language = format!("en-{},en;q=0.9", locale.gl);
        config.default_locale = locale;
    }

    // MARK: - Endpoints (raw JSON, app parses)

    pub async fn browse(&self, args: BrowseArgs) -> Result<JsonDict, InnerTubeError> {
        let mut body = JsonDict::new();
        body.insert(
            "context".into(),
            self.context_dict(args.client.as_ref(), args.locale.as_ref()),
        );
        insert_some(&mut body, "browseId", args.browse_id);
        insert_some(&mut body, "params", args.params);
        insert_some(&mut body, "continuation", args.continuation);
        insert_some(&mut body, "formData", args.form_data);
        if let Some(po_token) = args.po_token {
            body.insert(
                "serviceIntegrityDimensions".into(),
                json!({ "poToken": po_token }),
            );
        }
        self.post("browse", body, args.client, RetryPolicy::INNER_TUBE)
            .await
    }

    pub async fn search(&self, args: SearchArgs) -> Result<JsonDict, InnerTubeError> {
        let mut body = JsonDict::new();
        body.insert(
            "context".into(),
            self.context_dict(args.client.as_ref(), args.locale.as_ref()),
        );
        body.insert("query".into(), args.query.into());
        insert_some(&mut body, "params", args.params);
        insert_some(&mut body, "continuation", args.continuation);
        self.post("search", body, args.client, RetryPolicy::INNER_TUBE)
            .await
    }

    pub async fn next(&self, args: NextArgs) -> Result<JsonDict, InnerTubeError> {
        let mut body = JsonDict::new();
        body.insert(
            "context".into(),
            self.context_dict(args.client.as_ref(), args.locale.as_ref()),
        );
        insert_some(&mut body, "videoId", args.video_id);
        insert_some(&mut body, "playlistId", args.playlist_id);
        insert_some(&mut body, "index", args.index);
        // Comments, related rows and transcripts paginate through `next`.
        insert_some(&mut body, "continuation", args.continuation);
        self.post("next", body, args.client, RetryPolicy::INNER_TUBE)
            .await
    }

    pub async fn player(&self, args: PlayerArgs) -> Result<JsonDict, InnerTubeError> {
        let (body, client) = self.make_player_body(args);
        self.post("player", body, Some(client), RetryPolicy::INNER_TUBE)
            .await
    }

    /// Typed `/player` decode.
    pub async fn player_response<T: DeserializeOwned>(
        &self,
        args: PlayerArgs,
    ) -> Result<T, InnerTubeError> {
        let (body, client) = self.make_player_body(args);
        let data = self
            .post_data("player", &body, Some(client), RetryPolicy::INNER_TUBE)
            .await?;
        serde_json::from_slice(&data).map_err(|_| InnerTubeError::DecodingFailed)
    }

    // MARK: - Core POST

    /// `/player` body, plus the resolved client.
    fn make_player_body(&self, args: PlayerArgs) -> (JsonDict, YouTubeClient) {
        let config = self.config();
        let client = args.client.unwrap_or(config.default_client);
        let locale = args.locale.unwrap_or(config.default_locale);
        let mut body = JsonDict::new();
        body.insert(
            "context".into(),
            self.context_dict(Some(&client), Some(&locale)),
        );
        body.insert("videoId".into(), args.video_id.clone().into());
        body.insert("contentCheckOk".into(), true.into());
        body.insert("racyCheckOk".into(), true.into());
        if !client.use_music_player_endpoint {
            body.insert("videoCheckOk".into(), true.into());
        }
        insert_some(&mut body, "playlistId", args.playlist_id);
        if let Some(signature_timestamp) = args.options.signature_timestamp {
            let mut content_playback_context = JsonDict::new();
            content_playback_context
                .insert("signatureTimestamp".into(), signature_timestamp.into());
            if !client.use_music_player_endpoint {
                content_playback_context
                    .insert("html5Preference".into(), "HTML5_PREF_WANTS".into());
            }
            body.insert(
                "playbackContext".into(),
                json!({ "contentPlaybackContext": content_playback_context }),
            );
        }
        if let Some(po_token) = args.options.po_token {
            body.insert(
                "serviceIntegrityDimensions".into(),
                json!({ "poToken": po_token }),
            );
        }
        if client.is_embedded {
            let embed_url = format!("https://www.youtube.com/embed/{}", args.video_id);
            body.insert("thirdParty".into(), json!({ "embedUrl": embed_url }));
        }
        (body, client)
    }

    pub async fn post(
        &self,
        endpoint: &str,
        body: JsonDict,
        client: Option<YouTubeClient>,
        retry_policy: RetryPolicy,
    ) -> Result<JsonDict, InnerTubeError> {
        let data = self
            .post_data(endpoint, &body, client, retry_policy)
            .await?;
        let Ok(Value::Object(json)) = serde_json::from_slice(&data) else {
            return Err(InnerTubeError::DecodingFailed);
        };
        // Every response carries the visitor id; keep the latest for later calls.
        if let Some(visitor_data) = json
            .get("responseContext")
            .and_then(|c| c.get("visitorData"))
            .and_then(Value::as_str)
        {
            lock(&self.auth).visitor_data = Some(visitor_data.to_string());
        }
        Ok(json)
    }

    pub async fn post_data(
        &self,
        endpoint: &str,
        body: &JsonDict,
        client: Option<YouTubeClient>,
        retry_policy: RetryPolicy,
    ) -> Result<Vec<u8>, InnerTubeError> {
        let config = self.config();
        let auth = self.current_auth();
        let client = client.unwrap_or(config.default_client);
        let request_id = format!("{:08X}", random_u64() as u32);
        let request = RequestBuilder::build_request(&config, endpoint, body, &client, &auth);
        let has_cookie = request.headers().contains_key("Cookie");
        let has_auth = request.headers().contains_key("Authorization");
        let has_auth_user = request.headers().contains_key("X-Goog-AuthUser");
        let auth_available = auth.sapisid.is_some() && !auth.cookies.is_empty();
        let yes_no = |present: bool| if present { "yes" } else { "no" };
        SwiftyTubeLog::debug(&format!(
            "[HTTP][{request_id}] → {endpoint} url={} client={}/{} clientId={} clientLoginSupported={} \
             authAvailable={auth_available} cookies={} sapisid={} visitorData={} dataSyncId={} \
             hasCookie={has_cookie} hasAuth={has_auth} hasAuthUser={has_auth_user} poToken={} body={}",
            request.url(),
            client.client_name,
            client.client_version,
            client.client_id,
            client.login_supported,
            auth.cookies.len(),
            yes_no(auth.sapisid.is_some()),
            yes_no(auth.visitor_data.is_some()),
            yes_no(auth.data_sync_id.is_some()),
            yes_no(body.contains_key("serviceIntegrityDimensions")),
            Self::body_diagnostics(body),
        ));
        retry_policy
            .run(
                || {
                    // A reqwest::Request is consumed on send; retries send a copy.
                    let copy = request.try_clone().expect("JSON body is clonable bytes");
                    Self::post_once(copy, &self.session, &request_id, endpoint)
                },
                Self::is_retryable,
                |attempt, error| {
                    SwiftyTubeLog::debug(&format!(
                        "[HTTP][{request_id}] retry={} endpoint={endpoint} error={error}",
                        attempt + 1
                    ));
                },
            )
            .await
    }

    async fn post_once(
        request: reqwest::Request,
        session: &Session,
        request_id: &str,
        endpoint: &str,
    ) -> Result<Vec<u8>, InnerTubeError> {
        let response = HttpClient::data(request, session).await?;
        let status = response.status;
        let mut header_names: Vec<&str> = response.headers.keys().map(|k| k.as_str()).collect();
        header_names.sort();
        let content_type = response
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("none");
        SwiftyTubeLog::debug(&format!(
            "[HTTP][{request_id}] ← endpoint={endpoint} status={status} bytes={} contentType={content_type} headers=[{}]",
            response.data.len(),
            header_names.join(","),
        ));
        if !(200..=299).contains(&status) {
            let body = String::from_utf8_lossy(&response.data);
            let preview: String = body.chars().take(1000).collect();
            SwiftyTubeLog::error(&format!("[HTTP][{request_id}] errorBody={preview}"));
            return Err(InnerTubeError::HttpError {
                status_code: status,
                data: response.data,
            });
        }
        Ok(response.data)
    }

    fn body_diagnostics(body: &JsonDict) -> String {
        let mut keys: Vec<&str> = body.keys().map(String::as_str).collect();
        keys.sort();
        let context = body.get("context");
        let client = context.and_then(|c| c.get("client"));
        let user = context.and_then(|c| c.get("user"));
        let client_name = client
            .and_then(|c| c.get("clientName"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        let client_version = client
            .and_then(|c| c.get("clientVersion"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        let browse_id = body.get("browseId").and_then(Value::as_str).unwrap_or("-");
        let on_behalf = if user.and_then(|u| u.get("onBehalfOfUser")).is_some() {
            "yes"
        } else {
            "no"
        };
        let continuation = if body.get("continuation").and_then(Value::as_str).is_some() {
            "yes"
        } else {
            "no"
        };
        let query_length = body
            .get("query")
            .and_then(Value::as_str)
            .map_or(0, |q| q.chars().count());
        format!(
            "keys=[{}] contextClient={client_name}/{client_version} userOnBehalfOfUser={on_behalf} \
             browseId={browse_id} continuation={continuation} queryLength={query_length}",
            keys.join(",")
        )
    }

    pub fn is_retryable(error: &InnerTubeError) -> bool {
        match error {
            // 5xx and rate-limits are transient; 4xx (bad request, blocked) are not.
            InnerTubeError::HttpError { status_code, .. } => {
                *status_code >= 500 || *status_code == 429
            }
            // A 200 with an undecodable body will not recover by retrying.
            InnerTubeError::DecodingFailed | InnerTubeError::InvalidResponse => false,
            // Network errors (Swift URLError) retry, like every other case.
            _ => true,
        }
    }

    // MARK: - Context

    pub fn context_dict(
        &self,
        client: Option<&YouTubeClient>,
        locale: Option<&YouTubeLocale>,
    ) -> Value {
        let config = self.config();
        let c = client.unwrap_or(&config.default_client);
        let l = locale.unwrap_or(&config.default_locale);

        let mut client_dict = JsonDict::new();
        client_dict.insert("clientName".into(), c.client_name.into());
        client_dict.insert("clientVersion".into(), c.client_version.into());
        client_dict.insert("gl".into(), l.gl.clone().into());
        client_dict.insert("hl".into(), l.hl.clone().into());
        insert_some(
            &mut client_dict,
            "visitorData",
            lock(&self.auth).visitor_data.clone(),
        );
        if c.include_user_agent_in_context {
            client_dict.insert("userAgent".into(), c.user_agent.into());
        }
        insert_some(&mut client_dict, "osName", c.os_name);
        insert_some(&mut client_dict, "osVersion", c.os_version);
        insert_some(&mut client_dict, "deviceMake", c.device_make);
        insert_some(&mut client_dict, "deviceModel", c.device_model);
        insert_some(&mut client_dict, "androidSdkVersion", c.android_sdk_version);
        insert_some(&mut client_dict, "platform", c.platform);
        insert_some(&mut client_dict, "clientScreen", c.client_screen);
        insert_some(&mut client_dict, "embedUrl", c.embed_url);

        // DATASYNC_ID is only needed for explicit delegated/channel sessions.
        // Sending it for the normal signed-in account makes WEB browse and
        // account_menu return a generic backend 500.
        json!({
            "client": client_dict,
            "request": { "internalExperimentFlags": [], "useSsl": true },
            "user": { "lockedSafetyMode": false },
        })
    }
}

/// Swift `if let x { body["key"] = x }`.
pub(crate) fn insert_some(dict: &mut JsonDict, key: &str, value: Option<impl Into<Value>>) {
    if let Some(value) = value {
        dict.insert(key.to_string(), value.into());
    }
}
