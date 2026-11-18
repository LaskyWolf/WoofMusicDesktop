// Trop/Odesli/OdesliDefaults.swift.

pub struct OdesliDefaults;

impl OdesliDefaults {
    pub const API_URL: &str = "https://api.song.link/v1-alpha.1/links";
    pub const KEYCHAIN_SERVICE: &str = "com.woofmusic.odesli";
    pub const KEY_ACCOUNT: &str = "key";
    /// A public song, sent to check a new key so no link of the user's is involved.
    pub const VALIDATION_LINK: &str = "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT";
}
