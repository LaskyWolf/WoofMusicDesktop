// Swift `MarqueeText`: a title that does not fit slides sideways instead of
// ending in an ellipsis. It waits 2 s, scrolls at 20 px/s, rests 1 s, starts over.
// ponytail: no fade at the edges (Swift fades 12 px); add a gradient quad if the hard cut shows.

use std::time::Instant;

use iced::advanced::text::Paragraph as _;
use iced::advanced::widget::{Tree, tree};
use iced::advanced::{Layout, Shell, Widget, layout, mouse, renderer, text};
use iced::{Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Size, window};

const SPEED: f32 = 20.0;
const START_DELAY: f32 = 2.0;
const END_PAUSE: f32 = 1.0;
const FADE: f32 = 12.0;
/// Overflow below this stays static, like Swift's `overflowThreshold`.
const THRESHOLD: f32 = 15.0;

/// `text` in a box `width` wide; `center` centres it while it fits.
pub fn marquee<'a, Message: 'a>(
    content: &str,
    size: f32,
    width: f32,
    font: Font,
    color: Color,
    center: bool,
) -> Element<'a, Message> {
    Element::new(Marquee {
        content: content.to_owned(),
        size,
        width,
        font,
        color,
        center,
    })
}

struct Marquee {
    content: String,
    size: f32,
    width: f32,
    font: Font,
    color: Color,
    center: bool,
}

#[derive(Default)]
struct State {
    /// What the clock below belongs to: a new title restarts it.
    shown: String,
    started: Option<Instant>,
    now: Option<Instant>,
}

/// How far left the text is `elapsed` seconds into its cycle.
fn offset(elapsed: f32, overflow: f32) -> f32 {
    if overflow <= THRESHOLD {
        return 0.0;
    }
    let distance = overflow + FADE;
    let duration = distance / SPEED;
    let t = elapsed % (START_DELAY + duration + END_PAUSE);
    if t < START_DELAY {
        0.0
    } else if t < START_DELAY + duration {
        let p = (t - START_DELAY) / duration;
        -distance * p * p * (3.0 - 2.0 * p)
    } else {
        -distance
    }
}

impl Marquee {
    fn text_width(&self, renderer: &iced::Renderer) -> f32 {
        let _ = renderer;
        <iced::Renderer as text::Renderer>::Paragraph::with_text(self.text(&self.content))
            .min_width()
    }

    fn text<'a>(&self, content: &'a str) -> text::Text<&'a str> {
        text::Text {
            content,
            bounds: Size::new(f32::INFINITY, self.height()),
            size: Pixels(self.size),
            line_height: text::LineHeight::default(),
            font: self.font,
            align_x: text::Alignment::Default,
            align_y: iced::alignment::Vertical::Center,
            shaping: text::Shaping::Advanced,
            wrapping: text::Wrapping::None,
        }
    }

    fn height(&self) -> f32 {
        (self.size * 1.4).ceil()
    }
}

impl<Message> Widget<Message, iced::Theme, iced::Renderer> for Marquee {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.width), Length::Fixed(self.height()))
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(Size::new(self.width, self.height()))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        _clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let Event::Window(window::Event::RedrawRequested(now)) = event else {
            return;
        };
        let state = tree.state.downcast_mut::<State>();
        if state.shown != self.content {
            state.shown.clone_from(&self.content);
            state.started = Some(*now);
        }
        state.started.get_or_insert(*now);
        state.now = Some(*now);
        if self.text_width(renderer) - self.width > THRESHOLD {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        use iced::advanced::text::Renderer as _;
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let width = self.text_width(renderer);
        let overflow = width - bounds.width;
        let elapsed = match (state.started, state.now) {
            (Some(start), Some(now)) => now.saturating_duration_since(start).as_secs_f32(),
            _ => 0.0,
        };
        let mut x = bounds.x + offset(elapsed, overflow);
        if self.center && overflow <= 0.0 {
            x = bounds.x - overflow / 2.0;
        }
        let mut shown = self.text(&self.content);
        shown.bounds = Size::new(width + 2.0, bounds.height);
        let shown = shown.with_content(self.content.clone());
        renderer.with_layer(bounds, |renderer| {
            renderer.fill_text(
                shown,
                Point::new(x, bounds.y + bounds.height / 2.0),
                self.color,
                bounds,
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_then_scrolls_then_rests_then_restarts() {
        let overflow = 100.0;
        let distance = overflow + FADE;
        let duration = distance / SPEED;
        assert_eq!(offset(1.0, overflow), 0.0);
        let mid = offset(START_DELAY + duration / 2.0, overflow);
        assert!(mid < 0.0 && mid > -distance);
        assert_eq!(offset(START_DELAY + duration + 0.5, overflow), -distance);
        assert_eq!(
            offset(START_DELAY + duration + END_PAUSE + 1.0, overflow),
            0.0
        );
    }

    #[test]
    fn short_overflow_stays_still() {
        assert_eq!(offset(5.0, THRESHOLD), 0.0);
    }
}
