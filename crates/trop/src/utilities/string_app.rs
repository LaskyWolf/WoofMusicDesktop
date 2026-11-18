// Trop/Utilities/String+App.swift: `decodingHTMLEntities` only. `nilIfEmpty`
// and `isBlank` have no Rust caller yet.

use std::sync::LazyLock;

use regex::{Captures, Regex};

static DECIMAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#(\d+);").unwrap());
static HEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#x([0-9a-fA-F]+);").unwrap());

/// Swift `extension String`.
pub trait StringApp {
    /// Decodes common HTML entities (named + numeric/hex) into characters.
    fn decoding_html_entities(&self) -> String;
}

impl StringApp for str {
    fn decoding_html_entities(&self) -> String {
        // `&amp;` goes last, so `&amp;lt;` stays `&lt;`. Swift's dictionary order is random.
        let named = [
            ("&lt;", "<"),
            ("&gt;", ">"),
            ("&quot;", "\""),
            ("&apos;", "'"),
            ("&#39;", "'"),
            ("&nbsp;", " "),
            ("&amp;", "&"),
        ];
        let mut result = self.to_string();
        for (entity, char) in named {
            result = result.replace(entity, char);
        }
        let numeric = |radix| {
            move |caps: &Captures| {
                u32::from_str_radix(&caps[1], radix)
                    .ok()
                    .and_then(char::from_u32)
                    .map_or_else(|| caps[0].to_string(), String::from)
            }
        };
        let result = DECIMAL.replace_all(&result, numeric(10));
        HEX.replace_all(&result, numeric(16)).into_owned()
    }
}
