//! `Trop/Discord/`: what Discord shows while a song plays.
//!
//! Not ported: `DiscordAuth`, `DiscordTokenStore`, `DiscordRpcManager`,
//! `DiscordIntegration` and the Social SDK bridge. iOS signs in to Discord
//! through its Social SDK; the desktop app talks to the local Discord client
//! over IPC instead (no sign-in), in `crates/app/src/integrations.rs`.

pub mod discord_activity;
pub mod discord_activity_builder;
pub mod discord_defaults;
pub mod discord_presence;
pub mod discord_template_renderer;
