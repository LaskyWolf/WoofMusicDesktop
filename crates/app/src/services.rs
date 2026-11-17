//! The app's link to YouTube Music and libmpv: loads pages, covers and streams.

use std::sync::Arc;

use iced::widget::image;
use swiftytube::auth::auth_state::AuthState;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::core::youtube_client::YouTubeClient;
use swiftytube::networking::artwork_urls::ArtworkURLs;
use swiftytube::networking::inner_tube_client::InnerTubeClient;
use swiftytube::streaming::stream_cache::StreamCache;
use swiftytube::streaming::stream_fallback::{StreamFallback, StreamResolveRequest};
use swiftytube::streaming::stream_resolver::{StreamResolveOptions, StreamResolveProviders};
use swiftytube::sync::playlist_detail_service::PlaylistDetailService;
use trop::audio::player_controller::player_controller::PlayerController;
use trop::detail::album_detail_parser::AlbumDetailParser;
use trop::detail::artist_detail_parser::ArtistDetailParser;
use trop::detail::playlist_detail_parser::PlaylistDetailParser;
use trop::detail::podcast_detail_parser::PodcastDetailParser;
use trop::explore::explore_view_model::ExploreViewModel;
use trop::home::home_view_model::HomeViewModel;
use trop::networking::models::home_page::{ExploreSection, HomeSection};
use trop::search::search_parser::{SearchParser, SearchSection};
use trop::search::search_service::SearchService;
use trop::utilities::ui_image_color_extraction::{self as colors, Rgb};

use crate::{Detail, Route};

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

    /// Explore's feed for the US region (the Swift default until R5 settings).
    pub async fn load_explore(self: Arc<Self>) -> Result<Vec<ExploreSection>, String> {
        let mut explore = ExploreViewModel::new(self.client.clone(), "US");
        explore.load().await.map_err(|error| error.to_string())?;
        Ok(explore.sections)
    }

    pub async fn search(self: Arc<Self>, query: String) -> Result<Vec<SearchSection>, String> {
        let json = SearchService::new(self.client.clone())
            .search(&query, None, YouTubeClient::WEB_REMIX)
            .await
            .map_err(|error| error.to_string())?;
        Ok(SearchParser::parse_search_results(&json))
    }

    /// The data of an album, artist, playlist or mood page.
    pub async fn load_detail(self: Arc<Self>, route: Route) -> Result<Detail, String> {
        let client = &self.client;
        match route {
            Route::Album(id) => AlbumDetailParser::load(client, &id)
                .await
                .map(Detail::album)
                .map_err(|error| error.to_string()),
            Route::Artist(id) => ArtistDetailParser::load(client, &id)
                .await
                .map(Detail::artist)
                .map_err(|error| error.to_string()),
            Route::Playlist(id) => {
                PlaylistDetailParser::load(&PlaylistDetailService::new(client), &id)
                    .await
                    .map(Detail::playlist)
                    .map_err(|error| error.to_string())
            }
            Route::Podcast(id) => PodcastDetailParser::load(client, &id)
                .await
                .map(Detail::podcast)
                .map_err(|error| error.to_string()),
            Route::Mood(mood) => {
                let sections = ExploreViewModel::load_mood_detail(client, &mood).await;
                if sections.is_empty() {
                    Err(format!("{} has nothing to show.", mood.title))
                } else {
                    Ok(Detail::Mood(ExploreViewModel::mood_order(sections)))
                }
            }
            Route::Home | Route::Explore | Route::Library | Route::Search => {
                Err("Not a detail page.".to_owned())
            }
        }
    }

    /// The Now Playing cover, black borders cropped, and its two background colours.
    pub async fn artwork(self: Arc<Self>, url: String) -> Option<Artwork> {
        const PX: i64 = 1000;
        let sized = ArtworkURLs::artwork(Some(&url), PX).unwrap_or(url);
        let bytes = self.http.get(sized).send().await.ok()?.bytes().await.ok()?;
        // Decoding a 1000 px JPEG takes a few ms: off the async workers.
        tokio::task::spawn_blocking(move || Artwork::decode(&bytes))
            .await
            .ok()
            .flatten()
    }

    /// A playable stream URL: VISIONOS first, cipher clients after.
    pub async fn resolve(self: Arc<Self>, video_id: String) -> Result<String, String> {
        let request = StreamResolveRequest::playback(&video_id, StreamResolveOptions::default());
        StreamFallback::resolve_first_valid(
            &request,
            &self.client,
            StreamCache::shared(),
            &self.providers,
        )
        .await
        .map(|result| result.stream_url)
        .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct Artwork {
    pub image: image::Handle,
    /// Primary and secondary colours of the cover.
    pub colors: [Rgb; 2],
}

impl Artwork {
    fn decode(bytes: &[u8]) -> Option<Self> {
        let cover = ::image::load_from_memory(bytes).ok()?.into_rgba8();
        let (x, y, side, _) = colors::center_cropped_square(cover.width(), cover.height(), &cover);
        let square = ::image::imageops::crop_imm(&cover, x, y, side, side).to_image();
        let tiny =
            ::image::imageops::resize(&square, 8, 8, ::image::imageops::FilterType::Triangle);
        Some(Self {
            colors: colors::extract_dominant_colors(&tiny),
            image: image::Handle::from_rgba(side, side, square.into_raw()),
        })
    }
}
