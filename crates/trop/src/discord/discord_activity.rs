// Trop/Discord/DiscordActivity.swift.

use super::discord_presence::ActivityType;

#[derive(Clone, Debug, PartialEq)]
pub struct DiscordActivity {
    pub activity_type: ActivityType,
    pub name: Option<String>,
    pub state: String,
    pub details: Option<String>,
    /// Millis since 1970; 0 while paused.
    pub start_timestamp: i64,
    /// Millis; `None` while unknown.
    pub end_timestamp: Option<i64>,
    pub large_image: Option<String>,
    pub large_text: Option<String>,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    pub button1_label: Option<String>,
    pub button1_url: Option<String>,
    pub button2_label: Option<String>,
    pub button2_url: Option<String>,
}
