//! Trop/Services/DownloadManager.swift: saves songs for offline play.
//!
//! Desktop differences: the stream is saved as it comes (`.m4a` for AAC,
//! `.webm` for Opus) over plain HTTP, without ffmpeg. mpv plays both, so
//! there is no transcode, no embedded cover and no AVFoundation check.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::SystemTime;

use tokio::sync::Semaphore;

use crate::database::database_service::DatabaseService;
use crate::database::entities::downloaded_track_entity::DownloadedTrackEntity;
use crate::networking::models::yt_item::SongItem;

/// Real audio streams are well above this; error and HTML bodies are not.
const MINIMUM_AUDIO_BYTES: u64 = 8_192;
const MAX_CONCURRENT_DOWNLOADS: usize = 3;
/// Bytes per request; YouTube serves pieces this size at full speed.
const RANGE_CHUNK: u64 = 10 * 1024 * 1024;

/// Swift `DownloadState` without `notStarted` (no entry) and `completed`
/// (the entry leaves; the song is in `tracks()`).
#[derive(Clone, Debug, PartialEq)]
pub enum DownloadState {
    /// 0...1.
    Downloading(f64),
    /// Why, in words for the user.
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveDownload {
    pub song: SongItem,
    pub state: DownloadState,
}

/// The stream `download` saves, from SwiftyTube's `PlaybackResult`.
#[derive(Clone, Debug, Default)]
pub struct Resolved {
    pub url: String,
    pub mime_type: String,
    /// Seconds.
    pub duration: Option<i64>,
}

pub struct DownloadManager {
    db: Arc<DatabaseService>,
    dir: PathBuf,
    http: reqwest::Client,
    slots: Semaphore,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    downloads: HashMap<String, ActiveDownload>,
    /// The current run of each download. `cancel` drops the entry, so a run
    /// whose token is gone or replaced stops at its next check.
    tokens: HashMap<String, u64>,
    next_token: u64,
}

/// Why a run stopped early.
enum Stop {
    Cancelled,
    Failed(String),
}

impl From<std::io::Error> for Stop {
    fn from(error: std::io::Error) -> Self {
        log::error!("Download file error: {error}");
        Stop::Failed("Could not save the file. Check the disk has space.".to_owned())
    }
}

impl From<reqwest::Error> for Stop {
    fn from(error: reqwest::Error) -> Self {
        log::error!("Download request failed: {error}");
        Stop::Failed(if error.is_connect() {
            "No internet connection.".to_owned()
        } else if error.is_timeout() {
            "The download timed out.".to_owned()
        } else {
            "Download failed. Try again.".to_owned()
        })
    }
}

impl DownloadManager {
    /// `dir` is the Downloads folder (`default_dir()` for the original profile).
    pub fn new(db: Arc<DatabaseService>, dir: PathBuf) -> Self {
        Self {
            db,
            dir,
            http: reqwest::Client::new(),
            slots: Semaphore::new(MAX_CONCURRENT_DOWNLOADS),
            inner: Mutex::default(),
        }
    }

    /// Swift `ProfileStorage.downloadsDirectory`: `Downloads` next to the database.
    pub fn default_dir() -> Option<PathBuf> {
        Some(DatabaseService::default_path()?.parent()?.join("Downloads"))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // MARK: - Paths

    /// `Artist - Title [videoId].ext`, as Swift's `displayDestinationURL`.
    pub fn file_name(title: &str, artist: &str, video_id: &str, extension: &str) -> String {
        let raw = match (title.is_empty(), artist.is_empty()) {
            (true, _) => video_id.to_owned(),
            (false, true) => title.to_owned(),
            (false, false) => format!("{artist} - {title}"),
        };
        format!(
            "{} [{video_id}].{extension}",
            Self::sanitize_file_name(&raw)
        )
    }

    /// Strips characters illegal on filesystems, trims whitespace and dots
    /// and caps the length well under the 255-byte name limit.
    pub fn sanitize_file_name(name: &str) -> String {
        let mut stem: String = name
            .chars()
            .map(|c| {
                if "/:\\?%*|\"<>".contains(c) || c.is_control() {
                    '-'
                } else {
                    c
                }
            })
            .collect();
        stem = stem.trim().to_owned();
        while stem.len() > 150 {
            stem.pop();
        }
        let stem = stem.trim_matches(|c: char| c.is_whitespace() || c == '.');
        if stem.is_empty() {
            "Untitled".to_owned()
        } else {
            stem.to_owned()
        }
    }

    /// File extension for a stream's MIME type: `audio/mp4` is AAC, `audio/webm` Opus.
    pub fn extension(mime_type: &str) -> &'static str {
        if mime_type.contains("webm") {
            "webm"
        } else {
            "m4a"
        }
    }

    /// A stored `local_path`: a name in the Downloads folder, or an absolute
    /// path from an older build. Falls back to the same name in the folder.
    fn resolved_path(&self, stored: &str) -> Option<PathBuf> {
        let path = Path::new(stored);
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            self.dir.join(path)
        };
        if path.is_file() {
            return Some(path);
        }
        let by_name = self.dir.join(Path::new(stored).file_name()?);
        by_name.is_file().then_some(by_name)
    }

    /// The saved file of `video_id`, when it is on disk. Blocks on the database.
    pub fn local_path(&self, video_id: &str) -> Option<PathBuf> {
        let track = self
            .db
            .fetch_one::<DownloadedTrackEntity>(video_id)
            .ok()??;
        self.resolved_path(&track.local_path)
    }

    // MARK: - Queries

    /// Saved songs, newest first. Rows whose file is gone are dropped
    /// (Swift `reload`). Blocks on the database.
    // ponytail: files without a row are not adopted back (Swift's orphans);
    // add the folder scan if files get copied in by hand.
    pub fn tracks(&self) -> rusqlite::Result<Vec<DownloadedTrackEntity>> {
        let (kept, missing): (Vec<_>, Vec<_>) = self
            .db
            .fetch_all_sql::<DownloadedTrackEntity>(
                "SELECT * FROM downloaded_track ORDER BY downloaded_at DESC",
                [],
            )?
            .into_iter()
            .partition(|track| self.resolved_path(&track.local_path).is_some());
        for track in &missing {
            self.db.delete(track)?;
        }
        Ok(kept)
    }

    /// Downloads running, queued or failed, furthest along first.
    pub fn downloads(&self) -> Vec<ActiveDownload> {
        let mut downloads: Vec<_> = self.inner().downloads.values().cloned().collect();
        let progress = |d: &ActiveDownload| match d.state {
            DownloadState::Downloading(p) => p,
            DownloadState::Failed(_) => -1.0,
        };
        downloads.sort_by(|a, b| {
            progress(b)
                .total_cmp(&progress(a))
                .then_with(|| a.song.title.cmp(&b.song.title))
        });
        downloads
    }

    // MARK: - Download

    /// Saves `song`. `resolve` gives the stream to save.
    /// `Ok(true)` once saved, `Ok(false)` when cancelled or already running.
    pub async fn download<F>(&self, song: SongItem, resolve: F) -> Result<bool, String>
    where
        F: Future<Output = Result<Resolved, String>>,
    {
        let video_id = song.video_id.clone();
        let token = {
            let mut inner = self.inner();
            if let Some(ActiveDownload {
                state: DownloadState::Downloading(_),
                ..
            }) = inner.downloads.get(&video_id)
            {
                return Ok(false);
            }
            inner.next_token += 1;
            let token = inner.next_token;
            inner.tokens.insert(video_id.clone(), token);
            inner.downloads.insert(
                video_id.clone(),
                ActiveDownload {
                    song: song.clone(),
                    state: DownloadState::Downloading(0.0),
                },
            );
            token
        };
        // Queued downloads stay at 0 until a slot frees up.
        let result = match self.slots.acquire().await {
            Ok(_slot) => self.run(&song, token, resolve).await,
            Err(_) => Err(Stop::Cancelled),
        };
        let mut inner = self.inner();
        // A newer run for this id owns the state.
        if inner.tokens.get(&video_id) != Some(&token) {
            return Ok(false);
        }
        match result {
            Ok(()) => {
                inner.tokens.remove(&video_id);
                inner.downloads.remove(&video_id);
                Ok(true)
            }
            Err(Stop::Cancelled) => Ok(false),
            Err(Stop::Failed(reason)) => {
                inner.tokens.remove(&video_id);
                if let Some(download) = inner.downloads.get_mut(&video_id) {
                    download.state = DownloadState::Failed(reason.clone());
                }
                Err(reason)
            }
        }
    }

    async fn run<F>(&self, song: &SongItem, token: u64, resolve: F) -> Result<(), Stop>
    where
        F: Future<Output = Result<Resolved, String>>,
    {
        let video_id = &song.video_id;
        if !self.is_current(video_id, token) {
            return Err(Stop::Cancelled);
        }
        if self.local_path(video_id).is_some() {
            return Ok(());
        }
        self.set_progress(video_id, token, 0.05);
        let resolved = resolve.await.map_err(|error| {
            log::error!("Download resolve failed for {video_id}: {error}");
            Stop::Failed("This song has no stream to save. Try again later.".to_owned())
        })?;
        std::fs::create_dir_all(&self.dir)?;
        let artist = song
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let name = Self::file_name(
            &song.title,
            &artist,
            video_id,
            Self::extension(&resolved.mime_type),
        );
        // One file per run: a cancelled run removes its own file, not a retry's.
        let part = self.dir.join(format!(".{video_id}.{token}.part"));
        let fetched = self.fetch(&resolved.url, &part, video_id, token).await;
        // Deleted while saving: stop before the file gets its name.
        let fetched = fetched.and_then(|()| {
            if self.is_current(video_id, token) {
                Ok(std::fs::rename(&part, self.dir.join(&name))?)
            } else {
                Err(Stop::Cancelled)
            }
        });
        if fetched.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        fetched?;
        let track = DownloadedTrackEntity {
            id: video_id.clone(),
            title: song.title.clone(),
            artist,
            // Home's song cards come without a length.
            duration: match song.duration {
                0 => resolved.duration.unwrap_or_default(),
                known => known,
            },
            thumbnail_url: song.thumbnail_url.clone(),
            local_path: name,
            downloaded_at: SystemTime::now(),
        };
        self.db.insert_or_replace(&track).map_err(|error| {
            log::error!("Failed to persist download {video_id}: {error}");
            Stop::Failed("Could not record the download.".to_owned())
        })?;
        Ok(())
    }

    /// Saves `url` to `part` in `RANGE_CHUNK` pieces: YouTube slows a
    /// request for the whole file down to playback speed.
    async fn fetch(&self, url: &str, part: &Path, video_id: &str, token: u64) -> Result<(), Stop> {
        let mut file = std::fs::File::create(part)?;
        let mut written = 0_u64;
        let mut total = None;
        loop {
            let mut response = self
                .http
                .get(url)
                .header(reqwest::header::USER_AGENT, "Mozilla/5.0")
                .header(
                    reqwest::header::RANGE,
                    format!("bytes={written}-{}", written + RANGE_CHUNK - 1),
                )
                .send()
                .await?
                .error_for_status()?;
            // A server without ranges answers 200 with the whole file.
            let ranged = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
            total = if ranged {
                total.or_else(|| content_range_total(&response))
            } else {
                response.content_length()
            };
            let start = written;
            while let Some(chunk) = response.chunk().await? {
                if !self.is_current(video_id, token) {
                    return Err(Stop::Cancelled);
                }
                file.write_all(&chunk)?;
                written += chunk.len() as u64;
                self.set_progress(video_id, token, progress(written, total));
            }
            if !ranged || written == start || total.is_none_or(|total| written >= total) {
                break;
            }
        }
        file.flush()?;
        // A dropped connection can end the body early without an error.
        if total.is_some_and(|total| total != written) || written < MINIMUM_AUDIO_BYTES {
            log::error!("Download of {video_id} cut short: {written} of {total:?} bytes");
            return Err(Stop::Failed(
                "The download stopped early. Try again.".to_owned(),
            ));
        }
        Ok(())
    }

    fn is_current(&self, video_id: &str, token: u64) -> bool {
        self.inner().tokens.get(video_id) == Some(&token)
    }

    fn set_progress(&self, video_id: &str, token: u64, fraction: f64) {
        let mut inner = self.inner();
        if inner.tokens.get(video_id) != Some(&token) {
            return;
        }
        if let Some(download) = inner.downloads.get_mut(video_id) {
            download.state = DownloadState::Downloading(fraction.clamp(0.0, 0.99));
        }
    }

    // MARK: - Cancel & delete

    /// Stops a running or queued download, or clears a failed one.
    pub fn cancel(&self, video_id: &str) {
        let mut inner = self.inner();
        inner.tokens.remove(video_id);
        inner.downloads.remove(video_id);
    }

    /// Cancels `video_id` and removes its file and row. Blocks on the database.
    pub fn delete(&self, video_id: &str) -> rusqlite::Result<()> {
        self.cancel(video_id);
        if let Some(track) = self.db.fetch_one::<DownloadedTrackEntity>(video_id)? {
            if let Some(path) = self.resolved_path(&track.local_path) {
                let _ = std::fs::remove_file(path);
            }
            self.db.delete(&track)?;
        }
        Ok(())
    }
}

/// The full size in a `Content-Range: bytes 0-99/1234` header.
fn content_range_total(response: &reqwest::Response) -> Option<u64> {
    let header = response.headers().get(reqwest::header::CONTENT_RANGE)?;
    header.to_str().ok()?.rsplit_once('/')?.1.parse().ok()
}

/// 0.05...0.98 by bytes when the size is known. Otherwise an asymptotic
/// curve (Swift's), so the bar still moves.
fn progress(written: u64, total: Option<u64>) -> f64 {
    match total {
        Some(total) if total > 0 => 0.05 + 0.93 * written as f64 / total as f64,
        _ => 0.05 + 0.80 * (1.0 - (-(written as f64) / 5_000_000.0).exp()),
    }
}
