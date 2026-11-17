// SwiftyTube/Streaming/VideoAvailability.swift: only `rowMusicVideoType`,
// which the row parsers need. The playability checks come with R3.

use swiftytube::networking::inner_tube_decode::{InnerTubeDecode, JsonDict, as_dict};

pub struct VideoAvailability;

impl VideoAvailability {
    /// Watch-endpoint music type of a list row (`MUSIC_VIDEO_TYPE_ATV`...):
    /// the row's own endpoint, else the first title run's.
    pub fn row_music_video_type(renderer: &JsonDict) -> Option<String> {
        if let Some(nav) = as_dict(renderer.get("navigationEndpoint"))
            && let Some(kind) = Self::music_video_type(nav)
        {
            return Some(kind);
        }
        let flex_columns = renderer.get("flexColumns")?.as_array()?;
        let first = flex_columns.first()?.as_object()?;
        let col = as_dict(first.get("musicResponsiveListItemFlexColumnRenderer"))
            .or_else(|| as_dict(first.get("musicResponsiveListItemColumnRenderer")))?;
        let runs = as_dict(col.get("text"))?.get("runs")?.as_array()?;
        runs.iter()
            .filter_map(|run| as_dict(run.get("navigationEndpoint")))
            .find_map(Self::music_video_type)
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
