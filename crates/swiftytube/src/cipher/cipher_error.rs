#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CipherError {
    #[error("Player hash not found in iframe_api")]
    HashNotFound,
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    #[error("Cache directory unavailable")]
    CacheUnavailable,
    #[error("Signature timestamp not found in player.js")]
    SignatureTimestampNotFound,
    #[error("Cipher function not found: {0}")]
    FunctionNotFound(String),
    #[error("JS execution failed: {0}")]
    JsExecutionFailed(String),
    #[error("No config entry for this player hash")]
    ConfigNotAvailable,
    #[error("Deobfuscation failed: {0}")]
    DeobfuscationFailed(String),
    #[error("n-param transform failed: {0}")]
    NTransformFailed(String),
}
