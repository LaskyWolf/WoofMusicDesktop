//! Now Playing (the big cover over its colours) and the queue panel.

use std::f32::consts::PI;
use std::sync::Arc;

use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{Space, button, column, container, image, responsive, row, text};
use iced::{
    Alignment, Background, Border, Color, Element, Gradient, Length, Padding, Radians, Shadow,
    Vector,
};
use trop::networking::models::yt_item::SongItem;
use trop::utilities::ui_image_color_extraction::{FALLBACK_COLORS, Rgb, darkened};

use crate::services::Artwork;
use crate::ui::{self, Covers};
use crate::{Message, Route, theme, title_bar, virtual_list};

pub const QUEUE_WIDTH: f32 = 360.0;
pub const QUEUE_SCROLL: &str = "queue";
/// Text width of a queue row: the panel minus padding, cover, gaps and the length.
const QUEUE_TEXT: f32 = QUEUE_WIDTH - 140.0;

/// Room the title, artists and album take under the cover.
const INFO_HEIGHT: f32 = 120.0;
const MAX_COVER: f32 = 640.0;
const COVER_RADIUS: f32 = 18.0;

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

/// The cover as large as the content area allows, title and links under it.
pub fn view<'a>(
    covers: &'a Covers,
    artwork: Option<&'a Artwork>,
    song: &'a SongItem,
    queue_open: bool,
) -> Element<'a, Message> {
    let content = responsive(move |size| {
        let side = (size.height - INFO_HEIGHT)
            .min(size.width)
            .clamp(160.0, MAX_COVER);
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
            text(ui::fit(&song.title, side.max(420.0), 28.0))
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
        container(column![cover, info].spacing(28).align_x(Alignment::Center))
            .center(Length::Fill)
            .into()
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
