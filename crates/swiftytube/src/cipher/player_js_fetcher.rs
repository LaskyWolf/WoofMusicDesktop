use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

use super::cipher_error::CipherError;
use super::player_config_store::PlayerConfigStore;
use crate::core::swiftytube_log::SwiftyTubeLog;
use crate::networking::http_client::{HttpClient, Session};

const CACHE_LIFETIME: f64 = 6.0 * 60.0 * 60.0;

static HASH_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\?/s\\?/player\\?/([\w-]+)\\?/").unwrap());
static STS_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"signatureTimestamp["':\s]+(\d+)"#).unwrap());

// player.js download + 6h disk cache.
pub struct PlayerJsFetcher {
    cache_directory: Option<PathBuf>,
    session: Session,
    config_store: Arc<PlayerConfigStore>,
}

impl PlayerJsFetcher {
    pub fn shared() -> Arc<PlayerJsFetcher> {
        static SHARED: LazyLock<Arc<PlayerJsFetcher>> = LazyLock::new(|| {
            Arc::new(PlayerJsFetcher::new(
                PlayerJsFetcher::default_cache_directory(),
                Session::shared(),
                PlayerConfigStore::shared(),
            ))
        });
        SHARED.clone()
    }

    pub fn new(
        cache_directory: Option<PathBuf>,
        session: Session,
        config_store: Arc<PlayerConfigStore>,
    ) -> Self {
        Self {
            cache_directory,
            session,
            config_store,
        }
    }

    /// Swift: `Documents/Player`. Desktop: the user cache directory
    /// (`$XDG_CACHE_HOME` or `~/.cache` on Linux, `%LOCALAPPDATA%` on Windows) + `WoofMusic/Player`.
    pub fn default_cache_directory() -> Option<PathBuf> {
        let env = |name| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let base = env("XDG_CACHE_HOME")
            .or_else(|| env("LOCALAPPDATA"))
            .or_else(|| env("HOME").map(|home| home.join(".cache")))?;
        Some(base.join("WoofMusic").join("Player"))
    }

    // Returns the current player hash.
    pub async fn get_player_hash(&self) -> Result<String, CipherError> {
        self.ensure_cache_dir()?;
        if let Some((hash, _)) = self.load_hash_info() {
            return Ok(hash);
        }
        let (hash, _) = self.fetch_player_js().await?;
        Ok(hash)
    }

    // Returns the current player.js content, fetching if needed.
    pub async fn get_player_js(&self) -> Result<String, CipherError> {
        self.ensure_cache_dir()?;
        if let Some(cached) = self.load_cached_player_js() {
            return Ok(cached);
        }
        let (hash, js) = self.fetch_player_js().await?;
        self.save_cache(&hash, &js);
        Ok(js)
    }

    // Returns the signature timestamp from the current player.js,
    // falling back to the player config's sts when extraction fails.
    pub async fn get_signature_timestamp(&self) -> Result<i64, CipherError> {
        let js = self.get_player_js().await?;
        if let Ok(timestamp) = Self::extract_signature_timestamp(&js) {
            return Ok(timestamp);
        }
        let hash = self.get_player_hash().await?;
        if let Some(sts) = self.config_store.config(&hash).await.and_then(|c| c.sts) {
            SwiftyTubeLog::debug(&format!("signatureTimestamp from config: {sts}"));
            return Ok(sts);
        }
        Err(CipherError::SignatureTimestampNotFound)
    }

    // MARK: - Fetch Pipeline

    async fn fetch_player_js(&self) -> Result<(String, String), CipherError> {
        let hash = self.fetch_player_hash().await?;
        let js = self.download_player_js(&hash).await?;
        Ok((hash, js))
    }

    async fn get_text(&self, url: &str, what: &str) -> Result<String, CipherError> {
        let url = url.parse().expect("valid URL");
        let response = HttpClient::data_from(url, &self.session)
            .await
            .map_err(|error| CipherError::InvalidResponse(format!("{what}: {error}")))?;
        String::from_utf8(response.data)
            .map_err(|_| CipherError::InvalidResponse(format!("{what} not UTF-8")))
    }

    async fn fetch_player_hash(&self) -> Result<String, CipherError> {
        let text = self
            .get_text("https://www.youtube.com/iframe_api", "iframe_api")
            .await?;
        HASH_PATTERN
            .captures(&text)
            .map(|c| c[1].to_string())
            .ok_or(CipherError::HashNotFound)
    }

    // Downloads player.js for a given hash.
    async fn download_player_js(&self, hash: &str) -> Result<String, CipherError> {
        let url =
            format!("https://www.youtube.com/s/player/{hash}/player_ias.vflset/en_GB/base.js");
        self.get_text(&url, "player.js").await
    }

    // MARK: - Cache

    fn ensure_cache_dir(&self) -> Result<(), CipherError> {
        let dir = self
            .cache_directory
            .as_ref()
            .ok_or(CipherError::CacheUnavailable)?;
        std::fs::create_dir_all(dir).map_err(|_| CipherError::CacheUnavailable)
    }

    fn cache_path(&self, hash: &str) -> Option<PathBuf> {
        Some(
            self.cache_directory
                .as_ref()?
                .join(format!("player_{hash}.js")),
        )
    }

    fn hash_info_path(&self) -> Option<PathBuf> {
        Some(self.cache_directory.as_ref()?.join("current_hash.txt"))
    }

    /// (hash, seconds since epoch).
    fn load_hash_info(&self) -> Option<(String, f64)> {
        let info = std::fs::read_to_string(self.hash_info_path()?).ok()?;
        let (hash, timestamp) = info.split_once('\n')?;
        Some((hash.to_string(), timestamp.parse().ok()?))
    }

    fn load_cached_player_js(&self) -> Option<String> {
        let (hash, timestamp) = self.load_hash_info()?;
        // Check TTL.
        if now() - timestamp >= CACHE_LIFETIME {
            return None;
        }
        let js = std::fs::read_to_string(self.cache_path(&hash)?).ok()?;
        SwiftyTubeLog::debug(&format!("Using cached player.js (hash={hash})"));
        Some(js)
    }

    fn save_cache(&self, hash: &str, js: &str) {
        let (Some(path), Some(dir), Some(info)) = (
            self.cache_path(hash),
            self.cache_directory.as_ref(),
            self.hash_info_path(),
        ) else {
            return;
        };
        // Save JS file.
        let _ = std::fs::write(path, js);
        // Remove stale player JS files so the cache dir only holds the current player.
        let current_name = format!("player_{hash}.js");
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("player_") && name != current_name {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        // Save hash info.
        let _ = std::fs::write(info, format!("{hash}\n{}", now()));
        SwiftyTubeLog::debug(&format!("Cached player.js (hash={hash})"));
    }

    // MARK: - Signature Timestamp Extraction

    pub fn extract_signature_timestamp(js: &str) -> Result<i64, CipherError> {
        let captures = STS_PATTERN
            .captures(js)
            .ok_or(CipherError::SignatureTimestampNotFound)?;
        Ok(captures[1].parse().unwrap_or(0))
    }
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}
