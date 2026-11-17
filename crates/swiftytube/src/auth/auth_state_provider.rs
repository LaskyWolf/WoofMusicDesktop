use std::collections::BTreeMap;

/// Session source for `load_state`.
pub trait AuthStateProvider {
    fn cookies(&self) -> impl Future<Output = BTreeMap<String, String>> + Send;
    fn sapisid(&self) -> impl Future<Output = Option<String>> + Send;
    fn visitor_data(&self) -> impl Future<Output = Option<String>> + Send;
    fn data_sync_id(&self) -> impl Future<Output = Option<String>> + Send;
}
