//! `LyricsRomanizer+Chinese.swift`: Han to Hanyu Pinyin with tone marks (我爱你 → wǒ ài nǐ).
//!
//! Swift calls Apple's `kCFStringTransformMandarinLatin` (ICU Han-Latin). The `pinyin` crate
//! gives the same per-character readings from the pinyin-data tables.

use pinyin::ToPinyin;

use super::lyrics_romanizer::LyricsRomanizer;

impl LyricsRomanizer {
    // ponytail: one reading per character (the most common one), so polyphones such as 长 or 行
    // can differ from ICU, which picks some readings by word. Add a phrase table if lyrics need it.
    pub(crate) fn romanize_chinese(text: &str) -> String {
        let mut out = String::new();
        let mut previous_was_han = false;
        for c in text.chars() {
            match c.to_pinyin() {
                Some(reading) => {
                    // Adjacent syllables are separated by a space, like ICU.
                    if previous_was_han {
                        out.push(' ');
                    }
                    out.push_str(reading.with_tone());
                    previous_was_han = true;
                }
                None => {
                    out.push(c);
                    previous_was_han = false;
                }
            }
        }
        out
    }
}
