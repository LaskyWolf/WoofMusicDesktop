//! Trop/History/Shorts.swift: tells YouTube Shorts apart from other videos.
//!
//! YouTube Music's history lists Shorts watched on YouTube as plain user
//! videos (UGC), with nothing marking them; youtube.com/shorts/<id> answers
//! 200 for a Short and redirects otherwise.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use tokio::task::JoinSet;

use crate::networking::models::yt_item::SongItem;

pub struct Shorts;

/// Answers already known, by video id.
static CACHE: LazyLock<Mutex<HashMap<String, bool>>> = LazyLock::new(Default::default);

/// Never follows a redirect: the redirect itself is the answer.
static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .build()
        .expect("TLS backend initializes")
});

impl Shorts {
    /// `songs` without Shorts. Only user videos are checked; a failed check keeps the song.
    pub async fn removing(songs: Vec<SongItem>) -> Vec<SongItem> {
        let candidates: HashSet<String> = songs
            .iter()
            .filter(|song| song.music_video_type.as_deref() == Some("MUSIC_VIDEO_TYPE_UGC"))
            .map(|song| song.video_id.clone())
            .collect();
        if candidates.is_empty() {
            return songs;
        }
        let mut checks = JoinSet::new();
        for id in candidates {
            checks.spawn(async move { Self::is_short(&id).await.then_some(id) });
        }
        let mut shorts = HashSet::new();
        while let Some(result) = checks.join_next().await {
            if let Ok(Some(id)) = result {
                shorts.insert(id);
            }
        }
        songs
            .into_iter()
            .filter(|song| !shorts.contains(&song.video_id))
            .collect()
    }

    async fn is_short(video_id: &str) -> bool {
        if let Some(known) = CACHE.lock().ok().and_then(|c| c.get(video_id).copied()) {
            return known;
        }
        let response = HTTP
            .head(format!("https://www.youtube.com/shorts/{video_id}"))
            // Without the consent cookie, EU visitors are redirected to the consent page for every video.
            .header("Cookie", "SOCS=CAI")
            // A phone or app user agent is sent to m.youtube.com first, Short or not.
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15",
            )
            .send()
            .await;
        let Ok(response) = response else {
            return false;
        };
        let short = response.status().as_u16() == 200;
        if let Ok(mut cache) = CACHE.lock() {
            cache.insert(video_id.to_owned(), short);
        }
        short
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_user_videos_are_checked() {
        let song = SongItem {
            video_id: "a".to_owned(),
            music_video_type: Some("MUSIC_VIDEO_TYPE_ATV".to_owned()),
            ..SongItem::default()
        };
        // No network: an art track is never a Short.
        assert_eq!(Shorts::removing(vec![song.clone()]).await, vec![song]);
    }
}
