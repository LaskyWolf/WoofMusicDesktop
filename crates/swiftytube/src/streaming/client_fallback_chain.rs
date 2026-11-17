use crate::core::youtube_client::YouTubeClient;

/// Wrapper for a YouTubeClient with validation skip flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FallbackClient {
    pub client: YouTubeClient,
    pub skip_validation: bool,
}

impl FallbackClient {
    pub const fn new(client: YouTubeClient, skip_validation: bool) -> Self {
        Self {
            client,
            skip_validation,
        }
    }
}

/// Clients to try in order; first valid URL wins.
pub struct ClientFallbackChain;

impl ClientFallbackChain {
    /// Direct-URL clients first, token-backed web clients last.
    const DIRECT: [FallbackClient; 8] = [
        FallbackClient::new(YouTubeClient::VISION_OS, false),
        FallbackClient::new(YouTubeClient::ANDROID_VR_1_65_10, false),
        FallbackClient::new(YouTubeClient::ANDROID_VR_1_61_48, false),
        FallbackClient::new(YouTubeClient::ANDROID_VR_1_43_32, false),
        FallbackClient::new(YouTubeClient::TV_HTML5_SIMPLY_EMBEDDED, false),
        FallbackClient::new(YouTubeClient::IOS, true),
        FallbackClient::new(YouTubeClient::TV_HTML5, false),
        FallbackClient::new(YouTubeClient::MOBILE, false),
    ];

    /// Fallback chain for YouTube Music playback.
    pub fn preferred() -> Vec<FallbackClient> {
        let mut chain = Self::DIRECT.to_vec();
        chain.push(FallbackClient::new(YouTubeClient::WEB_REMIX, true));
        chain
    }

    /// Fallback chain for main-YouTube playback.
    pub fn youtube() -> Vec<FallbackClient> {
        let mut chain = Self::DIRECT.to_vec();
        chain.push(FallbackClient::new(YouTubeClient::WEB, true));
        chain
    }

    /// Same order for downloads (validation stays on).
    pub fn for_download() -> Vec<FallbackClient> {
        Self::preferred()
    }
}
