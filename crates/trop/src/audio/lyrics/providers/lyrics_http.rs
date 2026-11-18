//! Trop/Audio/Lyrics/Providers/LyricsHTTP.swift. Swift's `decoded` (`Decodable`)
//! has no Rust twin: JSON stays untyped, so Kugou and Genius use `get_json`.

use reqwest::header::{HeaderName, HeaderValue};
use reqwest::{Method, Request, Url};
use serde_json::Value;
use swiftytube::networking::http_client::{HttpClient, Session};
use swiftytube::networking::inner_tube_decode::JsonDict;

use super::lrclib_provider::LyricsError;
use crate::audio::lyrics::lyrics_service::LyricLine;

/// Shared request boilerplate for lyrics providers: perform the request,
/// require HTTP 200 (anything else is `NotFound`), and decode the body.
pub struct LyricsHTTP;

impl LyricsHTTP {
    /// A GET request for `url` with `query` items and `headers`; `InvalidUrl` when `url` is malformed.
    pub fn request(
        url: &str,
        query: &[(&str, &str)],
        headers: &[(&str, &str)],
    ) -> Result<Request, LyricsError> {
        let url = if query.is_empty() {
            Url::parse(url)
        } else {
            Url::parse_with_params(url, query)
        }
        .map_err(|_| LyricsError::InvalidUrl)?;
        let mut request = Request::new(Method::GET, url);
        for (name, value) in headers {
            if let (Ok(name), Ok(value)) =
                (HeaderName::try_from(*name), HeaderValue::from_str(value))
            {
                request.headers_mut().insert(name, value);
            }
        }
        Ok(request)
    }

    /// GETs `request`, failing with `NotFound` unless the status is exactly 200.
    pub async fn get(request: Request, session: &Session) -> Result<Vec<u8>, LyricsError> {
        let response = HttpClient::data(request, session).await?;
        if response.status != 200 {
            return Err(LyricsError::NotFound);
        }
        Ok(response.data)
    }

    /// `get` + JSON object decode, failing with `DecodingFailed` on bad JSON.
    pub async fn get_json(request: Request, session: &Session) -> Result<JsonDict, LyricsError> {
        match serde_json::from_slice(&Self::get(request, session).await?) {
            Ok(Value::Object(json)) => Ok(json),
            _ => Err(LyricsError::DecodingFailed),
        }
    }
}

/// Swift `json[key] as? Bool`: `JSONSerialization` numbers 0 and 1 bridge to
/// `Bool` too, and Musixmatch sends its flags as 0 / 1.
pub fn as_bool(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(flag) => Some(*flag),
        Value::Number(n) => match n.as_f64()? {
            0.0 => Some(false),
            1.0 => Some(true),
            _ => None,
        },
        _ => None,
    }
}

/// Shared plain-text line splitting for lyrics providers.
pub struct LyricsText;

impl LyricsText {
    /// Splits on newlines, trims, drops empties: untimed lines.
    pub fn plain_lines(s: &str) -> Vec<LyricLine> {
        s.split(['\n', '\r'])
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| LyricLine::new(line, None))
            .collect()
    }
}
