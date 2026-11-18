//! `LyricsRomanizer+Japanese.swift`: Hepburn-style kana to romaji.
//!
//! Kanji pass through unchanged, as in Swift.

use super::lyrics_romanizer::LyricsRomanizer;

impl LyricsRomanizer {
    pub(crate) fn romanize_japanese(text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        // Katakana folded onto hiragana once per line, for the reading tables.
        let normalized: Vec<char> = chars
            .iter()
            .map(|&c| katakana_to_hiragana(c).unwrap_or(c))
            .collect();
        let mut out = String::new();
        let mut last_vowel: Option<char> = None;
        let mut i = 0;

        while i < chars.len() {
            let raw = chars[i];

            // Long-vowel mark extends the previous vowel.
            if raw == 'ー' {
                if let Some(v) = last_vowel {
                    out.push(v);
                }
                i += 1;
                continue;
            }

            // Sokuon doubles the next consonant: one extra copy, the following reading supplies its own.
            if normalized[i] == 'っ' {
                if let Some((next, _)) = reading(&normalized, i + 1) {
                    append_reading(&mut out, &mut last_vowel, &next[..1]);
                }
                i += 1;
                continue;
            }

            if let Some((romaji, length)) = reading(&normalized, i) {
                append_reading(&mut out, &mut last_vowel, romaji);
                i += length;
                continue;
            }

            out.push(raw);
            if let Some(v) = raw.to_lowercase().last().filter(|v| "aeiou".contains(*v)) {
                last_vowel = Some(v);
            }
            i += 1;
        }
        out
    }
}

fn append_reading(out: &mut String, last_vowel: &mut Option<char>, romaji: &str) {
    out.push_str(romaji);
    if let Some(v) = romaji.chars().last().filter(|v| "aeiou".contains(*v)) {
        *last_vowel = Some(v);
    }
}

/// Maps a katakana character onto its hiragana counterpart (same gojūon grid).
fn katakana_to_hiragana(c: char) -> Option<char> {
    match c as u32 {
        v @ 0x30A1..=0x30F6 => char::from_u32(v - 0x60),
        _ => None,
    }
}

/// Reading at `index` of a line whose katakana is already folded onto hiragana.
fn reading(normalized: &[char], index: usize) -> Option<(&'static str, usize)> {
    let first = *normalized.get(index)?;
    if let Some(&second) = normalized.get(index + 1)
        && let Some(digraph) = digraph_reading(first, second)
    {
        return Some((digraph, 2));
    }
    hiragana_reading(first).map(|single| (single, 1))
}

#[rustfmt::skip]
fn hiragana_reading(c: char) -> Option<&'static str> {
    Some(match c {
        'あ' => "a", 'い' => "i", 'う' => "u", 'え' => "e", 'お' => "o",
        'か' => "ka", 'き' => "ki", 'く' => "ku", 'け' => "ke", 'こ' => "ko",
        'が' => "ga", 'ぎ' => "gi", 'ぐ' => "gu", 'げ' => "ge", 'ご' => "go",
        'さ' => "sa", 'し' => "shi", 'す' => "su", 'せ' => "se", 'そ' => "so",
        'ざ' => "za", 'じ' => "ji", 'ず' => "zu", 'ぜ' => "ze", 'ぞ' => "zo",
        'た' => "ta", 'ち' => "chi", 'つ' => "tsu", 'て' => "te", 'と' => "to",
        'だ' => "da", 'ぢ' => "di", 'づ' => "du", 'で' => "de", 'ど' => "do",
        'な' => "na", 'に' => "ni", 'ぬ' => "nu", 'ね' => "ne", 'の' => "no",
        'は' => "ha", 'ひ' => "hi", 'ふ' => "fu", 'へ' => "he", 'ほ' => "ho",
        'ば' => "ba", 'び' => "bi", 'ぶ' => "bu", 'べ' => "be", 'ぼ' => "bo",
        'ぱ' => "pa", 'ぴ' => "pi", 'ぷ' => "pu", 'ぺ' => "pe", 'ぽ' => "po",
        'ま' => "ma", 'み' => "mi", 'む' => "mu", 'め' => "me", 'も' => "mo",
        'や' => "ya", 'ゆ' => "yu", 'よ' => "yo",
        'ら' => "ra", 'り' => "ri", 'る' => "ru", 'れ' => "re", 'ろ' => "ro",
        'わ' => "wa", 'ゐ' => "wi", 'ゑ' => "we", 'を' => "wo", 'ん' => "n",
        'ゔ' => "vu",
        'ゃ' => "ya", 'ゅ' => "yu", 'ょ' => "yo",
        'ぁ' => "a", 'ぃ' => "i", 'ぅ' => "u", 'ぇ' => "e", 'ぉ' => "o",
        _ => return None,
    })
}

#[rustfmt::skip]
fn digraph_reading(first: char, second: char) -> Option<&'static str> {
    Some(match (first, second) {
        ('き', 'ゃ') => "kya", ('き', 'ゅ') => "kyu", ('き', 'ょ') => "kyo",
        ('し', 'ゃ') => "sha", ('し', 'ゅ') => "shu", ('し', 'ょ') => "sho",
        ('ち', 'ゃ') => "cha", ('ち', 'ゅ') => "chu", ('ち', 'ょ') => "cho",
        ('に', 'ゃ') => "nya", ('に', 'ゅ') => "nyu", ('に', 'ょ') => "nyo",
        ('ひ', 'ゃ') => "hya", ('ひ', 'ゅ') => "hyu", ('ひ', 'ょ') => "hyo",
        ('み', 'ゃ') => "mya", ('み', 'ゅ') => "myu", ('み', 'ょ') => "myo",
        ('り', 'ゃ') => "rya", ('り', 'ゅ') => "ryu", ('り', 'ょ') => "ryo",
        ('ぎ', 'ゃ') => "gya", ('ぎ', 'ゅ') => "gyu", ('ぎ', 'ょ') => "gyo",
        ('じ', 'ゃ') => "ja", ('じ', 'ゅ') => "ju", ('じ', 'ょ') => "jo",
        ('び', 'ゃ') => "bya", ('び', 'ゅ') => "byu", ('び', 'ょ') => "byo",
        ('ぴ', 'ゃ') => "pya", ('ぴ', 'ゅ') => "pyu", ('ぴ', 'ょ') => "pyo",
        ('ふ', 'ぁ') => "fa", ('ふ', 'ぃ') => "fi", ('ふ', 'ぇ') => "fe", ('ふ', 'ぉ') => "fo",
        ('て', 'ぃ') => "ti", ('で', 'ぃ') => "di", ('と', 'ぅ') => "tu",
        ('う', 'ぇ') => "we", ('う', 'ぃ') => "wi", ('う', 'ぉ') => "wo", ('い', 'ぇ') => "ye",
        ('ゔ', 'ぁ') => "va", ('ゔ', 'ぃ') => "vi", ('ゔ', 'ぇ') => "ve", ('ゔ', 'ぉ') => "vo",
        ('ぢ', 'ゃ') => "ja", ('ぢ', 'ゅ') => "ju", ('ぢ', 'ょ') => "jo",
        _ => return None,
    })
}
