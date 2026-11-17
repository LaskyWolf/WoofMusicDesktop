use serde_json::{Map, Value};

use crate::core::swiftytube_log::SwiftyTubeLog;

/// Swift `[String: Any]`.
pub type JsonDict = Map<String, Value>;

// Rust stand-ins for Swift's `as? [String: Any]`, `as? [[String: Any]]` and
// `as? String`: None when the value has another shape, never a panic.

pub fn as_dict(value: Option<&Value>) -> Option<&JsonDict> {
    value?.as_object()
}

/// None unless every element is a dict, like Swift's `as? [[String: Any]]`.
pub fn as_dict_list(value: Option<&Value>) -> Option<Vec<&JsonDict>> {
    value?.as_array()?.iter().map(Value::as_object).collect()
}

pub fn as_string(value: Option<&Value>) -> Option<&str> {
    value?.as_str()
}

/// Nil-safe path walking for InnerTube JSON.
pub struct InnerTubeDecode;

impl InnerTubeDecode {
    /// Walks ["a", "b"] paths. No array indexing - use `array`.
    /// An empty path returns None (Swift returns the dict itself).
    pub fn value<'a>(path: &[&str], dict: &'a JsonDict) -> Option<&'a Value> {
        let (first, rest) = path.split_first()?;
        let mut current = dict.get(*first)?;
        for key in rest {
            current = current.as_object()?.get(*key)?;
        }
        Some(current)
    }

    pub fn dict<'a>(path: &[&str], dict: &'a JsonDict) -> Option<&'a JsonDict> {
        as_dict(Self::value(path, dict))
    }

    pub fn array<'a>(path: &[&str], dict: &'a JsonDict) -> Option<Vec<&'a JsonDict>> {
        as_dict_list(Self::value(path, dict))
    }

    pub fn string<'a>(path: &[&str], dict: &'a JsonDict) -> Option<&'a str> {
        as_string(Self::value(path, dict))
    }

    /// continuations[0].nextContinuationData token.
    pub fn continuation_token(dict: Option<&JsonDict>) -> Option<&str> {
        let first = as_dict_list(dict?.get("continuations"))?
            .into_iter()
            .next()?;
        let next = as_dict(first.get("nextContinuationData"))?;
        as_string(next.get("continuation"))
    }

    /// `lengthSeconds`-style fields arrive as either String or Int.
    pub fn int_from_string_or_int(value: Option<&Value>) -> Option<i64> {
        match value? {
            Value::Number(number) => number
                .as_i64()
                .or_else(|| number.as_f64().map(|f| f as i64)),
            Value::String(string) => string.parse().ok(),
            _ => None,
        }
    }

    /// Runs, simpleText, text, or a bare string.
    pub fn runs_text_tolerant(dict: Option<&Value>) -> Option<String> {
        let dict = dict?;
        if let Some(string) = dict.as_str() {
            return Some(string.to_string());
        }
        Self::runs_text_tolerant_dict(dict.as_object()?)
    }

    /// Dict branch of `runs_text_tolerant`, so callers holding a dict skip a copy.
    pub fn runs_text_tolerant_dict(nested: &JsonDict) -> Option<String> {
        if let Some(runs) = as_dict_list(nested.get("runs")) {
            // Join all runs - some titles split across styled runs.
            let joined: String = runs
                .iter()
                .filter_map(|r| as_string(r.get("text")))
                .collect();
            if !joined.is_empty() {
                return Some(joined);
            }
        }
        // Fallbacks YouTube Music sometimes uses instead of runs.
        if let Some(simple) = as_string(nested.get("simpleText")).filter(|s| !s.is_empty()) {
            return Some(simple.to_string());
        }
        if let Some(text) = as_string(nested.get("text")).filter(|s| !s.is_empty()) {
            return Some(text.to_string());
        }
        None
    }

    /// Largest thumbnail in any known envelope.
    pub fn thumbnail_url_tolerant(dict: Option<&JsonDict>) -> Option<String> {
        let dict = dict?;
        fn last_url(list: Option<Vec<&JsonDict>>) -> Option<String> {
            as_string(list?.last()?.get("url")).map(str::to_string)
        }
        // musicThumbnailRenderer.thumbnail.thumbnails
        last_url(Self::array(
            &[
                "thumbnail",
                "musicThumbnailRenderer",
                "thumbnail",
                "thumbnails",
            ],
            dict,
        ))
        // thumbnail.thumbnails
        .or_else(|| last_url(Self::array(&["thumbnail", "thumbnails"], dict)))
        // bare thumbnails
        .or_else(|| last_url(as_dict_list(dict.get("thumbnails"))))
        // croppedSquareThumbnail
        .or_else(|| last_url(Self::array(&["croppedSquareThumbnail", "thumbnails"], dict)))
    }

    /// One-line diagnostics for unexpected shapes.
    pub fn warn_once(message: &str) {
        SwiftyTubeLog::notice(message);
    }
}
