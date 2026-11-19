//! `Trop/Audio/Lyrics/ShareCardStudio.swift`: the 9:16 share card (a
//! background, a framed cover, the picked lyric lines, stickers, the time and
//! a QR code) and the studio that styles it and saves it as a PNG. Lyric
//! cards and the song menu's Share Card both open it.

use std::f32::consts::{PI, TAU};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::advanced::widget::Id;
use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::canvas::{self, Frame, LineDash, Path, Stroke};
use iced::widget::{Space, button, column, container, image, mouse_area, pin, row, stack, text};
use iced::{
    Alignment, Background, Border, Color, Element, Font, Gradient, Length, Point, Radians,
    Rectangle, Shadow, Size, Subscription, Task, Vector, mouse, window,
};
use serde::{Deserialize, Serialize};
use trop::audio::now_playing::PlayerDefaults;
use trop::utilities::ui_image_color_extraction::Rgb;

use crate::{Message, theme, ui};

pub const WIDTH: f32 = 360.0;
pub const HEIGHT: f32 = 640.0;
const CARD_ID: &str = "share-card";
/// One loop of the moving parts (Swift `ShareCardVideo.seconds`).
const LOOP_SECONDS: f32 = 4.0;
const PAPER: Color = Color::from_rgb(1.0, 0.98, 0.94);
const INK: Color = Color::from_rgb(0.11, 0.08, 0.13);
const STICKERS: [&str; 10] = ["🐾", "✨", "🎧", "🌙", "💖", "🔥", "🐶", "🐺", "⭐", "🎤"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Backdrop {
    #[default]
    Glow,
    Equalizer,
    Paws,
    Halftone,
    Confetti,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FrameKind {
    #[default]
    Polaroid,
    Ticket,
    Cassette,
    Vinyl,
    Cd,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextKind {
    #[default]
    Paper,
    Marker,
    Typewriter,
    Chat,
}

const BACKDROPS: [(Backdrop, &str); 5] = [
    (Backdrop::Glow, "Cover Glow"),
    (Backdrop::Equalizer, "Equalizer"),
    (Backdrop::Paws, "Paws"),
    (Backdrop::Halftone, "Halftone"),
    (Backdrop::Confetti, "Confetti"),
];
const FRAMES: [(FrameKind, &str); 5] = [
    (FrameKind::Polaroid, "Polaroid"),
    (FrameKind::Ticket, "Ticket"),
    (FrameKind::Cassette, "Cassette"),
    (FrameKind::Vinyl, "Vinyl"),
    (FrameKind::Cd, "CD"),
];
const TEXTS: [(TextKind, &str); 4] = [
    (TextKind::Paper, "Paper"),
    (TextKind::Marker, "Marker"),
    (TextKind::Typewriter, "Typewriter"),
    (TextKind::Chat, "Chat"),
];

/// The last look, kept for the next card (Swift's `shareCard.*` keys).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Style {
    pub background: Backdrop,
    pub frame: FrameKind,
    pub text: TextKind,
    pub palette: usize,
    pub shows_timestamp: bool,
    pub shows_code: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            background: Backdrop::Glow,
            frame: FrameKind::Polaroid,
            text: TextKind::Paper,
            palette: 0,
            shows_timestamp: true,
            shows_code: false,
        }
    }
}

impl Style {
    fn path() -> Option<PathBuf> {
        PlayerDefaults::default_path().map(|path| path.with_file_name("share_card.json"))
    }

    fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(self) -> Task<Message> {
        let Some(path) = Self::path() else {
            return Task::none();
        };
        Task::future(tokio::task::spawn_blocking(move || {
            if let Ok(json) = serde_json::to_vec(&self)
                && let Err(error) = std::fs::write(&path, json)
            {
                log::warn!("Share card look not saved: {error}");
            }
        }))
        .discard()
    }
}

/// What a card shows, whatever its look.
#[derive(Clone, Debug)]
pub struct Content {
    pub lines: Vec<String>,
    pub title: String,
    pub artist: String,
    pub cover: Option<image::Handle>,
    pub colors: [Rgb; 2],
    /// Where the first picked line starts, in seconds; `None` for a song card.
    pub start_time: Option<f64>,
    pub video_id: Option<String>,
}

impl Content {
    /// The song link; with `at_line`, it starts playback at the first picked line.
    fn link(&self, at_line: bool) -> Option<String> {
        let id = self.video_id.as_ref()?;
        let mut address = format!("https://music.youtube.com/watch?v={id}");
        if at_line && let Some(start) = self.start_time.filter(|s| *s >= 1.0) {
            address += &format!("&t={}", start as i64);
        }
        Some(address)
    }

    /// Three colours from the cover, rotated so each palette leads with a different one.
    fn palette(&self, index: usize) -> [Color; 3] {
        let rgb = |[r, g, b]: Rgb| Color::from_rgb(r as f32, g as f32, b as f32);
        let (a, b) = (rgb(self.colors[0]), rgb(self.colors[1]));
        let three = [a, b, mix(b, Color::WHITE, 0.35)];
        let shift = index % 3;
        [three[shift], three[(shift + 1) % 3], three[(shift + 2) % 3]]
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgb(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
    )
}

fn alpha(color: Color, a: f32) -> Color {
    Color { a, ..color }
}

/// An emoji on the card, placed by the user.
#[derive(Clone, Debug)]
struct Sticker {
    emoji: &'static str,
    /// Centre, as a fraction of the card's width and height.
    x: f32,
    y: f32,
    scale: f32,
}

struct Drag {
    index: usize,
    moved: bool,
}

pub struct Studio {
    content: Content,
    style: Style,
    stickers: Vec<Sticker>,
    drag: Option<Drag>,
    started: Instant,
    phase: f32,
    code: Option<image::Handle>,
    saving: bool,
    saved: Option<Result<PathBuf, String>>,
    copied: bool,
    /// Back returns to the lyric picker.
    pub from_lyrics: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    Background(Backdrop),
    Frame(FrameKind),
    Text(TextKind),
    Palette(usize),
    Timestamp(bool),
    Code(bool),
    AddSticker(&'static str),
    ClearStickers,
    /// The button went down on this sticker.
    Grab(usize),
    /// The cursor moved over the card, in card points.
    Move(Point),
    Release,
    /// The wheel turned over this sticker.
    Resize(usize, f32),
    Shuffle,
    Tick,
    CopyLink,
    Save,
    Shot(window::Screenshot, Option<Rectangle>),
    Saved(Result<PathBuf, String>),
    /// Back to the lyric picker, or closes a song card.
    Back,
    Done,
}

/// A value in 0..1 that changes with each call (Swift's `.random`).
fn random() -> f32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    noise(nanos as usize, 7)
}

impl Studio {
    pub fn new(content: Content, from_lyrics: bool) -> Self {
        let mut studio = Self {
            content,
            style: Style::load(),
            stickers: Vec::new(),
            drag: None,
            started: Instant::now(),
            phase: 0.3,
            code: None,
            saving: false,
            saved: None,
            copied: false,
            from_lyrics,
        };
        studio.refresh_code();
        studio
    }

    fn refresh_code(&mut self) {
        self.code = self
            .style
            .shows_code
            .then(|| self.content.link(self.style.shows_timestamp))
            .flatten()
            .and_then(|link| qr_code(&link));
    }

    /// `Back` and `Done` are the app's: they close the studio.
    pub fn update(&mut self, message: Msg) -> Task<Message> {
        let restyled = match message {
            Msg::Background(kind) => {
                self.style.background = kind;
                true
            }
            Msg::Frame(kind) => {
                self.style.frame = kind;
                true
            }
            Msg::Text(kind) => {
                self.style.text = kind;
                true
            }
            Msg::Palette(index) => {
                self.style.palette = index;
                true
            }
            Msg::Timestamp(on) => {
                self.style.shows_timestamp = on;
                self.refresh_code();
                true
            }
            Msg::Code(on) => {
                self.style.shows_code = on;
                self.refresh_code();
                true
            }
            Msg::Shuffle => {
                let pick = |n: usize, salt: usize| {
                    (noise(random().to_bits() as usize, salt) * n as f32) as usize % n
                };
                self.style.background = BACKDROPS[pick(5, 1)].0;
                self.style.frame = FRAMES[pick(5, 2)].0;
                self.style.text = TEXTS[pick(4, 3)].0;
                self.style.palette = pick(3, 4);
                true
            }
            Msg::AddSticker(emoji) => {
                self.stickers.push(Sticker {
                    emoji,
                    x: 0.25 + 0.5 * random(),
                    y: 0.3 + 0.4 * noise(random().to_bits() as usize, 9),
                    scale: 1.0,
                });
                false
            }
            Msg::ClearStickers => {
                self.stickers.clear();
                false
            }
            Msg::Grab(index) => {
                self.drag = Some(Drag {
                    index,
                    moved: false,
                });
                false
            }
            Msg::Move(point) => {
                if let Some(drag) = &mut self.drag
                    && let Some(sticker) = self.stickers.get_mut(drag.index)
                {
                    drag.moved = true;
                    sticker.x = (point.x / WIDTH).clamp(0.05, 0.95);
                    sticker.y = (point.y / HEIGHT).clamp(0.05, 0.95);
                }
                false
            }
            // Swift: a tap removes a sticker; a drag moves it.
            Msg::Release => {
                if let Some(drag) = self.drag.take()
                    && !drag.moved
                    && drag.index < self.stickers.len()
                {
                    self.stickers.remove(drag.index);
                }
                false
            }
            Msg::Resize(index, delta) => {
                if let Some(sticker) = self.stickers.get_mut(index) {
                    sticker.scale = (sticker.scale * (1.0 + delta * 0.1)).clamp(0.5, 3.0);
                }
                false
            }
            Msg::Tick => {
                let seconds = self.started.elapsed().as_secs_f32();
                self.phase = (seconds % LOOP_SECONDS) / LOOP_SECONDS;
                false
            }
            Msg::CopyLink => {
                self.copied = true;
                return match self.content.link(self.style.shows_timestamp) {
                    Some(link) => iced::clipboard::write(link),
                    None => Task::none(),
                };
            }
            Msg::Save => {
                self.saving = true;
                self.saved = None;
                // What is on screen is what is saved: a screenshot cut to the card.
                return window::latest().and_then(window::screenshot).then(|shot| {
                    iced::advanced::widget::operate(crate::lyrics::find_bounds(Id::new(CARD_ID)))
                        .map(Some)
                        .chain(Task::done(None))
                        .collect()
                        .map(move |found| {
                            Message::Studio(Msg::Shot(
                                shot.clone(),
                                found.into_iter().flatten().next(),
                            ))
                        })
                });
            }
            Msg::Shot(shot, bounds) => {
                let name = format!("{} – {}", self.content.title, self.content.artist);
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            crate::lyrics::save_card(&shot, bounds, &name)
                        })
                        .await
                        .map_err(|error| error.to_string())?
                    },
                    |result| Message::Studio(Msg::Saved(result)),
                );
            }
            Msg::Saved(result) => {
                self.saving = false;
                self.saved = Some(result);
                false
            }
            Msg::Back | Msg::Done => false,
        };
        if restyled {
            self.saved = None;
            return self.style.save();
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut list = Vec::new();
        // ponytail: 30 fps while the studio is open; Swift pauses with Reduce Motion.
        if !self.saving {
            list.push(
                iced::time::every(Duration::from_millis(33)).map(|_| Message::Studio(Msg::Tick)),
            );
        }
        if self.drag.is_some() {
            list.push(iced::event::listen_with(|event, _, _| match event {
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                    Some(Message::Studio(Msg::Release))
                }
                _ => None,
            }));
        }
        Subscription::batch(list)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let card = mouse_area(card(
            &self.content,
            self.style,
            &self.stickers,
            self.phase,
            self.code.as_ref(),
            true,
        ))
        .on_move(|point| Message::Studio(Msg::Move(point)));
        let card = container(card).style(|_| container::Style {
            border: Border::default().rounded(20),
            shadow: Shadow {
                color: alpha(Color::BLACK, 0.3),
                offset: Vector::new(0.0, 8.0),
                blur_radius: 16.0,
            },
            ..container::Style::default()
        });
        let side = self.tray();
        let content = row![card, side].spacing(28).padding(24);
        ui::glass_panel(
            11,
            24.0,
            Length::Fixed(WIDTH + TRAY + 28.0 + 48.0),
            Length::Fixed(HEIGHT + 48.0),
            content,
        )
    }

    fn tray(&self) -> Element<'_, Message> {
        let style = self.style;
        let chips = |options: Vec<(String, bool, Msg)>| {
            iced::widget::Row::with_children(
                options
                    .into_iter()
                    .map(|(label, on, msg)| chip(label, on, Message::Studio(msg))),
            )
            .spacing(6)
            .wrap()
            .vertical_spacing(6)
        };
        let mut tray = column![
            row![
                text("Style")
                    .size(22)
                    .font(theme::weight(Weight::Bold))
                    .width(Length::Fill),
                chip("Shuffle".into(), false, Message::Studio(Msg::Shuffle)),
            ]
            .align_y(Alignment::Center),
            text(ui::fit(&self.content.title, TRAY, 13.0))
                .size(13)
                .color(theme::SECONDARY),
            group("Background"),
            chips(
                BACKDROPS
                    .iter()
                    .map(|&(kind, label)| (
                        label.into(),
                        style.background == kind,
                        Msg::Background(kind)
                    ))
                    .collect()
            ),
            group("Frame"),
            chips(
                FRAMES
                    .iter()
                    .map(|&(kind, label)| (label.into(), style.frame == kind, Msg::Frame(kind)))
                    .collect()
            ),
        ]
        .spacing(8)
        .width(Length::Fixed(TRAY))
        .height(Length::Fixed(HEIGHT));
        if !self.content.lines.is_empty() {
            tray = tray.push(group("Text")).push(chips(
                TEXTS
                    .iter()
                    .map(|&(kind, label)| (label.into(), style.text == kind, Msg::Text(kind)))
                    .collect(),
            ));
        }
        let swatches = row((0..3).map(|index| {
            let [a, b, _] = self.content.palette(index);
            let selected = style.palette == index;
            button(Space::new().width(30).height(30))
                .padding(0)
                .on_press(Message::Studio(Msg::Palette(index)))
                .style(move |_, _| button::Style {
                    background: Some(Background::Gradient(Gradient::Linear(
                        Linear::new(Radians(2.4)).add_stop(0.0, b).add_stop(1.0, a),
                    ))),
                    border: Border::default()
                        .rounded(15)
                        .width(if selected { 3 } else { 1 })
                        .color(if selected {
                            theme::TEXT
                        } else {
                            theme::PRESSED
                        }),
                    ..button::Style::default()
                })
                .into()
        }))
        .spacing(10);
        tray = tray.push(group("Colours")).push(swatches);
        let mut stickers = STICKERS
            .iter()
            .map(|&emoji| {
                button(container(text(emoji).size(18)).center(34))
                    .padding(0)
                    .on_press(Message::Studio(Msg::AddSticker(emoji)))
                    .style(round_button)
                    .into()
            })
            .collect::<Vec<Element<'_, Message>>>();
        if !self.stickers.is_empty() {
            stickers.insert(
                0,
                button(container(text("Clear").size(12).color(theme::CLOSE)).center_y(34))
                    .padding([0, 10])
                    .on_press(Message::Studio(Msg::ClearStickers))
                    .style(round_button)
                    .into(),
            );
        }
        tray = tray
            .push(group("Stickers"))
            .push(
                iced::widget::Row::with_children(stickers)
                    .spacing(4)
                    .wrap()
                    .vertical_spacing(4),
            )
            .push(
                text("Drag to move, scroll to resize, click to remove.")
                    .size(11)
                    .color(theme::SECONDARY),
            );
        let mut toggles = row![].spacing(6);
        if self.content.start_time.is_some() {
            toggles = toggles.push(chip(
                "Time".into(),
                style.shows_timestamp,
                Message::Studio(Msg::Timestamp(!style.shows_timestamp)),
            ));
        }
        if self.content.video_id.is_some() {
            toggles = toggles.push(chip(
                "QR Code".into(),
                style.shows_code,
                Message::Studio(Msg::Code(!style.shows_code)),
            ));
        }
        tray = tray.push(Space::new().height(4)).push(toggles);
        let status: Element<'_, Message> = match &self.saved {
            Some(Ok(path)) => text(format!("Saved to {}", path.display()))
                .size(12)
                .color(theme::SECONDARY)
                .into(),
            Some(Err(error)) => text(format!("The image was not saved: {error}"))
                .size(12)
                .color(theme::CLOSE)
                .into(),
            None => Space::new().into(),
        };
        let mut actions = row![ui::accent_button(
            if self.saving {
                "Saving…"
            } else {
                "Save Image"
            },
            None,
            (!self.saving).then_some(Message::Studio(Msg::Save)),
        )]
        .spacing(8);
        if self.content.video_id.is_some() {
            actions = actions.push(ui::pill_button(
                if self.copied {
                    "Link Copied"
                } else {
                    "Copy Link"
                },
                Message::Studio(Msg::CopyLink),
            ));
        }
        tray.push(Space::new().height(Length::Fill))
            .push(status)
            .push(actions)
            .push(
                row![
                    ui::pill_button(
                        if self.from_lyrics { "Back" } else { "Cancel" },
                        Message::Studio(Msg::Back)
                    ),
                    ui::pill_button("Done", Message::Studio(Msg::Done)),
                ]
                .spacing(8),
            )
            .into()
    }
}

const TRAY: f32 = 300.0;

fn group(title: &str) -> Element<'_, Message> {
    container(
        text(title.to_uppercase())
            .size(11)
            .font(theme::weight(Weight::Semibold))
            .color(theme::SECONDARY),
    )
    .padding(iced::Padding::ZERO.top(6))
    .into()
}

fn chip<'a>(label: String, selected: bool, message: Message) -> Element<'a, Message> {
    button(text(label).size(13).font(theme::weight(Weight::Semibold)))
        .padding([6, 12])
        .on_press(message)
        .style(move |_, status| button::Style {
            background: Some(Background::Color(match (selected, status) {
                (true, _) => theme::accent(),
                (false, button::Status::Hovered) => theme::PRESSED,
                (false, _) => theme::HOVER,
            })),
            border: Border::default().rounded(16),
            text_color: if selected {
                theme::BACKGROUND
            } else {
                theme::TEXT
            },
            ..button::Style::default()
        })
        .into()
}

fn round_button(_: &iced::Theme, status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(if status == button::Status::Hovered {
            theme::PRESSED
        } else {
            theme::HOVER
        })),
        border: Border::default().rounded(17),
        text_color: theme::TEXT,
        ..button::Style::default()
    }
}

/// A QR code of `link`, one pixel per module with a light edge; drawn without smoothing.
fn qr_code(link: &str) -> Option<image::Handle> {
    let code = qrcode::QrCode::with_error_correction_level(link, qrcode::EcLevel::M).ok()?;
    let width = code.width();
    let pixels = code
        .to_colors()
        .into_iter()
        .flat_map(|module| match module {
            qrcode::Color::Dark => [0, 0, 0, 255],
            qrcode::Color::Light => [255, 255, 255, 255],
        })
        .collect::<Vec<u8>>();
    Some(image::Handle::from_rgba(width as u32, width as u32, pixels))
}

// MARK: - Card

/// The 9:16 card. `phase` (0..1) is one loop of the moving parts.
fn card<'a>(
    content: &'a Content,
    style: Style,
    stickers: &'a [Sticker],
    phase: f32,
    code: Option<&'a image::Handle>,
    editing: bool,
) -> Element<'a, Message> {
    let colors = content.palette(style.palette);
    let mut layout = column![
        Space::new().height(52),
        container(frame_view(content, style.frame, colors, phase)).center_x(Length::Fill),
        Space::new().height(Length::Fill),
    ];
    if !content.lines.is_empty() {
        layout = layout
            .push(lines_view(&content.lines, style.text, colors))
            .push(Space::new().height(Length::Fill));
    }
    let timestamp = content
        .start_time
        .filter(|_| style.shows_timestamp)
        .map(|start| format!("▶ {}", ui::clock(start)));
    layout = layout.push(container(footer(timestamp, code)).padding(iced::Padding {
        left: 24.0,
        right: 24.0,
        bottom: 26.0,
        top: 16.0,
    }));
    let mut layers = stack![
        paint(WIDTH, HEIGHT, move |frame, size| {
            backdrop(frame, size, style.background, colors, phase);
        }),
        layout,
    ];
    for (index, sticker) in stickers.iter().enumerate() {
        let side = 52.0 * sticker.scale;
        let mut emoji: Element<'_, Message> = container(text(sticker.emoji).size(side))
            .center(side * 1.3)
            .into();
        if editing {
            emoji = mouse_area(emoji)
                .on_press(Message::Studio(Msg::Grab(index)))
                .on_scroll(move |delta| {
                    let y = match delta {
                        mouse::ScrollDelta::Lines { y, .. } => y,
                        mouse::ScrollDelta::Pixels { y, .. } => y / 40.0,
                    };
                    Message::Studio(Msg::Resize(index, y))
                })
                .interaction(mouse::Interaction::Grab)
                .into();
        }
        layers = layers.push(
            pin(emoji)
                .x(sticker.x * WIDTH - side * 0.65)
                .y(sticker.y * HEIGHT - side * 0.65),
        );
    }
    container(layers)
        .id(CARD_ID)
        .width(WIDTH)
        .height(HEIGHT)
        .clip(true)
        .into()
}

fn footer(timestamp: Option<String>, code: Option<&image::Handle>) -> Element<'_, Message> {
    let badge = container(
        text("🐾 WOOF MUSIC")
            .size(12)
            .font(theme::weight(Weight::ExtraBold))
            .color(INK),
    )
    .padding([6, 12])
    .style(|_| container::Style {
        background: Some(Background::Color(alpha(PAPER, 0.92))),
        border: Border::default().rounded(14),
        ..container::Style::default()
    });
    let mut credit = column![badge].spacing(6);
    if let Some(timestamp) = timestamp {
        credit = credit.push(
            text(timestamp)
                .size(13)
                .font(Font {
                    weight: Weight::Bold,
                    ..Font::MONOSPACE
                })
                .color(Color::WHITE),
        );
    }
    match code {
        Some(code) => row![
            credit.width(Length::Fill),
            container(
                image(code.clone())
                    .width(64)
                    .height(64)
                    .filter_method(image::FilterMethod::Nearest),
            )
            .padding(6)
            .style(|_| container::Style {
                background: Some(Background::Color(Color::WHITE)),
                border: Border::default().rounded(8),
                ..container::Style::default()
            }),
        ]
        .align_y(Alignment::End)
        .into(),
        None => container(credit.align_x(Alignment::Center))
            .center_x(Length::Fill)
            .into(),
    }
}

// MARK: - Frames

fn cover_image(
    content: &Content,
    colors: [Color; 3],
    side: f32,
    radius: f32,
) -> Element<'_, Message> {
    match &content.cover {
        Some(cover) => image(cover.clone())
            .width(side)
            .height(side)
            .content_fit(iced::ContentFit::Cover)
            .border_radius(radius)
            .into(),
        None => {
            let wash = Linear::new(Radians(2.4))
                .add_stop(0.0, colors[1])
                .add_stop(1.0, colors[0]);
            container(ui::icon(NOTE, side * 0.3))
                .center(side)
                .style(move |_| container::Style {
                    background: Some(Background::Gradient(Gradient::Linear(wash))),
                    border: Border::default().rounded(radius),
                    ..container::Style::default()
                })
                .into()
        }
    }
}

fn titles<'a>(
    content: &'a Content,
    size: f32,
    width: f32,
    color: Color,
    center: bool,
) -> Element<'a, Message> {
    let align = if center {
        Alignment::Center
    } else {
        Alignment::Start
    };
    column![
        text(ui::fit(&content.title, width, size))
            .size(size)
            .font(theme::weight(Weight::Bold))
            .color(color),
        text(ui::fit(&content.artist, width, size * 0.76))
            .size(size * 0.76)
            .font(theme::weight(Weight::Semibold))
            .color(alpha(color, 0.65)),
    ]
    .spacing(2)
    .align_x(align)
    .width(width)
    .into()
}

fn drop_shadow(blur: f32, y: f32, a: f32) -> Shadow {
    Shadow {
        color: alpha(Color::BLACK, a),
        offset: Vector::new(0.0, y),
        blur_radius: blur,
    }
}

// ponytail: frames and lines stand straight; Swift tilts some a few degrees,
// which iced widgets can't do.
fn frame_view<'a>(
    content: &'a Content,
    kind: FrameKind,
    colors: [Color; 3],
    phase: f32,
) -> Element<'a, Message> {
    match kind {
        // The cover as a taped photo with the title written under it.
        FrameKind::Polaroid => {
            let side = 184.0;
            let photo = container(
                column![
                    cover_image(content, colors, side, 0.0),
                    titles(content, 17.0, side, INK, true),
                ]
                .spacing(10),
            )
            .padding(iced::Padding {
                top: 11.0,
                left: 11.0,
                right: 11.0,
                bottom: 14.0,
            })
            .style(|_| container::Style {
                background: Some(Background::Color(Color::WHITE)),
                shadow: drop_shadow(10.0, 6.0, 0.3),
                ..container::Style::default()
            });
            let tape = container(Space::new().width(92).height(26)).style(|_| container::Style {
                background: Some(Background::Color(alpha(Color::WHITE, 0.55))),
                shadow: drop_shadow(1.0, 1.0, 0.1),
                ..container::Style::default()
            });
            stack![
                container(photo).padding(iced::Padding::ZERO.top(13)),
                container(tape).center_x(side + 22.0),
            ]
            .into()
        }
        // A concert ticket with punched notches on both sides.
        FrameKind::Ticket => {
            let (w, h) = (304.0, 142.0);
            let body = row![
                cover_image(content, colors, 110.0, 6.0),
                column![
                    text("ADMIT ONE · WOOF")
                        .size(10)
                        .font(Font {
                            weight: Weight::Bold,
                            ..Font::MONOSPACE
                        })
                        .color(alpha(INK, 0.55)),
                    titles(content, 19.0, 136.0, INK, false),
                ]
                .spacing(8),
            ]
            .spacing(14)
            .align_y(Alignment::Center)
            .padding([16, 22]);
            stack![
                paint(w, h, |frame, size| {
                    frame.fill(&ticket_shape(size), Color::from_rgb(1.0, 0.97, 0.9));
                }),
                container(body).width(w).height(h).center_y(h),
            ]
            .into()
        }
        // A tape with the title on its label and reels that turn.
        FrameKind::Cassette => {
            let label = container(
                row![
                    cover_image(content, colors, 58.0, 5.0),
                    column![
                        text("SIDE A")
                            .size(10)
                            .font(Font {
                                weight: Weight::ExtraBold,
                                ..Font::MONOSPACE
                            })
                            .color(alpha(INK, 0.55)),
                        titles(content, 16.0, 180.0, INK, false),
                    ]
                    .spacing(4),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .padding(10)
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(Background::Color(mix(colors[2], Color::WHITE, 0.55))),
                border: Border::default().rounded(8),
                ..container::Style::default()
            });
            let reels = container(paint(52.0 * 2.0 + 44.0, 52.0, move |frame, _| {
                for x in [26.0, 26.0 + 52.0 + 44.0] {
                    reel(frame, Point::new(x, 26.0), phase * TAU * 2.0);
                }
            }))
            .padding([8, 30])
            .style(|_| container::Style {
                background: Some(Background::Color(alpha(Color::BLACK, 0.35))),
                border: Border::default().rounded(40),
                ..container::Style::default()
            });
            container(column![label, reels].spacing(12).align_x(Alignment::Center))
                .padding(14)
                .width(300)
                .style(|_| container::Style {
                    background: Some(Background::Color(Color::from_rgb(0.13, 0.13, 0.15))),
                    border: Border::default().rounded(16),
                    shadow: drop_shadow(12.0, 8.0, 0.35),
                    ..container::Style::default()
                })
                .into()
        }
        // The record half out of its sleeve, turning once per loop.
        FrameKind::Vinyl => {
            let side = 200.0;
            let label = colors[1];
            let sleeve =
                container(cover_image(content, colors, side, 4.0)).style(|_| container::Style {
                    shadow: Shadow {
                        color: alpha(Color::BLACK, 0.4),
                        offset: Vector::new(4.0, 6.0),
                        blur_radius: 10.0,
                    },
                    ..container::Style::default()
                });
            column![
                stack![
                    container(paint(side, side, move |frame, size| {
                        record(frame, size, label, phase * TAU);
                    }))
                    .padding(iced::Padding::ZERO.left(70)),
                    sleeve,
                ],
                container(titles(content, 18.0, 280.0, Color::WHITE, true)).center_x(280),
            ]
            .spacing(14)
            .align_x(Alignment::Center)
            .into()
        }
        // The cover printed on a CD, turning once per loop.
        FrameKind::Cd => {
            let side = 250.0;
            // ponytail: the printed cover stands still and the sheen turns; iced's
            // image rotation turns the square, not the rounded disc.
            let disc: Element<'_, Message> = match &content.cover {
                Some(cover) => image(cover.clone())
                    .width(side)
                    .height(side)
                    .content_fit(iced::ContentFit::Cover)
                    .border_radius(side / 2.0)
                    .into(),
                None => container(Space::new())
                    .width(side)
                    .height(side)
                    .style(move |_| container::Style {
                        background: Some(Background::Gradient(Gradient::Linear(
                            Linear::new(Radians(2.4 + phase * TAU))
                                .add_stop(0.0, colors[1])
                                .add_stop(1.0, colors[0]),
                        ))),
                        border: Border::default().rounded(side / 2.0),
                        ..container::Style::default()
                    })
                    .into(),
            };
            column![
                container(stack![
                    disc,
                    paint(side, side, move |frame, size| cd_overlay(
                        frame,
                        size,
                        phase * TAU
                    )),
                ])
                .style(|_| container::Style {
                    border: Border::default().rounded(125),
                    shadow: drop_shadow(12.0, 8.0, 0.4),
                    ..container::Style::default()
                }),
                container(titles(content, 18.0, 280.0, Color::WHITE, true)).center_x(280),
            ]
            .spacing(16)
            .align_x(Alignment::Center)
            .into()
        }
    }
}

// MARK: - Lines

fn lines_view<'a>(lines: &'a [String], kind: TextKind, colors: [Color; 3]) -> Element<'a, Message> {
    let size = if lines.len() <= 2 { 21.0 } else { 17.0 };
    let rows = lines.iter().enumerate().map(|(index, line)| {
        let even = index % 2 == 0;
        let line = line.as_str();
        let row: Element<'a, Message> = match kind {
            TextKind::Paper => container(
                container(
                    text(line)
                        .size(size)
                        .font(theme::weight(Weight::Bold))
                        .color(INK),
                )
                .padding([8, 12])
                .style(|_| container::Style {
                    background: Some(Background::Color(PAPER)),
                    border: Border::default().rounded(6),
                    shadow: drop_shadow(0.0, 2.0, 0.18),
                    ..container::Style::default()
                }),
            )
            .padding(iced::Padding::ZERO.left(if even { 0.0 } else { 18.0 }))
            .into(),
            // A highlighter stroke behind the lower half of the words.
            TextKind::Marker => {
                let words = |color: Color| {
                    text(line)
                        .size(size + 3.0)
                        .font(Font {
                            weight: Weight::Black,
                            style: iced::font::Style::Italic,
                            ..theme::FONT
                        })
                        .color(color)
                };
                let band = colors[1];
                let highlight = container(
                    Space::new()
                        .width(Length::Fill)
                        .height((size * 0.55) as u32),
                )
                .style(move |_| container::Style {
                    background: Some(Background::Color(alpha(band, 0.85))),
                    ..container::Style::default()
                });
                // The words are the base layer; the stroke sits under them, at the bottom.
                stack![words(Color::WHITE)]
                    .push_under(
                        container(highlight)
                            .align_bottom(Length::Fill)
                            .padding(iced::Padding::ZERO.bottom(2)),
                    )
                    .push_under(
                        container(words(alpha(colors[0], 0.9))).padding(iced::Padding {
                            top: 2.0,
                            left: 2.0,
                            ..iced::Padding::ZERO
                        }),
                    )
                    .into()
            }
            // A typed strip with an accent bar on its left edge.
            TextKind::Typewriter => {
                let accent = colors[1];
                stack![
                    container(
                        text(line)
                            .size(size - 2.0)
                            .font(Font {
                                weight: Weight::Bold,
                                ..Font::MONOSPACE
                            })
                            .color(INK),
                    )
                    .padding([7, 12])
                    .width(Length::Fill)
                    .style(|_| container::Style {
                        background: Some(Background::Color(Color::from_rgb(0.96, 0.94, 0.89))),
                        ..container::Style::default()
                    }),
                    container(Space::new().width(4).height(Length::Fill)).style(move |_| {
                        container::Style {
                            background: Some(Background::Color(accent)),
                            ..container::Style::default()
                        }
                    }),
                ]
                .into()
            }
            // The lines answer each other, like messages.
            TextKind::Chat => {
                let fill = if even {
                    Color::WHITE
                } else {
                    mix(colors[2], Color::WHITE, 0.6)
                };
                let bubble = container(
                    text(line)
                        .size(size - 1.0)
                        .font(theme::weight(Weight::Semibold))
                        .color(INK),
                )
                .padding([9, 14])
                .max_width(260)
                .style(move |_| container::Style {
                    background: Some(Background::Color(fill)),
                    border: Border::default().rounded(iced::border::Radius {
                        top_left: 18.0,
                        top_right: 18.0,
                        bottom_left: if even { 4.0 } else { 18.0 },
                        bottom_right: if even { 18.0 } else { 4.0 },
                    }),
                    ..container::Style::default()
                });
                container(bubble)
                    .width(Length::Fill)
                    .align_x(if even {
                        Alignment::Start
                    } else {
                        Alignment::End
                    })
                    .into()
            }
        };
        row
    });
    container(column(rows).spacing(12).width(Length::Fill))
        .padding([0, 28])
        .into()
}

// MARK: - Drawing

/// A canvas that runs `draw` on a `width` × `height` frame.
fn paint<'a>(
    width: f32,
    height: f32,
    draw: impl Fn(&mut Frame, Size) + 'a,
) -> Element<'a, Message> {
    struct Painter<F>(F);
    impl<F: Fn(&mut Frame, Size)> canvas::Program<Message> for Painter<F> {
        type State = ();
        fn draw(
            &self,
            _: &(),
            renderer: &iced::Renderer,
            _: &iced::Theme,
            bounds: Rectangle,
            _: mouse::Cursor,
        ) -> Vec<canvas::Geometry> {
            let mut frame = Frame::new(renderer, bounds.size());
            (self.0)(&mut frame, bounds.size());
            vec![frame.into_geometry()]
        }
    }
    canvas(Painter(draw)).width(width).height(height).into()
}

use iced::widget::canvas::Canvas;
fn canvas<P: canvas::Program<Message>>(program: P) -> Canvas<P, Message> {
    Canvas::new(program)
}

/// A repeatable 0..1 value (Swift `ShareCardBackgroundView.noise`).
fn noise(index: usize, salt: usize) -> f32 {
    let value = ((index as f64) * 12.9898 + (salt as f64) * 78.233).sin() * 43_758.545_3;
    (value - value.floor()) as f32
}

/// A soft round light: rings of `color` that add up toward the centre.
// ponytail: stacked rings stand in for SwiftUI's RadialGradient, which iced's canvas lacks.
fn radial(frame: &mut Frame, center: Point, radius: f32, color: Color) {
    const RINGS: usize = 24;
    let each = 1.0 - (1.0 - color.a).powf(1.0 / RINGS as f32);
    for ring in 0..RINGS {
        let r = radius * (1.0 - ring as f32 / RINGS as f32);
        frame.fill(&Path::circle(center, r), alpha(color, each));
    }
}

fn backdrop(frame: &mut Frame, size: Size, kind: Backdrop, colors: [Color; 3], phase: f32) {
    let full = Path::rectangle(Point::ORIGIN, size);
    let (w, h) = (size.width, size.height);
    match kind {
        // Two glows circling in opposite directions and breathing twice per loop.
        Backdrop::Glow => {
            let angle = phase * TAU;
            let breath = 40.0 * (2.0 * angle).sin();
            frame.fill(&full, colors[0]);
            radial(
                frame,
                Point::new(
                    w * (0.3 + 0.18 * angle.cos()),
                    h * (0.25 + 0.12 * angle.sin()),
                ),
                340.0 + breath,
                colors[1],
            );
            radial(
                frame,
                Point::new(
                    w * (0.75 - 0.18 * angle.cos()),
                    h * (0.72 - 0.12 * angle.sin()),
                ),
                300.0 - breath,
                alpha(colors[2], 0.85),
            );
        }
        // Bars along the bottom; each bounces a whole number of times per loop.
        Backdrop::Equalizer => {
            frame.fill(
                &full,
                canvas::gradient::Linear::new(Point::ORIGIN, Point::new(0.0, h))
                    .add_stop(0.0, colors[0])
                    .add_stop(1.0, colors[1]),
            );
            let count = 18;
            let gap = 6.0;
            let width = (w - gap * (count as f32 + 1.0)) / count as f32;
            for index in 0..count {
                let bounces = (1 + index % 3) as f32;
                let wave = ((phase * bounces + noise(index, 4)) * TAU).sin();
                let height = h * (0.18 + 0.16 * (wave + 1.0) + 0.12 * noise(index, 5));
                let x = gap + index as f32 * (width + gap);
                let top = h - height;
                frame.fill(
                    &Path::rounded_rectangle(
                        Point::new(x, top),
                        Size::new(width, height + width / 2.0),
                        (width / 2.0).into(),
                    ),
                    canvas::gradient::Linear::new(Point::new(x, top), Point::new(x, h))
                        .add_stop(0.0, colors[2])
                        .add_stop(1.0, alpha(colors[2], 0.25)),
                );
            }
        }
        // Paw prints drifting up two rows and right one cell per loop.
        Backdrop::Paws => {
            frame.fill(&full, colors[1]);
            let ink = alpha(Color::WHITE, 0.3);
            let cell = 64.0;
            let mut row = 0;
            let mut y = -cell - phase * cell * 2.0;
            while y < h + cell {
                let mut x = -cell + phase * cell + if row % 2 == 0 { 0.0 } else { cell / 2.0 };
                while x < w + cell {
                    paw(
                        frame,
                        Point::new(x, y),
                        22.0,
                        if row % 2 == 0 { -0.3 } else { 0.3 },
                        ink,
                    );
                    x += cell;
                }
                y += cell;
                row += 1;
            }
        }
        Backdrop::Halftone => {
            frame.fill(
                &full,
                canvas::gradient::Linear::new(Point::ORIGIN, Point::new(w, h))
                    .add_stop(0.0, colors[2])
                    .add_stop(1.0, colors[1]),
            );
            let step = 12.0;
            let dot = alpha(colors[0], 0.55);
            let mut y = 0.0;
            while y < h + step {
                // Dots grow toward the bottom, like a comic print.
                let radius = 0.8 + 3.4 * y / h;
                let mut x = 0.0;
                while x < w + step {
                    frame.fill(&Path::circle(Point::new(x, y), radius), dot);
                    x += step;
                }
                y += step;
            }
        }
        // Pieces fall one or two card heights per loop.
        Backdrop::Confetti => {
            frame.fill(&full, colors[0]);
            let palette = [colors[1], colors[2], Color::WHITE];
            for index in 0..46 {
                let laps = (1 + index % 2) as f32;
                let fall = (noise(index, 2) + phase * laps).fract();
                let x = noise(index, 1) * w;
                let y = fall * (h + 40.0) - 20.0;
                frame.with_save(|frame| {
                    frame.translate(Vector::new(x, y));
                    frame.rotate(Radians((noise(index, 3) + phase * laps) * TAU));
                    frame.fill(
                        &Path::rounded_rectangle(
                            Point::new(-4.0, -7.0),
                            Size::new(8.0, 14.0),
                            2.0.into(),
                        ),
                        palette[index % 3],
                    );
                });
            }
        }
    }
    // Darker toward the bottom, so the lines and the footer stand out on light covers.
    frame.fill(
        &full,
        canvas::gradient::Linear::new(Point::new(0.0, h / 2.0), Point::new(0.0, h))
            .add_stop(0.0, Color::TRANSPARENT)
            .add_stop(1.0, alpha(Color::BLACK, 0.25)),
    );
}

fn paw(frame: &mut Frame, center: Point, size: f32, tilt: f32, color: Color) {
    frame.with_save(|frame| {
        frame.translate(Vector::new(center.x, center.y));
        frame.rotate(Radians(tilt));
        let oval = |cx: f32, cy: f32, rx: f32, ry: f32| {
            Path::new(|b| {
                b.ellipse(canvas::path::arc::Elliptical {
                    center: Point::new(cx, cy),
                    radii: Vector::new(rx, ry),
                    rotation: Radians(0.0),
                    start_angle: Radians(0.0),
                    end_angle: Radians(TAU),
                });
            })
        };
        frame.fill(&oval(0.0, size * 0.2, size * 0.32, size * 0.25), color);
        let toe = size * 0.2;
        for (x, y) in [(-0.36, -0.2), (-0.13, -0.38), (0.13, -0.38), (0.36, -0.2)] {
            frame.fill(&oval(x * size, y * size, toe / 2.0, toe * 0.575), color);
        }
    });
}

/// A rounded ticket with a semicircle punched out of each side.
fn ticket_shape(size: Size) -> Path {
    let (w, h, r, notch) = (size.width, size.height, 12.0, 13.0);
    Path::new(|b| {
        b.move_to(Point::new(r, 0.0));
        b.line_to(Point::new(w - r, 0.0));
        b.arc_to(Point::new(w, 0.0), Point::new(w, r), r);
        // Notches as quarter circles in Béziers: `arc` would start a new outline.
        let (cy, k) = (h / 2.0, notch * 0.5523);
        b.line_to(Point::new(w, cy - notch));
        b.bezier_curve_to(
            Point::new(w - k, cy - notch),
            Point::new(w - notch, cy - k),
            Point::new(w - notch, cy),
        );
        b.bezier_curve_to(
            Point::new(w - notch, cy + k),
            Point::new(w - k, cy + notch),
            Point::new(w, cy + notch),
        );
        b.line_to(Point::new(w, h - r));
        b.arc_to(Point::new(w, h), Point::new(w - r, h), r);
        b.line_to(Point::new(r, h));
        b.arc_to(Point::new(0.0, h), Point::new(0.0, h - r), r);
        b.line_to(Point::new(0.0, cy + notch));
        b.bezier_curve_to(
            Point::new(k, cy + notch),
            Point::new(notch, cy + k),
            Point::new(notch, cy),
        );
        b.bezier_curve_to(
            Point::new(notch, cy - k),
            Point::new(k, cy - notch),
            Point::new(0.0, cy - notch),
        );
        b.line_to(Point::new(0.0, r));
        b.arc_to(Point::ORIGIN, Point::new(r, 0.0), r);
        b.close();
    })
}

/// A cassette reel: a dashed ring round a hub, turned by `turn`.
fn reel(frame: &mut Frame, center: Point, turn: f32) {
    frame.with_save(|frame| {
        frame.translate(Vector::new(center.x, center.y));
        frame.rotate(Radians(turn));
        frame.stroke(
            &Path::circle(Point::ORIGIN, 22.5),
            Stroke {
                width: 7.0,
                line_dash: LineDash {
                    segments: &[5.0, 5.0],
                    offset: 0,
                },
                ..Stroke::default().with_color(alpha(Color::WHITE, 0.85))
            },
        );
        frame.fill(&Path::circle(Point::ORIGIN, 7.0), alpha(Color::WHITE, 0.9));
    });
}

/// Two soft wedges of light, turned by `turn`: Swift's angular sheen.
fn sheen(frame: &mut Frame, center: Point, radius: f32, turn: f32, colors: [Color; 2]) {
    for (index, color) in colors.into_iter().enumerate() {
        let start = turn + index as f32 * PI + 0.3;
        frame.fill(
            &Path::new(|b| {
                b.move_to(center);
                b.arc(canvas::path::Arc {
                    center,
                    radius,
                    start_angle: Radians(start),
                    end_angle: Radians(start + 0.9),
                });
                b.close();
            }),
            color,
        );
    }
}

fn record(frame: &mut Frame, size: Size, label: Color, turn: f32) {
    let r = size.width / 2.0;
    let c = Point::new(r, r);
    frame.fill(&Path::circle(c, r), Color::from_rgb(0.07, 0.07, 0.07));
    for ring in 0..9 {
        frame.stroke(
            &Path::circle(c, r - 10.0 - ring as f32 * 7.0),
            Stroke::default()
                .with_color(alpha(Color::WHITE, 0.07))
                .with_width(1.0),
        );
    }
    // Light catching the grooves.
    sheen(
        frame,
        c,
        r,
        turn,
        [alpha(Color::WHITE, 0.1), alpha(Color::WHITE, 0.06)],
    );
    frame.fill(&Path::circle(c, r * 0.34), label);
    frame.fill(&Path::circle(c, 4.0), Color::BLACK);
}

/// The disc's rainbow sheen, the clear ring and the hole over the printed cover.
fn cd_overlay(frame: &mut Frame, size: Size, turn: f32) {
    let r = size.width / 2.0;
    let c = Point::new(r, r);
    sheen(
        frame,
        c,
        r,
        turn,
        [
            Color::from_rgba(0.0, 1.0, 1.0, 0.2),
            Color::from_rgba(1.0, 0.4, 0.7, 0.18),
        ],
    );
    let ring = Path::circle(c, r * 0.15);
    frame.fill(&ring, alpha(Color::WHITE, 0.35));
    frame.stroke(
        &ring,
        Stroke::default()
            .with_color(alpha(Color::WHITE, 0.5))
            .with_width(1.0),
    );
    frame.fill(
        &Path::circle(c, r * 0.06),
        Color::from_rgb(0.07, 0.07, 0.09),
    );
    frame.stroke(
        &Path::circle(c, r - 0.5),
        Stroke::default()
            .with_color(alpha(Color::WHITE, 0.35))
            .with_width(1.0),
    );
}

/// `music.note`.
const NOTE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M9 18V5.5l11-2V16" fill="none" stroke="#fff" stroke-width="2" stroke-linejoin="round"/><circle cx="6.5" cy="18" r="2.5" fill="#fff"/><circle cx="17.5" cy="16" r="2.5" fill="#fff"/></svg>"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_starts_at_the_first_line_when_asked() {
        let content = Content {
            lines: vec!["Hello".into()],
            title: "Hello".into(),
            artist: "Adele".into(),
            cover: None,
            colors: [[0.1, 0.2, 0.3], [0.9, 0.5, 0.2]],
            start_time: Some(61.7),
            video_id: Some("abc".into()),
        };
        assert_eq!(
            content.link(true).as_deref(),
            Some("https://music.youtube.com/watch?v=abc&t=61")
        );
        assert_eq!(
            content.link(false).as_deref(),
            Some("https://music.youtube.com/watch?v=abc")
        );
        // Each palette leads with a different colour.
        assert_ne!(content.palette(0)[0], content.palette(1)[0]);
        assert_eq!(content.palette(0)[1], content.palette(2)[2]);
        assert!(qr_code("https://music.youtube.com/watch?v=abc").is_some());
    }
}
