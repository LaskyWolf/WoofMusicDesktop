use crate::cipher::cipher_error::CipherError;
use crate::networking::inner_tube_error::InnerTubeError;

/// Stream resolution errors.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("Not playable: {reason}")]
    Unplayable { reason: String },
    #[error("No streaming data in response")]
    NoStreams,
    #[error("No suitable format found")]
    NoSuitableFormat,
    #[error("Format has no direct stream URL and no cipher data")]
    NoStreamUrl,
    #[error("{0} stream URL failed Range validation")]
    ValidationFailed(String),
    #[error("All clients failed to resolve a valid stream URL")]
    AllClientsFailed,
    /// Swift rethrows the `/player` error as is (`any Error`); Rust needs one error type.
    #[error(transparent)]
    InnerTube(#[from] InnerTubeError),
    /// Swift rethrows the cipher provider's error as is.
    #[error(transparent)]
    Cipher(#[from] CipherError),
}
