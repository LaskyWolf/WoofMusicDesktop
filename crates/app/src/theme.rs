//! Colours and sizes, taken from the WoofMusic identity (same values as the Flutter build).

use iced::font::{Family, Weight};
use iced::{Color, Font};

pub const BACKGROUND: Color = Color::BLACK;
pub const TEXT: Color = Color::WHITE;
pub const SECONDARY: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.70);
pub const HOVER: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.08);
pub const PRESSED: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.12);
pub const CLOSE: Color = rgb(0xE5, 0x48, 0x4D);
pub const CLOSE_PRESSED: Color = rgb(0xC4, 0x36, 0x3B);
/// Dark tint over the glass, so white text on it keeps its contrast.
pub const GLASS_TINT: Color = Color::from_rgba(0.08, 0.08, 0.09, 0.45);

pub const INSET: f32 = 12.0;
pub const SIDEBAR_WIDTH: f32 = 232.0;
pub const CONTENT_LEFT: f32 = INSET + SIDEBAR_WIDTH + 28.0;
pub const CONTENT_RIGHT: f32 = 32.0;
pub const PLAYER_HEIGHT: f32 = 72.0;

pub const FONT: Font = Font {
    family: Family::Name("Inter Variable"),
    ..Font::DEFAULT
};
pub const fn weight(weight: Weight) -> Font {
    Font { weight, ..FONT }
}

/// Placeholder cover gradients until real covers load.
pub const PALETTES: [(Color, Color); 7] = [
    (rgb(0x5B, 0x3F, 0xC4), rgb(0xE0, 0x60, 0x9A)),
    (rgb(0xB3, 0x32, 0x4A), rgb(0xF5, 0x8A, 0x5C)),
    (rgb(0x5E, 0x5F, 0x6E), rgb(0xC9, 0xCB, 0xDA)),
    (rgb(0xE2, 0x70, 0x3A), rgb(0xF7, 0xC3, 0x5E)),
    (rgb(0x2F, 0x5F, 0xC9), rgb(0x7F, 0xD3, 0xF2)),
    (rgb(0x1F, 0x7A, 0x6A), rgb(0x7B, 0xE0, 0xA8)),
    (rgb(0x6F, 0xA4, 0x3A), rgb(0xD7, 0xE8, 0x6A)),
];

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb8(r, g, b)
}
