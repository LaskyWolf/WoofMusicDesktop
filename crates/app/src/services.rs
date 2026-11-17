//! The app's link to YouTube Music and libmpv: loads pages, covers and streams.

use std::sync::Arc;

use iced::widget::image;
use swiftytube::auth::auth_state::AuthState;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::networking::artwork_urls::ArtworkURLs;
use swiftytube::networking::inner_tube_client::InnerTubeClient;
use swiftytube::streaming::stream_cache::StreamCache;
use swiftytube::streaming::stream_fallback::{StreamFallback, StreamResolveRequest};
use swiftytube::streaming::stream_resolver::{StreamResolveOptions, StreamResolveProviders};
use trop::audio::player_controller::player_controller::PlayerController;
use trop::home::home_view_model::HomeViewModel;
use trop::networking::models::home_page::HomeSection;

pub struct Services {
    pub client: Arc<InnerTubeClient>,
    /// `None` when libmpv could not start; the UI says so instead of playing.
    pub player: Option<Arc<PlayerController>>,
    providers: StreamResolveProviders,
    http: reqwest::Client,
}

impl Services {
    pub fn new() -> Arc<Self> {
        let player = match PlayerController::new() {
            Ok(player) => {
                // Dev aid: WOOF_MUTE=1 starts muted.
                if std::env::var("WOOF_MUTE").is_ok() {
                    let _ = player.active_deck().set("mute", "yes");
                }
                Some(Arc::new(player))
            }
            Err(error) => {
                eprintln!("libmpv did not start: {error}");
                None
            }
        };
        Arc::new(Self {
            client: Arc::new(InnerTubeClient::new(
                SwiftyTubeConfig::music(),
                AuthState::guest(),
            )),
            player,
            providers: StreamResolveProviders::live(),
            http: reqwest::Client::new(),
        })
    }

    pub async fn load_home(self: Arc<Self>) -> Result<Vec<HomeSection>, String> {
        let mut home = HomeViewModel::new(self.client.clone());
        home.load().await.map_err(|error| error.to_string())?;
        Ok(home.home_sections())
    }

    /// Cover at about `px` physical pixels, from the YouTube image CDN.
    pub async fn cover(self: Arc<Self>, url: String, px: i64) -> Option<image::Handle> {
        let sized = ArtworkURLs::artwork(Some(&url), px).unwrap_or(url);
        let bytes = self.http.get(sized).send().await.ok()?.bytes().await.ok()?;
        Some(image::Handle::from_bytes(bytes))
    }

    /// Resolves a playable stream (VISIONOS first, cipher clients after) and plays it.
    pub async fn play(self: Arc<Self>, video_id: String) -> Result<(), String> {
        let player = self
            .player
            .clone()
            .ok_or("The audio player (libmpv) did not start.")?;
        let request = StreamResolveRequest::playback(&video_id, StreamResolveOptions::default());
        let result = StreamFallback::resolve_first_valid(
            &request,
            &self.client,
            StreamCache::shared(),
            &self.providers,
        )
        .await
        .map_err(|error| error.to_string())?;
        player
            .play(&result.stream_url, Some(&video_id))
            .map_err(|error| error.to_string())
    }
}
