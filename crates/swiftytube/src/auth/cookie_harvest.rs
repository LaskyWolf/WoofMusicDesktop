//! Harvests a usable YouTube session from a webview's cookie store.
//!
//! The rules are load-bearing (SwiftyTube Docs/Usage.md, "Webview sign-in"):
//!
//! 1. Google issues one session-cookie family per auth domain (`SID`, `HSID`,
//!    `__Secure-1PSIDTS`, …), with different values per domain.
//! 2. InnerTube requests carry ONLY the youtube.com family. Mixing a google.com
//!    value into an overlapping name makes every signed-in request answer as a
//!    guest, with healthy-looking 200s.
//! 3. The rotating `__Secure-1PSIDTS`/`__Secure-3PSIDTS` tokens live only in
//!    the youtube.com family; missing or stale ones are rejected the same way.

use std::collections::BTreeMap;

/// One cookie out of a webview's store. Swift reads `HTTPCookie`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
}

/// True for names in the .google.com auth family; everything else is a
/// youtube.com cookie.
pub fn is_google_auth_cookie(name: &str) -> bool {
    matches!(
        name,
        "SID"
            | "HSID"
            | "SSID"
            | "SIDCC"
            | "APISID"
            | "SAPISID"
            | "__Secure-1PSID"
            | "__Secure-3PSID"
            | "__Secure-1PAPISID"
            | "__Secure-3PAPISID"
            | "__Secure-1PSIDCC"
            | "__Secure-3PSIDCC"
            | "__Secure-1PSIDTS"
            | "__Secure-3PSIDTS"
            | "OSID"
            | "LSID"
            | "LSOSID"
    )
}

/// The session cookie map (the youtube.com family alone) and the SAPISID.
/// google.com families only supply a SAPISID when the youtube.com family has
/// none: its value is the same on every domain.
pub fn select(cookies: &[Cookie]) -> (BTreeMap<String, String>, Option<String>) {
    let mut youtube = BTreeMap::new();
    // Sorted by group, so the fallback is the same on every run (Swift takes
    // whichever its dictionary yields first).
    let mut google_sapisid = BTreeMap::new();
    for cookie in cookies.iter().filter(|cookie| !cookie.value.is_empty()) {
        let domain = cookie.domain.to_lowercase();
        if domain.contains("youtube.com") {
            youtube.insert(cookie.name.clone(), cookie.value.clone());
        } else if domain.contains("google.")
            && matches!(cookie.name.as_str(), "__Secure-3PSAPISID" | "SAPISID")
        {
            let group = if domain.contains("google.com.hk") {
                "google.com.hk".to_owned()
            } else if domain.contains("google.com") {
                "google.com".to_owned()
            } else {
                format!("google-other-{}", domain.replace('.', "-"))
            };
            google_sapisid.insert(group, cookie.value.clone());
        }
    }
    let sapisid = youtube
        .get("SAPISID")
        .or_else(|| youtube.get("__Secure-3PSAPISID"))
        .or_else(|| google_sapisid.values().next())
        .cloned();
    (youtube, sapisid)
}

/// True when the map carries the rotated youtube.com tokens Google requires.
pub fn has_fresh_rotating_tokens(cookies: &BTreeMap<String, String>) -> bool {
    cookies.contains_key("__Secure-1PSIDTS") || cookies.contains_key("__Secure-3PSIDTS")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(name: &str, value: &str, domain: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: value.into(),
            domain: domain.into(),
        }
    }

    #[test]
    fn keeps_only_the_youtube_family() {
        let (cookies, sapisid) = select(&[
            cookie("SID", "yt-sid", ".youtube.com"),
            cookie("SID", "google-sid", ".google.com"),
            cookie("SAPISID", "same", ".youtube.com"),
            cookie("HSID", "", ".youtube.com"),
        ]);
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies["SID"], "yt-sid");
        assert_eq!(sapisid.as_deref(), Some("same"));
    }

    #[test]
    fn falls_back_to_a_google_sapisid() {
        let (cookies, sapisid) = select(&[
            cookie("SID", "yt-sid", ".youtube.com"),
            cookie("__Secure-3PSAPISID", "from-google", ".google.com"),
        ]);
        assert!(!cookies.contains_key("__Secure-3PSAPISID"));
        assert_eq!(sapisid.as_deref(), Some("from-google"));
        assert_eq!(select(&[cookie("SID", "x", ".youtube.com")]).1, None);
    }

    #[test]
    fn fresh_tokens() {
        let (cookies, _) = select(&[cookie("__Secure-3PSIDTS", "t", ".youtube.com")]);
        assert!(has_fresh_rotating_tokens(&cookies));
        assert!(!has_fresh_rotating_tokens(&BTreeMap::new()));
        assert!(is_google_auth_cookie("LSID") && !is_google_auth_cookie("PREF"));
    }
}
