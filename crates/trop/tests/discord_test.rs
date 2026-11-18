//! The Discord activity a song gets, in both modes of the Swift settings.

use trop::discord::discord_activity_builder::DiscordActivityBuilder;
use trop::discord::discord_defaults::DiscordSettings;
use trop::discord::discord_presence::ActivityType;
use trop::networking::models::yt_item::SongItem;

fn song() -> SongItem {
    SongItem {
        video_id: "abc".into(),
        title: "Song".into(),
        thumbnail_url: Some("https://img/cover.jpg".into()),
        ..SongItem::default()
    }
}

#[test]
fn default_mode_shows_artist_title_and_watch_link() {
    let activity = DiscordActivityBuilder::build(
        &song(),
        "Artist",
        None,
        None,
        1_000,
        Some(5_000),
        &DiscordSettings::default(),
    );
    assert_eq!(activity.activity_type, ActivityType::Listening);
    assert_eq!(activity.name.as_deref(), Some("Artist"));
    assert_eq!(activity.state, "Artist");
    assert_eq!(activity.details.as_deref(), Some("Song"));
    assert_eq!(
        activity.button1_url.as_deref(),
        Some("https://music.youtube.com/watch?v=abc")
    );
    assert_eq!(
        activity.large_image.as_deref(),
        Some("https://img/cover.jpg")
    );
}

#[test]
fn advanced_mode_renders_templates_and_drops_disabled_buttons() {
    let settings = DiscordSettings {
        advanced_mode: true,
        activity_type: "0".into(),
        activity_name: "{song.name} radio".into(),
        state_template: "by {artist.name} on {album.name}".into(),
        details_template: String::new(),
        button2_enabled: false,
        ..DiscordSettings::default()
    };
    let activity = DiscordActivityBuilder::build(&song(), "Artist", None, None, 0, None, &settings);
    assert_eq!(activity.activity_type, ActivityType::Playing);
    assert_eq!(activity.name.as_deref(), Some("Song radio"));
    assert_eq!(activity.state, "by Artist on Unknown Album");
    // An emptied template falls back to the default one.
    assert_eq!(activity.details.as_deref(), Some("Song"));
    assert_eq!(
        activity.button1_url.as_deref(),
        Some("https://music.youtube.com/watch?v=abc")
    );
    assert_eq!(activity.button2_label, None);
}
