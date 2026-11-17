use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoTokenResult {
    pub player_request_po_token: String,
    pub streaming_data_po_token: String,
}
