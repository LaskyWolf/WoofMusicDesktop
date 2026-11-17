use std::sync::{Arc, LazyLock};

use regex::Regex;

use super::player_config_store::PlayerConfigStore;

/// Result of extracting cipher functions from player.js.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractedFunctions {
    /// Sig function body, callable.
    pub sig_js: String,
    /// n-transform body, callable.
    pub n_js: Option<String>,
    /// URL builder class on `window.g`.
    pub n_class: Option<String>,
}

static STS_PATTERNS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"signatureTimestamp["':\s]+(\d+)"#).unwrap(),
        Regex::new(r#"signatureTimestamp["']\s*:\s*(\d+)"#).unwrap(),
    ]
});
static FUNC_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bfunction\s*\([^)]*\)\s*\{").unwrap());
static METHOD_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\w+\s*=\s*function\s*\([^)]*\)\s*\{").unwrap());
static N_CLASS_PATTERNS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"\(?new\s+g\.(\w+)\([^,]+,\s*(?:!0|true)\)\)?\s*\.[\w$]+\(\s*['"]n['"]\s*\)"#)
            .unwrap(),
        Regex::new(r"new\s+g\.(\w+)\([^,]+,\s*!0\)").unwrap(),
    ]
});

/// Cipher + n-transform functions from player.js (config first, regex fallback).
pub struct FunctionNameExtractor {
    config_store: Arc<PlayerConfigStore>,
}

impl FunctionNameExtractor {
    pub fn shared() -> Arc<FunctionNameExtractor> {
        static SHARED: LazyLock<Arc<FunctionNameExtractor>> =
            LazyLock::new(|| Arc::new(FunctionNameExtractor::new(PlayerConfigStore::shared())));
        SHARED.clone()
    }

    pub fn new(config_store: Arc<PlayerConfigStore>) -> Self {
        Self { config_store }
    }

    /// Cipher functions from player.js (config lookup, else heuristics).
    pub async fn extract(&self, player_js: &str, player_hash: Option<&str>) -> ExtractedFunctions {
        // Try config store first.
        let config = match player_hash {
            Some(hash) => self.config_store.config(hash).await,
            None => None,
        };
        if let Some(config) = config {
            // If we already have the function bodies, use them.
            let sig = config.sig_function();
            let n = config.n_function();
            if let (Some(sig_body), Some(n_body)) = (&sig.body, &n.body) {
                return ExtractedFunctions {
                    sig_js: Self::wrap_sig_function(sig_body),
                    n_js: Some(Self::wrap_n_function(n_body)),
                    n_class: None,
                };
            }
            // If we have extraction patterns, try those.
            if let Some(sig_body) = sig
                .extract_pattern
                .as_deref()
                .and_then(|p| Self::extract_by_pattern(p, player_js))
            {
                let n_body = match &n.extract_pattern {
                    Some(pattern) => Self::extract_by_pattern(pattern, player_js),
                    None => Self::extract_n_function_heuristic(player_js),
                };
                return ExtractedFunctions {
                    sig_js: Self::wrap_sig_function(&sig_body),
                    n_js: n_body.map(|b| Self::wrap_n_function(&b)),
                    n_class: None,
                };
            }
        }

        // Fallback to heuristic extraction.
        Self::extract_heuristic(player_js)
    }

    /// Extracts the signature timestamp from player.js.
    pub fn extract_signature_timestamp(player_js: &str) -> Option<i64> {
        STS_PATTERNS
            .iter()
            .find_map(|p| p.captures(player_js))
            .and_then(|c| c[1].parse().ok())
    }

    // MARK: - Heuristic Extraction

    /// Swift `extractHeuristic` is private; public here so tests can drive it
    /// without a config store.
    pub fn extract_heuristic(js: &str) -> ExtractedFunctions {
        let sig_body = Self::extract_cipher_function_heuristic(js);
        let n_body = Self::extract_n_function_heuristic(js);
        ExtractedFunctions {
            sig_js: sig_body.map_or(String::new(), |b| Self::wrap_sig_function(&b)),
            n_js: n_body.map(|b| Self::wrap_n_function(&b)),
            n_class: Self::extract_n_class_heuristic(js),
        }
    }

    /// Deobfuscation fn: needs .split("") + .join("").
    fn extract_cipher_function_heuristic(js: &str) -> Option<String> {
        Self::scan_functions(js, Self::has_cipher_patterns, 3000)
    }

    /// n-transform fn: needs charCodeAt + fromCharCode.
    fn extract_n_function_heuristic(js: &str) -> Option<String> {
        Self::scan_functions(js, Self::has_n_transform_patterns, 3000)
    }

    /// First function body passing the validator.
    fn scan_functions(
        js: &str,
        body_validator: fn(&str) -> bool,
        max_body_length: usize,
    ) -> Option<String> {
        let body_at = |start: usize| {
            let end = Self::find_matching_brace(js, start)?;
            (end - start + 1 < max_body_length).then(|| &js[start..=end])
        };

        for found in FUNC_REGEX.find_iter(js) {
            if let Some(body) = body_at(found.start())
                && body_validator(body)
            {
                return Some(body.to_string());
            }
        }

        // Try alternate: method-assignment form (e.g. `x.y = function(...){...}`).
        for found in METHOD_REGEX.find_iter(js) {
            let Some(body) = body_at(found.start()) else {
                continue;
            };
            if body_validator(body)
                && let Some(func_start) = body.find("function")
            {
                let func_str = &body[func_start..];
                if body_validator(func_str) {
                    return Some(func_str.to_string());
                }
            }
        }

        None
    }

    /// A cipher function must contain both `.split("")` and `.join("")`.
    fn has_cipher_patterns(body: &str) -> bool {
        let has_split = body.contains(".split(\"\"") || body.contains(".split(''");
        let has_join = body.contains(".join(\"\"") || body.contains(".join(''");
        has_split && has_join
    }

    /// An n-transform function must invoke `charCodeAt` and `fromCharCode`.
    fn has_n_transform_patterns(body: &str) -> bool {
        body.contains("charCodeAt") && body.contains("fromCharCode")
    }

    /// Extract the URL builder class name (nClass) from player.js heuristically.
    pub fn extract_n_class_heuristic(js: &str) -> Option<String> {
        N_CLASS_PATTERNS
            .iter()
            .find_map(|p| p.captures(js))
            .map(|c| c[1].to_string())
    }

    // MARK: - Brace Matching

    /// Closing-brace match from the first `{` at/after `start`. Byte offsets:
    /// every character it looks for is ASCII.
    fn find_matching_brace(string: &str, start: usize) -> Option<usize> {
        let open_offset = start + string[start..].find('{')?;

        let mut depth = 0;
        let mut in_single_quote = false;
        let mut in_double_quote = false;
        let mut escaped = false;

        for (offset, &char) in string.as_bytes().iter().enumerate().skip(open_offset) {
            if escaped {
                escaped = false;
                continue;
            }
            match char {
                b'\\' => escaped = true,
                b'\'' if !in_double_quote => in_single_quote = !in_single_quote,
                b'"' if !in_single_quote => in_double_quote = !in_double_quote,
                b'{' if !in_single_quote && !in_double_quote => depth += 1,
                b'}' if !in_single_quote && !in_double_quote => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(offset);
                    }
                }
                _ => {}
            }
        }
        None
    }

    // MARK: - Pattern Extraction

    /// Extract a function body using a regex pattern.
    fn extract_by_pattern(pattern: &str, js: &str) -> Option<String> {
        let found = Regex::new(pattern).ok()?.find(js)?;
        let end = Self::find_matching_brace(js, found.start())?;
        Some(js[found.start()..=end].to_string())
    }

    // MARK: - Wrapping

    /// Wrap the extracted cipher function body as a callable JS expression.
    fn wrap_sig_function(body: &str) -> String {
        format!("({body})")
    }

    /// Wrap the extracted n-transform function body as a callable JS expression.
    fn wrap_n_function(body: &str) -> String {
        format!("({body})")
    }
}
