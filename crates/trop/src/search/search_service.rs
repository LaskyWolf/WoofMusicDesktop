// Network half of Trop/Search/SearchService.swift. `localSearch` (GRDB
// queries on the library tables) lands with the database port (R5).
//
// Swift cancels an older search with a generation counter (`CancellationError`).
// In Rust the caller drops or aborts the older future instead, so `search`
// has no counter.

use std::sync::Arc;

use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Method, Request, Url};
use serde_json::{Value, json};
use swiftytube::core::youtube_client::YouTubeClient;
use swiftytube::networking::http_client::HttpClient;
use swiftytube::networking::inner_tube_client::{InnerTubeClient, SearchArgs};
use swiftytube::networking::inner_tube_decode::{InnerTubeDecode, JsonDict, as_dict, as_dict_list};
use swiftytube::networking::inner_tube_error::InnerTubeError;

pub struct SearchService {
    pub inner_tube: Arc<InnerTubeClient>,
}

impl SearchService {
    pub fn new(inner_tube: Arc<InnerTubeClient>) -> Self {
        Self { inner_tube }
    }

    /// Raw `/search` JSON; parse it with `SearchParser::parse_search_results`.
    /// Swift default `client`: `WEB_REMIX`.
    pub async fn search(
        &self,
        query: &str,
        params: Option<String>,
        client: YouTubeClient,
    ) -> Result<JsonDict, InnerTubeError> {
        self.inner_tube
            .search(SearchArgs {
                query: query.to_string(),
                params,
                client: Some(client),
                ..SearchArgs::default()
            })
            .await
    }

    /// YouTube Music's autocomplete. Needs no account, and works for guests the
    /// same way. Swift default `country`: "".
    pub async fn suggestions(
        &self,
        query: &str,
        country: &str,
    ) -> Result<Vec<String>, InnerTubeError> {
        Ok(Self::parse_suggestions(
            &self.suggestions_data(query, country).await?,
        ))
    }

    /// Raw `music/get_search_suggestions` answer (also used to record test fixtures).
    pub async fn suggestions_data(
        &self,
        query: &str,
        country: &str,
    ) -> Result<Vec<u8>, InnerTubeError> {
        let config = self.inner_tube.config();
        let web = &config.default_client;
        let origin = config.service.origin();
        let url = Url::parse(config.service.base_url())
            .and_then(|base| base.join("music/get_search_suggestions"))
            .map_err(|_| InnerTubeError::InvalidResponse)?;
        let body = json!({
            "context": {
                "client": {
                    "clientName": web.client_name,
                    "clientVersion": web.client_version,
                    "gl": if country.is_empty() { "US" } else { country },
                    "hl": config.default_locale.hl,
                },
            },
            "input": query,
        });
        let mut request = Request::new(Method::POST, url);
        for (name, value) in [
            ("content-type", "application/json".to_string()),
            ("x-origin", origin.to_string()),
            ("referer", format!("{origin}/")),
            ("user-agent", web.user_agent.to_string()),
        ] {
            if let Ok(value) = HeaderValue::from_str(&value) {
                request
                    .headers_mut()
                    .insert(HeaderName::from_static(name), value);
            }
        }
        *request.body_mut() = Some(body.to_string().into());
        let response = HttpClient::data(request, self.inner_tube.session()).await?;
        if response.status != 200 {
            return Err(InnerTubeError::HttpError {
                status_code: response.status,
                data: response.data,
            });
        }
        Ok(response.data)
    }

    /// Takes the text suggestions. The answer's other section lists songs and
    /// artists, which have no `searchEndpoint`.
    pub fn parse_suggestions(data: &[u8]) -> Vec<String> {
        let Ok(Value::Object(root)) = serde_json::from_slice::<Value>(data) else {
            return Vec::new();
        };
        let Some(sections) = as_dict_list(root.get("contents")) else {
            return Vec::new();
        };
        sections
            .into_iter()
            .filter_map(|section| as_dict(section.get("searchSuggestionsSectionRenderer")))
            .flat_map(|renderer| as_dict_list(renderer.get("contents")).unwrap_or_default())
            .filter_map(|item| {
                let suggestion = as_dict(item.get("searchSuggestionRenderer"))
                    .or_else(|| as_dict(item.get("historySuggestionRenderer")))?;
                InnerTubeDecode::string(
                    &["navigationEndpoint", "searchEndpoint", "query"],
                    suggestion,
                )
                .map(str::to_string)
            })
            .collect()
    }
}
