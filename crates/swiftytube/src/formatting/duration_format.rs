use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::networking::inner_tube_decode::{JsonDict, as_dict, as_dict_list, as_string};

static CLOCK_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{1,3})[:.,](\d{2})(?:[:.,](\d{2}))?$").unwrap());
static SPOKEN_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d+)\s*(hours?|hrs?|\bh\b|minutes?|mins?|\bm\b|seconds?|secs?|\bs\b)").unwrap()
});

/// Duration parsing and formatting shared by parsers and apps.
pub struct DurationFormat;

impl DurationFormat {
    /// Parses "m:ss" / "h:mm:ss" clock strings ("." and "," tolerated as
    /// separators); None when unparseable. Anchored to the full string so
    /// counts ("1,234") and years can never read as durations.
    pub fn parse_clock(text: &str) -> Option<i64> {
        let captures = CLOCK_PATTERN.captures(text)?;
        let group = |i: usize| {
            captures
                .get(i)
                .and_then(|m| m.as_str().parse::<i64>().ok())
                .unwrap_or(0)
        };
        if captures.get(3).is_some() {
            return Some(group(1) * 3600 + group(2) * 60 + group(3));
        }
        Some(group(1) * 60 + group(2))
    }

    /// "m:ss" for playback positions. Never empty; clamps negatives and
    /// non-finite values to "0:00".
    pub fn playback_time(t: f64) -> String {
        if !t.is_finite() {
            return "0:00".to_string();
        }
        let total = (t as i64).max(0);
        let hours = total / 3600;
        let minutes = (total % 3600) / 60;
        let seconds = total % 60;
        if hours > 0 {
            return format!("{hours}:{minutes:02}:{seconds:02}");
        }
        format!("{minutes}:{seconds:02}")
    }

    /// Parses spoken durations like "1 hr 23 mins", "45 mins" or "2 hours".
    pub fn parse_spoken(text: &str) -> Option<i64> {
        let lower = text.to_lowercase();
        let mut total = 0;
        let mut found = false;
        // Matches "<number> <unit>" pairs; units cover hr/hour/h, min/m, sec/s.
        for captures in SPOKEN_PATTERN.captures_iter(&lower) {
            let Ok(number) = captures[1].parse::<i64>() else {
                continue;
            };
            let unit = &captures[2];
            if unit.starts_with('h') {
                total += number * 3600;
                found = true;
            } else if unit.starts_with('m') {
                total += number * 60;
                found = true;
            } else if unit.starts_with('s') {
                total += number;
                found = true;
            }
        }
        found.then_some(total)
    }

    pub fn short_duration(seconds: i64) -> String {
        if seconds < 60 {
            return format!("{seconds}s");
        }
        let m = seconds / 60;
        let s = seconds % 60;
        if s == 0 {
            format!("{m}m")
        } else {
            format!("{m}m {s}s")
        }
    }

    /// Duration seconds from any known renderer shape. Checks fixed columns,
    /// `lengthSeconds`, `lengthText`, thumbnail badges, subtitle runs and
    /// flex-column subtitles, in that order; 0 when unparseable.
    pub fn parse_renderer_duration(renderer: &JsonDict) -> i64 {
        fn first_run_text(text_dict: Option<&Value>) -> Option<&str> {
            let runs = as_dict_list(as_dict(text_dict)?.get("runs"))?;
            as_string(runs.into_iter().next()?.get("text"))
        }

        // 1. fixedColumns[0] — primary source for MusicResponsiveListItemRenderer
        if let Some(fixed_columns) = as_dict_list(renderer.get("fixedColumns")) {
            for col in fixed_columns {
                let fixed = as_dict(col.get("musicResponsiveListItemFixedColumnRenderer"));
                if let Some(parsed) =
                    first_run_text(fixed.and_then(|f| f.get("text"))).and_then(Self::parse_clock)
                {
                    return parsed;
                }
            }
        }

        // 2. lengthSeconds — raw seconds string from player response
        match renderer.get("lengthSeconds") {
            Some(Value::String(length_seconds)) => {
                if let Ok(seconds) = length_seconds.parse() {
                    return seconds;
                }
            }
            Some(Value::Number(length_seconds)) => {
                if let Some(seconds) = length_seconds.as_i64() {
                    return seconds;
                }
            }
            _ => {}
        }

        // 3. lengthText — used by PlaylistPanelVideoRenderer (up next / queue)
        if let Some(parsed) = first_run_text(renderer.get("lengthText")).and_then(Self::parse_clock)
        {
            return parsed;
        }

        // 4. Badges — thumbnail badges with duration
        if let Some(badges) = as_dict_list(renderer.get("badges")) {
            for badge_dict in badges {
                let badge = as_dict(badge_dict.get("musicInlineBadgeRenderer"));
                if let Some(parsed) =
                    first_run_text(badge.and_then(|b| b.get("text"))).and_then(Self::parse_clock)
                {
                    return parsed;
                }
            }
        }

        // 5. Subtitle runs — first clock-shaped run wins (durations trail,
        // but scanning beats assuming the last segment holds it).
        let subtitle_runs =
            as_dict_list(as_dict(renderer.get("subtitle")).and_then(|s| s.get("runs")));
        if let Some(found) = subtitle_runs.and_then(|runs| Self::first_clock(&runs)) {
            return found;
        }

        // 6. flexColumns[1] subtitle text (for MusicResponsiveListItemRenderer)
        if let Some(flex_columns) = as_dict_list(renderer.get("flexColumns"))
            && flex_columns.len() > 1
        {
            let col = flex_columns[1];
            let flex_renderer = as_dict(col.get("musicResponsiveListItemFlexColumnRenderer"))
                .or_else(|| as_dict(col.get("musicResponsiveListItemColumnRenderer")));
            let runs = as_dict_list(
                as_dict(flex_renderer.and_then(|f| f.get("text"))).and_then(|t| t.get("runs")),
            );
            if let Some(found) = runs.and_then(|runs| Self::first_clock(&runs)) {
                return found;
            }
        }

        0
    }

    fn first_clock(runs: &[&JsonDict]) -> Option<i64> {
        runs.iter()
            .filter_map(|run| as_string(run.get("text")))
            .find_map(|text| Self::parse_clock(text.trim()))
    }
}

/// Swift `extension Int { formattedDuration, wordsDuration }`.
pub trait DurationFormatInt {
    /// "m:ss" / "h:mm:ss"; empty string when not positive.
    fn formatted_duration(self) -> String;
    fn words_duration(self) -> String;
}

impl DurationFormatInt for i64 {
    fn formatted_duration(self) -> String {
        if self <= 0 {
            return String::new();
        }
        let hours = self / 3600;
        let minutes = (self % 3600) / 60;
        let secs = self % 60;
        if hours > 0 {
            return format!("{hours}:{minutes:02}:{secs:02}");
        }
        format!("{minutes}:{secs:02}")
    }

    fn words_duration(self) -> String {
        if self <= 0 {
            return String::new();
        }
        let hours = self / 3600;
        let minutes = (self % 3600) / 60;
        match (hours, minutes) {
            (0, 0) => format!("{} sec", self % 60),
            (0, _) => format!("{minutes} min"),
            (_, 0) => format!("{hours} h"),
            _ => format!("{hours} h {minutes} min"),
        }
    }
}
