// Trop/Utilities/MusicLink.swift and Trop/Utilities/ShazamLink.swift.

use reqwest::Url;

/// A YouTube or YouTube Music link, read from its text alone, with no network call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MusicLink {
    Song(String),
    Playlist(String),
    Album(String),
    Artist(String),
    Podcast(String),
    /// Names its song only on the page behind it; the app looks it up.
    Shazam(String),
}

impl MusicLink {
    /// The link inside woofmusic://open?url=<encoded link>.
    pub fn wrapped_link(open_url: &str) -> Option<String> {
        let url = Url::parse(open_url).ok()?;
        url.query_pairs()
            .find(|(name, _)| name == "url")
            .map(|(_, value)| value.into_owned())
    }

    pub fn parse(link: &str) -> Option<Self> {
        let url = Url::parse(link.trim()).ok()?;
        if !["http", "https"].contains(&url.scheme()) {
            return None;
        }
        let host = url.host_str()?.to_ascii_lowercase();
        let path: Vec<&str> = url
            .path_segments()
            .map(|segments| segments.filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();
        let query = |name: &str| {
            url.query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        if host == "youtu.be" {
            return match path.as_slice() {
                [id] => Self::video(id),
                _ => None,
            };
        }
        if [
            "youtube.com",
            "www.youtube.com",
            "m.youtube.com",
            "music.youtube.com",
        ]
        .contains(&host.as_str())
        {
            return match path.as_slice() {
                ["watch", ..] if query("v").is_some() => Self::video(&query("v")?),
                ["watch" | "playlist", ..] => Self::playlist(&query("list").unwrap_or_default()),
                ["shorts" | "embed" | "live", id] => Self::video(id),
                // The app's own artist share links use /channel; YouTube Music shares podcasts as /podcast.
                ["browse" | "channel" | "podcast", id] => Self::browse(id),
                _ => None,
            };
        }
        is_shazam(&host, &path).then(|| Self::Shazam(link.trim().to_owned()))
    }

    fn video(id: &str) -> Option<Self> {
        (id.len() == 11 && is_id(id)).then(|| Self::Song(id.to_owned()))
    }

    fn playlist(id: &str) -> Option<Self> {
        is_id(id).then(|| Self::Playlist(id.to_owned()))
    }

    fn browse(id: &str) -> Option<Self> {
        if !is_id(id) {
            return None;
        }
        let id = id.to_owned();
        if id.starts_with("MPREb_") {
            Some(Self::Album(id))
        } else if id.starts_with("UC") || id.starts_with("MPCH") {
            Some(Self::Artist(id))
        } else if id.starts_with("VL") {
            Some(Self::Playlist(id))
        } else if id.starts_with("MPSP") {
            Some(Self::Podcast(id))
        } else {
            None
        }
    }
}

fn is_id(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Whether a link's host is Shazam's (shz.am is the short link the app shares).
fn is_shazam(host: &str, path: &[&str]) -> bool {
    match host {
        "shz.am" => !path.is_empty(),
        "shazam.com" | "www.shazam.com" => matches!(path.first(), Some(&"track" | &"song")),
        _ => false,
    }
}

/// A Shazam song, as its public page names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShazamTrack {
    pub title: String,
    pub artist: String,
}

/// A phone browser's identity: the page names the song in its meta tags for it.
pub const SHAZAM_USER_AGENT: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";

/// "Listen to <title> by <artist>. See lyrics…" in og:description; the title
/// "<title> - <artist>: Song Lyrics…" is the fallback. The last " by " and
/// " - " split them, because titles contain those and artists rarely do.
pub fn shazam_track(html: &str) -> Option<ShazamTrack> {
    let split = |text: String, pattern: &str| {
        let caps = regex::Regex::new(pattern).ok()?.captures(&text)?;
        Some(ShazamTrack {
            title: caps[1].to_owned(),
            artist: caps[2].to_owned(),
        })
    };
    meta("og:description", html)
        .and_then(|text| split(text, r"Listen to (.+) by (.+?)\. See lyrics"))
        .or_else(|| {
            meta("og:title", html).and_then(|text| split(text, r"(.+) - (.+?): Song Lyrics"))
        })
}

fn meta(property: &str, html: &str) -> Option<String> {
    let pattern = format!(
        r#"<meta property="{}" content="([^"]*)""#,
        regex::escape(property)
    );
    let caps = regex::Regex::new(&pattern).ok()?.captures(html)?;
    Some(
        caps[1]
            .replace("&#x27;", "'")
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_youtube_links() {
        let song = Some(MusicLink::Song("dQw4w9WgXcQ".into()));
        assert_eq!(MusicLink::parse("https://youtu.be/dQw4w9WgXcQ"), song);
        assert_eq!(
            MusicLink::parse("https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=RDAMVM"),
            song
        );
        assert_eq!(
            MusicLink::parse("https://www.youtube.com/shorts/dQw4w9WgXcQ"),
            song
        );
        assert_eq!(
            MusicLink::parse("https://music.youtube.com/playlist?list=PL123-_x"),
            Some(MusicLink::Playlist("PL123-_x".into()))
        );
        assert_eq!(
            MusicLink::parse("https://music.youtube.com/browse/MPREb_abc"),
            Some(MusicLink::Album("MPREb_abc".into()))
        );
        assert_eq!(
            MusicLink::parse("https://music.youtube.com/channel/UCabc"),
            Some(MusicLink::Artist("UCabc".into()))
        );
        assert_eq!(
            MusicLink::parse("https://music.youtube.com/podcast/MPSPabc"),
            Some(MusicLink::Podcast("MPSPabc".into()))
        );
        assert_eq!(MusicLink::parse("https://youtu.be/short"), None);
        assert_eq!(MusicLink::parse("ftp://youtu.be/dQw4w9WgXcQ"), None);
        assert_eq!(MusicLink::parse("billie eilish"), None);
        assert_eq!(
            MusicLink::parse("https://example.com/watch?v=dQw4w9WgXcQ"),
            None
        );
    }

    #[test]
    fn reads_wrapped_and_shazam_links() {
        assert_eq!(
            MusicLink::wrapped_link("woofmusic://open?url=https%3A%2F%2Fyoutu.be%2FdQw4w9WgXcQ")
                .as_deref(),
            Some("https://youtu.be/dQw4w9WgXcQ")
        );
        assert!(matches!(
            MusicLink::parse("https://www.shazam.com/track/52331483/dtmf"),
            Some(MusicLink::Shazam(_))
        ));
        assert_eq!(MusicLink::parse("https://www.shazam.com/charts"), None);
    }

    #[test]
    fn reads_shazam_pages() {
        let page = r#"<meta property="og:description" content="Listen to Stand by Me by Ben E. King. See lyrics and music videos, find tour dates."/>"#;
        assert_eq!(
            shazam_track(page),
            Some(ShazamTrack {
                title: "Stand by Me".into(),
                artist: "Ben E. King".into()
            })
        );
        let title_only = r#"<meta property="og:title" content="Rock &amp; Roll - AC/DC: Song Lyrics, Music Videos &amp; Concerts"/>"#;
        assert_eq!(
            shazam_track(title_only),
            Some(ShazamTrack {
                title: "Rock & Roll".into(),
                artist: "AC/DC".into()
            })
        );
        assert_eq!(shazam_track("<html></html>"), None);
    }
}
