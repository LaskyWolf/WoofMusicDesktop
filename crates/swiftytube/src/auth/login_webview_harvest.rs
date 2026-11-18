//! Rules for collecting a session from a webview on Google sign-in, once it
//! lands on music.youtube.com. The app owns the webview; this owns the rules:
//!
//! 1. The webview MUST use a desktop Safari user agent: Google serves a
//!    cookie-incomplete flow to other agents.
//! 2. The first snapshot after the redirect is incomplete. Poll the cookie
//!    store once a second and import once the youtube.com family has a SAPISID
//!    and fresh `__Secure-*PSIDTS` tokens.

use std::collections::BTreeMap;

/// Desktop Safari UA, required for the login webview.
pub const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.5 Safari/605.1.15";

/// Where sign-in starts; Google redirects to music.youtube.com when done.
pub const LOGIN_URL: &str =
    "https://accounts.google.com/ServiceLogin?service=youtube&continue=https://music.youtube.com";

/// Polls, one a second, before giving up.
pub const MAX_ATTEMPTS: u32 = 25;

/// Reads the account's data sync id from the landed page.
pub const DATA_SYNC_ID_SCRIPT: &str = "window.yt?.config_?.DATASYNC_ID ?? ''";

/// Swift `collectSession`'s check after poll `attempt` (1-based): a SAPISID,
/// and the rotated tokens after a minimum 3 s settle, or the last snapshot at
/// the cap.
pub fn is_complete(
    attempt: u32,
    cookies: &BTreeMap<String, String>,
    sapisid: Option<&str>,
) -> bool {
    sapisid.is_some()
        && ((super::cookie_harvest::has_fresh_rotating_tokens(cookies) && attempt >= 3)
            || attempt >= MAX_ATTEMPTS)
}

/// True once the webview is on music.youtube.com.
pub fn landed(url: &str) -> bool {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .is_some_and(|host| host.contains("music.youtube.com"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_tokens_and_the_settle() {
        let fresh = BTreeMap::from([("__Secure-1PSIDTS".to_owned(), "t".to_owned())]);
        let stale = BTreeMap::new();
        assert!(!is_complete(2, &fresh, Some("s")));
        assert!(is_complete(3, &fresh, Some("s")));
        assert!(!is_complete(3, &fresh, None));
        assert!(!is_complete(24, &stale, Some("s")));
        assert!(is_complete(MAX_ATTEMPTS, &stale, Some("s")));
    }

    #[test]
    fn landing() {
        assert!(landed("https://music.youtube.com/"));
        assert!(!landed(
            "https://accounts.google.com/x?continue=https://music.youtube.com"
        ));
    }
}
