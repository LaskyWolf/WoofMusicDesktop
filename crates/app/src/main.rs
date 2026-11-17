//! WoofMusic Desktop. R1 spike: own title bar, resize edges and Liquid Glass
//! over scrolling content (see docs/MILESTONES.md).

mod glass;
mod theme;
mod title_bar;

use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{Space, column, container, row, scrollable, stack, svg, text};
use iced::{
    Background, Border, Color, Element, Gradient, Length, Padding, Radians, Size, Subscription,
    Task, window,
};

const APP_ID: &str = "com.laskywolf.woofmusic";

pub fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("WoofMusic")
        .subscription(App::subscription)
        .font(include_bytes!("../assets/InterVariable.ttf").as_slice())
        .default_font(theme::FONT)
        .style(|_, _| iced::theme::Style {
            background_color: theme::BACKGROUND,
            text_color: theme::TEXT,
        })
        .window(window::Settings {
            size: Size::new(1280.0, 800.0),
            min_size: Some(Size::new(880.0, 560.0)),
            decorations: false,
            #[cfg(target_os = "linux")]
            platform_specific: window::settings::PlatformSpecific {
                application_id: APP_ID.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}

#[derive(Default)]
struct App {
    maximized: bool,
}

#[derive(Debug, Clone)]
enum Message {
    TitleBar(title_bar::Message),
    Resized,
    Maximized(bool),
    Snapshot(window::Screenshot),
}

impl App {
    fn new() -> (Self, Task<Message>) {
        // Dev aid: WOOF_SNAPSHOT=/path.png saves the window contents after
        // WOOF_SNAPSHOT_DELAY ms (default 1500) and quits.
        let snapshot = match std::env::var("WOOF_SNAPSHOT") {
            Ok(_) => {
                let delay = std::env::var("WOOF_SNAPSHOT_DELAY")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1500);
                Task::future(tokio::time::sleep(std::time::Duration::from_millis(delay)))
                    .then(|_| window::latest())
                    .and_then(window::screenshot)
                    .map(Message::Snapshot)
            }
            Err(_) => Task::none(),
        };
        (Self::default(), snapshot)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TitleBar(message) => title_bar::update(message),
            Message::Resized => window::latest()
                .and_then(window::is_maximized)
                .map(Message::Maximized),
            Message::Maximized(maximized) => {
                self.maximized = maximized;
                Task::none()
            }
            Message::Snapshot(shot) => {
                let path = std::env::var("WOOF_SNAPSHOT").expect("set when the snapshot runs");
                if let Err(error) = save_png(&path, &shot) {
                    eprintln!("WOOF snapshot failed: {error}");
                }
                window::latest().and_then(window::close)
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        window::resize_events().map(|_| Message::Resized)
    }

    fn view(&self) -> Element<'_, Message> {
        let mut layers = stack![
            wash(),
            page(),
            sidebar(),
            player_bar(),
            top_strip(self.maximized),
        ]
        .width(Length::Fill)
        .height(Length::Fill);
        if !self.maximized {
            layers = layers.push(title_bar::resize_edges().map(Message::TitleBar));
        }
        layers.into()
    }
}

/// A faint wash of the playing cover's colours behind everything.
fn wash<'a>() -> Element<'a, Message> {
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

/// Scrolling content: sections of placeholder covers.
fn page<'a>() -> Element<'a, Message> {
    let sections = [
        "Listen again",
        "Quick picks",
        "New releases",
        "Mixed for you",
    ]
    .into_iter()
    .enumerate()
    .map(|(section, title)| {
        let covers = (0..10)
            .map(|i| cover_tile(section * 10 + i))
            .collect::<Vec<_>>();
        column![
            text(title).size(22).font(theme::weight(Weight::Bold)),
            iced::widget::Row::with_children(covers)
                .spacing(20)
                .wrap()
                .vertical_spacing(24),
        ]
        .spacing(14)
        .into()
    })
    .collect::<Vec<Element<'a, Message>>>();
    scrollable(column(sections).spacing(40).padding(Padding {
        top: title_bar::HEIGHT + 8.0,
        right: theme::CONTENT_RIGHT,
        bottom: theme::PLAYER_HEIGHT + 2.0 * theme::INSET + 32.0,
        left: theme::CONTENT_LEFT,
    }))
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn cover_tile<'a>(index: usize) -> Element<'a, Message> {
    column![
        cover(index, 168.0, 10.0),
        text(format!("Album {}", index + 1))
            .size(14)
            .font(theme::weight(Weight::Medium)),
        text("Artist").size(13).color(theme::SECONDARY),
    ]
    .spacing(4)
    .width(168)
    .into()
}

/// Gradient square standing in for a cover image.
fn cover<'a>(index: usize, size: f32, radius: f32) -> Element<'a, Message> {
    let (from, to) = theme::PALETTES[index % theme::PALETTES.len()];
    let angle = if index.is_multiple_of(2) { 0.8 } else { 2.3 };
    let gradient = Linear::new(Radians(angle))
        .add_stop(0.0, from)
        .add_stop(1.0, to);
    container(Space::new())
        .width(size)
        .height(size)
        .style(move |_| container::Style {
            background: Some(Background::Gradient(Gradient::Linear(gradient))),
            border: Border::default().rounded(radius),
            ..container::Style::default()
        })
        .into()
}

/// A glass panel: the glass under `content`, both filling `width` × `height`.
fn glass_panel<'a>(
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

fn sidebar<'a>() -> Element<'a, Message> {
    let item = |label: &'a str, selected: bool| {
        container(
            text(label)
                .size(15)
                .font(theme::weight(Weight::Medium))
                .color(if selected {
                    theme::TEXT
                } else {
                    theme::SECONDARY
                }),
        )
        .padding([8, 12])
        .width(Length::Fill)
        .style(move |_| container::Style {
            background: selected.then_some(Background::Color(theme::PRESSED)),
            border: Border::default().rounded(10),
            ..container::Style::default()
        })
    };
    let brand = row![
        svg(svg::Handle::from_memory(
            include_bytes!("../assets/woofmusic.svg").as_slice()
        ))
        .width(22)
        .height(22),
        text("WoofMusic").size(15).font(theme::weight(Weight::Bold)),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);
    let content = column![
        title_bar::drag_area(
            container(brand)
                .padding([0, 6])
                .center_y(title_bar::HEIGHT - 8.0)
        )
        .map(Message::TitleBar),
        item("Home", true),
        item("Explore", false),
        item("Library", false),
        item("Search", false),
    ]
    .spacing(2)
    .padding(10);
    container(glass_panel(
        1,
        22.0,
        Length::Fixed(theme::SIDEBAR_WIDTH),
        Length::Fill,
        content,
    ))
    .padding(theme::INSET)
    .into()
}

fn player_bar<'a>() -> Element<'a, Message> {
    let now_playing = row![
        cover(4, 48.0, 8.0),
        column![
            text("Song title")
                .size(14)
                .font(theme::weight(Weight::Medium)),
            text("Artist").size(13).color(theme::SECONDARY),
        ]
        .spacing(2),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center);
    let icon = |bytes: &'static [u8], size: f32| {
        svg(svg::Handle::from_memory(bytes))
            .width(size)
            .height(size)
            .style(|_, _| svg::Style {
                color: Some(theme::TEXT),
            })
    };
    let transport = row![
        icon(icons::PREVIOUS, 22.0),
        icon(icons::PLAY, 30.0),
        icon(icons::NEXT, 22.0),
    ]
    .spacing(24)
    .align_y(iced::Alignment::Center);
    let content = row![
        container(now_playing).width(Length::FillPortion(1)),
        container(transport).center_x(Length::FillPortion(1)),
        Space::new().width(Length::FillPortion(1)),
    ]
    .padding([0, 14])
    .height(Length::Fill)
    .align_y(iced::Alignment::Center);
    container(glass_panel(
        2,
        24.0,
        Length::Fill,
        Length::Fixed(theme::PLAYER_HEIGHT),
        content,
    ))
    .padding(Padding {
        top: 0.0,
        right: theme::INSET,
        bottom: theme::INSET,
        left: theme::INSET + theme::SIDEBAR_WIDTH + theme::INSET,
    })
    .align_bottom(Length::Fill)
    .into()
}

/// The top strip right of the sidebar: moves the window, holds the window buttons.
fn top_strip<'a>(maximized: bool) -> Element<'a, Message> {
    Element::from(
        row![
            Space::new().width(theme::INSET + theme::SIDEBAR_WIDTH),
            title_bar::drag_area(Space::new().width(Length::Fill).height(title_bar::HEIGHT)),
            container(title_bar::buttons(maximized)).center_y(title_bar::HEIGHT),
            Space::new().width(10),
        ]
        .height(title_bar::HEIGHT),
    )
    .map(Message::TitleBar)
}

fn save_png(path: &str, shot: &window::Screenshot) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, shot.size.width, shot.size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&shot.rgba)?;
    Ok(())
}

/// Transport glyphs, filled, coloured by the bar.
mod icons {
    pub const PLAY: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M7 4.8v14.4a1 1 0 0 0 1.5.86l11.6-7.2a1 1 0 0 0 0-1.72L8.5 3.94A1 1 0 0 0 7 4.8z" fill="#fff"/></svg>"##;
    pub const NEXT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M3 6.2v11.6a.8.8 0 0 0 1.2.7l8.6-5.8a.8.8 0 0 0 0-1.4L4.2 5.5a.8.8 0 0 0-1.2.7zM12 6.2v11.6a.8.8 0 0 0 1.2.7l8.6-5.8a.8.8 0 0 0 0-1.4l-8.6-5.8a.8.8 0 0 0-1.2.7z" fill="#fff"/></svg>"##;
    pub const PREVIOUS: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M21 6.2v11.6a.8.8 0 0 1-1.2.7l-8.6-5.8a.8.8 0 0 1 0-1.4l8.6-5.8a.8.8 0 0 1 1.2.7zM12 6.2v11.6a.8.8 0 0 1-1.2.7l-8.6-5.8a.8.8 0 0 1 0-1.4l8.6-5.8a.8.8 0 0 1 1.2.7z" fill="#fff"/></svg>"##;
}
