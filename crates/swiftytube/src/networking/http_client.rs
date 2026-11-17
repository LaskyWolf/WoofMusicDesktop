use std::sync::{Arc, LazyLock};

use reqwest::header::HeaderMap;

/// Swift `(Data, HTTPURLResponse)`.
#[derive(Clone, Debug, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub data: Vec<u8>,
}

/// Answers requests in tests without the network (Swift tests use a `URLProtocol` stub).
pub type MockHandler = Arc<dyn Fn(reqwest::Request) -> HttpResponse + Send + Sync>;

/// Swift `URLSession`.
#[derive(Clone)]
pub enum Session {
    Live(reqwest::Client),
    Mock(MockHandler),
}

impl Session {
    /// Swift `URLSession.shared`.
    pub fn shared() -> Session {
        static SHARED: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);
        Session::Live(SHARED.clone())
    }

    pub fn mock(
        handler: impl Fn(reqwest::Request) -> HttpResponse + Send + Sync + 'static,
    ) -> Session {
        Session::Mock(Arc::new(handler))
    }
}

/// reqwest helpers returning data + HTTP response.
pub struct HttpClient;

impl HttpClient {
    /// Data + response. No status check.
    pub async fn data(
        request: reqwest::Request,
        session: &Session,
    ) -> Result<HttpResponse, HttpError> {
        match session {
            Session::Live(client) => {
                let response = client.execute(request).await?;
                let status = response.status().as_u16();
                let headers = response.headers().clone();
                let data = response.bytes().await?.to_vec();
                Ok(HttpResponse {
                    status,
                    headers,
                    data,
                })
            }
            Session::Mock(handler) => Ok(handler(request)),
        }
    }

    /// Performs a GET for `url`. See `data`.
    pub async fn data_from(
        url: reqwest::Url,
        session: &Session,
    ) -> Result<HttpResponse, HttpError> {
        Self::data(reqwest::Request::new(reqwest::Method::GET, url), session).await
    }

    /// Data + response, failing on non-2xx.
    pub async fn validated_data(
        request: reqwest::Request,
        session: &Session,
    ) -> Result<HttpResponse, HttpError> {
        let response = Self::data(request, session).await?;
        if !(200..=299).contains(&response.status) {
            return Err(HttpError::Status(response.status, response.data));
        }
        Ok(response)
    }
}

/// Swift `HttpError`. `notHTTPResponse` has no Rust case: reqwest always returns an HTTP response;
/// transport failures (Swift `URLError`) are `Transport`.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("HTTP {0}")]
    Status(u16, Vec<u8>),
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
}
