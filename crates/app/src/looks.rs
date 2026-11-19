// Swift's player looks: `FullscreenCoverOverlay`, `RadioBloom`, `SleepEndedAnimation`.
// They draw over the full player and never touch playback.
// ponytail: the radio's antenna and the sleep glow (a Metal shader in Swift) are left out;
// the cover does not fly from the player, it lands. Add keyframes if they are missed.

use std::time::{Duration, Instant};

use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Canvas, Frame, Path, Stroke};
use iced::widget::{Space, button, container, image, mouse_area, stack, text};
use iced::{
    Background, Border, Color, Degrees, Element, Length, Padding, Pixels, Point, Rectangle,
    Rotation, Size, Subscription, window,
};
use trop::networking::models::yt_item::SongItem;
use trop::settings::settings_store::PlayerBackgroundStyle;

use crate::services::Artwork;
use crate::{Message, now_playing, theme, ui};

const TEAL: Color = Color::from_rgb(0.37, 0.70, 0.66);
/// Shortest ripple, so a radio that loads at once still blooms.
const MINIMUM_RIPPLE: f32 = 0.9;
const RIPPLE_LIMIT: f32 = 20.0;
/// Covers burst out (0.35 s), rest, fly to the queue button (from 1.2 s), badge until the end.
const FLIGHT_START: f32 = 1.2;
const FLIGHT_END: f32 = 1.8;
const BLOOM_END: f32 = 3.4;
/// Orb spots around the cover's centre, as fractions of its side.
const ORB_SPOTS: [(f32, f32); 5] = [
    (-0.49, -0.37),
    (0.0, -0.53),
    (0.49, -0.37),
    (-0.54, 0.15),
    (0.54, 0.15),
];
/// Seconds before the end the night sky starts, and how long it lasts after it.
const PRELUDE: f32 = 10.0;
const CELEBRATION: f32 = 15.0;

#[derive(Default)]
pub struct Looks {
    /// The full-screen cover: the angle it lands at, in degrees.
    pub cover: Option<f32>,
    bloom: Option<Bloom>,
    /// When the sleep timer ended, and what the sky says.
    slept: Option<(Instant, &'static str)>,
}

struct Bloom {
    started: Instant,
    loaded: Option<Loaded>,
}

struct Loaded {
    at: Instant,
    covers: Vec<Option<String>>,
    added: usize,
}

/// What `overlay` reads from the app.
pub struct Scene<'a> {
    pub window: Size,
    pub queue_open: bool,
    pub lyrics_open: bool,
    pub covers: &'a ui::Covers,
    pub sleep_left: Option<Duration>,
}

fn ramp(t: f32, from: f32, to: f32) -> f32 {
    ((t - from) / (to - from)).clamp(0.0, 1.0)
}

fn ease(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}

fn smoother(x: f32) -> f32 {
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

impl Looks {
    /// 4° to 12° either way, never upright (Swift picks once per opening).
    pub fn open_cover(&mut self) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let angle = 4.0 + (nanos % 800) as f32 / 100.0;
        self.cover = Some(if (nanos / 1000).is_multiple_of(2) { angle } else { -angle });
    }

    pub fn bloom_start(&mut self, now: Instant) {
        self.bloom = Some(Bloom {
            started: now,
            loaded: None,
        });
    }

    /// The radio's songs arrived (`covers` empty and `added` 0 when it failed).
    pub fn bloom_loaded(&mut self, now: Instant, songs: &[SongItem], added: usize) {
        if let Some(bloom) = &mut self.bloom {
            bloom.loaded = Some(Loaded {
                at: now,
                covers: songs
                    .iter()
                    .take(5)
                    .map(|song| song.thumbnail_url.clone())
                    .collect(),
                added,
            });
        }
    }

    pub fn slept(&mut self, now: Instant, message: &'static str) {
        self.slept = Some((now, message));
    }

    /// Seconds into the night sky: negative before the timer ends.
    fn sky_time(&self, now: Instant, left: Option<Duration>) -> Option<f32> {
        if let Some((end, _)) = self.slept {
            let t = now.saturating_duration_since(end).as_secs_f32();
            return (t < CELEBRATION).then_some(t);
        }
        left.map(|left| -left.as_secs_f32())
            .filter(|t| *t > -PRELUDE)
    }

    fn bloom_time(&self, now: Instant) -> Option<(&Bloom, f32)> {
        let bloom = self.bloom.as_ref()?;
        Some((
            bloom,
            now.saturating_duration_since(bloom.started).as_secs_f32(),
        ))
    }

    /// True while something moves; the app then asks for frames.
    pub fn active(&self, now: Instant, left: Option<Duration>) -> bool {
        self.sky_time(now, left).is_some()
            || self
                .bloom_time(now)
                .is_some_and(|(bloom, _)| match &bloom.loaded {
                    Some(loaded) => {
                        now.saturating_duration_since(loaded.at).as_secs_f32() < BLOOM_END
                    }
                    None => true,
                })
    }

    pub fn subscription(&self, left: Option<Duration>) -> Subscription<Message> {
        if self.active(Instant::now(), left) {
            window::frames().map(Message::LooksFrame)
        } else {
            Subscription::none()
        }
    }

    /// The bloom and the night sky over the full player, if either runs.
    pub fn overlay<'a>(&'a self, now: Instant, scene: &Scene<'a>) -> Option<Element<'a, Message>> {
        let mut layers = stack![];
        let mut any = false;
        if let Some(t) = self.sky_time(now, scene.sleep_left) {
            let message = self.slept.map_or("", |(_, message)| message);
            layers = layers.push(
                Canvas::new(Sky { t, message })
                    .width(Length::Fill)
                    .height(Length::Fill),
            );
            any = true;
        }
        if let Some((bloom, elapsed)) = self.bloom_time(now) {
            let cover = cover_rect(scene);
            let since = bloom
                .loaded
                .as_ref()
                .map(|loaded| now.saturating_duration_since(loaded.at).as_secs_f32());
            let rippling = if since.is_none() {
                elapsed < RIPPLE_LIMIT
            } else {
                elapsed < MINIMUM_RIPPLE
            };
            if rippling {
                layers = layers.push(
                    Canvas::new(Rings { cover, elapsed })
                        .width(Length::Fill)
                        .height(Length::Fill),
                );
                any = true;
            }
            if let (Some(loaded), Some(since)) = (&bloom.loaded, since)
                && since < BLOOM_END
            {
                layers = layers.push(orbs(loaded, since, cover, scene));
                any = true;
            }
        }
        any.then(|| {
            container(layers)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        })
    }
}

/// Where the cover sits in the full player (`now_playing::stage`'s maths).
fn cover_rect(scene: &Scene<'_>) -> Rectangle {
    let (w, h) = (scene.window.width, scene.window.height);
    let top = crate::title_bar::HEIGHT + 8.0;
    let right = if scene.queue_open {
        now_playing::QUEUE_WIDTH + theme::INSET + theme::CONTENT_RIGHT
    } else {
        theme::CONTENT_RIGHT
    };
    let bottom = theme::PLAYER_HEIGHT + 2.0 * theme::INSET + 24.0;
    let area = Size::new(w - theme::CONTENT_LEFT - right, h - top - bottom);
    let (width, left) = if scene.lyrics_open && area.width >= now_playing::SPLIT_MIN {
        (
            (area.width - now_playing::SPLIT_GAP) * 0.42,
            theme::CONTENT_LEFT,
        )
    } else {
        (area.width, theme::CONTENT_LEFT)
    };
    let side = (area.height - now_playing::INFO_HEIGHT)
        .min(width)
        .clamp(160.0, now_playing::MAX_COVER);
    let column = side + 28.0 + now_playing::INFO_HEIGHT - 28.0;
    Rectangle::new(
        Point::new(
            left + (width - side) / 2.0,
            top + (area.height - column) / 2.0,
        ),
        Size::new(side, side),
    )
}

struct Rings {
    cover: Rectangle,
    elapsed: f32,
}

impl canvas::Program<Message> for Rings {
    type State = ();

    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        _: &iced::Theme,
        bounds: Rectangle,
        _: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let center = self.cover.center();
        for ring in 0..3 {
            let time = self.elapsed - ring as f32 * 0.3;
            if time < 0.0 {
                continue;
            }
            let p = time.rem_euclid(1.2) / 1.2;
            let size = self.cover.size() * (1.0 + 0.5 * p);
            let path = Path::rounded_rectangle(
                Point::new(center.x - size.width / 2.0, center.y - size.height / 2.0),
                size,
                (18.0 + 20.0 * p).into(),
            );
            frame.stroke(
                &path,
                Stroke::default()
                    .with_color(Color {
                        a: 0.85 * (1.0 - p),
                        ..TEAL
                    })
                    .with_width(2.0),
            );
        }
        vec![frame.into_geometry()]
    }
}

/// The covers burst around the cover, fly to the queue button, and a badge counts them.
fn orbs<'a>(
    loaded: &'a Loaded,
    since: f32,
    cover: Rectangle,
    scene: &Scene<'a>,
) -> Element<'a, Message> {
    // ponytail: the queue button is where the bar puts it by default, not measured.
    let target = Point::new(scene.window.width - 150.0, scene.window.height - 44.0);
    let center = cover.center();
    let side = cover.width;
    let burst = ease(ramp(since, 0.0, 0.35));
    let flight = ease(ramp(since, FLIGHT_START, FLIGHT_END));
    let mut layers = stack![];
    if flight < 1.0 {
        for (index, url) in loaded.covers.iter().enumerate() {
            let (dx, dy) = ORB_SPOTS[index % ORB_SPOTS.len()];
            let spot = Point::new(center.x + dx * side, center.y + dy * side);
            let from = Point::new(
                center.x + (spot.x - center.x) * burst,
                center.y + (spot.y - center.y) * burst,
            );
            let at = Point::new(
                from.x + (target.x - from.x) * flight,
                from.y + (target.y - from.y) * flight,
            );
            let size = 52.0 * (0.3 + 0.7 * burst) * (1.0 - 0.75 * flight);
            layers = layers.push(
                container(ui::cover(scene.covers, url.as_deref(), size, 10.0)).padding(Padding {
                    top: at.y - size / 2.0,
                    left: at.x - size / 2.0,
                    ..Padding::ZERO
                }),
            );
        }
    }
    if since >= FLIGHT_END && loaded.added > 0 {
        layers = layers.push(
            container(
                container(
                    text(format!("+{}", loaded.added))
                        .size(11)
                        .font(theme::weight(iced::font::Weight::ExtraBold))
                        .color(Color::from_rgb(0.02, 0.13, 0.11)),
                )
                .padding([2, 6])
                .style(|_| container::Style {
                    background: Some(Background::Color(TEAL)),
                    border: Border::default().rounded(10),
                    ..container::Style::default()
                }),
            )
            .padding(Padding {
                top: target.y - 24.0,
                left: target.x + 4.0,
                ..Padding::ZERO
            }),
        );
    }
    layers.into()
}

/// Swift `SleepAnimationFrame` without the Metal glow: a tint, stars, comets and the title.
struct Sky {
    t: f32,
    message: &'static str,
}

/// Cheap fixed pseudo-random in 0..1 for star `i`, channel `k`.
fn noise(i: usize, k: usize) -> f32 {
    let x = (i * 7919 + k * 104_729 + 13) as f32;
    (x.sin() * 43_758.545).fract().abs()
}

impl canvas::Program<Message> for Sky {
    type State = ();

    fn draw(
        &self,
        _: &(),
        renderer: &iced::Renderer,
        _: &iced::Theme,
        bounds: Rectangle,
        _: iced::mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let t = self.t;
        let (w, h) = (bounds.width, bounds.height);
        let mut frame = Frame::new(renderer, bounds.size());
        let out = 1.0 - ease(ramp(t, 12.5, 15.0));
        let pre = 0.4 * ease(ramp(t, -PRELUDE, 0.0));
        let tint = (pre + (1.0 - pre) * ease(ramp(t, 0.0, 3.2))) * out;
        let title = ease(ramp(t, 3.0, 4.6)) * (1.0 - ease(ramp(t, 12.3, 14.5)));
        let top = Color::from_rgba(40.0 / 255.0, 36.0 / 255.0, 120.0 / 255.0, 0.4 * tint);
        let bottom = Color::from_rgba(24.0 / 255.0, 30.0 / 255.0, 90.0 / 255.0, 0.28 * tint);
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            canvas::Fill::from(canvas::Gradient::Linear(
                canvas::gradient::Linear::new(Point::ORIGIN, Point::new(0.0, h))
                    .add_stop(0.0, top)
                    .add_stop(1.0, bottom),
            )),
        );
        let fade_in = smoother(ramp(t, -PRELUDE, -7.0));
        let stars = Color::from_rgb(226.0 / 255.0, 223.0 / 255.0, 1.0);
        for i in 0..36 {
            let twinkle = 0.5 + 0.5 * (t * (1.0 + noise(i, 3) * 2.0) + noise(i, 2) * 6.3).sin();
            let alpha = fade_in * out * (0.25 + 0.75 * twinkle) * (0.4 + 0.6 * tint);
            let radius = 0.8 + noise(i, 4) * 1.6;
            frame.fill(
                &Path::circle(Point::new(noise(i, 0) * w, noise(i, 1) * h * 0.7), radius),
                Color { a: alpha, ..stars },
            );
        }
        if t > 0.0 {
            for k in 0..3 {
                let phase = ((t - k as f32 * 1.9).rem_euclid(5.7)) / 1.3;
                if phase >= 1.0 || t - (k as f32 * 1.9) < 0.0 {
                    continue;
                }
                let start = Point::new(w * (0.35 + 0.3 * noise(k, 7) + 0.25 * k as f32), h * 0.05);
                let head = Point::new(start.x - phase * w * 0.35, start.y + phase * h * 0.3);
                let tail = Point::new(head.x + 70.0, head.y - 49.0);
                let alpha = (1.0 - phase) * out;
                frame.stroke(
                    &Path::line(tail, head),
                    Stroke::default()
                        .with_color(Color {
                            a: 0.5 * alpha,
                            ..stars
                        })
                        .with_width(2.0),
                );
                frame.fill(
                    &Path::circle(head, 2.4),
                    Color {
                        a: alpha,
                        ..Color::WHITE
                    },
                );
            }
        }
        if title > 0.0 {
            frame.fill_text(canvas::Text {
                content: self.message.to_owned(),
                position: Point::new(w / 2.0, h * 0.42),
                color: Color {
                    a: title,
                    ..Color::WHITE
                },
                size: Pixels(w.min(h) * 0.09 * (0.94 + 0.06 * title)),
                font: theme::weight(iced::font::Weight::Bold),
                align_x: iced::advanced::text::Alignment::Center,
                align_y: Vertical::Center,
                ..canvas::Text::default()
            });
        }
        vec![frame.into_geometry()]
    }
}

/// The full-screen cover: the cover large and tilted over the player's colours.
/// A click anywhere, or Escape, closes it.
pub fn fullscreen_cover<'a>(
    angle: f32,
    artwork: Option<&'a Artwork>,
    song: &'a SongItem,
    style: PlayerBackgroundStyle,
    covers: &'a ui::Covers,
    playing: bool,
    window: Size,
) -> Element<'a, Message> {
    let side = (window.width - 48.0).min(window.height * 0.5).max(0.0);
    let cover: Element<'a, Message> = match artwork {
        Some(artwork) => image(artwork.image.clone())
            .width(side)
            .height(side)
            .border_radius(12)
            .rotation(Rotation::Solid(Degrees(angle).into()))
            .into(),
        None => ui::cover(covers, song.thumbnail_url.as_deref(), side, 12.0),
    };
    let play = crate::transport_button(
        if playing {
            crate::icons::PAUSE
        } else {
            crate::icons::PLAY
        },
        26.0,
        Some(Message::TogglePlay),
    )
    .padding(14)
    .style(|_, status| button::Style {
        background: Some(Background::Color(Color {
            a: if status == button::Status::Hovered {
                0.3
            } else {
                0.18
            },
            ..Color::WHITE
        })),
        border: Border::default().rounded(32),
        ..button::Style::default()
    });
    let layers = stack![
        now_playing::background(artwork, style),
        container(cover)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Center)
            .padding(Padding {
                top: (window.height * 0.46 - side / 2.0).max(0.0),
                ..Padding::ZERO
            }),
        container(play)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Horizontal::Right)
            .align_y(Vertical::Bottom)
            .padding(24),
        Space::new(),
    ];
    mouse_area(layers)
        .on_press(Message::FullscreenCover(false))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sky_runs_from_ten_seconds_before_to_fifteen_after() {
        let mut looks = Looks::default();
        let now = Instant::now();
        assert_eq!(looks.sky_time(now, None), None);
        assert_eq!(looks.sky_time(now, Some(Duration::from_secs(30))), None);
        let t = looks.sky_time(now, Some(Duration::from_secs(4))).unwrap();
        assert!((t + 4.0).abs() < 0.01);
        looks.slept(now, "Sleep well");
        assert!(looks.sky_time(now + Duration::from_secs(5), None).is_some());
        assert_eq!(looks.sky_time(now + Duration::from_secs(16), None), None);
    }

    #[test]
    fn the_cover_lands_tilted() {
        let mut looks = Looks::default();
        for _ in 0..20 {
            looks.open_cover();
            let angle = looks.cover.unwrap().abs();
            assert!((4.0..=12.0).contains(&angle));
        }
    }

    #[test]
    fn bloom_ends_after_its_flight() {
        let mut looks = Looks::default();
        let now = Instant::now();
        looks.bloom_start(now);
        assert!(looks.active(now, None));
        looks.bloom_loaded(now, &[], 0);
        assert!(!looks.active(now + Duration::from_secs_f32(BLOOM_END + 0.1), None));
    }
}
