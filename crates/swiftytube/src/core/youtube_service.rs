/// InnerTube host: `.music` (YouTube Music) or `.youtube` (YouTube).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum YouTubeService {
    Music,
    Youtube,
}

impl YouTubeService {
    pub fn base_url(self) -> &'static str {
        match self {
            YouTubeService::Music => "https://music.youtube.com/youtubei/v1/",
            YouTubeService::Youtube => "https://www.youtube.com/youtubei/v1/",
        }
    }

    pub fn origin(self) -> &'static str {
        match self {
            YouTubeService::Music => "https://music.youtube.com",
            YouTubeService::Youtube => "https://www.youtube.com",
        }
    }
}
