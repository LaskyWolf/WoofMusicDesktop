//! Now Playing (the big cover over its colours, the lyrics beside it) and the queue panel.

use std::f32::consts::PI;
use std::sync::Arc;

use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{
    Column, Space, button, column, container, image, responsive, row, scrollable, text,
};
use iced::{
    Alignment, Background, Border, Color, Element, Gradient, Length, Padding, Radians, Shadow,
    Vector,
};
use trop::networking::models::yt_item::SongItem;
use trop::utilities::ui_image_color_extraction::{FALLBACK_COLORS, Rgb, darkened};

use crate::services::{Artwork, Lyrics};
use crate::ui::{self, Covers};
use crate::{Load, Message, Route, theme, title_bar, virtual_list};

pub const QUEUE_WIDTH: f32 = 360.0;
pub const QUEUE_SCROLL: &str = "queue";
/// Text width of a queue row: the panel minus padding, cover, gaps and the length.
const QUEUE_TEXT: f32 = QUEUE_WIDTH - 140.0;

/// Room the title, artists and album take under the cover.
const INFO_HEIGHT: f32 = 120.0;
const MAX_COVER: f32 = 640.0;
const COVER_RADIUS: f32 = 18.0;
/// Below this content width the lyrics take the whole stage, without the cover.
const SPLIT_MIN: f32 = 760.0;
/// The current lyric line sits two fifths down the lyrics column.
const LINES_ABOVE: u16 = 2;
const LINES_BELOW: u16 = 3;

/// Swift `FullPlayerView` background: the cover's two colours, darkened until
/// white text reads on them, top to bottom, with a glow of the first one at
/// the top. `None` while the artwork loads: Swift's fallback pair.
pub fn background<'a>(artwork: Option<&Artwork>) -> Element<'a, Message> {
    let [top, bottom] = artwork.map_or(FALLBACK_COLORS, |artwork| artwork.colors);
    let (top, bottom) = (readable(top), readable(bottom));
    let base = Linear::new(Radians(PI))
        .add_stop(0.0, top)
        .add_stop(1.0, bottom);
    // Swift blurs a circle of the first colour at 45% above centre; a fading
    // band does the same job without a blur pass.
    let glow = Linear::new(Radians(PI))
        .add_stop(0.0, Color { a: 0.45, ..top })
        .add_stop(0.6, Color::TRANSPARENT);
    let fill = |gradient: Linear| {
        container(Space::new())
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Gradient(Gradient::Linear(gradient))),
                ..container::Style::default()
            })
    };
    iced::widget::stack![fill(base), fill(glow)].into()
}

fn readable(color: Rgb) -> Color {
    let [r, g, b] = darkened(color, 0.18);
    Color::from_rgb(r as f32, g as f32, b as f32)
}

/// The cover as large as the content area allows, title and links under it;
/// with `lyrics` (and the playback time), the cover moves left and the lyrics
/// fill the right.
pub fn view<'a>(
    covers: &'a Covers,
    artwork: Option<&'a Artwork>,
    song: &'a SongItem,
    queue_open: bool,
    lyrics: Option<(&'a Load<Arc<Lyrics>>, f64)>,
) -> Element<'a, Message> {
    let content = responsive(move |size| match lyrics {
        None => container(stage(covers, artwork, song, size.width, size.height))
            .center(Length::Fill)
            .into(),
        Some((lyrics, time)) if size.width < SPLIT_MIN => lyric_lines(lyrics, time),
        Some((lyrics, time)) => {
            let width = (size.width - SPLIT_GAP) * 0.42;
            row![
                container(stage(covers, artwork, song, width, size.height))
                    .width(width)
                    .center_y(Length::Fill),
                container(lyric_lines(lyrics, time)).width(Length::Fill),
            ]
            .spacing(SPLIT_GAP)
            .into()
        }
    });
    container(content)
        .padding(Padding {
            top: title_bar::HEIGHT + 8.0,
            // An open queue panel takes the right side: the cover centres in what is left.
            right: if queue_open {
                QUEUE_WIDTH + theme::INSET + theme::CONTENT_RIGHT
            } else {
                theme::CONTENT_RIGHT
            },
            bottom: theme::PLAYER_HEIGHT + 2.0 * theme::INSET + 24.0,
            left: theme::CONTENT_LEFT,
        })
        .into()
}

const SPLIT_GAP: f32 = 56.0;

/// The cover, as large as `width` × `height` allows, with the title and links under it.
fn stage<'a>(
    covers: &'a Covers,
    artwork: Option<&'a Artwork>,
    song: &'a SongItem,
    width: f32,
    height: f32,
) -> Element<'a, Message> {
    let side = (height - INFO_HEIGHT).min(width).clamp(160.0, MAX_COVER);
    let cover: Element<'a, Message> = match artwork {
        Some(artwork) => image(artwork.image.clone())
            .width(side)
            .height(side)
            .border_radius(COVER_RADIUS)
            .into(),
        None => ui::cover(covers, song.thumbnail_url.as_deref(), side, COVER_RADIUS),
    };
    let cover = container(cover).style(|_| container::Style {
        border: Border::default().rounded(COVER_RADIUS),
        shadow: Shadow {
            color: Color {
                a: 0.45,
                ..Color::BLACK
            },
            offset: Vector::new(0.0, 18.0),
            blur_radius: 48.0,
        },
        background: Some(Background::Color(Color::BLACK)),
        ..container::Style::default()
    });
    let mut info = column![
        text(ui::fit(&song.title, side.max(width.min(420.0)), 28.0))
            .size(28)
            .font(theme::weight(Weight::Bold))
            .wrapping(text::Wrapping::None),
        artist_links(song),
    ]
    .spacing(4)
    .align_x(Alignment::Center);
    if let Some(album) = song.album.as_deref().filter(|album| !album.is_empty()) {
        info = info.push(link(album, 14.0, song.album_id.clone().map(Route::Album)));
    }
    column![cover, info]
        .spacing(28)
        .align_x(Alignment::Center)
        .into()
}

/// Swift `LyricsView` on the player's colours: synced lines keep the current
/// one two fifths down, the others fading with their distance from it.
fn lyric_lines(lyrics: &Load<Arc<Lyrics>>, time: f64) -> Element<'_, Message> {
    let lyrics = match lyrics {
        Load::Loading => return note(note_text("Looking for lyrics…")),
        Load::Failed(_) => {
            return note(
                column![
                    note_text("Lyrics didn't load"),
                    ui::pill_button("Try again", Message::RetryLyrics),
                ]
                .spacing(16)
                .into(),
            );
        }
        Load::Loaded(lyrics) if lyrics.lines.is_empty() => {
            return note(note_text("No lyrics for this song"));
        }
        Load::Loaded(lyrics) => lyrics,
    };
    let credit = lyrics.provider.as_deref().map(|provider| {
        container(
            text(format!("Lyrics from {provider}"))
                .size(12)
                .color(theme::SECONDARY),
        )
        .padding([8, 12])
    });
    let hidden = scrollable::Direction::Vertical(scrollable::Scrollbar::hidden());
    let lines = |range: std::ops::Range<usize>, distance: &dyn Fn(usize) -> usize| {
        Column::with_children(range.map(|index| lyric_line(lyrics, index, distance(index))))
    };
    if !lyrics.synced() {
        // Plain text: every line readable, scrolled by hand.
        let all = lines(0..lyrics.lines.len(), &|_| 1).extend(credit.map(Into::into));
        return scrollable(all)
            .direction(hidden)
            .height(Length::Fill)
            .into();
    }
    // `None` before the first line starts: every line is still ahead.
    let current = lyrics
        .lines
        .iter()
        .rposition(|line| line.start_time.is_some_and(|start| start <= time));
    let distance = |index: usize| current.map_or(index + 1, |current| current.abs_diff(index));
    let ahead = current.map_or(0, |current| current + 1);
    // Anchored to the bottom, the lines just sung stay next to the current one;
    // the wheel scrolls back to the start of the song.
    let sung = scrollable(lines(0..current.unwrap_or(0), &distance))
        .direction(hidden)
        .anchor_bottom()
        .height(Length::Shrink);
    let now: Element<'_, Message> = match current {
        Some(current) => lyric_line(lyrics, current, 0),
        None => Space::new().into(),
    };
    column![
        container(sung)
            .height(Length::FillPortion(LINES_ABOVE))
            .align_bottom(Length::FillPortion(LINES_ABOVE)),
        now,
        container(lines(ahead..lyrics.lines.len(), &distance).extend(credit.map(Into::into)))
            .height(Length::FillPortion(LINES_BELOW))
            .clip(true),
    ]
    .into()
}

/// One line, white at `distance` 0 and fading further away (Swift
/// `opacityForDistance` / `scaleForDistance`); a timed one seeks there on click.
fn lyric_line(lyrics: &Lyrics, index: usize, distance: usize) -> Element<'_, Message> {
    let (alpha, size) = match distance {
        0 => (1.0, 34.0),
        1 => (0.7, 33.0),
        2 => (0.55, 32.0),
        _ => (0.4, 32.0),
    };
    let white = |a: f32| Color { a, ..Color::WHITE };
    let line = &lyrics.lines[index];
    let mut body = column![
        text(line.text.as_str())
            .size(size)
            .font(theme::weight(Weight::Bold))
            .line_height(1.2)
            .color(white(alpha))
    ]
    .spacing(2);
    if let Some(romanized) = &lyrics.romanized[index] {
        body = body.push(
            text(romanized.as_str())
                .size(size * 0.55)
                .color(white(alpha * 0.75)),
        );
    }
    let timed = line.start_time.is_some();
    button(body)
        .width(Length::Fill)
        .padding([8, 12])
        .on_press_maybe(line.start_time.map(Message::SeekTo))
        .style(move |_, status| button::Style {
            background: (timed && status == button::Status::Hovered)
                .then_some(Background::Color(theme::HOVER)),
            border: Border::default().rounded(14),
            ..button::Style::default()
        })
        .into()
}

fn note_text(label: &str) -> Element<'_, Message> {
    text(label)
        .size(22)
        .font(theme::weight(Weight::Bold))
        .color(theme::SECONDARY)
        .into()
}

/// A state of the lyrics column, where the current line would be.
fn note(content: Element<'_, Message>) -> Element<'_, Message> {
    column![
        Space::new().height(Length::FillPortion(LINES_ABOVE)),
        container(content).padding([8, 12]),
        Space::new().height(Length::FillPortion(LINES_BELOW)),
    ]
    .into()
}

/// "Artist A, Artist B", each one opening its page when YouTube sent its id.
fn artist_links(song: &SongItem) -> Element<'_, Message> {
    let mut links = row![].align_y(Alignment::Center);
    for (index, artist) in song.artists.iter().enumerate() {
        if index > 0 {
            links = links.push(text(", ").size(17).color(theme::SECONDARY));
        }
        links = links.push(link(
            &artist.name,
            17.0,
            artist.id.clone().map(Route::Artist),
        ));
    }
    links.into()
}

/// Secondary text that turns white and opens `route` on click.
fn link(label: &str, size: f32, route: Option<Route>) -> Element<'_, Message> {
    let has_route = route.is_some();
    button(text(label).size(size))
        .padding(0)
        .on_press_maybe(route.map(Message::Navigate))
        .style(move |_, status| button::Style {
            text_color: if has_route && status == button::Status::Hovered {
                theme::TEXT
            } else {
                theme::SECONDARY
            },
            ..button::Style::default()
        })
        .into()
}

/// The queue at the right edge, over the page: every song, the current one in teal.
pub fn queue<'a>(
    covers: &'a Covers,
    state: &virtual_list::State,
    songs: &Arc<Vec<SongItem>>,
    current: usize,
) -> Element<'a, Message> {
    let header = row![
        text("Queue").size(18).font(theme::weight(Weight::Bold)),
        Space::new().width(Length::Fill),
        text(match songs.len() {
            1 => "1 song".to_owned(),
            n => format!("{n} songs"),
        })
        .size(13)
        .color(theme::SECONDARY),
    ]
    .align_y(Alignment::Center)
    .padding([16, 18]);
    let list = {
        let songs = songs.clone();
        virtual_list::view(
            state,
            QUEUE_SCROLL,
            songs.len(),
            ui::TRACK_ROW,
            Padding::from([0, 8]).bottom(8),
            move |index| {
                let subtitle = ui::artists(&songs[index].artists);
                ui::track_row(
                    covers,
                    &songs,
                    index,
                    true,
                    index == current,
                    subtitle,
                    Some(QUEUE_TEXT),
                )
            },
            Message::QueueScrolled,
        )
    };
    container(ui::glass_panel(
        3,
        22.0,
        Length::Fixed(QUEUE_WIDTH),
        Length::Fill,
        column![header, list],
    ))
    .padding(Padding {
        top: title_bar::HEIGHT,
        right: theme::INSET,
        bottom: theme::PLAYER_HEIGHT + 2.0 * theme::INSET,
        left: 0.0,
    })
    .align_right(Length::Fill)
    .into()
}
