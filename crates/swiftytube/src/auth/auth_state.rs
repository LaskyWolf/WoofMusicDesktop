use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Session fields for signed-in requests. Apps own storage; the lib only reads.
/// `BTreeMap` keeps cookies sorted, so the `Cookie` header is stable (Swift dictionaries have no order).
/// Serde keys match Swift's synthesized `Codable`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    #[serde(default)]
    pub cookies: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sapisid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visitor_data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_sync_id: Option<String>,
}

impl AuthState {
    /// Swift `AuthState.guest`.
    pub fn guest() -> Self {
        Self::default()
    }
}
