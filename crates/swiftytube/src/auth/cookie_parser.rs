use std::collections::BTreeMap;

/// Splits a raw `Set-Cookie`-style string into name→value pairs.
pub struct CookieParser;

impl CookieParser {
    pub fn parse(cookie_string: &str) -> BTreeMap<String, String> {
        cookie_string
            .split(';')
            .filter_map(|pair| pair.trim().split_once('='))
            // Swift split drops empty name/value.
            .filter(|(name, value)| !name.is_empty() && !value.is_empty())
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }
}
