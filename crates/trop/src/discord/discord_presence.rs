// The `ActivityType` enum of Trop/Discord/DiscordPresence.swift. Not ported:
// `PresenceStatus` (Discord IPC can't set the user's status) and
// `ActivityPayload` (the IPC crate has its own activity type).

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActivityType {
    Playing = 0,
    Streaming = 1,
    #[default]
    Listening = 2,
    Watching = 3,
    Custom = 4,
    Competing = 5,
}

impl ActivityType {
    /// Swift `ActivityType(rawValue:)`.
    pub fn from_raw(raw: i64) -> Option<Self> {
        [
            Self::Playing,
            Self::Streaming,
            Self::Listening,
            Self::Watching,
            Self::Custom,
            Self::Competing,
        ]
        .into_iter()
        .find(|kind| *kind as i64 == raw)
    }
}
