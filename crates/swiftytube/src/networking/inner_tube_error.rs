use super::http_client::HttpError;
use super::retry_policy::RetryError;

#[derive(Debug, thiserror::Error)]
pub enum InnerTubeError {
    #[error("Invalid response from server")]
    InvalidResponse,
    #[error("Invalid response from server: {0}")]
    InvalidResponseContent(String),
    #[error("HTTP {status_code}")]
    HttpError { status_code: u16, data: Vec<u8> },
    #[error("Failed to decode response JSON")]
    DecodingFailed,
    #[error("Truncated response: {0}")]
    TruncatedResponse(String),
    #[error("Session expired — sign in again")]
    SessionExpired,
    /// Swift rethrows URLSession's `URLError` as is; Rust needs one error type, so it is wrapped here.
    #[error(transparent)]
    Transport(#[from] reqwest::Error),
    /// Swift throws `RetryError` as is (zero-attempt policy).
    #[error(transparent)]
    Retry(#[from] RetryError),
}

impl From<HttpError> for InnerTubeError {
    fn from(error: HttpError) -> Self {
        match error {
            HttpError::Status(status_code, data) => InnerTubeError::HttpError { status_code, data },
            HttpError::Transport(error) => InnerTubeError::Transport(error),
        }
    }
}
