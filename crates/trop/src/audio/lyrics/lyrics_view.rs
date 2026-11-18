//! Logic half of Trop/Audio/Lyrics/LyricsView.swift: which lines to show,
//! which one is active at a time, and the instrumental breaks between them.
//! The view romanizes each shown line's text with `LyricsRomanizer.romanize`
//! (`romanizer/`) when the toggle is on.

use super::lyrics_service::LyricLine;

/// A break between sung lines, shown as a ring after line `after_index` of
/// `display_lines`. Seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct InstrumentalGap {
    pub after_index: usize,
    pub start: f64,
    pub end: f64,
}

/// Blank / ♪-only timestamped lines declare instrumental spans.
pub fn is_instrumental_marker(line: &LyricLine) -> bool {
    line.text
        .chars()
        .all(|c| matches!(c, '♪' | '*' | '·' | ' '))
}

/// The lines the view lists: all but the instrumental markers, or all of them
/// when there is nothing else.
pub fn display_lines(all: &[LyricLine]) -> Vec<LyricLine> {
    let without_markers: Vec<LyricLine> = all
        .iter()
        .filter(|l| !is_instrumental_marker(l))
        .cloned()
        .collect();
    if without_markers.is_empty() {
        all.to_vec()
    } else {
        without_markers
    }
}

/// The active line of `display_lines` at `time`: the last line that started
/// at or before it, 0 before the first. `time` is the playback position plus
/// the lyrics offset setting, in seconds.
pub fn active_line_index(lines: &[LyricLine], time: f64) -> usize {
    let mut index = 0;
    for (i, line) in lines.iter().enumerate() {
        let Some(start) = line.start_time else {
            continue;
        };
        if start > time {
            break;
        }
        index = i;
    }
    index
}

/// Mirrors Metrolist's updateMergedList: a gap is created only from positive
/// evidence, a blank interlude marker whose timestamp says the words already
/// ended. Nothing is estimated, so the ring never shows over actual singing.
/// `all` is the fetched lines, markers included.
pub fn detect_gaps(all: &[LyricLine]) -> Vec<InstrumentalGap> {
    let mut display_indices = vec![None; all.len()];
    let mut display_count = 0;
    for (i, line) in all.iter().enumerate() {
        if !is_instrumental_marker(line) {
            display_indices[i] = Some(display_count);
            display_count += 1;
        }
    }

    let mut gaps = Vec::new();
    for (i, marker) in all.iter().enumerate() {
        let Some(gap_start) = marker.start_time.filter(|_| is_instrumental_marker(marker)) else {
            continue;
        };
        // Attach to the nearest preceding vocal line; intro markers have none.
        let Some(after_index) = display_indices[..i].iter().rev().find_map(|d| *d) else {
            continue;
        };
        // The break ends when the next lyrics start.
        let Some(gap_end) = all[i + 1..].iter().find_map(|l| l.start_time) else {
            continue;
        };
        if gap_end - gap_start > 4.0 {
            gaps.push(InstrumentalGap {
                after_index,
                start: gap_start,
                end: gap_end,
            });
        }
    }
    gaps
}
