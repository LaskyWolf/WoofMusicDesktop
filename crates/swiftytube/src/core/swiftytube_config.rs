use super::youtube_client::YouTubeClient;
use super::youtube_locale::YouTubeLocale;
use super::youtube_service::YouTubeService;

/// Static setup. Per-request overrides (client, locale) go on each method.
#[derive(Clone, Debug, PartialEq)]
pub struct SwiftyTubeConfig {
    pub service: YouTubeService,
    pub default_client: YouTubeClient,
    pub default_locale: YouTubeLocale,
    pub timeout_seconds: f64,
    pub accept_language: String,
}

impl SwiftyTubeConfig {
    pub fn new(service: YouTubeService, default_client: YouTubeClient) -> Self {
        Self {
            service,
            default_client,
            default_locale: YouTubeLocale::default(),
            timeout_seconds: 30.0,
            accept_language: "en-US,en;q=0.9".to_string(),
        }
    }

    /// YouTube Music host + WEB_REMIX.
    pub fn music() -> Self {
        Self::new(YouTubeService::Music, YouTubeClient::WEB_REMIX)
    }

    /// Main YouTube host + WEB.
    pub fn youtube() -> Self {
        Self::new(YouTubeService::Youtube, YouTubeClient::WEB)
    }
}
