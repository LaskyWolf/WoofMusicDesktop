use std::sync::LazyLock;

use regex::Regex;

static SIZE_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"w\d+-h\d+").unwrap());
/// Swift `(?<=[sh]\d+)-c`; Rust regex has no lookbehind, so the prefix is captured and put back.
static CROP_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([sh]\d+)-c").unwrap());

/// Artwork URL builders shared by apps.
pub struct ArtworkURLs;

impl ArtworkURLs {
    /// i.ytimg fallback artwork for a video id.
    pub fn fallback(video_id: &str) -> String {
        format!("https://i.ytimg.com/vi/{video_id}/hqdefault.jpg")
    }

    /// Rewrites a `w<N>-h<N>` thumbnail URL to the requested square size.
    pub fn sized(url: Option<&str>, width: i64, height: i64) -> Option<String> {
        Some(
            SIZE_PATTERN
                .replace_all(url?, format!("w{width}-h{height}"))
                .into_owned(),
        )
    }

    // MARK: - Size ladder

    /// One rung per surface so each shares a single cache entry instead of
    /// every view fetching its own size
    pub const ROW_PX: i64 = 160;
    pub const CARD_PX: i64 = 480;
    pub const HEADER_PX: i64 = 720;
    pub const PLAYER_PX: i64 = 1200;

    const RUNGS: [i64; 4] = [
        Self::ROW_PX,
        Self::CARD_PX,
        Self::HEADER_PX,
        Self::PLAYER_PX,
    ];

    /// Smallest ladder rung covering `pixels` (never below the row rung).
    pub fn rung(pixels: f64) -> i64 {
        let need = (pixels.ceil() as i64).max(1);
        Self::RUNGS
            .into_iter()
            .find(|rung| *rung >= need)
            .unwrap_or(Self::PLAYER_PX)
    }

    // MARK: - CDN upgrade

    pub fn artwork(url: Option<&str>, px: i64) -> Option<String> {
        let url = url.filter(|u| !u.is_empty())?;
        let url = CROP_PATTERN.replace_all(url, "$1").into_owned();
        if !url.contains("googleusercontent.com") && !url.contains("ggpht.com") {
            return Some(url);
        }
        if SIZE_PATTERN.is_match(&url) {
            return Some(
                SIZE_PATTERN
                    .replace_all(&url, format!("w{px}-h{px}"))
                    .into_owned(),
            );
        }
        match url.split('=').find(|s| !s.is_empty()) {
            Some(base) => Some(format!("{base}=w{px}-h{px}")),
            None => Some(url),
        }
    }

    /// Convenience: artwork at the player rung
    pub fn high_res(url: Option<&str>) -> Option<String> {
        Self::artwork(url, Self::PLAYER_PX)
    }

    pub fn max_res_variant(url: Option<&str>) -> Option<String> {
        Self::ytimg_variant(url, "maxresdefault.jpg")
    }

    /// 640px variant. Exists far more often than maxres, >hqdefault
    pub fn sd_variant(url: Option<&str>) -> Option<String> {
        Self::ytimg_variant(url, "sddefault.jpg")
    }

    fn ytimg_variant(url: Option<&str>, file: &str) -> Option<String> {
        let url = url.filter(|u| u.contains("i.ytimg.com/vi/"))?;
        let base = url.split('?').find(|s| !s.is_empty()).unwrap_or(url);
        let last_slash = base.rfind('/')?;
        Some(format!("{}/{file}", &base[..last_slash]))
    }
}
