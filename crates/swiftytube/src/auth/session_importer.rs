use std::collections::BTreeMap;

use super::auth_state::AuthState;
use super::cookie_parser::CookieParser;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SessionImportError {
    #[error("Cookie string is empty or invalid")]
    InvalidCookie,
    #[error("Both SAPISID and __Secure-3PSAPISID missing from cookies")]
    SapisidNotFound,
}

/// Cookie string → sign-in state. Lenient: only empty input fails.
pub struct SessionImporter;

impl SessionImporter {
    pub fn import_session(
        cookie_string: &str,
        data_sync_id: Option<String>,
    ) -> Result<AuthState, SessionImportError> {
        if cookie_string.is_empty() {
            return Err(SessionImportError::InvalidCookie);
        }
        let cookies = CookieParser::parse(cookie_string);
        Ok(AuthState {
            sapisid: Self::extract_sapisid(&cookies),
            visitor_data: Self::extract_visitor_data(&cookies),
            cookies,
            data_sync_id,
        })
    }

    /// Prefers the `__Secure-3PSAPISID` cookie over legacy `SAPISID`.
    pub fn extract_sapisid(cookies: &BTreeMap<String, String>) -> Option<String> {
        cookies
            .get("__Secure-3PSAPISID")
            .or_else(|| cookies.get("SAPISID"))
            .cloned()
    }

    /// YouTube sets visitor data as a plain cookie; reuse it as the header value.
    pub fn extract_visitor_data(cookies: &BTreeMap<String, String>) -> Option<String> {
        cookies.get("visitor_data").cloned()
    }
}
