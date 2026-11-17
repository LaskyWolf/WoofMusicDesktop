/// Signed-in account (from account menu).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct AccountInfo {
    pub name: String,
    pub email: Option<String>,
    pub channel_handle: Option<String>,
    pub thumbnail_url: Option<String>,
    pub channel_id: Option<String>,
}
