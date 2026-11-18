// Trop/Discord/DiscordTemplateRenderer.swift.

use super::discord_defaults::DiscordDefaults;

pub struct DiscordTemplateRenderer;

impl DiscordTemplateRenderer {
    pub const PLACEHOLDERS: [&str; 4] =
        ["{song.name}", "{artist.name}", "{album.name}", "{song.id}"];

    pub fn render(
        template: &str,
        title: &str,
        artist: &str,
        album: Option<&str>,
        song_id: &str,
    ) -> String {
        template
            .replace("{song.name}", title)
            .replace("{song.id}", song_id)
            .replace("{artist.name}", artist)
            .replace(
                "{album.name}",
                album.unwrap_or(DiscordDefaults::UNKNOWN_ALBUM),
            )
    }
}
