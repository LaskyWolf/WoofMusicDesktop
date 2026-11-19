//! The app's link to YouTube Music and libmpv: loads pages, covers and streams.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use iced::widget::image;
use swiftytube::auth::account_info::AccountInfo;
use swiftytube::auth::auth_state::AuthState;
use swiftytube::auth::profile_storage::{self, Registry, SavedProfile};
use swiftytube::auth::secure_store::{SecureAuthStore, SecureStore};
use swiftytube::auth::session_importer::SessionImporter;
use swiftytube::core::swiftytube_config::SwiftyTubeConfig;
use swiftytube::core::youtube_client::YouTubeClient;
use swiftytube::core::youtube_locale::YouTubeLocale;
use swiftytube::networking::artwork_urls::ArtworkURLs;
use swiftytube::networking::http_client::Session;
use swiftytube::networking::inner_tube_client::InnerTubeClient;
use swiftytube::streaming::stream_cache::StreamCache;
use swiftytube::streaming::stream_fallback::{StreamFallback, StreamResolveRequest};
use swiftytube::streaming::stream_resolver::{StreamResolveOptions, StreamResolveProviders};
use swiftytube::sync::incremental_sync_service::IncrementalSyncService;
use swiftytube::sync::playlist_detail_service::PlaylistDetailService;
use trop::audio::lyrics::lyrics_manager::{LyricSearchResult, LyricsManager};
use trop::audio::lyrics::lyrics_service::{LyricLine, LyricsService, LyricsSong};
use trop::audio::lyrics::lyrics_view::{self, InstrumentalGap};
use trop::audio::lyrics::providers::lyrics_provider::LyricsQuery;
use trop::audio::lyrics::romanizer::lyrics_romanizer::LyricsRomanizer;
use trop::audio::player_controller::player_controller::PlayerController;
use trop::audio::sponsor_block::{self, Segment};
use trop::database::database_service::DatabaseService;
use trop::database::entities::format_entity::FormatEntity;
use trop::database::entities::playlist_entity::PlaylistEntity;
use trop::database::entities::song_entity::SongEntity;
use trop::detail::album_detail_parser::AlbumDetailParser;
use trop::detail::artist_detail_parser::ArtistDetailParser;
use trop::detail::playlist_detail_parser::PlaylistDetailParser;
use trop::detail::podcast_detail_parser::PodcastDetailParser;
use trop::explore::explore_view_model::ExploreViewModel;
use trop::home::home_view_model::HomeViewModel;
use trop::networking::models::home_page::{ExploreSection, HomeSection};
use trop::networking::models::yt_item::{
    AlbumItem, ArtistItem, PlaylistItem, PodcastItem, SongItem, YTArtist, YTItem,
};
use trop::playback::playback_state_service::{PlayRecord, save};
use trop::playback::youtube_history_report::YouTubeHistoryReport;
use trop::search::search_parser::{SearchParser, SearchSection};
use trop::search::search_service::SearchService;
use trop::search::search_view_model;

use trop::services::artist_kind_service;
use trop::services::comments_service::CommentsService;
use trop::services::download_manager::{DownloadManager, Resolved};
use trop::services::downloaded_collections::DownloadedCollections;
use trop::services::personalization_service::{self, RadioOptions};
use trop::services::playlist_song_edits::PlaylistSongEdits;
use trop::services::sync_bridge::SyncBridge;
use trop::settings::settings_store::SettingsStore;
use trop::utilities::ui_image_color_extraction::{self as colors, Rgb};

use crate::{Detail, Route};
use trop::odesli::odesli::Odesli;
use trop::utilities::music_link::{self, MusicLink};

/// Swift `MusicLinkOpener.open(nil)`: what the app can open.
pub const LINK_HELP: &str = "Woof Music opens YouTube and YouTube Music links to songs, playlists, albums, artists and podcasts, Shazam links to songs, and Spotify and Apple Music links through Odesli.";

/// Where a music link leads.
#[derive(Debug, Clone)]
pub enum LinkTarget {
    Route(Route),
    /// A radio that starts on the linked song.
    Play(Arc<Vec<SongItem>>, usize),
}

/// What the Song Details page adds to the song row.
#[derive(Clone, Debug, Default)]
pub struct SongDetails {
    pub duration: Option<i64>,
    pub stream: Option<swiftytube::streaming::playback_result::PlaybackResult>,
    pub format: Option<FormatEntity>,
    pub views: Option<i64>,
}

pub struct Services {
    pub client: Arc<InnerTubeClient>,
    /// `None` when libmpv could not start; the UI says so instead of playing.
    pub player: Option<Arc<PlayerController>>,
    /// The synced library; in memory when the file cannot open.
    pub db: Arc<DatabaseService>,
    pub downloads: DownloadManager,
    pub(crate) sync: SyncBridge,
    lyrics: Arc<LyricsService>,
    providers: StreamResolveProviders,
    http: reqwest::Client,
    /// A song's YouTube comments.
    pub comments: Arc<CommentsService>,
    /// The app's copy, for the requests that read a setting.
    settings: std::sync::RwLock<SettingsStore>,
}

/// A song's lyrics as Now Playing lists them.
#[derive(Debug)]
pub struct Lyrics {
    /// Without the instrumental markers. Empty: no provider had lyrics.
    pub lines: Vec<LyricLine>,
    /// Each line's romanization, when its script has one.
    pub romanized: Vec<Option<String>>,
    /// The provider that answered, for the credit under the lines.
    pub provider: Option<String>,
    /// Instrumental breaks, after which line they come.
    pub gaps: Vec<InstrumentalGap>,
}

impl Lyrics {
    /// Lines with start times scroll along with the song; others are a plain text.
    pub fn synced(&self) -> bool {
        self.lines.iter().any(|line| line.start_time.is_some())
    }
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
        let client = Arc::new(InnerTubeClient::new(
            SwiftyTubeConfig::music(),
            AuthState::guest(),
        ));
        let db = Arc::new(match DatabaseService::default_path() {
            Some(path) => DatabaseService::open(&path),
            None => DatabaseService::open_in_memory(),
        });
        let last_sync = IncrementalSyncService::default_defaults_path()
            .unwrap_or_else(|| std::env::temp_dir().join("woofmusic-sync.json"));
        let downloads_dir = DownloadManager::default_dir()
            .unwrap_or_else(|| std::env::temp_dir().join("WoofMusic").join("Downloads"));
        Arc::new(Self {
            downloads: DownloadManager::new(db.clone(), downloads_dir),
            sync: SyncBridge::new(client.clone(), db.clone(), last_sync),
            lyrics: Arc::new(LyricsService::new(
                LyricsManager::new(Session::shared()),
                LyricsService::default_defaults_path()
                    .unwrap_or_else(|| std::env::temp_dir().join("woofmusic-lyrics.json")),
            )),
            comments: Arc::new(CommentsService::new(client.clone())),
            client,
            player,
            db,
            providers: StreamResolveProviders::live(),
            http: reqwest::Client::new(),
            settings: Default::default(),
        })
    }

    /// Hands the settings to the region, lyrics providers and library sync;
    /// later streams, Home loads and plays read them too.
    pub fn apply_settings(&self, settings: &SettingsStore) {
        let (gl, hl) = settings.locale();
        self.client.update_locale(YouTubeLocale::new(gl, hl));
        *self.lyrics.manager.settings.lock().unwrap() = settings.lyrics_settings();
        self.sync.set_options(settings.sync_options());
        crate::theme::set_accent(&settings.accent_name);
        if let Some(player) = &self.player {
            player.set_normalization(settings.audio_normalization);
            player.set_persistent_play_pause(settings.persistent_play_pause);
            player.set_gapless(settings.gapless_playback);
        }
        *self.settings.write().unwrap() = settings.clone();
    }

    fn settings(&self) -> SettingsStore {
        self.settings.read().unwrap().clone()
    }

    /// Fetches `song`'s lyrics from the Trop providers and romanizes them.
    /// A provider that answered without lyrics is not an error: the lines are empty.
    pub async fn lyrics(self: Arc<Self>, song: SongItem) -> Result<Arc<Lyrics>, String> {
        let query = lyrics_song(&song);
        let (lines, gaps) = match self.lyrics.fetch_lyrics(&query).await {
            Ok(lines) => (
                lyrics_view::display_lines(&lines),
                lyrics_view::detect_gaps(&lines),
            ),
            Err(error) if error.is_answer() => (Vec::new(), Vec::new()),
            Err(error) => return Err(error.to_string()),
        };
        let provider = self
            .lyrics
            .state()
            .provider_names
            .get(&song.video_id)
            .cloned();
        blocking(move || {
            let romanized = lines
                .iter()
                .map(|line| LyricsRomanizer::romanize(&line.text))
                .collect();
            Arc::new(Lyrics {
                lines,
                romanized,
                provider,
                gaps,
            })
        })
        .await
        .ok_or_else(|| "Romanizing the lyrics failed".to_owned())
    }

    /// Asks the providers again, replacing an edited or picked copy.
    pub async fn refetch_lyrics(self: Arc<Self>, song: SongItem) {
        self.lyrics.refetch(&lyrics_song(&song)).await;
    }

    /// The active lyrics as text with `[mm:ss.xx]` stamps, for Edit Lyrics.
    pub fn editable_lyrics(&self, video_id: &str) -> String {
        self.lyrics.editable_text(video_id)
    }

    /// Keeps `text` as the song's lyrics: edited (`provider` `None`) or picked in a search.
    pub fn save_lyrics(&self, video_id: &str, text: &str, provider: Option<&str>) {
        self.lyrics.save_custom(video_id, text, provider);
    }

    /// Every enabled provider's lyrics for `title` by `artist` (Search Lyrics).
    pub async fn search_lyrics(
        self: Arc<Self>,
        title: String,
        artist: String,
        duration: f64,
    ) -> Vec<LyricSearchResult> {
        let query = LyricsQuery {
            title,
            artist,
            album: None,
            duration,
        };
        self.lyrics.manager.search_all(&query).await
    }

    /// The lyrics without timestamps, for the clipboard.
    pub fn lyrics_text(&self, video_id: &str) -> Option<String> {
        self.lyrics.plain_text_for_copy(video_id)
    }

    pub fn karaoke_enabled(&self) -> bool {
        self.lyrics.karaoke_enabled()
    }

    /// Saves the karaoke switch; turning it on looks for word-timed lyrics of `song`.
    pub fn set_karaoke(&self, on: bool, song: Option<&SongItem>) {
        self.lyrics.set_karaoke_enabled(on);
        if let Some(song) = song.filter(|_| on) {
            self.lyrics.request_word_timing(&lyrics_song(song));
        }
    }

    /// Bumped when a song's lyrics changed in the background (word timing found).
    pub fn lyrics_refresh_token(&self) -> u64 {
        self.lyrics.state().refresh_token
    }

    pub async fn load_home(self: Arc<Self>) -> Result<Vec<HomeSection>, String> {
        let settings = self.settings();
        let mut home = HomeViewModel::new(self.client.clone());
        home.hide_explicit = settings.hide_explicit;
        home.show_quick_picks = settings.show_quick_picks;
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

    /// YouTube Music's results with the artist hero lifted out, and the
    /// library's matches (empty when the database cannot be read).
    pub async fn search(self: Arc<Self>, query: String) -> Result<crate::search::Results, String> {
        let db = self.db.clone();
        let local_query = query.clone();
        // Starts at once, so the library is read while YouTube answers.
        let library =
            tokio::task::spawn_blocking(move || SearchService::local_search(&db, &local_query));
        let mut sections = self.web_search(&query).await?;
        let hero = search_view_model::extract_hero(&mut sections, &query);
        Ok(crate::search::Results {
            sections,
            hero,
            library: library.await.ok().and_then(Result::ok).unwrap_or_default(),
        })
    }

    /// YouTube Music's results for `query`, as the parser reads them.
    async fn web_search(&self, query: &str) -> Result<Vec<SearchSection>, String> {
        let json = SearchService::new(self.client.clone())
            .search(query, None, YouTubeClient::WEB_REMIX)
            .await
            .map_err(|error| error.to_string())?;
        Ok(SearchParser::parse_search_results(&json))
    }

    /// YouTube Music's completions for `query` in the content country.
    pub async fn suggestions(self: Arc<Self>, query: String) -> Vec<String> {
        let country = self.settings().content_country;
        SearchService::new(self.client.clone())
            .suggestions(&query, &country)
            .await
            .unwrap_or_default()
    }

    /// Latest play per song, for the Recently Played sort.
    pub async fn recency(self: Arc<Self>) -> HashMap<String, std::time::SystemTime> {
        let db = self.db.clone();
        blocking(move || db.fetch_history(Some(200)))
            .await
            .and_then(Result::ok)
            .map(|entries| search_view_model::recency_map(&entries))
            .unwrap_or_default()
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
            Route::LikedSongs | Route::TopSongs => {
                let db = self.db.clone();
                let liked = matches!(route, Route::LikedSongs);
                let songs = blocking(move || {
                    if liked {
                        db.fetch_all_liked_songs(None)
                    } else {
                        db.fetch_top_songs(100)
                    }
                })
                .await
                .ok_or("The library could not be read.")?
                .map_err(|error| error.to_string())?;
                Ok(Detail::Songs(Arc::new(
                    songs.iter().map(song_item).collect(),
                )))
            }
            Route::Similar(id) => {
                let options = RadioOptions::default();
                let (songs, _) = personalization_service::fetch_radio(client, &id, options).await?;
                Ok(Detail::Songs(Arc::new(
                    songs
                        .into_iter()
                        .filter(|song| song.video_id != id)
                        .collect(),
                )))
            }
            Route::Downloads => {
                let this = self.clone();
                let tracks = blocking(move || this.downloads.tracks())
                    .await
                    .ok_or("Downloads could not be read.")?
                    .map_err(|error| error.to_string())?;
                Ok(Detail::Songs(Arc::new(
                    tracks.iter().map(crate::downloads::song).collect(),
                )))
            }
            Route::Home
            | Route::Explore
            | Route::Library
            | Route::Search
            | Route::Settings
            | Route::Equalizer
            | Route::History
            | Route::Activity
            | Route::Profile => Err("Not a detail page.".to_owned()),
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

    /// Swift `LibraryView.loadContent`: what the library tab shows, from the database.
    pub async fn load_library(self: Arc<Self>) -> Result<Library, String> {
        let db = self.db.clone();
        blocking(move || Library::read(&db))
            .await
            .ok_or("The library could not be read.")?
            .map_err(|error| error.to_string())
    }

    /// Syncs the library unless it synced in the last 30 minutes. A sync
    /// already running is shared, not started twice.
    pub async fn sync_library(self: Arc<Self>) {
        self.sync.check_and_sync_if_stale().await;
    }

    pub async fn save_plays(self: Arc<Self>, song: Option<SongItem>, records: Vec<PlayRecord>) {
        // Swift "Track Play History" off, or incognito: playback is not recorded.
        let settings = self.settings();
        if !settings.track_play_history || settings.is_incognito {
            return;
        }
        let db = self.db.clone();
        if let Some(Err(error)) = blocking(move || save(&db, song.as_ref(), &records)).await {
            eprintln!("Could not save play time: {error}");
        }
    }

    /// Opens `report`'s play in YouTube Music's history and reports its first `seconds`.
    pub async fn start_report(self: Arc<Self>, report: Arc<YouTubeHistoryReport>, seconds: f64) {
        let timestamp = match &self.providers.signature_timestamp {
            Some(provider) => provider().await,
            None => None,
        };
        report.start(&self.client, timestamp, seconds).await;
    }

    pub async fn send_report(self: Arc<Self>, url: String) {
        YouTubeHistoryReport::send(&self.client, url).await;
    }

    // MARK: - Session

    /// Signs in with the session saved in the system keyring, if any.
    pub async fn restore(self: Arc<Self>) -> bool {
        match blocking(|| keyring().load()).await {
            Some(Ok(auth)) if auth.sapisid.is_some() => {
                self.client.update_auth(auth);
                true
            }
            _ => false,
        }
    }

    /// Google sign-in in the login window; false when it closed without a session.
    pub async fn sign_in(self: Arc<Self>) -> bool {
        match blocking(|| webview("--login", None)).await.flatten() {
            Some(auth) => {
                self.adopt(auth).await;
                true
            }
            None => false,
        }
    }

    /// The cookie-paste fallback: a `Cookie` header copied from a signed-in browser.
    pub async fn import_cookies(self: Arc<Self>, header: String) -> bool {
        match SessionImporter::import_session(header.trim(), None) {
            Ok(auth) if auth.sapisid.is_some() => {
                self.adopt(auth).await;
                true
            }
            _ => false,
        }
    }

    pub async fn sign_out(self: Arc<Self>) {
        self.client.update_auth(AuthState::guest());
        if let Some(Err(error)) = blocking(|| keyring().clear()).await {
            eprintln!("Could not remove the saved session: {error}");
        }
    }

    /// Uses `auth` now and saves it for the next launch.
    async fn adopt(&self, auth: AuthState) {
        self.client.update_auth(auth.clone());
        // ponytail: Windows Credential Manager caps a secret at 2560 bytes and
        // a cookie set can be longer; split the secret if saving fails there.
        if let Some(Err(error)) = blocking(move || keyring().save(&auth)).await {
            eprintln!("Could not save the session; sign in again next time: {error}");
        }
    }

    // MARK: - Profiles (Swift `AccountState`)

    /// Swift `rememberCurrentProfile`: saves the signed-in session under its
    /// account, so it can be switched back to.
    pub async fn remember_profile(self: Arc<Self>, info: AccountInfo) {
        let auth = self.client.current_auth();
        if auth.sapisid.is_none() {
            return;
        }
        blocking(move || {
            let mut registry = Registry::load();
            let id = match registry.current.clone() {
                Some(id) => id,
                None => {
                    // A session signed in before profiles existed: it owns the app's current data.
                    let id = auth
                        .data_sync_id
                        .clone()
                        .or_else(|| info.email.clone())
                        .unwrap_or_else(|| info.name.clone());
                    registry.claim(&id);
                    registry.current = Some(id.clone());
                    id
                }
            };
            save_profile_session(&id, &auth);
            let profile = SavedProfile {
                id: id.clone(),
                name: info.name,
                thumbnail_url: info.thumbnail_url,
            };
            match registry.profiles.iter_mut().find(|p| p.id == id) {
                Some(saved) => *saved = profile,
                None => registry.profiles.push(profile),
            }
            if let Err(error) = registry.save() {
                log::error!("Could not save the profiles: {error}");
            }
        })
        .await;
    }

    /// Swift `beginAddingAccount` and `openProfileOfNewSession`: signs in to
    /// another account. `Ok(true)`: the account has its own data, so the app
    /// must restart on it.
    pub async fn add_account(self: Arc<Self>) -> Result<bool, String> {
        let current = self.client.current_auth();
        blocking(move || save_current_session(&current)).await;
        let auth = blocking(|| webview("--login", None))
            .await
            .flatten()
            .ok_or("Sign-in did not finish.")?;
        self.adopt(auth.clone()).await;
        let info = self.clone().account().await.ok();
        blocking(move || {
            let mut registry = Registry::load();
            let email = info.as_ref().and_then(|info| info.email.clone());
            let name = info
                .as_ref()
                .map(|info| info.name.clone())
                .filter(|name| name != "Guest");
            let seen = registry
                .claimed(auth.data_sync_id.iter().chain(&email).map(String::as_str))
                .map(str::to_owned);
            // ponytail: without a sync id or a menu this login gets a new
            // profile; it can't be matched to an old one.
            let id = seen
                .or_else(|| auth.data_sync_id.clone())
                .or(email)
                .or_else(|| name.clone())
                .unwrap_or_else(|| format!("{:x}", std::process::id()));
            let storage = registry.claim(&id);
            registry.current = Some(id.clone());
            save_profile_session(&id, &auth);
            if !registry.profiles.iter().any(|p| p.id == id) {
                registry.profiles.push(SavedProfile {
                    id,
                    name: name.unwrap_or_default(),
                    thumbnail_url: info.and_then(|info| info.thumbnail_url),
                });
            }
            let moves = storage != profile_storage::active();
            registry.active = storage;
            registry.save().map_err(|error| error.to_string())?;
            Ok(moves)
        })
        .await
        .unwrap_or_else(|| Err("Could not save the account.".into()))
    }

    /// Swift `switchTo`: the next launch opens `id` with its session and data.
    pub async fn switch_profile(self: Arc<Self>, id: String) -> Result<(), String> {
        let current = self.client.current_auth();
        blocking(move || {
            save_current_session(&current);
            let auth = load_profile_session(&id)
                .ok_or("This account's session is missing. Sign in again.")?;
            keyring().save(&auth).map_err(|error| error.to_string())?;
            let mut registry = Registry::load();
            registry.active = registry.claim(&id);
            registry.current = Some(id);
            registry.save().map_err(|error| error.to_string())
        })
        .await
        .unwrap_or_else(|| Err("Could not switch accounts.".into()))
    }

    /// Swift `signOut`: removes this account from the list. `Some(id)`:
    /// another saved account to switch to; `None`: now a guest.
    pub async fn sign_out_profile(self: Arc<Self>) -> Option<String> {
        let next = blocking(|| {
            let mut registry = Registry::load();
            let leaving = registry.current.take();
            if let Some(leaving) = &leaving {
                registry.profiles.retain(|p| &p.id != leaving);
                let _ = SecureStore::new(crate::APP_ID).delete(&profile_key(leaving));
            }
            let next = registry.profiles.first().map(|p| p.id.clone());
            if let Err(error) = registry.save() {
                log::error!("Could not save the profiles: {error}");
            }
            next
        })
        .await
        .flatten();
        if next.is_none() {
            self.sign_out().await;
        }
        next
    }

    /// The signed-in account. A guest-shaped answer means stale rotating
    /// tokens (Swift `AccountState.fetchAccountInfo`): ask three times, then
    /// renew the session once through Google and ask again.
    pub async fn account(self: Arc<Self>) -> Result<AccountInfo, String> {
        let signed_in = |info: &AccountInfo| !info.name.is_empty() && info.name != "Guest";
        for attempt in 1..=3 {
            let info = self
                .client
                .account_info(None, None)
                .await
                .map_err(|error| error.to_string())?;
            if signed_in(&info) {
                return Ok(info);
            }
            if attempt < 3 {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
        let current = self.client.current_auth();
        let renewed = blocking(move || webview("--refresh", Some(&current)))
            .await
            .flatten()
            .ok_or("Google no longer accepts this session. Sign in again.")?;
        self.adopt(renewed).await;
        let info = self
            .client
            .account_info(None, None)
            .await
            .map_err(|error| error.to_string())?;
        if signed_in(&info) {
            Ok(info)
        } else {
            Err("Google no longer accepts this session. Sign in again.".to_owned())
        }
    }

    /// A playable stream URL: VISIONOS first, cipher clients after.
    /// A downloaded song plays its saved file instead.
    /// The stream URL and its loudness (dB) for Audio Normalization.
    pub async fn resolve(
        self: Arc<Self>,
        video_id: String,
    ) -> Result<(String, Option<f64>), String> {
        let this = self.clone();
        let id = video_id.clone();
        if let Some(Some(path)) = blocking(move || this.downloads.local_path(&id)).await {
            return Ok((path.to_string_lossy().into_owned(), None));
        }
        let options = StreamResolveOptions {
            audio_quality: self.settings().audio_quality,
            ..StreamResolveOptions::default()
        };
        let request = StreamResolveRequest::playback(&video_id, options);
        StreamFallback::resolve_first_valid(
            &request,
            &self.client,
            StreamCache::shared(),
            &self.providers,
        )
        .await
        .map(|result| (result.stream_url, result.loudness_db))
        .map_err(|error| error.to_string())
    }

    /// Saves `song` for offline play: `Ok(true)` once saved, `Ok(false)` if cancelled.
    pub async fn download(self: Arc<Self>, song: SongItem) -> Result<bool, String> {
        let options = StreamResolveOptions {
            download_quality: self.settings().download_quality,
            ..StreamResolveOptions::default()
        };
        let request = StreamResolveRequest::download(&song.video_id, options);
        let resolve = async {
            StreamFallback::resolve_first_valid(
                &request,
                &self.client,
                StreamCache::shared(),
                &self.providers,
            )
            .await
            .map(|result| Resolved {
                url: result.stream_url,
                mime_type: result.mime_type,
                duration: result.duration,
            })
            .map_err(|error| error.to_string())
        };
        self.downloads.download(song, resolve).await
    }

    /// The ids of the liked songs (Swift `LikeStore.refresh`).
    pub async fn liked_ids(self: Arc<Self>) -> HashSet<String> {
        blocking(move || self.db.fetch_liked_song_ids())
            .await
            .and_then(Result::ok)
            .map(|ids| ids.into_iter().collect())
            .unwrap_or_default()
    }

    /// Likes or un-likes `song` on the account (Swift `LikeStore.toggle`).
    pub async fn set_liked(self: Arc<Self>, song: SongItem, liked: bool) -> Result<(), String> {
        let mutations = &self.sync.mutations;
        let result = if liked {
            mutations.like_song(&song.video_id).await
        } else {
            mutations.unlike_song(&song.video_id).await
        };
        result.map_err(|error| error.to_string())?;
        crate::integrations::love(&song, liked);
        Ok(())
    }

    /// Song Details' extras: the duration when the row lacks it, the stream
    /// in use, the saved format and the view count.
    pub async fn song_details(self: Arc<Self>, video_id: String, duration: i64) -> SongDetails {
        let client = self.client.clone();
        let resolved = if duration > 0 {
            None
        } else {
            client.resolve_duration(&video_id).await
        };
        let id = video_id.clone();
        let this = self.clone();
        let format = blocking(move || this.db.fetch_one::<FormatEntity>(id))
            .await
            .and_then(Result::ok)
            .flatten();
        let views = client
            .player_response::<swiftytube::streaming::player_response::PlayerResponse>(
                swiftytube::networking::inner_tube_client::PlayerArgs {
                    video_id: video_id.clone(),
                    ..Default::default()
                },
            )
            .await
            .ok()
            .and_then(|response| response.video_details?.view_count?.parse().ok());
        SongDetails {
            duration: resolved,
            stream: StreamCache::shared().get(&video_id),
            format,
            views,
        }
    }

    /// The artist page of the first search hit named `name`, the exact name first.
    pub async fn artist_id_by_name(self: Arc<Self>, name: String) -> Option<String> {
        let wanted = name.trim().to_lowercase();
        let sections = self.clone().search(name).await.ok()?;
        let artists: Vec<_> = sections
            .iter()
            .flat_map(|section| &section.items)
            .filter_map(|item| match item {
                YTItem::Artist(artist) => Some(artist),
                _ => None,
            })
            .collect();
        artists
            .iter()
            .find(|artist| artist.name.trim().to_lowercase() == wanted)
            .or(artists.first())
            .map(|artist| artist.browse_id.clone())
    }

    /// MusicBrainz's word for the act ("Band", "Rapper"…), cached.
    pub async fn artist_kind(self: Arc<Self>, name: String) -> Option<String> {
        artist_kind_service::kind(&self.http, &name).await
    }

    /// A podcast page was read: remembers its episodes and, with Auto-Download
    /// New Episodes on, returns the newest ten never seen before.
    pub async fn new_episodes(
        self: Arc<Self>,
        info: trop::detail::detail_models::PodcastDetailInfo,
        browse_id: String,
    ) -> Vec<SongItem> {
        blocking(move || {
            trop::playback::episode_playback_store::sync_known_episodes(
                &self.db,
                &info.episodes,
                &browse_id,
                &info.title,
            )
        })
        .await
        .and_then(Result::ok)
        .unwrap_or_default()
        .iter()
        .take(10)
        .map(crate::ui::episode_song)
        .collect()
    }

    /// A queue of songs like `video_id`, and its index in it.
    pub async fn radio(
        self: Arc<Self>,
        video_id: String,
        options: RadioOptions,
    ) -> Result<(Vec<SongItem>, usize), String> {
        personalization_service::fetch_radio(&self.client, &video_id, options).await
    }

    /// Swift `MusicLinkOpener.open(url:)`: what a pasted or launched link
    /// opens. Spotify and Apple Music links go through Odesli first; the
    /// error is a sentence for the user.
    pub async fn open_link(self: Arc<Self>, text: String) -> Result<LinkTarget, String> {
        let text = text.trim().to_owned();
        let link = match MusicLink::parse(&text) {
            Some(link) => link,
            None if Odesli::is_convertible(&text) => Odesli::new(Session::shared())
                .convert(&text)
                .await
                .map_err(|error| error.to_string())?,
            None => return Err(LINK_HELP.to_owned()),
        };
        let video_id = match link {
            MusicLink::Song(id) => id,
            MusicLink::Playlist(id) => return Ok(LinkTarget::Route(Route::Playlist(id))),
            MusicLink::Album(id) => return Ok(LinkTarget::Route(Route::Album(id))),
            MusicLink::Artist(id) => return Ok(LinkTarget::Route(Route::Artist(id))),
            MusicLink::Podcast(id) => return Ok(LinkTarget::Route(Route::Podcast(id))),
            MusicLink::Shazam(url) => self.clone().shazam_song(&url).await?,
        };
        // The radio carries the song's title and artwork, which a bare ID doesn't have.
        let unavailable =
            || "This song couldn't be loaded. It may be unavailable, or you may be offline.";
        let (songs, index) = self
            .radio(video_id.clone(), RadioOptions::default())
            .await
            .map_err(|_| unavailable())?;
        match songs.get(index) {
            Some(song) if song.video_id == video_id && song.is_available() => {
                Ok(LinkTarget::Play(Arc::new(songs), index))
            }
            _ => Err(unavailable().to_owned()),
        }
    }

    /// Swift `MusicLinkOpener.playShazam`: the first song result with the
    /// Shazam page's title, else the first song.
    async fn shazam_song(self: Arc<Self>, url: &str) -> Result<String, String> {
        let unreadable = "This Shazam link couldn't be read. Check your connection, or search for the song by name.";
        let page = async {
            self.http
                .get(url)
                .header(reqwest::header::USER_AGENT, music_link::SHAZAM_USER_AGENT)
                .timeout(std::time::Duration::from_secs(15))
                .send()
                .await?
                .text()
                .await
        };
        let track = page
            .await
            .ok()
            .and_then(|html| music_link::shazam_track(&html))
            .ok_or(unreadable)?;
        let sections = self
            .web_search(&format!("{} {}", track.title, track.artist))
            .await
            .map_err(|_| {
                format!(
                    "Couldn't search for \"{}\". Check your connection.",
                    track.title
                )
            })?;
        let songs: Vec<SongItem> = sections
            .iter()
            .flat_map(|section| crate::ui::songs_of(&section.items).to_vec())
            .filter(|song| song.is_available())
            .collect();
        let wanted = track.title.to_lowercase();
        songs
            .iter()
            .find(|song| song.title.to_lowercase().contains(&wanted))
            .or(songs.first())
            .map(|song| song.video_id.clone())
            .ok_or_else(|| {
                format!(
                    "\"{}\" by {} wasn't found on YouTube Music.",
                    track.title, track.artist
                )
            })
    }

    /// The saved playlists "Add to Playlist" offers (Swift `fetchPlaylists(limit: 100)`).
    /// The song's non-music parts; empty when SponsorBlock has none.
    pub async fn sponsor_segments(self: Arc<Self>, video_id: String) -> Vec<Segment> {
        sponsor_block::fetch_segments(&self.http, &video_id).await
    }

    pub async fn playlists(self: Arc<Self>) -> Vec<PlaylistEntity> {
        blocking(move || self.db.fetch_playlists(Some(100)))
            .await
            .and_then(Result::ok)
            .unwrap_or_default()
    }

    pub async fn add_to_playlist(
        self: Arc<Self>,
        playlist_id: String,
        video_id: String,
    ) -> Result<(), String> {
        self.sync
            .mutations
            .add_to_playlist(&playlist_id, &video_id, None)
            .await
            .map_err(|error| error.to_string())
    }

    /// Swift `AddSongToPlaylistSheet.createAndAdd`.
    pub async fn create_playlist_with(
        self: Arc<Self>,
        title: String,
        video_id: String,
    ) -> Result<(), String> {
        let id = self
            .sync
            .mutations
            .create_playlist(&title, None)
            .await
            .map_err(|error| error.to_string())?;
        self.add_to_playlist(id, video_id).await
    }

    /// The ids of the saved songs.
    pub async fn downloaded_ids(self: Arc<Self>) -> Vec<String> {
        blocking(move || self.downloads.tracks())
            .await
            .and_then(Result::ok)
            .map(|tracks| tracks.into_iter().map(|track| track.id).collect())
            .unwrap_or_default()
    }

    /// Which albums and playlists have their songs saved.
    pub async fn downloaded_collections(self: Arc<Self>) -> DownloadedCollections {
        blocking(move || DownloadedCollections::read(&self.db))
            .await
            .and_then(Result::ok)
            .unwrap_or_default()
    }

    /// Remembers an album's songs for `downloaded_collections` (Swift `saveSongs(_:ofAlbum:)`).
    pub async fn save_album_songs(self: Arc<Self>, browse_id: String, song_ids: Vec<String>) {
        blocking(move || DownloadedCollections::save_album_songs(&self.db, &browse_id, &song_ids))
            .await;
    }

    /// The ids of a playlist's songs, under its bare and VL ids (Swift `removeDownloads(of:)`).
    pub async fn playlist_song_ids(self: Arc<Self>, playlist_id: String) -> Vec<String> {
        blocking(move || {
            let bare = PlaylistSongEdits::bare_id(&playlist_id).to_owned();
            self.db.read(|conn| {
                conn.prepare("SELECT song_id FROM playlist_song_map WHERE playlist_id IN (?, ?)")?
                    .query_map([&playlist_id, &bare], |row| row.get(0))?
                    .collect()
            })
        })
        .await
        .and_then(Result::ok)
        .unwrap_or_default()
    }

    /// Removes the saved songs `ids`, or every one when `ids` is `None` (Swift `deleteAll`).
    pub async fn delete_downloads(self: Arc<Self>, ids: Option<Vec<String>>) -> Result<(), String> {
        blocking(move || match ids {
            Some(ids) => ids.iter().try_for_each(|id| self.downloads.delete(id)),
            None => self.downloads.delete_all(),
        })
        .await
        .ok_or("Deleting failed.")?
        .map_err(|error| error.to_string())
    }

    /// Settings' storage card: bytes used and how many songs are saved.
    pub async fn download_stats(self: Arc<Self>) -> (u64, usize) {
        blocking(move || {
            let count = self.downloads.tracks().map_or(0, |tracks| tracks.len());
            (self.downloads.total_storage_bytes(), count)
        })
        .await
        .unwrap_or_default()
    }

    pub async fn delete_download(self: Arc<Self>, video_id: String) -> Result<(), String> {
        blocking(move || self.downloads.delete(&video_id))
            .await
            .ok_or("Deleting failed.")?
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

fn keyring() -> SecureAuthStore {
    SecureAuthStore::new(SecureStore::new(crate::APP_ID))
}

/// The keyring entry of a saved account's session.
fn profile_key(id: &str) -> String {
    format!("profile.{id}")
}

fn save_profile_session(id: &str, auth: &AuthState) {
    let json = serde_json::to_vec(auth).expect("AuthState always serializes");
    // ponytail: Windows Credential Manager caps a secret at 2560 bytes, like
    // the main session; split it if saving fails there.
    if let Err(error) = SecureStore::new(crate::APP_ID).save(&json, &profile_key(id)) {
        log::error!("Could not save the account's session: {error}");
    }
}

fn load_profile_session(id: &str) -> Option<AuthState> {
    let json = SecureStore::new(crate::APP_ID)
        .load(&profile_key(id))
        .ok()?;
    serde_json::from_slice(&json).ok()
}

/// Swift `saveCurrentSession`: Google rotates its tokens, so the saved copy goes out of date.
fn save_current_session(auth: &AuthState) {
    if auth.sapisid.is_none() {
        return;
    }
    if let Some(id) = Registry::load().current {
        save_profile_session(&id, auth);
    }
}

/// Keyring and child-process calls block: off the async workers. `None` if the task panicked.
fn lyrics_song(song: &SongItem) -> LyricsSong {
    LyricsSong {
        video_id: song.video_id.clone(),
        title: song.title.clone(),
        artist: crate::ui::artists(&song.artists),
        album: song.album.clone().filter(|album| !album.is_empty()),
        duration: song.duration as f64,
    }
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(work).await.ok()
}

/// Runs this binary in its webview mode (`login.rs`) and reads the session it prints.
fn webview(mode: &str, input: Option<&AuthState>) -> Option<AuthState> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(std::env::current_exe().ok()?)
        .arg(mode)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    if let Some(auth) = input {
        // Dropping stdin after the write closes it: the child reads to the end.
        serde_json::to_writer(child.stdin.take()?, auth).ok()?;
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

/// The library tab's content, as page items.
#[derive(Debug, Default, Clone)]
pub struct Library {
    pub liked_count: i64,
    /// First covers of Liked Songs and My Top 100, in the order their pages open with.
    pub liked_covers: Vec<String>,
    pub top_covers: Vec<String>,
    pub downloaded_count: i64,
    pub downloaded_covers: Vec<String>,
    /// Each item with the subtitle its tile shows.
    pub playlists: Vec<(YTItem, String)>,
    pub albums: Vec<(YTItem, String)>,
    pub artists: Vec<(YTItem, String)>,
    pub podcasts: Vec<(YTItem, String)>,
}

impl Library {
    pub fn cover_urls(&self) -> Vec<String> {
        let tiles = [&self.playlists, &self.albums, &self.artists, &self.podcasts]
            .into_iter()
            .flatten()
            .filter_map(|(item, _)| item.thumbnail_url().map(str::to_owned));
        self.liked_covers
            .iter()
            .chain(&self.top_covers)
            .chain(&self.downloaded_covers)
            .cloned()
            .chain(tiles)
            .collect()
    }

    fn read(db: &DatabaseService) -> rusqlite::Result<Self> {
        let hidden = |id: &str| PlaylistEntity::HIDDEN_IDS.contains(&id);
        let strings = |sql: &str| {
            db.read(|conn| {
                conn.prepare(sql)?
                    .query_map([], |row| row.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
        };
        let counts: std::collections::HashMap<String, i64> = db.read(|conn| {
            conn.prepare(
                "SELECT playlist_id, COUNT(*) FROM playlist_song_map \
                 WHERE EXISTS (SELECT 1 FROM song WHERE id = song_id) GROUP BY playlist_id",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect()
        })?;
        let playlists = db
            .fetch_all_sql::<PlaylistEntity>(
                "SELECT * FROM playlist WHERE bookmarked_at IS NOT NULL ORDER BY name",
                [],
            )?
            .into_iter()
            .filter(|playlist| !hidden(&playlist.id))
            .map(|playlist| {
                let subtitle = counts
                    .get(&playlist.id)
                    .map_or_else(String::new, |n| songs_label(*n));
                let item = YTItem::Playlist(PlaylistItem {
                    id: playlist.id,
                    title: playlist.name,
                    author: None,
                    thumbnail_url: playlist.thumbnail_url,
                    song_count: None,
                });
                (item, subtitle)
            })
            .collect();
        let albums = db
            .fetch_all_albums()?
            .into_iter()
            .map(|album| {
                let subtitle = songs_label(album.song_count);
                let item = YTItem::Album(AlbumItem {
                    browse_id: album.id,
                    title: album.title,
                    artists: Vec::new(),
                    year: None,
                    thumbnail_url: album.thumbnail_url,
                    playlist_id: album.playlist_id,
                    is_explicit: false,
                });
                (item, subtitle)
            })
            .collect();
        let artists = db
            .fetch_all_sql::<trop::database::entities::artist_entity::ArtistEntity>(
                "SELECT * FROM artist WHERE bookmarked_at IS NOT NULL ORDER BY name",
                [],
            )?
            .into_iter()
            .map(|artist| {
                let subtitle = if artist.is_podcast_channel {
                    "Podcast Channel"
                } else {
                    "Artist"
                };
                let item = YTItem::Artist(ArtistItem {
                    browse_id: artist.id,
                    name: artist.name,
                    thumbnail_url: artist.thumbnail_url,
                    is_subscribed: true,
                });
                (item, subtitle.to_owned())
            })
            .collect();
        let podcasts = db
            .fetch_all_podcasts()?
            .into_iter()
            .filter(|podcast| !hidden(&podcast.id))
            .map(|podcast| {
                let item = YTItem::Podcast(PodcastItem {
                    browse_id: podcast.id,
                    title: podcast.name,
                    author: None,
                    thumbnail_url: podcast.thumbnail_url,
                });
                // The library stores no author.
                (item, "Podcast".to_owned())
            })
            .collect();
        Ok(Self {
            liked_count: db.fetch_all_liked_song_count()?,
            liked_covers: strings(
                "SELECT thumbnail_url FROM song WHERE liked = 1 AND thumbnail_url <> '' \
                 ORDER BY create_date DESC LIMIT 2",
            )?,
            top_covers: strings(
                "SELECT thumbnail_url FROM song WHERE total_play_time > 0 AND thumbnail_url <> '' \
                 ORDER BY total_play_time DESC LIMIT 2",
            )?,
            downloaded_count: db.read(|conn| {
                conn.query_row("SELECT COUNT(*) FROM downloaded_track", [], |row| {
                    row.get(0)
                })
            })?,
            downloaded_covers: strings(
                "SELECT thumbnail_url FROM downloaded_track WHERE thumbnail_url <> '' \
                 ORDER BY downloaded_at DESC LIMIT 2",
            )?,
            playlists,
            albums,
            artists,
            podcasts,
        })
    }
}

fn songs_label(count: i64) -> String {
    match count {
        0 => String::new(),
        1 => "1 song".to_owned(),
        n => format!("{n} songs"),
    }
}

/// A library song as a queue entry. The database keeps names, not ids.
pub(crate) fn song_item(song: &SongEntity) -> SongItem {
    SongItem {
        video_id: song.id.clone(),
        title: song.title.clone(),
        artists: song
            .artist_name
            .iter()
            .map(|name| YTArtist {
                name: name.clone(),
                id: None,
            })
            .collect(),
        album: song.album_name.clone(),
        duration: song.duration,
        thumbnail_url: song.thumbnail_url.clone(),
        ..SongItem::default()
    }
}

#[cfg(test)]
impl Services {
    /// No libmpv and an in-memory database; nothing reaches the network until a task runs.
    pub fn for_test() -> Arc<Self> {
        let dir = std::env::temp_dir().join("woofmusic-test");
        let client = Arc::new(InnerTubeClient::new(
            SwiftyTubeConfig::music(),
            AuthState::guest(),
        ));
        let db = Arc::new(DatabaseService::open_in_memory());
        Arc::new(Self {
            downloads: DownloadManager::new(db.clone(), dir.join("downloads")),
            sync: SyncBridge::new(client.clone(), db.clone(), dir.join("sync.json")),
            lyrics: Arc::new(LyricsService::new(
                LyricsManager::new(Session::shared()),
                dir.join("lyrics.json"),
            )),
            comments: Arc::new(CommentsService::new(client.clone())),
            client,
            player: None,
            db,
            providers: StreamResolveProviders::none(),
            http: reqwest::Client::new(),
            settings: Default::default(),
        })
    }
}
