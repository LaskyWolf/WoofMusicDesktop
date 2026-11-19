//! The window frame drawn by the app: drag area, window buttons, resize edges.
//! The system title bar is off (`decorations: false`) on every platform.

use iced::widget::{Space, button, container, mouse_area, row, svg};
use iced::window::Direction;
use iced::{Background, Border, Element, Length, Task, mouse, window};

use crate::theme;

#[derive(Debug, Clone, Copy)]
pub enum Message {
    Drag,
    ToggleMaximize,
    Minimize,
    Close,
    Resize(Direction),
}

pub fn update<T: Send + 'static>(message: Message) -> Task<T> {
    let latest = window::latest();
    match message {
        Message::Drag => latest.and_then(window::drag),
        Message::ToggleMaximize => latest.and_then(window::toggle_maximize),
        Message::Minimize => latest.and_then(|id| window::minimize(id, true)),
        Message::Close => latest.and_then(window::close),
        Message::Resize(direction) => latest.and_then(move |id| window::drag_resize(id, direction)),
    }
}

/// Height of the strip at the top of the window that moves it.
pub const HEIGHT: f32 = 48.0;

/// An empty area that moves the window when dragged and maximizes it on double click.
pub fn drag_area<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    mouse_area(content)
        .on_press(Message::Drag)
        .on_double_click(Message::ToggleMaximize)
        .into()
}

/// Minimize, maximize/restore and close.
pub fn buttons<'a>(maximized: bool) -> Element<'a, Message> {
    row![
        window_button(icons::MINIMIZE, Message::Minimize, false),
        window_button(
            if maximized {
                icons::RESTORE
            } else {
                icons::MAXIMIZE
            },
            Message::ToggleMaximize,
            false,
        ),
        window_button(icons::CLOSE, Message::Close, true),
    ]
    .spacing(4)
    .into()
}

fn window_button<'a>(icon: &'static [u8], message: Message, close: bool) -> Element<'a, Message> {
    let icon = svg(svg::Handle::from_memory(icon))
        .width(16)
        .height(16)
        .style(|_, _| svg::Style {
            color: Some(theme::TEXT),
        });
    button(container(icon).center(Length::Fill))
        .width(36)
        .height(28)
        .padding(0)
        .on_press(message)
        .style(move |_, status| {
            let background = match status {
                button::Status::Hovered if close => Some(theme::CLOSE),
                button::Status::Pressed if close => Some(theme::CLOSE_PRESSED),
                button::Status::Hovered => Some(theme::HOVER),
                button::Status::Pressed => Some(theme::PRESSED),
                _ => None,
            };
            button::Style {
                background: background.map(Background::Color),
                border: Border::default().rounded(8),
                text_color: theme::TEXT,
                ..button::Style::default()
            }
        })
        .into()
}

/// Thin areas along the window edges that resize it. Not shown when maximized.
pub fn resize_edges<'a>() -> Element<'a, Message> {
    const EDGE: f32 = 6.0;
    let area = |direction, interaction, width: Length, height: Length| -> Element<'a, Message> {
        mouse_area(Space::new().width(width).height(height))
            .on_press(Message::Resize(direction))
            .interaction(interaction)
            .into()
    };
    let fixed = Length::Fixed(EDGE);
    iced::widget::column![
        row![
            area(
                Direction::NorthWest,
                mouse::Interaction::ResizingDiagonallyDown,
                fixed,
                fixed
            ),
            area(
                Direction::North,
                mouse::Interaction::ResizingVertically,
                Length::Fill,
                fixed
            ),
            area(
                Direction::NorthEast,
                mouse::Interaction::ResizingDiagonallyUp,
                fixed,
                fixed
            ),
        ],
        row![
            area(
                Direction::West,
                mouse::Interaction::ResizingHorizontally,
                fixed,
                Length::Fill
            ),
            Space::new().width(Length::Fill).height(Length::Fill),
            area(
                Direction::East,
                mouse::Interaction::ResizingHorizontally,
                fixed,
                Length::Fill
            ),
        ],
        row![
            area(
                Direction::SouthWest,
                mouse::Interaction::ResizingDiagonallyUp,
                fixed,
                fixed
            ),
            area(
                Direction::South,
                mouse::Interaction::ResizingVertically,
                Length::Fill,
                fixed
            ),
            area(
                Direction::SouthEast,
                mouse::Interaction::ResizingDiagonallyDown,
                fixed,
                fixed
            ),
        ],
    ]
    .into()
}

/// Window button glyphs: 16×16, 1.5 px strokes, coloured by the button.
pub(crate) mod icons {
    pub const MINIMIZE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M3.5 8h9" stroke="#fff" stroke-width="1.5" stroke-linecap="round"/></svg>"##;
    pub const MAXIMIZE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="3.75" y="3.75" width="8.5" height="8.5" rx="2" fill="none" stroke="#fff" stroke-width="1.5"/></svg>"##;
    pub const RESTORE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect x="3" y="5.5" width="7.5" height="7.5" rx="1.75" fill="none" stroke="#fff" stroke-width="1.5"/><path d="M6 3h5.25A1.75 1.75 0 0 1 13 4.75V10" fill="none" stroke="#fff" stroke-width="1.5" stroke-linecap="round"/></svg>"##;
    pub const CLOSE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M4.25 4.25l7.5 7.5M11.75 4.25l-7.5 7.5" stroke="#fff" stroke-width="1.5" stroke-linecap="round"/></svg>"##;
}
