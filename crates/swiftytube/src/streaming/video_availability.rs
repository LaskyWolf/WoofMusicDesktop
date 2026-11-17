use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard};

use super::playback_result::PlaybackResult;
use super::player_response::PlayerResponse;
use crate::core::swiftytube_log::SwiftyTubeLog;
use crate::core::youtube_client::YouTubeClient;
use crate::networking::inner_tube_client::{InnerTubeClient, PlayerArgs};
use crate::networking::inner_tube_decode::{InnerTubeDecode, JsonDict, as_dict, as_dict_list};

// ponytail: Swift also joins concurrent lookups of one videoId (`inflight`);
// here two rows asking at once both call /player. Add it if a list view does that.
#[derive(Default)]
pub struct VideoAvailability {
    verdicts: Mutex<HashMap<String, bool>>,
}

impl VideoAvailability {
    const MAX_CACHED_VERDICTS: usize = 1000;

    pub fn shared() -> &'static VideoAvailability {
        static SHARED: LazyLock<VideoAvailability> = LazyLock::new(VideoAvailability::default);
        &SHARED
    }

    fn verdicts(&self) -> MutexGuard<'_, HashMap<String, bool>> {
        self.verdicts.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether `video_id` has a watchable video (not an auto-generated still).
    pub async fn has_video(
        &self,
        video_id: &str,
        row_type: Option<&str>,
        result: &PlaybackResult,
        client: &InnerTubeClient,
    ) -> bool {
        let known = row_type.or(result.music_video_type.as_deref());
        if known.is_some() || !result.has_video_content {
            return PlaybackResult::has_watchable_video(known, result.has_video_content);
        }
        if let Some(cached) = self.verdicts().get(video_id) {
            return *cached;
        }
        Self::fetch_verdict(video_id, result, client).await
    }

    async fn fetch_verdict(
        video_id: &str,
        result: &PlaybackResult,
        client: &InnerTubeClient,
    ) -> bool {
        let response = client
            .player_response::<PlayerResponse>(PlayerArgs {
                video_id: video_id.to_string(),
                client: Some(YouTubeClient::WEB_REMIX),
                ..PlayerArgs::default()
            })
            .await;
        match response {
            Ok(response) => {
                let verdict = PlaybackResult::has_watchable_video(
                    response
                        .video_details
                        .as_ref()
                        .and_then(|d| d.music_video_type.as_deref()),
                    result.has_video_content,
                );
                Self::shared().store(video_id, verdict);
                verdict
            }
            Err(error) => {
                SwiftyTubeLog::debug(&format!(
                    "VideoAvailability lookup failed for {video_id}: {error}"
                ));
                PlaybackResult::has_watchable_video(None, result.has_video_content)
            }
        }
    }

    fn store(&self, video_id: &str, verdict: bool) {
        let mut verdicts = self.verdicts();
        if verdicts.len() >= Self::MAX_CACHED_VERDICTS {
            verdicts.clear();
        }
        verdicts.insert(video_id.to_string(), verdict);
    }

    /// Watch-endpoint music type of a row (`MUSIC_VIDEO_TYPE_ATV` for
    /// auto-generated art-tracks), from the renderer's own endpoint first,
    /// then the title-run endpoints. None when the row carries none.
    pub fn row_music_video_type(renderer: &JsonDict) -> Option<String> {
        if let Some(kind) =
            as_dict(renderer.get("navigationEndpoint")).and_then(Self::music_video_type)
        {
            return Some(kind);
        }
        let first = as_dict_list(renderer.get("flexColumns"))?
            .into_iter()
            .next()?;
        let col = as_dict(first.get("musicResponsiveListItemFlexColumnRenderer"))
            .or_else(|| as_dict(first.get("musicResponsiveListItemColumnRenderer")));
        let runs = col
            .and_then(|c| InnerTubeDecode::array(&["text", "runs"], c))
            .unwrap_or_default();
        runs.into_iter()
            .find_map(|run| as_dict(run.get("navigationEndpoint")).and_then(Self::music_video_type))
    }

    fn music_video_type(nav: &JsonDict) -> Option<String> {
        InnerTubeDecode::string(
            &[
                "watchEndpoint",
                "watchEndpointMusicSupportedConfigs",
                "watchEndpointMusicConfig",
                "musicVideoType",
            ],
            nav,
        )
        .map(str::to_string)
    }
}
