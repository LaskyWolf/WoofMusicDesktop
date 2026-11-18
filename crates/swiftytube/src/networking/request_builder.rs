use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Method, Request, Url};

use super::inner_tube_decode::JsonDict;
use super::retry_policy::random_u64;
use crate::auth::auth_state::AuthState;
use crate::auth::sapisid_auth::SAPISIDAuth;
use crate::core::swiftytube_config::SwiftyTubeConfig;
use crate::core::youtube_client::YouTubeClient;

/// reqwest Requests from config + client + auth. No app singletons.
pub struct RequestBuilder;

impl RequestBuilder {
    /// Swift `request.timeoutInterval = 15` on tracking requests.
    pub const PLAYBACK_TRACKING_TIMEOUT: Duration = Duration::from_secs(15);

    pub fn build_request(
        config: &SwiftyTubeConfig,
        endpoint: &str,
        body: &JsonDict,
        client: &YouTubeClient,
        auth: &AuthState,
    ) -> Request {
        let url = Url::parse(config.service.base_url())
            .and_then(|base| base.join(endpoint))
            .expect("InnerTube base URL + endpoint is a valid URL");
        let mut request = Request::new(Method::POST, url);
        let headers = request.headers_mut();
        Self::apply_client_headers(headers, config, client);
        set(headers, "Accept-Language", &config.accept_language);
        Self::apply_auth_headers(headers, config, client, auth);
        *request.body_mut() = Some(
            serde_json::to_vec(body)
                .expect("JSON body always serializes")
                .into(),
        );
        request
    }

    pub fn build_playback_tracking_request(
        config: &SwiftyTubeConfig,
        tracking_url: &str,
        client: &YouTubeClient,
        auth: &AuthState,
        playlist_id: Option<&str>,
    ) -> Option<Request> {
        let mut url = Url::parse(tracking_url).ok()?;
        // Rust only: a URL that names its viewing (`cpn`) keeps it, so all
        // reports of one play share it (Trop `YouTubeHistoryReport`).
        let has_cpn = url.query_pairs().any(|(key, _)| key == "cpn");
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("c", client.client_name);
            if !has_cpn {
                query.append_pair("cpn", &Self::random_cpn());
            }
            query.append_pair("ver", "2");
            if let Some(playlist_id) = playlist_id {
                query.append_pair("list", playlist_id);
                let referrer = format!("{}/playlist?list={playlist_id}", config.service.origin());
                query.append_pair("referrer", &referrer);
            }
        }
        let mut request = Request::new(Method::GET, url);
        *request.timeout_mut() = Some(Self::PLAYBACK_TRACKING_TIMEOUT);
        let headers = request.headers_mut();
        Self::apply_client_headers(headers, config, client);
        Self::apply_auth_headers(headers, config, client, auth);
        Some(request)
    }

    /// Shared head of both Swift builders.
    fn apply_client_headers(
        headers: &mut HeaderMap,
        config: &SwiftyTubeConfig,
        client: &YouTubeClient,
    ) {
        set(headers, "Content-Type", "application/json");
        set(headers, "X-Goog-Api-Format-Version", "1");
        set(
            headers,
            "X-YouTube-Client-Name",
            &client.client_id.to_string(),
        );
        set(headers, "X-YouTube-Client-Version", client.client_version);
        set(headers, "X-Origin", config.service.origin());
        set(headers, "Referer", &format!("{}/", config.service.origin()));
        set(headers, "User-Agent", client.user_agent);
    }

    /// Shared tail of both Swift builders (visitor id + login headers).
    fn apply_auth_headers(
        headers: &mut HeaderMap,
        config: &SwiftyTubeConfig,
        client: &YouTubeClient,
        auth: &AuthState,
    ) {
        if let Some(visitor_data) = &auth.visitor_data {
            set(headers, "X-Goog-Visitor-Id", visitor_data);
        }

        if client.login_supported {
            // YouTube's web client expects the active Google account index on
            // authenticated InnerTube requests. The default account is 0.
            set(headers, "X-Goog-AuthUser", "0");
            if !auth.cookies.is_empty() {
                let cookie_string = auth
                    .cookies
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                set(headers, "Cookie", &cookie_string);
            }
            if let Some(sapisid) = &auth.sapisid {
                let authorization =
                    SAPISIDAuth::authorization_header(sapisid, config.service.origin());
                set(headers, "Authorization", &authorization);
            }
        }
    }

    fn random_cpn() -> String {
        const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";
        (0..16)
            .map(|_| CHARS[(random_u64() % CHARS.len() as u64) as usize] as char)
            .collect()
    }
}

/// Swift `setValue(_:forHTTPHeaderField:)`. A value with control characters
/// (only possible from a pasted cookie) is dropped instead of failing the request.
fn set(headers: &mut HeaderMap, name: &'static str, value: &str) {
    match HeaderValue::from_bytes(value.as_bytes()) {
        Ok(value) => {
            headers.insert(name, value);
        }
        Err(_) => crate::core::swiftytube_log::SwiftyTubeLog::error(&format!(
            "[HTTP] dropped invalid {name} header"
        )),
    }
}
