use std::time::{SystemTime, UNIX_EPOCH};

use sha1::{Digest, Sha1};

/// SAPISIDHASH auth header.
pub struct SAPISIDAuth;

impl SAPISIDAuth {
    pub fn authorization_header(sapisid: &str, origin: &str) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self::authorization_header_at(sapisid, origin, now)
    }

    /// Fixed `timestamp` (Unix seconds) for tests.
    pub fn authorization_header_at(sapisid: &str, origin: &str, timestamp: u64) -> String {
        let input = format!("{timestamp} {sapisid} {origin}");
        let hash: String = Sha1::digest(input.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        format!("SAPISIDHASH {timestamp}_{hash}")
    }
}
