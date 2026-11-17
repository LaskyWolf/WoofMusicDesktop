//! A scrolling list that builds only the rows on screen.
//!
//! iced's `scrollable(column![...])` lays out every row on each update, which
//! is slow for a 5,000-song playlist. This list keeps the full height with two
//! spacers and creates only the visible rows (plus a few above and below).
//! Rows must all have the same height.

use std::ops::Range;

use iced::widget::{Space, column, scrollable};
use iced::{Element, Length, Padding};

use crate::theme;

/// Rows built beyond each edge of the viewport, so fast scrolls show no gap.
const OVERSCAN: usize = 6;

/// Scroll state the caller keeps: update it from the `on_scroll` message.
#[derive(Debug, Clone, Copy, Default)]
pub struct State {
    pub offset_y: f32,
    pub viewport_height: f32,
}

impl State {
    pub fn update(&mut self, viewport: scrollable::Viewport) {
        self.offset_y = viewport.absolute_offset().y;
        self.viewport_height = viewport.bounds().height;
    }
}

pub fn view<'a, Message: 'a>(
    state: &State,
    len: usize,
    row_height: f32,
    padding: Padding,
    row: impl Fn(usize) -> Element<'a, Message>,
    on_scroll: impl Fn(scrollable::Viewport) -> Message + 'a,
) -> Element<'a, Message> {
    // Before the first scroll event the viewport height is unknown: assume a tall window.
    let viewport = if state.viewport_height > 0.0 {
        state.viewport_height
    } else {
        2000.0
    };
    let range = visible_range(len, row_height, state.offset_y - padding.top, viewport);
    let above = range.start as f32 * row_height;
    let below = (len - range.end) as f32 * row_height;
    let rows = range.map(row);
    scrollable(
        column![Space::new().height(above)]
            .extend(rows)
            .push(Space::new().height(below))
            .width(Length::Fill)
            .padding(padding),
    )
    .direction(theme::scrollbar())
    .style(theme::scroll_style)
    .on_scroll(on_scroll)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Indices of the rows that intersect the viewport, widened by `OVERSCAN`.
fn visible_range(len: usize, row_height: f32, offset_y: f32, viewport: f32) -> Range<usize> {
    if len == 0 || row_height <= 0.0 {
        return 0..0;
    }
    let first = (offset_y.max(0.0) / row_height) as usize;
    let last = ((offset_y.max(0.0) + viewport) / row_height).ceil() as usize;
    first.saturating_sub(OVERSCAN).min(len)..(last + OVERSCAN).min(len)
}

#[cfg(test)]
mod tests {
    use super::visible_range;

    #[test]
    fn top_of_a_long_list() {
        assert_eq!(visible_range(5000, 56.0, 0.0, 560.0), 0..16);
    }

    #[test]
    fn middle_of_a_long_list() {
        // Rows 100..110 are on screen; 6 more on each side.
        assert_eq!(visible_range(5000, 56.0, 5600.0, 560.0), 94..116);
    }

    #[test]
    fn end_and_short_lists() {
        assert_eq!(visible_range(20, 56.0, 600.0, 560.0), 4..20);
        assert_eq!(visible_range(3, 56.0, 0.0, 560.0), 0..3);
        assert_eq!(visible_range(0, 56.0, 0.0, 560.0), 0..0);
    }
}
