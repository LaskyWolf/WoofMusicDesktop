//! The pages: Home, Explore, Search, album, artist, playlist, podcast, mood, Library.

use std::sync::Arc;

use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Gradient, Length, Radians};
use trop::detail::detail_models::{
    AlbumDetailInfo, ArtistDetailInfo, PlaylistDetailInfo, PodcastDetailInfo,
};
use trop::networking::models::home_page::{
    ExploreSection, ExploreSectionKind, HomeSection, MoodItem,
};
use trop::networking::models::yt_item::{SongItem, YTItem};
use trop::search::search_parser::SearchSection;

use crate::ui::{self, Covers};
use crate::{Load, Message, Route, theme, virtual_list};

pub const SEARCH_INPUT: &str = "search";

pub fn home<'a>(
    covers: &'a Covers,
    home: &'a Load<Vec<HomeSection>>,
    playing: Option<&str>,
) -> Element<'a, Message> {
    let sections = match home {
        Load::Loaded(sections) => sections,
        Load::Loading => return ui::status(None, Message::Retry),
        Load::Failed(error) => return ui::status(Some(error), Message::Retry),
    };
    ui::scroll_page(
        sections
            .iter()
            .filter(|section| !section.items().is_empty())
            .map(|section| {
                let limit = if section.is_songs_only() { 12 } else { 10 };
                ui::section(
                    covers,
                    section.display_title(),
                    section.items(),
                    limit,
                    playing,
                )
            })
            .collect(),
    )
}

/// Explore feed, or a mood's page: shelves of rows and cards, mood chips.
pub fn explore<'a>(
    covers: &'a Covers,
    sections: &'a [ExploreSection],
    playing: Option<&str>,
) -> Element<'a, Message> {
    ui::scroll_page(
        sections
            .iter()
            .filter(|section| !section.items.is_empty() || !section.moods.is_empty())
            .map(|section| match section.kind {
                ExploreSectionKind::Moods => moods(&section.title, &section.moods),
                ExploreSectionKind::Rows => {
                    ui::section(covers, &section.title, &section.items, 12, playing)
                }
                ExploreSectionKind::Cards => {
                    ui::section(covers, &section.title, &section.items, 10, playing)
                }
            })
            .collect(),
    )
}

/// Mood chips: one line each, so every chip in a row has the same height.
fn moods<'a>(title: &'a str, moods: &'a [MoodItem]) -> Element<'a, Message> {
    const CHIP: f32 = 228.0;
    // Chip width minus padding, swatch and gap.
    const LABEL: f32 = CHIP - 2.0 * 14.0 - 4.0 - 12.0;
    let chips = moods.iter().map(|mood| {
        let color = mood
            .color
            .map(|rgb| Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
            .unwrap_or(theme::ACCENT);
        let swatch = container(Space::new())
            .width(4)
            .height(22)
            .style(move |_| container::Style {
                background: Some(Background::Color(color)),
                border: Border::default().rounded(2),
                ..container::Style::default()
            });
        button(
            row![
                swatch,
                text(ui::fit(&mood.title, LABEL, 14.0))
                    .size(14)
                    .font(theme::weight(Weight::Medium))
                    .wrapping(text::Wrapping::None)
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding([10, 14])
        .width(CHIP)
        .on_press(Message::Navigate(Route::Mood(mood.clone())))
        .style(|_, status| button::Style {
            background: Some(Background::Color(if status == button::Status::Hovered {
                theme::PRESSED
            } else {
                theme::HOVER
            })),
            border: Border::default().rounded(12),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
        .into()
    });
    column![
        ui::section_title(title),
        iced::widget::Row::with_children(chips)
            .spacing(10)
            .wrap()
            .vertical_spacing(10),
    ]
    .spacing(14)
    .into()
}

/// Search field and results. Results are `None` before the first search.
pub fn search<'a>(
    covers: &'a Covers,
    query: &'a str,
    results: Option<&'a Load<Vec<SearchSection>>>,
    playing: Option<&str>,
) -> Element<'a, Message> {
    let field = text_input("Songs, albums, artists, playlists, podcasts", query)
        .id(SEARCH_INPUT)
        .on_input(Message::SearchInput)
        .on_submit(Message::SearchSubmit)
        .size(16)
        .padding([12, 16])
        .style(ui::field_style);
    let mut page = vec![container(field).max_width(640).into()];
    match results {
        None => {}
        Some(Load::Loading) => page.push(text("Searching…").color(theme::SECONDARY).into()),
        Some(Load::Failed(error)) => page.push(
            text(format!("Search failed: {error}. Press Enter to try again."))
                .color(theme::SECONDARY)
                .into(),
        ),
        Some(Load::Loaded(sections)) if sections.is_empty() => {
            page.push(text("No results").color(theme::SECONDARY).into());
        }
        Some(Load::Loaded(sections)) => page.extend(
            sections
                .iter()
                .filter(|section| !section.items.is_empty())
                .map(|section| ui::section(covers, &section.title, &section.items, 12, playing)),
        ),
    }
    ui::scroll_page(page)
}

pub fn album<'a>(
    covers: &'a Covers,
    album: &'a AlbumDetailInfo,
    songs: &Arc<Vec<SongItem>>,
    playing: Option<&str>,
) -> Element<'a, Message> {
    let line = [
        ui::artists(&album.artists),
        album.year.map(|y| y.to_string()).unwrap_or_default(),
        count(album.song_count.max(album.songs.len() as i64), "song"),
        ui::total_length(album.duration),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" • ");
    let mut page = vec![ui::header(
        covers,
        album.thumbnail_url.as_deref(),
        false,
        &album.title,
        vec![line],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
    )];
    page.push(tracks(covers, songs, false, playing));
    ui::scroll_page(page)
}

pub fn playlist<'a>(
    covers: &'a Covers,
    playlist: &'a PlaylistDetailInfo,
    songs: &Arc<Vec<SongItem>>,
    playing: Option<&str>,
) -> Element<'a, Message> {
    let line = [
        playlist.author_name.clone().unwrap_or_default(),
        count(playlist.song_count.max(playlist.songs.len() as i64), "song"),
        ui::total_length(playlist.duration),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" • ");
    let description = playlist
        .description_text
        .as_deref()
        .map(|d| ui::fit(d.lines().next().unwrap_or_default(), 560.0, 14.0))
        .unwrap_or_default();
    let mut page = vec![ui::header(
        covers,
        playlist.thumbnail_url.as_deref(),
        false,
        &playlist.title,
        vec![line, description],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
    )];
    page.push(tracks(covers, songs, true, playing));
    ui::scroll_page(page)
}

pub fn artist<'a>(
    covers: &'a Covers,
    artist: &'a ArtistDetailInfo,
    songs: &Arc<Vec<SongItem>>,
    related: &'a [YTItem],
    playing: Option<&str>,
) -> Element<'a, Message> {
    let mut page = vec![ui::header(
        covers,
        artist.thumbnail_url.as_deref(),
        true,
        &artist.name,
        vec![
            artist
                .subscriber_count_text
                .as_deref()
                .map(subscriber_label)
                .unwrap_or_default(),
        ],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
    )];
    if !songs.is_empty() {
        page.push(
            column![
                ui::section_title("Songs"),
                tracks(covers, songs, true, playing)
            ]
            .spacing(14)
            .into(),
        );
    }
    // Shelves of albums, singles, playlists and related artists, kept as YTItems.
    for (title, items) in shelves(related) {
        page.push(ui::section(covers, title, items, 10, playing));
    }
    if let Some(about) = artist.description_text.as_deref().filter(|d| !d.is_empty()) {
        page.push(
            column![
                ui::section_title("About"),
                container(text(about).size(14).color(theme::SECONDARY)).max_width(720),
            ]
            .spacing(14)
            .into(),
        );
    }
    ui::scroll_page(page)
}

/// A podcast: header, description, then its episodes, newest first as YouTube sends them.
pub fn podcast<'a>(
    covers: &'a Covers,
    podcast: &'a PodcastDetailInfo,
    songs: &Arc<Vec<SongItem>>,
    playing: Option<&str>,
) -> Element<'a, Message> {
    let mut page = vec![ui::header(
        covers,
        podcast.thumbnail_url.as_deref(),
        false,
        &podcast.title,
        vec![
            podcast.author.clone().unwrap_or_default(),
            count(podcast.episodes.len() as i64, "episode"),
        ],
        (!songs.is_empty()).then(|| Message::PlayQueue(songs.clone(), 0)),
    )];
    if let Some(about) = podcast
        .description_text
        .as_deref()
        .filter(|d| !d.is_empty())
    {
        page.push(
            container(text(about).size(14).color(theme::SECONDARY))
                .max_width(720)
                .into(),
        );
    }
    // `songs` holds the episodes in the same order (`Detail::podcast`).
    let rows = podcast.episodes.iter().enumerate().map(|(index, episode)| {
        let subtitle = [
            episode.publish_date.clone().unwrap_or_default(),
            episode.views_text.clone().unwrap_or_default(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" • ");
        let is_playing = playing == Some(episode.video_id.as_str());
        ui::track_row(covers, songs, index, true, is_playing, subtitle, None)
    });
    page.push(column(rows).spacing(2).into());
    ui::scroll_page(page)
}

/// The artist page's item shelves, as (title, items) slices of one list built
/// by `artist_items`: albums, singles, playlists, related artists.
fn shelves(items: &[YTItem]) -> Vec<(&'static str, &[YTItem])> {
    let mut out = Vec::new();
    let mut start = 0;
    for (title, is) in [
        ("Albums", is_album as fn(&YTItem) -> bool),
        ("Singles & EPs", is_single),
        ("Playlists", |i| matches!(i, YTItem::Playlist(_))),
        ("Fans also like", |i| matches!(i, YTItem::Artist(_))),
    ] {
        let len = items[start..].iter().take_while(|item| is(item)).count();
        if len > 0 {
            out.push((title, &items[start..start + len]));
        }
        start += len;
    }
    out
}

fn is_album(item: &YTItem) -> bool {
    matches!(item, YTItem::Album(album) if !album.browse_id.is_empty() && album.playlist_id.as_deref() != Some("single"))
}

fn is_single(item: &YTItem) -> bool {
    matches!(item, YTItem::Album(album) if album.playlist_id.as_deref() == Some("single"))
}

/// The artist's shelves flattened in `shelves` order. Singles are marked with
/// a `playlist_id` of "single" so the two album shelves stay apart.
pub fn artist_items(artist: &ArtistDetailInfo) -> Vec<YTItem> {
    let mut items: Vec<YTItem> = artist.albums.iter().cloned().map(YTItem::Album).collect();
    items.extend(artist.singles.iter().cloned().map(|mut single| {
        single.playlist_id = Some("single".to_owned());
        YTItem::Album(single)
    }));
    items.extend(artist.playlists.iter().cloned().map(YTItem::Playlist));
    items.extend(artist.related_artists.iter().cloned().map(YTItem::Artist));
    items
}

fn tracks<'a>(
    covers: &'a Covers,
    songs: &Arc<Vec<SongItem>>,
    show_cover: bool,
    playing: Option<&str>,
) -> Element<'a, Message> {
    // ponytail: builds every row; a 5,000-song playlist should move to
    // virtual_list once the header can scroll with it.
    column((0..songs.len()).map(|index| {
        let is_playing = playing == Some(songs[index].video_id.as_str());
        let subtitle = ui::artists(&songs[index].artists);
        ui::track_row(covers, songs, index, show_cover, is_playing, subtitle, None)
    }))
    .spacing(2)
    .into()
}

/// Swift `ArtistDetailView.subscriberLabel`: "63.5M" becomes "63.5M subscribers".
fn subscriber_label(count: &str) -> String {
    if count.is_empty() || count.to_lowercase().contains("subscriber") {
        count.to_owned()
    } else {
        format!("{count} subscribers")
    }
}

fn count(n: i64, noun: &str) -> String {
    match n {
        0 => String::new(),
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
    }
}

/// Library demo: 5,000 song rows in a virtualized list (real library in R5).
/// Library while signed out: Google sign-in, and the cookie paste under it.
pub fn sign_in<'a>(
    signing_in: bool,
    cookie_header: &'a str,
    error: Option<&'a str>,
) -> Element<'a, Message> {
    let mut prompt = column![
        text("Sign in to see your library")
            .size(28)
            .font(theme::weight(Weight::Bold)),
        text(
            "Your playlists, albums, artists and liked songs come from your YouTube Music account."
        )
        .size(15)
        .color(theme::SECONDARY)
        .width(Length::Fill)
        .align_x(Alignment::Center),
        Space::new().height(12),
        ui::accent_button(
            if signing_in {
                "Finish in the Google window"
            } else {
                "Sign in with Google"
            },
            None,
            (!signing_in).then_some(Message::SignIn),
        ),
    ]
    .spacing(8)
    .align_x(Alignment::Center)
    .max_width(520);
    if let Some(error) = error {
        prompt = prompt.push(text(error).size(13).color(theme::CLOSE));
    }
    let paste = row![
        text_input("Cookie header", cookie_header)
            .secure(true)
            .on_input(Message::CookieInput)
            .on_submit(Message::ImportCookies)
            .size(14)
            .padding([10, 14])
            .style(ui::field_style),
        ui::pill_button("Use cookies", Message::ImportCookies),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    ui::centered(
        prompt
            .push(Space::new().height(28))
            .push(
                text("Or paste the Cookie header of a signed-in music.youtube.com tab.")
                    .size(13)
                    .color(theme::SECONDARY),
            )
            .push(paste),
    )
}

pub fn library(state: &virtual_list::State) -> Element<'_, Message> {
    const ROW_HEIGHT: f32 = 56.0;
    virtual_list::view(
        state,
        "library",
        5000,
        ROW_HEIGHT,
        ui::CONTENT_PADDING,
        |i| demo_song_row(i, ROW_HEIGHT),
        Message::LibraryScrolled,
    )
}

fn demo_song_row<'a>(index: usize, height: f32) -> Element<'a, Message> {
    let seconds = 150 + (index * 37) % 180;
    let (from, to) = theme::PALETTES[index % theme::PALETTES.len()];
    let gradient = Linear::new(Radians(0.8))
        .add_stop(0.0, from)
        .add_stop(1.0, to);
    let swatch = container(Space::new())
        .width(40)
        .height(40)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(gradient))),
            border: Border::default().rounded(6),
            ..container::Style::default()
        });
    let content = row![
        swatch,
        column![
            text(format!("Song {}", index + 1))
                .size(14)
                .font(theme::weight(Weight::Medium)),
            text("Demo row: your library arrives with sign-in")
                .size(13)
                .color(theme::SECONDARY),
        ]
        .spacing(2)
        .width(Length::Fill),
        text(ui::clock(seconds as f64))
            .size(13)
            .color(theme::SECONDARY),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .height(height);
    ui::clickable(container(content).padding([0, 8]), None, 0.0)
}
