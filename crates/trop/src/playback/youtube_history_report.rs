//! Trop/Playback/YouTubeHistoryReport.swift: tells YouTube Music about one
//! play, so it reaches the account's history that the History page and other
//! devices read.
//!
//! The reporting addresses come from a `player` request as the signed-in
//! client: the ones inside a stream's own response belong to the client that
//! resolved it, and are not tied to the account.
//!
//! Swift is an actor. Here the state sits behind a lock: `watch` decides
//! synchronously whether a report is due, and the caller sends it with `send`.

use std::collections::HashSet;
use std::hash::{BuildHasher, RandomState};
use std::sync::Mutex;

use reqwest::Url;
use serde_json::Value;
use swiftytube::networking::inner_tube_client::{InnerTubeClient, PlayerArgs};
use swiftytube::streaming::player_request_options::PlayerRequestOptions;

pub struct YouTubeHistoryReport {
    video_id: String,
    /// YouTube's name for one viewing (`cpn`): 16 random characters, the same on every report.
    session: String,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    watchtime_url: Option<String>,
    /// Seconds of the play already reported.
    reported: f64,
}

impl YouTubeHistoryReport {
    /// Seconds between watch-time reports while a song plays.
    const INTERVAL: f64 = 30.0;

    pub fn new(video_id: &str) -> Self {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let random = RandomState::new();
        let session = (0..16u64)
            .map(|i| CHARS[(random.hash_one(i) % CHARS.len() as u64) as usize] as char)
            .collect();
        Self {
            video_id: video_id.to_owned(),
            session,
            state: Mutex::default(),
        }
    }

    pub fn video_id(&self) -> &str {
        &self.video_id
    }

    /// Opens the play, then reports the first `seconds`. Silent when signed out or offline.
    pub async fn start(
        &self,
        client: &InnerTubeClient,
        signature_timestamp: Option<i64>,
        seconds: f64,
    ) {
        let auth = client.current_auth();
        if auth.sapisid.is_none() || auth.cookies.is_empty() {
            return;
        }
        let args = PlayerArgs {
            video_id: self.video_id.clone(),
            options: PlayerRequestOptions {
                signature_timestamp,
                po_token: None,
            },
            ..PlayerArgs::default()
        };
        let Ok(json) = client.player(args).await else {
            log::debug!(
                "No tracking addresses for {}: offline or refused",
                self.video_id
            );
            return;
        };
        let address = |key: &str| {
            json.get("playbackTracking")
                .and_then(|tracking| tracking.get(key))
                .and_then(|address| address.get("baseUrl"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        self.lock().watchtime_url = address("videostatsWatchtimeUrl");
        let Some(playback) = address("videostatsPlaybackUrl") else {
            return;
        };
        if let Some(url) = self.address(&playback, &[("cmt", text(0.0))]) {
            Self::send(client, url).await;
        }
        if let Some(url) = self.watch(seconds, true) {
            Self::send(client, url).await;
        }
    }

    /// The report of the stretch since the last one, once it is long enough
    /// or when the song ends (`last`). `None` when nothing is due.
    pub fn watch(&self, seconds: f64, last: bool) -> Option<String> {
        let mut state = self.lock();
        let url = state.watchtime_url.clone()?;
        let step = if last { 1.0 } else { Self::INTERVAL };
        if seconds - state.reported < step {
            return None;
        }
        let start = state.reported;
        state.reported = seconds;
        drop(state);
        self.address(
            &url,
            &[
                ("st", text(start)),
                ("et", text(seconds)),
                ("cmt", text(seconds)),
                ("final", if last { "1" } else { "0" }.to_owned()),
            ],
        )
    }

    /// Opens a report address as the signed-in user. A failure is only logged:
    /// the play is already in the local history.
    pub async fn send(client: &InnerTubeClient, url: String) {
        if let Err(error) = client.register_playback(&url, None, None).await {
            log::info!("YouTube history report failed: {error}");
        }
    }

    /// `address` with `values`, the viewing and the page added. Only YouTube's
    /// own https addresses: they receive the sign-in.
    fn address(&self, address: &str, values: &[(&str, String)]) -> Option<String> {
        let mut url = Url::parse(address).ok()?;
        let host = url.host_str()?;
        if url.scheme() != "https" || !(host == "youtube.com" || host.ends_with(".youtube.com")) {
            return None;
        }
        let mut added: Vec<(&str, &str)> = values.iter().map(|(k, v)| (*k, v.as_str())).collect();
        added.push(("cpn", &self.session));
        added.push(("el", "detailpage"));
        let keys: HashSet<&str> = added.iter().map(|(key, _)| *key).collect();
        let kept: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(key, _)| !keys.contains(key.as_ref()))
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        url.query_pairs_mut()
            .clear()
            .extend_pairs(kept)
            .extend_pairs(added);
        Some(url.into())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn text(seconds: f64) -> String {
    format!("{seconds:.3}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(url: &str) -> Vec<(String, String)> {
        Url::parse(url)
            .unwrap()
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect()
    }

    fn value(url: &str, key: &str) -> Option<String> {
        query(url)
            .into_iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
    }

    #[test]
    fn watch_reports_every_30_seconds_and_at_the_end() {
        let report = YouTubeHistoryReport::new("v");
        // Nothing before `start` found the address.
        assert_eq!(report.watch(40.0, false), None);
        report.lock().watchtime_url =
            Some("https://s.youtube.com/api/stats/watchtime?ns=yt&el=embedded&docid=v".to_owned());

        let first = report.watch(31.0, false).unwrap();
        assert_eq!(value(&first, "st").as_deref(), Some("0.000"));
        assert_eq!(value(&first, "et").as_deref(), Some("31.000"));
        assert_eq!(value(&first, "final").as_deref(), Some("0"));
        // The page is replaced, not doubled; the address's own values stay.
        assert_eq!(value(&first, "el").as_deref(), Some("detailpage"));
        assert_eq!(query(&first).iter().filter(|(k, _)| k == "el").count(), 1);
        assert_eq!(value(&first, "docid").as_deref(), Some("v"));

        assert_eq!(report.watch(50.0, false), None);
        let last = report.watch(50.0, true).unwrap();
        assert_eq!(value(&last, "st").as_deref(), Some("31.000"));
        assert_eq!(value(&last, "final").as_deref(), Some("1"));
        // One viewing: the same `cpn` on every report.
        assert_eq!(value(&first, "cpn"), value(&last, "cpn"));
        assert_eq!(value(&last, "cpn").unwrap().len(), 16);
    }

    #[test]
    fn only_youtube_addresses_get_the_sign_in() {
        let report = YouTubeHistoryReport::new("v");
        assert!(
            report
                .address("https://evil.example/api?x=1", &[])
                .is_none()
        );
        assert!(report.address("http://s.youtube.com/api", &[]).is_none());
        assert!(report.address("https://notyoutube.com/api", &[]).is_none());
        assert!(report.address("https://youtube.com/api", &[]).is_some());
    }
}
