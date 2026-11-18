// Trop/Discord/DiscordActivityBuilder.swift. Swift takes each setting as a
// parameter, read by DiscordIntegration.swift from `UserDefaults`; here they
// come in one `DiscordSettings`.

use crate::networking::models::yt_item::SongItem;

use super::discord_activity::DiscordActivity;
use super::discord_defaults::{DiscordDefaults, DiscordSettings};
use super::discord_presence::ActivityType;
use super::discord_template_renderer::DiscordTemplateRenderer;

pub struct DiscordActivityBuilder;

impl DiscordActivityBuilder {
    /// Swift `build(song:...)`. `artist_name` is never empty: callers pass
    /// `DiscordDefaults::UNKNOWN_ARTIST` instead.
    pub fn build(
        song: &SongItem,
        artist_name: &str,
        album_name: Option<&str>,
        artist_thumbnail: Option<&str>,
        start_timestamp: i64,
        end_timestamp: Option<i64>,
        settings: &DiscordSettings,
    ) -> DiscordActivity {
        let title = song.title.as_str();
        let render = |template: &str| {
            DiscordTemplateRenderer::render(
                template,
                title,
                artist_name,
                album_name,
                &song.video_id,
            )
        };
        // An emptied setting falls back to its default, like Swift.
        let or = |value: &str, fallback: &str| {
            if value.is_empty() { fallback } else { value }.to_owned()
        };

        let (state, details, button1, button2) = if settings.advanced_mode {
            let button = |enabled: bool, label: String, url: String| {
                enabled.then(|| (render(&label), render(&url)))
            };
            (
                render(&or(
                    &settings.state_template,
                    DiscordDefaults::STATE_TEMPLATE,
                )),
                render(&or(
                    &settings.details_template,
                    DiscordDefaults::DETAILS_TEMPLATE,
                )),
                button(
                    settings.button1_enabled,
                    or(&settings.button1_label, DiscordDefaults::BUTTON1_LABEL),
                    or(&settings.button1_url, DiscordDefaults::BUTTON1_URL_TEMPLATE),
                ),
                button(
                    settings.button2_enabled,
                    or(&settings.button2_label, DiscordDefaults::BUTTON2_LABEL),
                    or(&settings.button2_url, DiscordDefaults::BUTTON2_URL),
                ),
            )
        } else {
            (
                artist_name.to_owned(),
                title.to_owned(),
                Some((
                    DiscordDefaults::BUTTON1_LABEL.to_owned(),
                    format!("{}{}", DiscordDefaults::YOUTUBE_WATCH_URL, song.video_id),
                )),
                Some((
                    DiscordDefaults::BUTTON2_LABEL.to_owned(),
                    DiscordDefaults::BUTTON2_URL.to_owned(),
                )),
            )
        };

        let name = match settings.activity_name.as_str() {
            "" => artist_name.to_owned(),
            name if settings.advanced_mode => render(name),
            name => name.to_owned(),
        };
        let activity_type = settings
            .activity_type
            .parse()
            .ok()
            .and_then(ActivityType::from_raw)
            .unwrap_or_default();
        let (button1_label, button1_url) = button1.unzip();
        let (button2_label, button2_url) = button2.unzip();

        DiscordActivity {
            activity_type,
            name: Some(name),
            state,
            details: Some(details),
            start_timestamp,
            end_timestamp,
            large_image: song.thumbnail_url.clone(),
            large_text: album_name.map(str::to_owned),
            small_image: artist_thumbnail.map(str::to_owned),
            small_text: Some(artist_name.to_owned()),
            button1_label,
            button1_url,
            button2_label,
            button2_url,
        }
    }
}
