//! Renews the rotating `__Secure-*PSIDTS` tokens. Google stops trusting a
//! session that keeps presenting stale ones: `account/account_menu` answers
//! guest-shaped while browsing still works.
//!
//! Loading music.youtube.com re-mints nothing. Only an accounts.google.com
//! round trip reissues the youtube.com cookies, so the app seeds the saved
//! cookies into an offscreen, non-persistent webview (desktop Safari UA),
//! loads [`LOGIN_ENTRY_URL`], waits up to [`POLL_SECONDS`] to land on
//! music.youtube.com, settles [`SETTLE_SECONDS`], and passes the cookie store
//! to [`reissued`].

use std::collections::BTreeMap;

use super::cookie_harvest::{self, Cookie};

pub const LOGIN_ENTRY_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";
pub const POLL_SECONDS: u32 = 40;
pub const SETTLE_SECONDS: u32 = 4;

/// Domain a saved cookie is seeded under.
pub fn seed_domain(name: &str) -> &'static str {
    if cookie_harvest::is_google_auth_cookie(name) {
        ".google.com"
    } else {
        ".youtube.com"
    }
}

/// The reissued cookie set, or empty when Google did not prove the session
/// (no SAPISID: a real sign-in is needed) or nothing rotated.
pub fn reissued(base: &BTreeMap<String, String>, store: &[Cookie]) -> BTreeMap<String, String> {
    let (harvested, sapisid) = cookie_harvest::select(store);
    if sapisid.is_none() || harvested.is_empty() || harvested == *base {
        return BTreeMap::new();
    }
    let changed: Vec<_> = harvested
        .iter()
        .filter(|(name, value)| base.get(*name) != Some(value))
        .map(|(name, _)| name.as_str())
        .collect();
    log::info!(
        "SessionTokenRefresher: session reissued, changed cookies: {}",
        changed.join(",")
    );
    harvested
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yt(name: &str, value: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: value.into(),
            domain: ".youtube.com".into(),
        }
    }

    #[test]
    fn only_a_changed_proven_set_counts() {
        let base = BTreeMap::from([
            ("SAPISID".to_owned(), "s".to_owned()),
            ("__Secure-1PSIDTS".to_owned(), "old".to_owned()),
        ]);
        assert!(reissued(&base, &[yt("SAPISID", "s"), yt("__Secure-1PSIDTS", "old")]).is_empty());
        assert!(reissued(&base, &[yt("__Secure-1PSIDTS", "new")]).is_empty());
        let renewed = reissued(&base, &[yt("SAPISID", "s"), yt("__Secure-1PSIDTS", "new")]);
        assert_eq!(renewed["__Secure-1PSIDTS"], "new");
        assert_eq!(seed_domain("SID"), ".google.com");
        assert_eq!(seed_domain("PREF"), ".youtube.com");
    }
}
