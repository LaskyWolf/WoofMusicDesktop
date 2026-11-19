//! Lyrics parsing, providers, fallback order and line lookup, offline.
//! The `parsing_*` tests port the Swift `LyricsParsing.selfCheck`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use reqwest::Url;
use swiftytube::networking::http_client::{HttpResponse, Session};
use trop::audio::lyrics::lyrics_manager::{LyricsManager, LyricsSettings};
use trop::audio::lyrics::lyrics_service::{LyricLine, LyricWord, LyricsService, LyricsSong};
use trop::audio::lyrics::lyrics_view::{active_line_index, detect_gaps, display_lines};
use trop::audio::lyrics::providers::genius_provider::GeniusProvider;
use trop::audio::lyrics::providers::kugou_provider::KugouProvider;
use trop::audio::lyrics::providers::lrclib_provider::{LRCLIBProvider, LyricsError};
use trop::audio::lyrics::providers::lyrics_provider::{
    LyricsFuture, LyricsParsing, LyricsProvider, LyricsQuery,
};
use trop::audio::lyrics::providers::musixmatch_provider::MusixmatchProvider;
use trop::audio::lyrics::providers::netease_provider::NeteaseProvider;
use trop::utilities::string_app::StringApp;

// Helpers.

fn near(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 0.0005)
}

fn texts(lines: &[LyricLine]) -> Vec<&str> {
    lines.iter().map(|l| l.text.as_str()).collect()
}

fn word(text: &str, start: f64, end: Option<f64>) -> LyricWord {
    LyricWord {
        text: text.into(),
        start_time: start,
        end_time: end,
    }
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../fixtures/lyrics/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// A session answering each request with `route(url)`, or 404; also returns the requested URLs.
fn session(
    route: impl Fn(&Url) -> Option<Vec<u8>> + Send + Sync + 'static,
) -> (Session, Arc<Mutex<Vec<Url>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let session = Session::mock(move |request| {
        log.lock().unwrap().push(request.url().clone());
        match route(request.url()) {
            Some(data) => HttpResponse {
                status: 200,
                data,
                ..HttpResponse::default()
            },
            None => HttpResponse {
                status: 404,
                ..HttpResponse::default()
            },
        }
    });
    (session, seen)
}

/// `host/path` of a URL.
fn at(url: &Url) -> String {
    format!("{}{}", url.host_str().unwrap_or(""), url.path())
}

fn param(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

fn query() -> LyricsQuery {
    LyricsQuery {
        title: "Sample Song".into(),
        artist: "Sample Artist".into(),
        album: Some("Sample Album".into()),
        duration: 200.0,
    }
}

// Parsing (Swift selfCheck).

#[test]
fn parsing_enhanced_lrc_words_survive_storage() {
    let enhanced = LyricsParsing::parse_lrc("[00:12.00]<00:12.00>Never <00:12.50>gonna<00:13.25>");
    assert_eq!(enhanced.len(), 1);
    assert_eq!(enhanced[0].text, "Never gonna");
    assert_eq!(enhanced[0].start_time, Some(12.0));
    assert_eq!(
        enhanced[0].words,
        Some(vec![
            word("Never ", 12.0, Some(12.5)),
            word("gonna", 12.5, Some(13.25))
        ])
    );
    let stored = LyricsParsing::serialize_lines(&enhanced);
    assert_eq!(
        LyricsParsing::parse_stored_text(&stored)[0].words,
        enhanced[0].words
    );
}

#[test]
fn parsing_plain_lrc_line_has_no_words() {
    let plain = LyricsParsing::parse_lrc("[01:02.50]Give you up");
    assert_eq!(texts(&plain), ["Give you up"]);
    assert_eq!(plain[0].start_time, Some(62.5));
    assert_eq!(plain[0].words, None);
}

#[test]
fn parsing_klyric_durations_add_up_from_line_start() {
    let klyric =
        LyricsParsing::parse_klyric("[ar:Someone]\n[1000,700](0,500)Don't(0,1) (0,199)want");
    assert_eq!(texts(&klyric), ["Don't want"]);
    assert_eq!(klyric[0].start_time, Some(1.0));
    let words = klyric[0].words.as_ref().unwrap();
    assert_eq!(
        words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
        ["Don't ", "want"]
    );
    assert!(near(Some(words[1].start_time), 1.501) && near(words[1].end_time, 1.7));
}

#[test]
fn parsing_richsync_offsets_and_single_piece_lines() {
    let rich = LyricsParsing::parse_richsync(
        r#"[{"ts":5.0,"te":6.0,"l":[{"c":"Hi","o":0},{"c":" ","o":0.3},{"c":"there","o":0.4}]}]"#,
    );
    assert_eq!(texts(&rich), ["Hi there"]);
    let words = rich[0].words.as_ref().unwrap();
    assert_eq!(words.len(), 2);
    assert!(near(words[0].end_time, 5.3));
    assert!(near(Some(words[1].start_time), 5.4) && near(words[1].end_time, 6.0));

    // Text without spaces: one piece per line is line timing, not karaoke.
    let ja = LyricsParsing::parse_richsync(
        r#"[{"ts":1,"te":2,"l":[{"c":"あなたは","o":0}]},{"ts":3,"te":4,"l":[{"c":"形だけ","o":0},{"c":" ","o":0.3},{"c":"今日","o":0.4}]},{"ts":5,"te":6,"l":[{"c":"言うけど","o":0}]}]"#,
    );
    assert_eq!(texts(&ja), ["あなたは", "形だけ 今日", "言うけど"]);
    assert_eq!(ja[1].start_time, Some(3.0));
    assert!(ja.iter().all(|l| l.words.is_none()));
}

#[test]
fn parsing_netease_yrc_word_starts_are_song_time() {
    let yrc = LyricsParsing::parse_word_timed_lines(
        "{\"t\":0,\"c\":[{\"tx\":\"Lyrics\"}]}\n[27360,1290](27360,240,0)I've (27600,90,0)been (27690,360,0)tryna",
        false,
    );
    assert_eq!(texts(&yrc), ["I've been tryna"]);
    assert!(near(yrc[0].start_time, 27.36));
    let words = yrc[0].words.as_ref().unwrap();
    assert_eq!(
        words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(),
        ["I've ", "been ", "tryna"]
    );
    assert!(near(Some(words[1].start_time), 27.6) && near(words[2].end_time, 28.05));
}

#[test]
fn parsing_kugou_krc_decodes_and_offsets_from_line_start() {
    let file = STANDARD
        .decode("a3JjMTjb6lmXhgaLev/l2Ib/pOalzVdC74agdmUDAaV7475Ydne2Ru7BIPdQB71h46ZihXJVqiVLXjiKHUMdoIOnYoUKdVP85zL4oEDp")
        .unwrap();
    let text = LyricsParsing::decode_krc(&file).unwrap();
    assert!(text.starts_with("[ti:Anti-Hero]\n[5499,1500]<0,307,0>I "));
    let krc = LyricsParsing::parse_word_timed_lines(&text, true);
    assert_eq!(texts(&krc), ["I have this"]);
    assert!(near(krc[0].start_time, 5.499));
    let words = krc[0].words.as_ref().unwrap();
    assert!(near(Some(words[1].start_time), 5.806) && near(words[2].end_time, 6.999));
    assert_eq!(LyricsParsing::decode_krc(b"not krc"), None);
}

#[test]
fn parsing_credits_and_placeholders_drop() {
    let lines = |t: &[&str]| {
        t.iter()
            .map(|t| LyricLine::new(*t, None))
            .collect::<Vec<_>>()
    };
    let credited = lines(&[
        "AIYOU - Eve",
        "词：Eve",
        "编曲：Eve/Numa",
        "Lyrics by: Someone",
        "あなたは",
        "Note: still sung",
    ]);
    assert_eq!(
        texts(&LyricsParsing::dropping_credits(credited)),
        ["あなたは", "Note: still sung"]
    );
    assert!(
        LyricsParsing::dropping_placeholder(vec![LyricLine::new("纯音乐，请欣赏", Some(0.0))])
            .is_empty()
    );
    let sung = lines(&["纯音乐", "line 2", "line 3", "line 4"]);
    assert_eq!(LyricsParsing::dropping_placeholder(sung).len(), 4);
    assert_eq!(
        LyricsParsing::dropping_credits(lines(&["Night - Day", "First line"])).len(),
        2
    );
}

// LRC details.

#[test]
fn lrc_timestamps_with_two_or_three_digit_fractions_or_none() {
    let lines =
        LyricsParsing::parse_lrc("[00:01]One\n[0:02.5]Two\r\n[00:03.250]Three\n[10:00.01]Four");
    assert_eq!(texts(&lines), ["One", "Two", "Three", "Four"]);
    let times: Vec<f64> = lines.iter().map(|l| l.start_time.unwrap()).collect();
    assert_eq!(times, [1.0, 2.5, 3.25, 600.01]);
}

#[test]
fn lrc_with_byte_order_mark_keeps_first_line() {
    let lines = LyricsParsing::parse_lrc("\u{FEFF}[00:01.00]hello\n[00:02.00]world");
    assert_eq!(texts(&lines), ["hello", "world"]);
}

#[test]
fn lyrics_past_100_minutes_round_trip() {
    let mut lines = LyricsParsing::parse_lrc("[99:59.00]a");
    lines[0].start_time = Some(6000.0 + 5.0);
    let raw = LyricsParsing::serialize_lines(&lines);
    assert_eq!(raw, "[100:05.00]a");
    let back = LyricsParsing::parse_stored_text(&raw);
    assert_eq!(back[0].start_time, Some(6005.0));
}

#[test]
fn compressed_lrc_line_shows_at_every_timestamp() {
    let lines = LyricsParsing::parse_lrc("[00:10.00][01:20.00]Chorus\n[00:20.00]Verse [00:99]");
    assert_eq!(texts(&lines), ["Chorus", "Verse [00:99]", "Chorus"]);
    let times: Vec<f64> = lines.iter().map(|l| l.start_time.unwrap()).collect();
    assert_eq!(times, [10.0, 20.0, 80.0]);
    assert_ne!(lines[0].id, lines[2].id);
}

#[test]
fn out_of_order_lrc_lines_are_sorted_by_time() {
    let lines = LyricsParsing::parse_lrc("[00:20.00]second\n[00:10.00]first\n[00:30.00]third");
    assert_eq!(texts(&lines), ["first", "second", "third"]);
    assert_eq!(lines[active_line_index(&lines, 15.0)].text, "first");
}

#[test]
fn lrc_metadata_tags_are_skipped_and_offset_shows_lyrics_earlier() {
    let lines = LyricsParsing::parse_lrc(
        "[ar:Sample Artist]\n[ti:Sample Song]\n[offset:+500]\n[00:00.20]Intro\n[00:10.00]<00:10.00>Line",
    );
    assert_eq!(texts(&lines), ["Intro", "Line"]);
    assert_eq!(lines[0].start_time, Some(0.0));
    assert_eq!(lines[1].start_time, Some(9.5));
    assert_eq!(lines[1].words.as_ref().unwrap()[0].start_time, 9.5);
    let later = LyricsParsing::parse_lrc("[offset:-250]\n[00:10.00]Line");
    assert_eq!(later[0].start_time, Some(10.25));
}

#[test]
fn lrc_blank_timed_lines_are_kept_as_instrumental_markers() {
    let lines = LyricsParsing::parse_lrc("[00:01.00]Sung\n[00:05.00]\n[00:20.00]Again");
    assert_eq!(texts(&lines), ["Sung", "", "Again"]);
}

#[test]
fn stored_text_falls_back_to_plain_lines_and_round_trips() {
    let plain = LyricsParsing::parse_stored_text("  First \n\nSecond\n");
    assert_eq!(texts(&plain), ["First", "Second"]);
    assert!(plain.iter().all(|l| l.start_time.is_none()));
    assert_eq!(LyricsParsing::serialize_lines(&plain), "First\nSecond");
    let synced = LyricsParsing::parse_lrc("[00:01.50]First\n[01:04.00]Second");
    assert_eq!(
        LyricsParsing::serialize_lines(&synced),
        "[00:01.50]First\n[01:04.00]Second"
    );
    assert_eq!(LyricsParsing::plain_text(&synced), "First\nSecond");
}

#[test]
fn untimed_line_in_saved_lrc_keeps_its_place() {
    let raw = "[ar:Sample Artist]\n[00:03.00]three\n[00:01.00]one\nadded line\n[00:02.00]two";
    let lines = LyricsParsing::parse_stored_text(raw);
    assert_eq!(texts(&lines), ["one", "added line", "two", "three"]);
    assert_eq!(lines[1].start_time, None);
    assert_eq!(
        LyricsParsing::serialize_lines(&lines),
        "[00:01.00]one\nadded line\n[00:02.00]two\n[00:03.00]three"
    );
}

fn edit_round_trip(raw: &str) -> Vec<LyricLine> {
    let first = LyricsParsing::parse_stored_text(raw);
    let text = LyricsParsing::serialize_lines(&first);
    let second = LyricsParsing::parse_stored_text(&text);
    assert_eq!(
        texts(&first),
        texts(&second),
        "text changed by the editor round trip of {raw:?}"
    );
    let times = |l: &[LyricLine]| l.iter().map(|x| x.start_time).collect::<Vec<_>>();
    assert_eq!(times(&first), times(&second), "{raw:?}");
    let words = |l: &[LyricLine]| l.iter().map(|x| x.words.clone()).collect::<Vec<_>>();
    assert_eq!(words(&first), words(&second), "{raw:?} -> {text:?}");
    second
}

#[test]
fn editor_round_trip_keeps_parsing_fixes_and_odd_word_timing() {
    // BOM, compressed lines, offset header, untimed line.
    let lines = edit_round_trip(
        "\u{FEFF}[offset:+500]\n[00:10.00][01:20.00]Chorus\nadded\n[00:20.00]<00:20.00>Verse <00:20.00>zero<00:19.00>",
    );
    assert_eq!(lines.len(), 4);
    // Zero length, overlapping and backwards words, romanized text.
    edit_round_trip("[00:01.00]<00:01.00>a<00:01.00><00:01.00>b<00:03.00> <00:02.00>c<00:01.50>");
    edit_round_trip("[00:01.00]<00:01.00>안녕<00:01.20> <00:01.20>an<00:01.40>nyeong<00:02.00>");
    edit_round_trip("[00:01.00]<00:01.00>x<00:01.00>\n[00:01.00]<00:01.00>y");
    edit_round_trip("[00:05.00]\n[00:06.00]word\n[00:07.00]");
    edit_round_trip("[00:01.00]a <3 b\n[00:02.00]<tag> not a time");
}

// Providers, over a mock session.

#[tokio::test]
async fn lrclib_parses_synced_lyrics_and_sends_the_duration() {
    let (session, seen) =
        session(|url| (at(url) == "lrclib.net/api/get").then(|| fixture("lrclib_get.json")));
    let lines = LRCLIBProvider.fetch(&query(), &session).await.unwrap();
    assert_eq!(
        texts(&lines),
        [
            "First sample line",
            "Second sample line",
            "",
            "Third sample line"
        ]
    );
    assert_eq!(lines[3].start_time, Some(9.0));
    let url = seen.lock().unwrap()[0].clone();
    assert_eq!(param(&url, "duration").as_deref(), Some("200"));
    assert_eq!(param(&url, "album_name").as_deref(), Some("Sample Album"));
}

#[tokio::test]
async fn lrclib_falls_back_to_plain_lyrics() {
    let (session, _) = session(|_| Some(fixture("lrclib_get_plain.json")));
    let lines = LRCLIBProvider.fetch(&query(), &session).await.unwrap();
    assert_eq!(texts(&lines), ["First sample line", "Second sample line"]);
    assert!(lines.iter().all(|l| l.start_time.is_none()));
}

#[tokio::test]
async fn lrclib_instrumental_and_not_found() {
    let (instrumental, _) = session(|_| Some(br#"{"instrumental":true}"#.to_vec()));
    let lines = LRCLIBProvider.fetch(&query(), &instrumental).await.unwrap();
    assert_eq!(texts(&lines), ["♪ Instrumental ♪"]);
    let (missing, _) = session(|_| None);
    let error = LRCLIBProvider.fetch(&query(), &missing).await.unwrap_err();
    assert!(matches!(error, LyricsError::NotFound) && error.is_answer());
}

fn musixmatch_session(macro_fixture: Vec<u8>) -> Session {
    session(move |url| match url.path() {
        "/ws/1.1/token.get" => Some(fixture("musixmatch_token.json")),
        "/ws/1.1/macro.subtitles.get" => Some(macro_fixture.clone()),
        _ => None,
    })
    .0
}

#[tokio::test]
async fn musixmatch_prefers_richsync_word_timing() {
    let lines = MusixmatchProvider
        .fetch(
            &query(),
            &musixmatch_session(fixture("musixmatch_richsync.json")),
        )
        .await
        .unwrap();
    assert_eq!(texts(&lines), ["First sample line", "Second line"]);
    assert_eq!(
        lines[0].words,
        Some(vec![
            word("First ", 1.5, Some(1.9)),
            word("sample ", 2.0, Some(2.4)),
            word("line", 2.5, Some(3.0)),
        ])
    );
}

#[tokio::test]
async fn musixmatch_falls_back_to_subtitles_and_skips_blank_lines() {
    let lines = MusixmatchProvider
        .fetch(
            &query(),
            &musixmatch_session(fixture("musixmatch_subtitles.json")),
        )
        .await
        .unwrap();
    assert_eq!(texts(&lines), ["First sample line", "Second sample line"]);
    assert_eq!(lines[1].start_time, Some(4.0));
    assert!(lines.iter().all(|l| l.words.is_none()));
}

#[tokio::test]
async fn musixmatch_restricted_lyrics_are_not_found() {
    let restricted = String::from_utf8(fixture("musixmatch_richsync.json"))
        .unwrap()
        .replace("\"restricted\":0", "\"restricted\":1");
    let error = MusixmatchProvider
        .fetch(&query(), &musixmatch_session(restricted.into_bytes()))
        .await
        .unwrap_err();
    assert!(matches!(error, LyricsError::NotFound));
    // Musixmatch sends its flags as 0 / 1.
    let instrumental = String::from_utf8(fixture("musixmatch_richsync.json"))
        .unwrap()
        .replace("\"instrumental\":0", "\"instrumental\":1");
    let lines = MusixmatchProvider
        .fetch(&query(), &musixmatch_session(instrumental.into_bytes()))
        .await
        .unwrap();
    assert_eq!(texts(&lines), ["♪ Instrumental ♪"]);
}

#[tokio::test]
async fn netease_picks_the_best_scored_song_and_its_yrc() {
    let (session, seen) = session(|url| match url.path() {
        "/api/search/get" => Some(fixture("netease_search.json")),
        "/api/song/lyric" => Some(fixture("netease_yrc.json")),
        _ => None,
    });
    let lines = NeteaseProvider.fetch(&query(), &session).await.unwrap();
    assert_eq!(texts(&lines), ["First sample line", "Second sample line"]);
    assert_eq!(lines[0].words.as_ref().unwrap().len(), 3);
    let lyric_url = seen.lock().unwrap()[1].clone();
    assert_eq!(param(&lyric_url, "id").as_deref(), Some("2222"));
    assert_eq!(param(&lyric_url, "yv").as_deref(), Some("-1"));
}

#[tokio::test]
async fn netease_lrc_keeps_credits_for_the_manager_to_drop() {
    let (session, _) = session(|url| match url.path() {
        "/api/search/get" => Some(fixture("netease_search.json")),
        "/api/song/lyric" => Some(fixture("netease_lrc.json")),
        _ => None,
    });
    let lines = NeteaseProvider.fetch(&query(), &session).await.unwrap();
    assert_eq!(lines.len(), 4);
    assert_eq!(
        texts(&LyricsParsing::dropping_credits(lines)),
        ["First sample line", "Second sample line"]
    );
}

#[tokio::test]
async fn kugou_matches_by_duration_and_decodes_krc() {
    let (session, seen) = session(|url| match at(url).as_str() {
        "mobileservice.kugou.com/api/v3/search/song" => Some(fixture("kugou_search_song.json")),
        "lyrics.kugou.com/search" => Some(fixture("kugou_search_lyrics.json")),
        "lyrics.kugou.com/download" => Some(fixture("kugou_download_krc.json")),
        _ => None,
    });
    let lines = KugouProvider.fetch(&query(), &session).await.unwrap();
    assert_eq!(texts(&lines), ["I have this"]);
    assert!(lines[0].words.is_some());
    let seen = seen.lock().unwrap();
    // The 400 s song is skipped: only the 200 s song's hash is searched.
    assert_eq!(
        param(&seen[1], "hash").as_deref(),
        Some("8D3E1A2B4C5D6E7F8091A2B3C4D5E6F7")
    );
    assert_eq!(param(&seen[2], "fmt").as_deref(), Some("krc"));
    assert_eq!(
        param(&seen[2], "accesskey").as_deref(),
        Some("FIXTUREACCESSKEY")
    );
}

#[tokio::test]
async fn kugou_falls_back_to_lrc() {
    let (session, _) = session(|url| match at(url).as_str() {
        "mobileservice.kugou.com/api/v3/search/song" => Some(fixture("kugou_search_song.json")),
        "lyrics.kugou.com/search" => Some(fixture("kugou_search_lyrics.json")),
        "lyrics.kugou.com/download" if param(url, "fmt").as_deref() == Some("lrc") => {
            Some(fixture("kugou_download_lrc.json"))
        }
        _ => None,
    });
    let lines = KugouProvider.fetch(&query(), &session).await.unwrap();
    assert_eq!(texts(&lines), ["First sample line", "Second sample line"]);
    assert_eq!(lines[1].start_time, Some(4.5));
}

#[tokio::test]
async fn genius_scrapes_the_lyrics_containers() {
    let (session, _) = session(|url| match at(url).as_str() {
        "genius.com/api/search/song" => Some(fixture("genius_search.json")),
        "genius.com/Sample-artist-sample-song-lyrics" => Some(fixture("genius_page.html")),
        _ => None,
    });
    let lines = GeniusProvider.fetch(&query(), &session).await.unwrap();
    assert_eq!(
        texts(&lines),
        [
            "Sample Song Lyrics[Verse 1]",
            "First sample line",
            "Rock & roll 'n' blues",
            "Third line",
            "[Chorus]",
            "Fourth sample line \"quoted\"",
        ]
    );
}

#[test]
fn html_entities_decode() {
    assert_eq!(
        "a &amp;lt; b &#233; &#x41; &quot;".decoding_html_entities(),
        "a &lt; b é A \""
    );
}

// Fallback order.

struct Fake {
    id: &'static str,
    name: &'static str,
    delay_ms: u64,
    answer: fn() -> Result<Vec<LyricLine>, LyricsError>,
    calls: AtomicUsize,
}

impl LyricsProvider for Fake {
    fn id(&self) -> &'static str {
        self.id
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn fetch<'a>(&'a self, _: &'a LyricsQuery, _: &'a Session) -> LyricsFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            (self.answer)()
        })
    }
}

fn fake(
    id: &'static str,
    delay_ms: u64,
    answer: fn() -> Result<Vec<LyricLine>, LyricsError>,
) -> Arc<Fake> {
    Arc::new(Fake {
        id,
        name: id,
        delay_ms,
        answer,
        calls: AtomicUsize::new(0),
    })
}

fn synced() -> Result<Vec<LyricLine>, LyricsError> {
    Ok(LyricsParsing::parse_lrc("[00:01.00]Synced line"))
}

fn word_timed() -> Result<Vec<LyricLine>, LyricsError> {
    Ok(LyricsParsing::parse_lrc(
        "[00:01.00]<00:01.00>Word <00:01.50>timed",
    ))
}

fn plain() -> Result<Vec<LyricLine>, LyricsError> {
    Ok(vec![LyricLine::new("Plain line", None)])
}

fn not_found() -> Result<Vec<LyricLine>, LyricsError> {
    Err(LyricsError::NotFound)
}

fn offline() -> Result<Vec<LyricLine>, LyricsError> {
    Err(LyricsError::TimedOut)
}

fn manager(providers: Vec<Arc<Fake>>) -> LyricsManager {
    let order = providers.iter().map(|p| p.id.to_string()).collect();
    let manager = LyricsManager::with_providers(
        providers
            .into_iter()
            .map(|p| p as Arc<dyn LyricsProvider>)
            .collect(),
        session(|_| None).0,
    );
    *manager.settings.lock().unwrap() = LyricsSettings::from_saved(order, HashSet::new());
    manager
}

#[tokio::test]
async fn fallback_skips_providers_without_lyrics_in_order() {
    let m = manager(vec![
        fake("a", 0, not_found),
        fake("b", 20, synced),
        fake("c", 0, word_timed),
    ]);
    let (lines, provider) = m.fetch_lyrics_returning_provider(&query()).await.unwrap();
    assert_eq!(
        (texts(&lines), provider.as_str()),
        (vec!["Synced line"], "b")
    );
}

#[tokio::test]
async fn synced_lyrics_beat_plain_text_from_a_higher_provider() {
    let m = manager(vec![fake("a", 0, plain), fake("b", 0, synced)]);
    assert_eq!(
        m.fetch_lyrics_returning_provider(&query()).await.unwrap().1,
        "b"
    );
}

#[tokio::test]
async fn disabled_providers_and_custom_order_are_respected() {
    let m = manager(vec![
        fake("a", 0, synced),
        fake("b", 0, word_timed),
        fake("c", 0, plain),
    ]);
    *m.settings.lock().unwrap() = LyricsSettings::from_saved(
        vec!["c".into(), "b".into(), "a".into()],
        HashSet::from(["b".to_string()]),
    );
    assert_eq!(
        m.fetch_lyrics_returning_provider(&query()).await.unwrap().1,
        "a"
    );
}

#[tokio::test]
async fn first_provider_wins_without_waiting_for_slower_ones() {
    let m = manager(vec![fake("a", 0, synced), fake("b", 60_000, word_timed)]);
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        m.fetch_lyrics_returning_provider(&query()),
    )
    .await;
    assert_eq!(result.unwrap().unwrap().1, "a");
}

#[tokio::test]
async fn not_found_versus_network_error() {
    let m = manager(vec![fake("a", 0, not_found), fake("b", 0, offline)]);
    assert!(matches!(
        m.fetch_lyrics(&query()).await,
        Err(LyricsError::NotFound)
    ));
    let m = manager(vec![fake("a", 0, offline)]);
    assert!(matches!(
        m.fetch_lyrics(&query()).await,
        Err(LyricsError::TimedOut)
    ));
}

#[tokio::test]
async fn word_timed_lookup_excludes_the_shown_provider() {
    let m = manager(vec![
        fake("a", 0, word_timed),
        fake("b", 0, synced),
        fake("c", 0, word_timed),
    ]);
    let (_, provider) = m.fetch_word_timed(&query(), Some("a")).await.unwrap();
    assert_eq!(provider, "c");
    let results = m.search_all(&query()).await;
    assert_eq!(
        results
            .iter()
            .map(|r| r.provider_name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert!(results[1].is_synced());
}

#[test]
fn saved_order_appends_missing_default_providers() {
    let settings =
        LyricsSettings::from_saved(vec!["genius".into(), "lrclib".into()], HashSet::new());
    assert_eq!(
        settings.provider_order,
        ["genius", "lrclib", "musixmatch", "netease", "kugou"]
    );
}

// Service.

fn defaults_path(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("trop_lyrics_{}_{name}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn song() -> LyricsSong {
    LyricsSong {
        video_id: "vid1".into(),
        title: "Sample Song".into(),
        artist: "Sample Artist".into(),
        album: None,
        duration: 200.0,
    }
}

#[tokio::test]
async fn service_caches_lyrics_and_records_availability() {
    let provider = fake("a", 0, synced);
    let service = Arc::new(LyricsService::new(
        manager(vec![provider.clone()]),
        defaults_path("cache"),
    ));
    let first = service.fetch_lyrics(&song()).await.unwrap();
    let again = service.fetch_lyrics(&song()).await.unwrap();
    assert_eq!(first, again);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    let state = service.state();
    assert!(state.available_video_ids.contains("vid1"));
    assert_eq!(state.provider_names["vid1"], "a");
    assert_eq!(service.editable_text("vid1"), "[00:01.00]Synced line");
}

#[tokio::test]
async fn service_remembers_not_found_songs() {
    let provider = fake("a", 0, not_found);
    let service = Arc::new(LyricsService::new(
        manager(vec![provider.clone()]),
        defaults_path("missing"),
    ));
    assert!(service.fetch_lyrics(&song()).await.is_err());
    assert!(service.fetch_lyrics(&song()).await.is_err());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert!(!service.state().available_video_ids.contains("vid1"));
}

#[tokio::test]
async fn service_custom_lyrics_persist_in_the_defaults_file() {
    let path = defaults_path("custom");
    let service = LyricsService::new(manager(vec![fake("a", 0, synced)]), path.clone());
    service.save_custom("vid1", "[00:02.00]Edited line", Some("LRCLIB"));
    assert_eq!(service.state().refresh_token, 1);

    let provider = fake("a", 0, synced);
    let reopened = Arc::new(LyricsService::new(manager(vec![provider.clone()]), path));
    let lines = reopened.fetch_lyrics(&song()).await.unwrap();
    assert_eq!(texts(&lines), ["Edited line"]);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(reopened.state().provider_names["vid1"], "LRCLIB");
    assert_eq!(
        reopened.plain_text_for_copy("vid1").as_deref(),
        Some("Edited line")
    );
}

#[tokio::test]
async fn service_upgrades_to_word_timing_in_the_background_when_karaoke_is_on() {
    let service = Arc::new(LyricsService::new(
        manager(vec![fake("a", 0, synced), fake("b", 50, word_timed)]),
        defaults_path("karaoke"),
    ));
    service.set_karaoke_enabled(true);
    let shown = service.fetch_lyrics(&song()).await.unwrap();
    assert_eq!(texts(&shown), ["Synced line"]);
    for _ in 0..100 {
        if service.state().refresh_token > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let upgraded = service.fetch_lyrics(&song()).await.unwrap();
    assert_eq!(texts(&upgraded), ["Word timed"]);
    assert_eq!(service.state().provider_names["vid1"], "b");
}

#[test]
fn query_cleaning_splits_channel_uploads_and_strips_extras() {
    let channel = LyricsSong {
        title: "Real Artist - Real Song【MV】(Official Video)".into(),
        artist: "Sample Music Channel".into(),
        album: Some("Album [Deluxe] (2020)".into()),
        ..song()
    };
    let q = LyricsService::resolve_query(&channel).unwrap();
    assert_eq!(
        (q.title.as_str(), q.artist.as_str()),
        ("Real Song", "Real Artist")
    );
    assert_eq!(q.album.as_deref(), Some("Album"));

    let topic = LyricsSong {
        title: "Sample Song | Lyrics".into(),
        artist: "Sample Artist - Topic, Other".into(),
        ..song()
    };
    let q = LyricsService::resolve_query(&topic).unwrap();
    assert_eq!(
        (q.title.as_str(), q.artist.as_str()),
        ("Sample Song", "Sample Artist, Other")
    );

    assert_eq!(
        LyricsService::resolve_query(&LyricsSong {
            title: String::new(),
            ..song()
        }),
        None
    );
}

#[test]
fn query_matches_channel_words_only_as_whole_words() {
    let query = |title: &str, artist: &str| {
        let q = LyricsService::resolve_query(&LyricsSong {
            title: title.into(),
            artist: artist.into(),
            ..song()
        })
        .unwrap();
        (q.title, q.artist)
    };
    let pair = |title: &str, artist: &str| (title.to_string(), artist.to_string());
    assert_eq!(
        query("Officially Missing You", "Tamia"),
        pair("Officially Missing You", "Tamia")
    );
    assert_eq!(
        query("Paranoid Android - Remastered", "Radiohead"),
        pair("Paranoid Android - Remastered", "Radiohead")
    );
    assert_eq!(
        query("Feel It Still - Lido Remix", "Portugal. The Man"),
        pair("Feel It Still - Lido Remix", "Portugal. The Man")
    );
    assert_eq!(query("Breaking Me", "Topic"), pair("Breaking Me", "Topic"));
    assert_eq!(query("Adele - Hello", "AdeleVEVO"), pair("Hello", "Adele"));
    assert_eq!(query("Hello", "Adele-Topic"), pair("Hello", "Adele"));
}

// Line lookup.

#[test]
fn active_line_is_the_last_started_line() {
    let lines = LyricsParsing::parse_stored_text("[00:01.00]One\n[00:05.00]Two\n[00:09.00]Three");
    assert_eq!(active_line_index(&lines, 0.0), 0);
    assert_eq!(active_line_index(&lines, 5.0), 1);
    assert_eq!(active_line_index(&lines, 8.99), 1);
    assert_eq!(active_line_index(&lines, 100.0), 2);
    // The lyrics offset setting is added to the playback time by the caller.
    assert_eq!(active_line_index(&lines, 4.5 + 0.5), 1);
    let untimed = LyricsParsing::parse_stored_text("One\nTwo");
    assert_eq!(active_line_index(&untimed, 50.0), 0);
}

#[test]
fn instrumental_markers_become_gaps_after_the_previous_sung_line() {
    let all = LyricsParsing::parse_lrc(
        "[00:00.50]♪\n[00:01.00]One\n[00:05.00]\n[00:20.00]Two\n[00:21.00]♪\n[00:23.00]Three",
    );
    assert_eq!(texts(&display_lines(&all)), ["One", "Two", "Three"]);
    let gaps = detect_gaps(&all);
    assert_eq!(gaps.len(), 1);
    assert_eq!(
        (gaps[0].after_index, gaps[0].start, gaps[0].end),
        (0, 5.0, 20.0)
    );
    let only_markers = LyricsParsing::parse_lrc("[00:01.00]♪");
    assert_eq!(display_lines(&only_markers).len(), 1);
}
