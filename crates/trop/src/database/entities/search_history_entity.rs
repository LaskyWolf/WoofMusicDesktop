use std::time::SystemTime;

/// A past search. Not a table: the search view model keeps the list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SearchHistoryEntity {
    pub query: String,
    pub timestamp: SystemTime,
}
