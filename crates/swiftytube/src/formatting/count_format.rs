use std::sync::LazyLock;

use regex::Regex;

static COMPACT_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^([\d,.]+)\s*([KMBT]?)\s*views?$").unwrap());

/// Shared compact-count parsing and formatting.
pub struct CountFormat;

impl CountFormat {
    /// Parses view counts like "1.2M views", "500K views", "12,345 views" into an
    /// integer. The "views" word is required, so years and bare numbers in a
    /// subtitle never read as counts.
    pub fn parse_compact_count(text: &str) -> Option<i64> {
        let captures = COMPACT_PATTERN.captures(text.trim())?;
        let number: f64 = captures[1].replace(',', "").parse().ok()?;
        let multiplier = match captures[2].to_uppercase().as_str() {
            "K" => 1e3,
            "M" => 1e6,
            "B" => 1e9,
            "T" => 1e12,
            _ => 1.0,
        };
        Some((number * multiplier) as i64)
    }
}

/// Swift `extension Int { formattedViews(suffix:) }`.
pub trait CountFormatInt {
    /// "1,234,567 views" with decimal grouping (en-US separators; Swift uses the device locale).
    /// Swift's default `suffix` is `" views"`.
    fn formatted_views(self, suffix: &str) -> String;
}

impl CountFormatInt for i64 {
    fn formatted_views(self, suffix: &str) -> String {
        let digits = self.unsigned_abs().to_string();
        let mut grouped = String::new();
        for (i, ch) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                grouped.push(',');
            }
            grouped.push(ch);
        }
        let sign = if self < 0 { "-" } else { "" };
        format!("{sign}{grouped}{suffix}")
    }
}
