//! Widgets shared by the pages: covers, tiles, song rows, buttons, text helpers.

use std::collections::HashMap;
use std::sync::Arc;

use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{
    Space, button, column, container, image, row, scrollable, stack, svg, text, text_input,
};
use iced::{
    Alignment, Background, Border, Color, ContentFit, Element, Gradient, Length, Padding, Radians,
};
use trop::networking::models::yt_item::{EpisodeItem, SongItem, YTArtist, YTItem};

use crate::{Message, Route, glass, theme, title_bar};

/// Cover sizes, logical px. Covers are fetched at twice the size for HiDPI.
pub const TILE: f32 = 168.0;
pub const ROW_COVER: f32 = 48.0;
/// Width of the text next to a row cover in a song cell.
pub const SONG_TEXT: f32 = 236.0;

/// Downloaded covers by their original URL.
pub type Covers = HashMap<String, image::Handle>;

/// Space around the scrolling content: below the title strip, left of the
/// sidebar, above the player bar.
pub const CONTENT_PADDING: Padding = Padding {
    top: title_bar::HEIGHT + 8.0,
    right: theme::CONTENT_RIGHT,
    bottom: theme::PLAYER_HEIGHT + 2.0 * theme::INSET + 32.0,
    left: theme::CONTENT_LEFT,
};

/// Id of the page scrollable: navigation saves and restores its offset.
pub const PAGE_SCROLL: &str = "page";

/// A page's scrolling column of sections.
pub fn scroll_page<'a>(sections: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    scrollable(column(sections).spacing(40).padding(CONTENT_PADDING))
        .id(PAGE_SCROLL)
        .on_scroll(|viewport| Message::PageScrolled(viewport.absolute_offset().y))
        .direction(theme::scrollbar())
        .style(theme::scroll_style)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// A titled section: tiles, or compact song cells when every item is a song.
/// The song whose id is `playing` shows its title in teal.
pub fn section<'a>(
    covers: &'a Covers,
    title: &'a str,
    items: &'a [YTItem],
    limit: usize,
    playing: Option<&str>,
) -> Element<'a, Message> {
    let songs = songs_of(items);
    let items = items.iter().take(limit);
    let songs_only = items.clone().all(|item| matches!(item, YTItem::Song(_)));
    let is_playing = |item: &YTItem| playing.is_some() && item.video_id() == playing;
    let body: Element<'a, Message> = if songs_only {
        iced::widget::Row::with_children(
            items.map(|item| song_cell(covers, item, &songs, is_playing(item))),
        )
        .spacing(12)
        .wrap()
        .vertical_spacing(4)
        .into()
    } else {
        iced::widget::Row::with_children(
            items.map(|item| tile(covers, item, &songs, is_playing(item))),
        )
        .spacing(20)
        .wrap()
        .vertical_spacing(24)
        .into()
    };
    column![section_title(title), body].spacing(14).into()
}

pub fn section_title(title: &str) -> iced::widget::Text<'_> {
    text(title).size(22).font(theme::weight(Weight::Bold))
}

/// The songs among `items`, in order: the queue a click in this section plays.
pub fn songs_of(items: &[YTItem]) -> Arc<Vec<SongItem>> {
    Arc::new(
        items
            .iter()
            .filter_map(|item| match item {
                YTItem::Song(song) => Some(song.clone()),
                YTItem::Episode(episode) => Some(episode_song(episode)),
                _ => None,
            })
            .collect(),
    )
}

/// A podcast episode as a queue entry.
pub fn episode_song(episode: &EpisodeItem) -> SongItem {
    SongItem {
        video_id: episode.video_id.clone(),
        title: episode.title.clone(),
        artists: episode.artists.clone(),
        duration: episode.duration,
        thumbnail_url: episode.thumbnail_url.clone(),
        ..SongItem::default()
    }
}

/// What a click on `item` does: play it within `songs`, or open its page.
pub fn open(item: &YTItem, songs: &Arc<Vec<SongItem>>) -> Option<Message> {
    match item {
        YTItem::Song(_) | YTItem::Episode(_) => {
            let id = item.video_id()?;
            let index = songs.iter().position(|song| song.video_id == id)?;
            Some(Message::PlayQueue(songs.clone(), index))
        }
        YTItem::Album(album) => Some(Message::Navigate(Route::Album(album.browse_id.clone()))),
        YTItem::Artist(artist) => Some(Message::Navigate(Route::Artist(artist.browse_id.clone()))),
        YTItem::Playlist(playlist) => Some(Message::Navigate(Route::Playlist(playlist.id.clone()))),
        YTItem::Podcast(podcast) => {
            Some(Message::Navigate(Route::Podcast(podcast.browse_id.clone())))
        }
    }
}

/// Album, playlist, artist… tile: cover with title and subtitle below.
pub fn tile<'a>(
    covers: &'a Covers,
    item: &'a YTItem,
    songs: &Arc<Vec<SongItem>>,
    playing: bool,
) -> Element<'a, Message> {
    let round = matches!(item, YTItem::Artist(_));
    let content = column![
        cover(
            covers,
            item.thumbnail_url(),
            TILE,
            if round { TILE / 2.0 } else { 10.0 }
        ),
        text(fit(item.title(), TILE, 14.0))
            .size(14)
            .font(theme::weight(Weight::Medium))
            .color(title_color(playing))
            .wrapping(text::Wrapping::None),
        text(fit(&subtitle(item), TILE, 13.0))
            .size(13)
            .color(theme::SECONDARY)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(4)
    .width(TILE)
    .clip(true);
    clickable(content, open(item, songs), 6.0)
}

/// Song in a songs-only section (Quick picks): a compact cell.
pub fn song_cell<'a>(
    covers: &'a Covers,
    item: &'a YTItem,
    songs: &Arc<Vec<SongItem>>,
    playing: bool,
) -> Element<'a, Message> {
    let content = row![
        cover(covers, item.thumbnail_url(), ROW_COVER, 6.0),
        column![
            text(fit(item.title(), SONG_TEXT, 14.0))
                .size(14)
                .font(theme::weight(Weight::Medium))
                .color(title_color(playing))
                .wrapping(text::Wrapping::None),
            text(fit(&subtitle(item), SONG_TEXT, 13.0))
                .size(13)
                .color(theme::SECONDARY)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(2)
        .clip(true),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .width(300);
    clickable(content, open(item, songs), 8.0)
}

/// Teal for the playing song's title, white otherwise.
fn title_color(playing: bool) -> Color {
    if playing { theme::ACCENT } else { theme::TEXT }
}

/// Height of a `track_row`.
pub const TRACK_ROW: f32 = 52.0;

/// Full-width track row of an album, playlist or the queue: number or cover,
/// title, `subtitle` (usually the artists), length. `playing` highlights the
/// current song. `text_width` shortens the text to fit a narrow column.
pub fn track_row<'a>(
    covers: &'a Covers,
    songs: &Arc<Vec<SongItem>>,
    index: usize,
    show_cover: bool,
    playing: bool,
    subtitle: String,
    text_width: Option<f32>,
) -> Element<'a, Message> {
    let song = &songs[index];
    // In a narrow column the text ends with an ellipsis instead of a cut glyph.
    let (title, subtitle) = match text_width {
        Some(width) => (fit(&song.title, width, 14.0), fit(&subtitle, width, 13.0)),
        None => (song.title.clone(), subtitle),
    };
    let lead: Element<'a, Message> = if show_cover {
        cover(covers, song.thumbnail_url.as_deref(), 40.0, 6.0)
    } else {
        container(text((index + 1).to_string()).size(14).color(if playing {
            theme::ACCENT
        } else {
            theme::SECONDARY
        }))
        .center_x(28)
        .into()
    };

    let content = row![
        lead,
        column![
            text(title)
                .size(14)
                .font(theme::weight(Weight::Medium))
                .color(title_color(playing))
                .wrapping(text::Wrapping::None),
            text(subtitle)
                .size(13)
                .color(theme::SECONDARY)
                .wrapping(text::Wrapping::None),
        ]
        .spacing(2)
        .width(Length::Fill)
        .clip(true),
        text(if song.duration > 0 {
            clock(song.duration as f64)
        } else {
            String::new()
        })
        .size(13)
        .color(theme::SECONDARY),
    ]
    .spacing(14)
    .align_y(Alignment::Center)
    .height(TRACK_ROW);
    clickable(
        container(content).padding([0, 8]),
        Some(Message::PlayQueue(songs.clone(), index)),
        0.0,
    )
}

/// Big header of a detail page: cover, title, subtitle lines, play button.
pub fn header<'a>(
    covers: &'a Covers,
    thumbnail: Option<&str>,
    round: bool,
    title: &'a str,
    lines: Vec<String>,
    play: Option<Message>,
) -> Element<'a, Message> {
    const SIZE: f32 = 220.0;
    let mut info = column![text(title).size(34).font(theme::weight(Weight::Bold))].spacing(6);
    for line in lines.into_iter().filter(|line| !line.is_empty()) {
        info = info.push(text(line).size(14).color(theme::SECONDARY));
    }
    if let Some(play) = play {
        info = info.push(Space::new().height(10)).push(accent_button(
            "Play",
            Some(crate::icons::PLAY),
            Some(play),
        ));
    }
    row![
        cover(
            covers,
            thumbnail,
            SIZE,
            if round { SIZE / 2.0 } else { 14.0 }
        ),
        info,
    ]
    .spacing(28)
    .align_y(Alignment::End)
    .into()
}

/// The cover once downloaded, a dark square until then.
pub fn cover<'a>(
    covers: &'a Covers,
    url: Option<&str>,
    size: f32,
    radius: f32,
) -> Element<'a, Message> {
    match url.and_then(|url| covers.get(url)) {
        Some(handle) => image(handle.clone())
            .width(size)
            .height(size)
            .content_fit(ContentFit::Cover)
            .border_radius(radius)
            .into(),
        None => container(Space::new())
            .width(size)
            .height(size)
            .style(move |_| container::Style {
                background: Some(Background::Color(theme::PLACEHOLDER)),
                border: Border::default().rounded(radius),
                ..container::Style::default()
            })
            .into(),
    }
}

pub fn subtitle(item: &YTItem) -> String {
    match item {
        YTItem::Song(song) => artists(&song.artists),
        YTItem::Album(album) => {
            let names = artists(&album.artists);
            match album.year {
                Some(year) if names.is_empty() => year.to_string(),
                Some(year) => format!("{names} • {year}"),
                None => names,
            }
        }
        YTItem::Artist(_) => "Artist".to_owned(),
        YTItem::Playlist(playlist) => playlist
            .author
            .clone()
            .unwrap_or_else(|| "Playlist".to_owned()),
        YTItem::Podcast(_) => "Podcast".to_owned(),
        YTItem::Episode(episode) => artists(&episode.artists),
    }
}

pub fn artists(artists: &[YTArtist]) -> String {
    artists
        .iter()
        .map(|artist| artist.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// "3 h 12 min" or "42 min" for a total length in seconds.
pub fn total_length(seconds: i64) -> String {
    let minutes = seconds / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => String::new(),
        (0, m) => format!("{m} min"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// Shortens `text` with an ellipsis so it fits `width` at font `size`.
// ponytail: estimates Inter's average glyph width (0.55 em) instead of
// measuring; wide glyphs (W, CJK) can still clip. Measure with the renderer's
// paragraph if that shows.
pub fn fit(text: &str, width: f32, size: f32) -> String {
    let max = (width / (size * 0.55)).floor() as usize;
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// "3:51", or "1:57:00" from an hour on.
pub fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

pub fn centered<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(CONTENT_PADDING)
        .center(Length::Fill)
        .into()
}

/// "Loading…" or an error with a retry button, for a page that has no data yet.
pub fn status<'a>(error: Option<&'a str>, retry: Message) -> Element<'a, Message> {
    match error {
        None => centered(text("Loading…").color(theme::SECONDARY)),
        Some(error) => centered(
            column![
                text("This page could not load")
                    .size(18)
                    .font(theme::weight(Weight::Bold)),
                text(error).size(13).color(theme::SECONDARY),
                pill_button("Try again", retry),
            ]
            .spacing(10)
            .align_x(Alignment::Center),
        ),
    }
}

pub fn icon<'a>(bytes: &'static [u8], size: f32) -> svg::Svg<'a> {
    svg(svg::Handle::from_memory(bytes))
        .width(size)
        .height(size)
        .style(|_, _| svg::Style {
            color: Some(theme::TEXT),
        })
}

/// A clickable area with the hover and press wash; inert when `message` is `None`.
pub fn clickable<'a>(
    content: impl Into<Element<'a, Message>>,
    message: Option<Message>,
    pad: f32,
) -> Element<'a, Message> {
    button(content)
        .padding(pad)
        .on_press_maybe(message)
        .style(|_, status| button::Style {
            background: match status {
                button::Status::Hovered => Some(Background::Color(theme::HOVER)),
                button::Status::Pressed => Some(Background::Color(theme::PRESSED)),
                _ => None,
            },
            border: Border::default().rounded(12),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
        .into()
}

pub fn pill_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(14).font(theme::weight(Weight::Medium)))
        .padding([8, 16])
        .on_press(message)
        .style(|_, status| button::Style {
            background: Some(Background::Color(if status == button::Status::Hovered {
                theme::PRESSED
            } else {
                theme::HOVER
            })),
            border: Border::default().rounded(18),
            text_color: theme::TEXT,
            ..button::Style::default()
        })
        .into()
}

/// Teal button with dark text: white on teal is too faint to read.
pub fn accent_button<'a>(
    label: &'a str,
    icon_bytes: Option<&'static [u8]>,
    message: Option<Message>,
) -> Element<'a, Message> {
    let mut content = row![].spacing(8).align_y(Alignment::Center);
    if let Some(bytes) = icon_bytes {
        content = content.push(icon(bytes, 16.0).style(|_, _| svg::Style {
            color: Some(theme::BACKGROUND),
        }));
    }
    button(content.push(text(label).size(15).font(theme::weight(Weight::Bold))))
        .padding([10, 22])
        .on_press_maybe(message)
        .style(|_, status| button::Style {
            background: Some(Background::Color(match status {
                button::Status::Hovered => theme::ACCENT_HOVER,
                button::Status::Disabled => Color {
                    a: 0.4,
                    ..theme::ACCENT
                },
                _ => theme::ACCENT,
            })),
            border: Border::default().rounded(22),
            text_color: theme::BACKGROUND,
            ..button::Style::default()
        })
        .into()
}

/// A text field on the dark glass: teal outline while focused.
pub fn field_style(_: &iced::Theme, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(theme::HOVER),
        border: Border::default().rounded(14).width(1).color(match status {
            text_input::Status::Focused { .. } => Color {
                a: 0.6,
                ..theme::ACCENT
            },
            _ => theme::HOVER,
        }),
        icon: theme::SECONDARY,
        placeholder: theme::SECONDARY,
        value: theme::TEXT,
        selection: Color {
            a: 0.4,
            ..theme::ACCENT
        },
    }
}

/// A glass panel: the glass under `content`, both filling `width` × `height`.
pub fn glass_panel<'a>(
    id: u64,
    radius: f32,
    width: Length,
    height: Length,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    stack![glass::glass(id, radius, theme::GLASS_TINT), content.into()]
        .width(width)
        .height(height)
        .into()
}

/// The fade under the top strip while a page is scrolled (iOS 26 scroll edge).
pub fn scroll_edge<'a>() -> Element<'a, Message> {
    let fade = Linear::new(Radians(std::f32::consts::PI))
        .add_stop(
            0.0,
            Color {
                a: 0.9,
                ..theme::BACKGROUND
            },
        )
        .add_stop(
            0.55,
            Color {
                a: 0.6,
                ..theme::BACKGROUND
            },
        )
        .add_stop(1.0, Color::TRANSPARENT);
    container(
        container(Space::new())
            .width(Length::Fill)
            .height(title_bar::HEIGHT + 24.0)
            .style(move |_| container::Style {
                background: Some(Background::Gradient(Gradient::Linear(fade))),
                ..container::Style::default()
            }),
    )
    .padding(Padding {
        left: theme::INSET + theme::SIDEBAR_WIDTH,
        ..Padding::ZERO
    })
    .into()
}

/// A faint wash behind everything.
pub fn wash<'a>() -> Element<'a, Message> {
    let (from, to) = theme::PALETTES[4];
    let gradient = Linear::new(Radians(2.4))
        .add_stop(0.0, Color { a: 0.30, ..from })
        .add_stop(0.55, Color { a: 0.08, ..to })
        .add_stop(1.0, Color::TRANSPARENT);
    container(Space::new())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(gradient))),
            ..container::Style::default()
        })
        .into()
}
