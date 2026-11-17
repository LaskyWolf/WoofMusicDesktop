use serde::{Deserialize, Serialize};

/// Streaming quality preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioQuality {
    #[default]
    Auto,
    High,
    Medium,
    Low,
}

/// Download quality preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadQuality {
    Auto,
    #[default]
    High,
    Standard,
}

/// Video resolution preference for the muxed stream. `Auto` picks the best.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VideoQuality {
    #[default]
    Auto,
    P2160,
    P1440,
    P1080,
    P720,
    P480,
    P360,
    P240,
}

impl VideoQuality {
    /// Target height in pixels, or None for `Auto`.
    pub fn height(self) -> Option<i64> {
        match self {
            Self::Auto => None,
            Self::P2160 => Some(2160),
            Self::P1440 => Some(1440),
            Self::P1080 => Some(1080),
            Self::P720 => Some(720),
            Self::P480 => Some(480),
            Self::P360 => Some(360),
            Self::P240 => Some(240),
        }
    }
}
