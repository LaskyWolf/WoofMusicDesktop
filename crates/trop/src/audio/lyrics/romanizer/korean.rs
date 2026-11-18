//! `LyricsRomanizer+Korean.swift`: Hangul to Revised Romanization via jamo decomposition,
//! including standalone compatibility jamo (ㄱ ㅏ …).
//!
//! Each syllable is romanized on its own: no sound-change rules across syllables, as in Swift.

use super::lyrics_romanizer::LyricsRomanizer;

impl LyricsRomanizer {
    pub(crate) fn romanize_korean(text: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            let value = c as u32;

            // Precomposed syllables.
            if (0xAC00..=0xD7A3).contains(&value) {
                let index = (value - 0xAC00) as usize;
                out.push_str(CHOSEONG[index / (21 * 28)]);
                out.push_str(JUNGSEONG[(index % (21 * 28)) / 28]);
                out.push_str(JONGSEONG[index % 28]);
                continue;
            }

            // Standalone compatibility jamo.
            if (0x3131..=0x3163).contains(&value) {
                out.push_str(COMPATIBILITY_JAMO[(value - 0x3131) as usize]);
                continue;
            }

            out.push(c);
        }
        out
    }
}

/// U+3131 ㄱ through U+3163 ㅣ.
#[rustfmt::skip]
const COMPATIBILITY_JAMO: [&str; 51] = [
    "g", "kk", "ks", "n", "nj", "nh", "d", "tt", "r", "rk",
    "rm", "rb", "rs", "rs", "rp", "rh", "m", "b", "pp", "bs",
    "s", "ss", "", "j", "jj", "ch", "k", "t", "p", "h",
    "a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa",
    "wae", "oe", "yo", "u", "wo", "we", "wi", "yu", "eu", "ui",
    "i",
];

#[rustfmt::skip]
const CHOSEONG: [&str; 19] = [
    "g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p", "h",
];

#[rustfmt::skip]
const JUNGSEONG: [&str; 21] = [
    "a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae",
    "oe", "yo", "u", "wo", "we", "wi", "yu", "eu", "yi", "i",
];

/// Finals in jongseong order: none, ㄱ ㄲ ㄳ ㄴ ㄵ ㄶ ㄷ ㄹ ㄺ ㄻ ㄼ ㄽ ㄾ ㄿ ㅀ ㅁ ㅂ ㅄ ㅅ ㅆ ㅇ ㅈ ㅊ ㅋ ㅌ ㅍ ㅎ.
#[rustfmt::skip]
const JONGSEONG: [&str; 28] = [
    "", "k", "k", "k", "n", "n", "n", "t", "l", "k", "m", "p", "l", "l",
    "p", "l", "m", "p", "t", "t", "ss", "ng", "t", "t", "k", "t", "p", "t",
];
