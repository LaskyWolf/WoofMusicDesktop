use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde_json::Value;

use super::player_config::PlayerConfig;
use super::player_js_fetcher::PlayerJsFetcher;
use crate::core::swiftytube_log::SwiftyTubeLog;
use crate::networking::http_client::{HttpClient, Session};

/// Swift `Bundle.module` resource `player_configs.json`.
pub const BUNDLED_PLAYER_CONFIGS: &str = include_str!("../player_configs.json");

const REMOTE_URL: &str = "https://raw.githubusercontent.com/ZemerTeam/zemer-cipher/master/library/src/main/assets/player_configs.json";
const REFRESH_TTL: f64 = 6.0 * 60.0 * 60.0;
const FORCED_REFRESH_COOLDOWN: f64 = 5.0 * 60.0;

static SIG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9$_]{1,8}\([0-9]+,[0-9]+,INPUT\)$").unwrap());
static N_CLASS_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9$_]{1,8}$").unwrap());
static HASH_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-f0-9]{8}$").unwrap());

type Configs = HashMap<String, PlayerConfig>;

/// Swift actor state. A tokio `Mutex` held for a whole call stands in for the
/// actor, so a refresh and a lookup never interleave.
#[derive(Default)]
struct State {
    /// Bumps on table change; the JS engine rebuilds when it advances.
    config_epoch: u64,
    bundled_configs: Configs,
    alias_to_hash: HashMap<String, String>,
    merged_configs: Configs,
    loaded: bool,
    /// Seconds since epoch, like Swift `TimeInterval`.
    last_fetch_time: f64,
    etag: Option<String>,
    last_forced_attempt: f64,
    last_rejection_attempt: f64,
    /// Cooldowns arm only when the server was actually reached.
    last_attempt_reached_server: bool,
}

/// Known player hashes → cipher info. Bundled JSON by default, overlaid by
/// zemer-cipher's remote copy so rotations self-heal. Entries are regex-locked
/// (sig = name(i,i,INPUT), nClass = identifier) since they run as JS.
pub struct PlayerConfigStore {
    state: tokio::sync::Mutex<State>,
    /// Swift `Bundle`: the bundled JSON text, or None.
    bundled_text: Option<String>,
    session: Session,
    store_dir: Option<PathBuf>,
}

impl PlayerConfigStore {
    pub fn shared() -> Arc<PlayerConfigStore> {
        static SHARED: LazyLock<Arc<PlayerConfigStore>> = LazyLock::new(|| {
            Arc::new(PlayerConfigStore::new(
                Some(BUNDLED_PLAYER_CONFIGS.to_string()),
                Session::shared(),
                PlayerJsFetcher::default_cache_directory(),
            ))
        });
        SHARED.clone()
    }

    /// `store_dir`: where the remote copy is cached (Swift: the player.js cache directory).
    pub fn new(bundled_text: Option<String>, session: Session, store_dir: Option<PathBuf>) -> Self {
        Self {
            state: tokio::sync::Mutex::default(),
            bundled_text,
            session,
            store_dir,
        }
    }

    fn now() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64())
    }

    // MARK: - Lookup

    pub async fn config(&self, hash: &str) -> Option<PlayerConfig> {
        let mut state = self.state.lock().await;
        self.load_if_needed(&mut state).await;
        if let Some(direct) = state.merged_configs.get(hash) {
            return Some(direct.clone());
        }
        let aliased = state.alias_to_hash.get(hash)?;
        state.merged_configs.get(aliased).cloned()
    }

    pub async fn epoch(&self) -> u64 {
        let mut state = self.state.lock().await;
        self.load_if_needed(&mut state).await;
        state.config_epoch
    }

    fn known(state: &State, hash: &str) -> bool {
        state.merged_configs.contains_key(hash) || state.alias_to_hash.contains_key(hash)
    }

    /// Refresh on hash miss. True if the hash is now known.
    pub async fn force_refresh(&self, missing_hash: &str) -> bool {
        let mut state = self.state.lock().await;
        self.load_if_needed(&mut state).await;
        if Self::known(&state, missing_hash) {
            return true;
        }

        let now = Self::now();
        if Self::within_window(now, state.last_forced_attempt, FORCED_REFRESH_COOLDOWN) {
            SwiftyTubeLog::debug("forceRefresh skipped (cooldown)");
            return false;
        }
        state.last_forced_attempt = now;
        self.fetch_and_apply(&mut state).await;
        if !state.last_attempt_reached_server {
            state.last_forced_attempt = 0.0;
        }
        Self::known(&state, missing_hash)
    }

    /// Refresh after a 403 (stale config may sign wrong).
    pub async fn notify_stream_rejection(&self) {
        let mut state = self.state.lock().await;
        self.load_if_needed(&mut state).await;
        let now = Self::now();
        if Self::within_window(now, state.last_rejection_attempt, FORCED_REFRESH_COOLDOWN) {
            return;
        }
        state.last_rejection_attempt = now;
        self.fetch_and_apply(&mut state).await;
        if !state.last_attempt_reached_server {
            state.last_rejection_attempt = 0.0;
        }
    }

    /// In-range check (immune to backward clock jumps).
    fn within_window(now: f64, stamp: f64, window: f64) -> bool {
        let delta = now - stamp;
        (0.0..window).contains(&delta)
    }

    // MARK: - Loading

    async fn load_if_needed(&self, state: &mut State) {
        if state.loaded {
            return;
        }
        state.loaded = true;
        state.bundled_configs = self
            .bundled_text
            .as_deref()
            .and_then(Self::parse)
            .unwrap_or_default();
        Self::rebuild_alias_map(state, &state.bundled_configs.clone());
        // Swift starts merged empty and fills it on the first remote apply;
        // seed it with the bundled table so lookups work offline.
        state.merged_configs = state.bundled_configs.clone();
        SwiftyTubeLog::debug(&format!(
            "Loaded {} bundled configs",
            state.bundled_configs.len()
        ));

        let configs_cache = self.configs_cache_file();
        let cached_text = configs_cache.as_deref().and_then(read_file);
        if let Some(cached) = cached_text.as_deref().and_then(Self::parse) {
            self.apply_remote(state, cached, None);
            if let Some((etag, timestamp)) = self.read_meta() {
                state.etag = Some(etag);
                state.last_fetch_time = timestamp;
            }
        } else if let Some(configs_cache) = configs_cache.filter(|f| f.exists()) {
            let _ = std::fs::remove_file(configs_cache);
            if let Some(meta) = self.meta_cache_file() {
                let _ = std::fs::remove_file(meta);
            }
        }

        if !Self::within_window(Self::now(), state.last_fetch_time, REFRESH_TTL) {
            self.fetch_and_apply(state).await;
        }
    }

    // MARK: - Fetch & Apply

    async fn fetch_and_apply(&self, state: &mut State) {
        let mut request =
            reqwest::Request::new(reqwest::Method::GET, REMOTE_URL.parse().expect("valid URL"));
        request
            .headers_mut()
            .insert("User-Agent", "Mozilla/5.0".parse().expect("valid header"));
        if let Some(etag) = state.etag.as_deref().and_then(|e| e.parse().ok()) {
            request.headers_mut().insert("If-None-Match", etag);
        }

        let response = match tokio::time::timeout(
            Duration::from_secs(15),
            HttpClient::data(request, &self.session),
        )
        .await
        {
            Ok(Ok(response)) => response,
            _ => {
                state.last_attempt_reached_server = false;
                SwiftyTubeLog::notice(
                    "Remote config fetch failed (network) - keeping previous configs",
                );
                return;
            }
        };
        state.last_attempt_reached_server = true;
        let response_etag = response
            .headers
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        if response.status == 304 {
            state.etag = response_etag.or(state.etag.take());
            state.last_fetch_time = Self::now();
            self.write_meta(state);
            return;
        }
        let body = if response.status == 200 {
            String::from_utf8_lossy(&response.data).into_owned()
        } else {
            String::new()
        };
        if body.is_empty() {
            SwiftyTubeLog::notice(&format!(
                "Remote config fetch HTTP {} - keeping previous configs",
                response.status
            ));
            return;
        }

        let Some(remote) = Self::parse(&body) else {
            SwiftyTubeLog::notice(
                "Remote configs rejected (validation) - keeping previous configs",
            );
            return;
        };

        state.etag = Some(response_etag.unwrap_or_default());
        state.last_fetch_time = Self::now();
        self.apply_remote(state, remote, Some(&body));
        self.write_meta(state);
    }

    /// Merge remote over bundled, persist, bump epoch on change.
    fn apply_remote(&self, state: &mut State, remote: Configs, raw_body: Option<&str>) {
        let remote_count = remote.len();
        let mut merged = state.bundled_configs.clone();
        merged.extend(remote);
        Self::rebuild_alias_map(state, &merged);

        let changed = merged != state.merged_configs;
        let merged_count = merged.len();
        state.merged_configs = merged;
        if changed {
            state.config_epoch += 1;
        }
        SwiftyTubeLog::debug(&format!(
            "Remote configs applied ({remote_count} hashes, merged={merged_count}, changed={changed})"
        ));

        if let (Some(raw_body), Some(file)) = (raw_body, self.configs_cache_file()) {
            write_file(&file, raw_body);
        }
    }

    fn rebuild_alias_map(state: &mut State, configs: &Configs) {
        let mut map = HashMap::new();
        for (hash, entry) in configs {
            for alias in entry.aliases.iter().flatten() {
                if alias != hash {
                    map.insert(alias.clone(), hash.clone());
                }
            }
        }
        state.alias_to_hash = map;
    }

    // MARK: - Parsing / validation

    /// Validated table or None. Bad entries skipped, dup keys reject all.
    pub fn parse(text: &str) -> Option<Configs> {
        let Ok(Value::Object(root)) = serde_json::from_str::<Value>(text) else {
            return None;
        };
        let schema_version = int_primitive(root.get("schemaVersion"))?;
        if !(1..=1).contains(&schema_version) {
            return None;
        }
        let players = root.get("players")?.as_object()?;

        let mut configs = Configs::new();
        for (hash, entry) in players {
            let Some(entry) = entry.as_object() else {
                continue;
            };
            if !HASH_REGEX.is_match(hash) {
                continue;
            }
            let Some(sig) = entry
                .get("sig")
                .and_then(Value::as_str)
                .filter(|s| SIG_REGEX.is_match(s))
            else {
                continue;
            };
            let Some(n_class) = entry
                .get("nClass")
                .and_then(Value::as_str)
                .filter(|s| N_CLASS_REGEX.is_match(s))
            else {
                continue;
            };
            let Some(sts) = int_primitive(entry.get("sts")).filter(|sts| *sts > 0) else {
                continue;
            };

            let mut aliases = Vec::new();
            if let Some(alias_array) = entry.get("aliases").and_then(Value::as_array) {
                let valid: Option<Vec<String>> = alias_array
                    .iter()
                    .map(|a| {
                        a.as_str()
                            .filter(|a| HASH_REGEX.is_match(a))
                            .map(str::to_string)
                    })
                    .collect();
                let Some(valid) = valid else { continue };
                aliases = valid;
            }

            let keys: Vec<&String> = std::iter::once(hash).chain(&aliases).collect();
            let unique: std::collections::HashSet<&&String> = keys.iter().collect();
            if unique.len() != keys.len() || keys.iter().any(|k| configs.contains_key(*k)) {
                return None;
            }
            configs.insert(
                hash.clone(),
                PlayerConfig {
                    sig: Some(sig.to_string()),
                    n_class: Some(n_class.to_string()),
                    sts: Some(sts),
                    aliases: Some(aliases),
                },
            );
        }
        Some(configs)
    }

    // MARK: - Disk helpers

    // Deliberately not prefixed "player_" - PlayerJsFetcher purges player_* files here.
    fn configs_cache_file(&self) -> Option<PathBuf> {
        Some(self.store_dir.as_ref()?.join("configs_remote.json"))
    }

    fn meta_cache_file(&self) -> Option<PathBuf> {
        Some(self.store_dir.as_ref()?.join("configs_remote.meta"))
    }

    fn read_meta(&self) -> Option<(String, f64)> {
        let text = read_file(&self.meta_cache_file()?)?;
        let (etag, timestamp) = text.split_once('\n')?;
        Some((etag.to_string(), timestamp.parse().ok()?))
    }

    fn write_meta(&self, state: &State) {
        if let Some(file) = self.meta_cache_file() {
            let etag = state.etag.as_deref().unwrap_or("");
            write_file(&file, &format!("{etag}\n{}", state.last_fetch_time));
        }
    }
}

/// Non-string primitives only: a string-typed "1" must fail like upstream.
fn int_primitive(value: Option<&Value>) -> Option<i64> {
    let number = value?.as_number()?;
    number
        .as_i64()
        .or_else(|| number.as_f64().map(|f| f as i64))
}

fn read_file(file: &Path) -> Option<String> {
    std::fs::read_to_string(file).ok().filter(|t| !t.is_empty())
}

fn write_file(file: &Path, content: &str) {
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(file, content);
}
