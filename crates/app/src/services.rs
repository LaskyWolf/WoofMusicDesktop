//! The app's link to YouTube Music and libmpv: loads pages, covers and streams.

use std::sync::Arc;

use iced::widget::image;
use swiftytube::auth::account_info::AccountInfo;
use swiftytube::auth::auth_state::AuthState;
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
use trop::audio::lyrics::lyrics_manager::LyricsManager;
use trop::audio::lyrics::lyrics_service::{LyricLine, LyricsService, LyricsSong};
use trop::audio::lyrics::lyrics_view::{self, InstrumentalGap};
use trop::audio::lyrics::romanizer::lyrics_romanizer::LyricsRomanizer;
use trop::audio::player_controller::player_controller::PlayerController;
use trop::database::database_service::DatabaseService;
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
use trop::search::search_parser::{SearchParser, SearchSection};
use trop::search::search_service::SearchService;
use trop::services::download_manager::{DownloadManager, Resolved};
use trop::services::sync_bridge::SyncBridge;
use trop::settings::settings_store::SettingsStore;
use trop::utilities::ui_image_color_extraction::{self as colors, Rgb};

use crate::{Detail, Route};

pub struct Services {
    pub client: Arc<InnerTubeClient>,
    /// `None` when libmpv could not start; the UI says so instead of playing.
    pub player: Option<Arc<PlayerController>>,
    /// The synced library; in memory when the file cannot open.
    pub db: Arc<DatabaseService>,
    pub downloads: DownloadManager,
    sync: SyncBridge,
    lyrics: Arc<LyricsService>,
    providers: StreamResolveProviders,
    http: reqwest::Client,
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
            Route::Home | Route::Explore | Route::Library | Route::Search | Route::Settings => {
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
        // Swift "Track Play History" off: playback is not recorded.
        if !self.settings().track_play_history {
            return;
        }
        let db = self.db.clone();
        if let Some(Err(error)) = blocking(move || save(&db, song.as_ref(), &records)).await {
            eprintln!("Could not save play time: {error}");
        }
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
    pub async fn resolve(self: Arc<Self>, video_id: String) -> Result<String, String> {
        let this = self.clone();
        let id = video_id.clone();
        if let Some(Some(path)) = blocking(move || this.downloads.local_path(&id)).await {
            return Ok(path.to_string_lossy().into_owned());
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
        .map(|result| result.stream_url)
        .map_err(|error| error.to_string())
    }

    /// Saves `song` for offline play: `Ok(true)` once saved, `Ok(false)` if cancelled.
    pub async fn download(self: Arc<Self>, song: SongItem) -> Result<bool, String> {
        let request =
            StreamResolveRequest::download(&song.video_id, StreamResolveOptions::default());
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

    /// The ids of the saved songs.
    pub async fn downloaded_ids(self: Arc<Self>) -> Vec<String> {
        blocking(move || self.downloads.tracks())
            .await
            .and_then(Result::ok)
            .map(|tracks| tracks.into_iter().map(|track| track.id).collect())
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
fn song_item(song: &SongEntity) -> SongItem {
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
