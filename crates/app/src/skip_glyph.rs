// Swift `AnimatedSkipLabel`: pressing Next or Previous rolls its glyph for 0.27 s.
// ponytail: three triangles slide half a glyph while the first fades in and the
// last fades out; Swift also grows and shrinks them (`RollingSkipGlyph`).

use std::time::{Duration, Instant};

use iced::widget::{Space, container, row, svg};
use iced::{Alignment, Element, Length, Subscription, window};

use crate::{Message, theme};

const DURATION: Duration = Duration::from_millis(270);
/// Triangle width in 24-unit icon space.
const WIDTH: f32 = 9.0;

const RIGHT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="3 5 9 14"><path d="M3 6.2v11.6a.8.8 0 0 0 1.2.7l8.6-5.8a.8.8 0 0 0 0-1.4L4.2 5.5a.8.8 0 0 0-1.2.7z" fill="#fff"/></svg>"##;
const LEFT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="12 5 9 14"><path d="M21 6.2v11.6a.8.8 0 0 1-1.2.7l-8.6-5.8a.8.8 0 0 1 0-1.4l8.6-5.8a.8.8 0 0 1 1.2.7z" fill="#fff"/></svg>"##;

#[derive(Default)]
pub struct Skip {
    started: Option<Instant>,
    now: Option<Instant>,
    backwards: bool,
}

impl Skip {
    pub fn start(&mut self, backwards: bool) {
        self.started = Some(Instant::now());
        self.now = self.started;
        self.backwards = backwards;
    }

    pub fn frame(&mut self, now: Instant) {
        self.now = Some(now);
        if self.progress(self.backwards) >= 1.0 {
            self.started = None;
        }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if self.started.is_some() {
            window::frames().map(Message::SkipFrame)
        } else {
            Subscription::none()
        }
    }

    /// 0 at rest, up to 1 at the end of the roll; `backwards` picks which button asks.
    fn progress(&self, backwards: bool) -> f32 {
        match (self.started, self.now) {
            (Some(start), Some(now)) if self.backwards == backwards => {
                (now.saturating_duration_since(start).as_secs_f32() / DURATION.as_secs_f32())
                    .min(1.0)
            }
            _ => 0.0,
        }
    }

    /// The Next (or Previous) glyph at its size, rolled to where the clock is.
    pub fn glyph<'a>(&self, backwards: bool, size: f32, enabled: bool) -> Element<'a, Message> {
        let dim = if enabled { 1.0 } else { 0.3 };
        let p = self.progress(backwards);
        let unit = size / 24.0;
        let (w, h) = (WIDTH * unit, 14.0 * unit);
        let tri = |opacity: f32| -> Element<'a, Message> {
            svg(svg::Handle::from_memory(if backwards {
                LEFT
            } else {
                RIGHT
            }))
            .width(w)
            .height(h)
            .opacity(opacity * dim)
            .style(|_, _| svg::Style {
                color: Some(theme::TEXT),
            })
            .into()
        };
        let shift = Space::new().width(w * p);
        let triangles = if backwards {
            row![tri(1.0 - p), tri(1.0), tri(p), shift]
        } else {
            row![shift, tri(p), tri(1.0), tri(1.0 - p)]
        };
        // Room for the extra triangle on both sides; the box clips what rolls out.
        container(triangles.align_y(Alignment::Center))
            .width(Length::Fixed(size + 12.0 * unit))
            .height(size)
            .center_y(size)
            .align_x(if backwards {
                iced::alignment::Horizontal::Right
            } else {
                iced::alignment::Horizontal::Left
            })
            .clip(true)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolls_for_270_ms_then_rests() {
        let mut skip = Skip::default();
        skip.start(false);
        let start = skip.started.unwrap();
        skip.frame(start + Duration::from_millis(135));
        assert!((skip.progress(false) - 0.5).abs() < 0.01);
        assert_eq!(skip.progress(true), 0.0);
        skip.frame(start + Duration::from_millis(300));
        assert!(skip.started.is_none());
        assert_eq!(skip.progress(false), 0.0);
    }
}
