use reqwest::{Method, Request, Url};
use serde_json::Value;

use super::http_client::{HttpClient, Session};
use super::inner_tube_error::InnerTubeError;

/// Autocomplete suggestions. Unlike the InnerTube endpoints this hits the
/// public suggestion service (no auth, no context).
pub struct SearchSuggestions;

impl SearchSuggestions {
    const ENDPOINT: &str = "https://suggestqueries-clients6.youtube.com/complete/search";

    /// Swift defaults: `gl: "US"`, `hl: "en"`, `session: .shared`.
    pub async fn suggest(
        query: &str,
        gl: &str,
        hl: &str,
        session: &Session,
    ) -> Result<Vec<String>, InnerTubeError> {
        let url = Url::parse_with_params(
            Self::ENDPOINT,
            [
                ("client", "firefox"),
                ("ds", "yt"),
                ("q", query),
                ("hl", hl),
                ("gl", gl),
            ],
        )
        .map_err(|_| InnerTubeError::InvalidResponse)?;
        let mut request = Request::new(Method::GET, url);
        request.headers_mut().insert(
            "Accept-Language",
            reqwest::header::HeaderValue::from_static("en"),
        );
        let http = HttpClient::data(request, session).await?;
        if !(200..=299).contains(&http.status) {
            return Err(InnerTubeError::HttpError {
                status_code: http.status,
                data: http.data,
            });
        }
        Ok(Self::parse(&http.data))
    }

    /// Plain JSON array, or a `window.google.ac.h(...)` JSONP wrapper.
    pub fn parse(data: &[u8]) -> Vec<String> {
        let mut payload = data;
        if let Ok(text) = std::str::from_utf8(data)
            && let (Some(open), Some(close)) = (text.find('('), text.rfind(')'))
            && open < close
        {
            payload = &data[open + 1..close];
        }
        let Ok(Value::Array(root)) = serde_json::from_slice(payload) else {
            return Vec::new();
        };
        let Some(Value::Array(entries)) = root.get(1) else {
            return Vec::new();
        };
        entries
            .iter()
            .filter_map(|entry| entry.as_array()?.first()?.as_str().map(str::to_string))
            .collect()
    }
}
