/// Playback-specific request knobs for `/player` calls.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerRequestOptions {
    /// Player.js signature timestamp, required by web clients.
    pub signature_timestamp: Option<i64>,
    /// PoToken for web clients (`serviceIntegrityDimensions`).
    pub po_token: Option<String>,
}
