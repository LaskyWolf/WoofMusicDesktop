//! `LyricsRomanizer+Arabic.swift`: Arabic to Latin.
//!
//! Pointed text uses harakat: fatha/damma/kasra, tanwin, shadda doubling, sun/moon letters,
//! ta marbuta word-final "a". Unpointed lyrics get vowels from matres (ا = a, medial و = u,
//! non-initial ي = i) plus an epenthetic "a" between consonants (مرحبا → marahaba).

use super::lyrics_romanizer::{LyricsRomanizer, Phoneme};

impl LyricsRomanizer {
    pub(crate) fn romanize_arabic(text: &str) -> String {
        let s: Vec<u32> = text.chars().map(|c| c as u32).collect();
        let mut phonemes: Vec<Phoneme> = Vec::new();
        let mut starts_word: Vec<bool> = Vec::new();
        let mut pending_word_start = true;
        let mut has_any_points = false;
        let mut i = 0;
        // The previous letter carried tanwin: a following bare alif is only its seat (شُكْرًا → shukran).
        let mut previous_had_tanwin = false;

        while i < s.len() {
            let value = s[i];

            if !is_base(value) {
                if !is_point(value) {
                    pending_word_start = true;
                }
                previous_had_tanwin = false;
                i += 1;
                continue;
            }

            let mut j = i + 1;
            let mut haraka: Option<&str> = None;
            let mut shadda = false;
            while j < s.len() && is_point(s[j]) {
                has_any_points = true;
                match s[j] {
                    0x064B => haraka = Some("an"), // fathatan
                    0x064C => haraka = Some("un"), // dammatan
                    0x064D => haraka = Some("in"), // kasratan
                    0x064E => haraka = Some("a"),  // fatha
                    0x064F => haraka = Some("u"),  // damma
                    0x0650 => haraka = Some("i"),  // kasra
                    0x0651 => shadda = true,
                    0x0670 => haraka = Some("a"), // superscript alef
                    _ => {}                       // sukun etc.
                }
                j += 1;
            }

            // Bare alif right after a tanwin only seats the mark.
            if value == 0x0627 && haraka.is_none() && previous_had_tanwin {
                previous_had_tanwin = false;
                i = j;
                continue;
            }

            // Bare waw after a "u" phoneme and bare yāʾ after an "i" phoneme only prolong the vowel.
            let lengthening = |code: u32, suffix: &str| {
                haraka.is_none()
                    && !shadda
                    && value == code
                    && phonemes.last().is_some_and(|p| p.0.ends_with(suffix))
            };
            if lengthening(0x0648, "u") || lengthening(0x064A, "i") {
                previous_had_tanwin = false;
                i = j;
                continue;
            }

            let mut sound = letter_sound(value);

            // Sun-letter assimilation: the article's lām is silent before a sun letter
            // (الشمس → ash-shams). Pointed text needs a shadda on the sun letter; unpointed
            // lyrics accept an alif+lām prefix as the article.
            if value == 0x0644
                && haraka.is_none()
                && !shadda
                && j < s.len()
                && is_base(s[j])
                && is_sun_letter(s[j])
            {
                let pointed_assimilation = next_has_shadda(&s, j);
                let article_prefix = i > 0 && s[i - 1] == 0x0627 && is_first_base(&s, i - 1);
                if pointed_assimilation || (!has_any_points && article_prefix) {
                    i = j;
                    continue;
                }
            }

            // Word-final ta marbuta sounds "a".
            if value == 0x0629 {
                let mut k = j;
                while k < s.len() && is_point(s[k]) {
                    k += 1;
                }
                let medial = k < s.len() && (is_base(s[k]) || s[k] == 0x0640);
                sound = if medial { "t" } else { "a" };
            }
            // آ already reads "aa".
            if value == 0x0622 && haraka.is_none() {
                haraka = Some("");
            }

            // Unpointed و and ي inflect by position.
            if haraka.is_none() && !shadda && !ends_with_vowel(sound) {
                if value == 0x0648 {
                    sound = if is_first_base(&s, i) { "w" } else { "u" };
                } else if value == 0x064A {
                    sound = if is_first_base(&s, i) { "y" } else { "i" };
                }
            }

            let is_vowel = haraka.is_some() || shadda || ends_with_vowel(sound);
            let emitted = format!("{sound}{}", haraka.unwrap_or(""));
            let doubled = if shadda && !sound.is_empty() {
                format!("{sound}{emitted}")
            } else {
                emitted
            };
            phonemes.push((doubled, is_vowel));
            starts_word.push(pending_word_start);
            pending_word_start = false;

            previous_had_tanwin = matches!(haraka, Some("an" | "un" | "in"));
            i = j;
        }

        let phonemes = if has_any_points {
            phonemes
        } else {
            Self::insert_epenthetic(&phonemes, "a", &starts_word)
        };
        phonemes.into_iter().map(|p| p.0).collect()
    }
}

fn ends_with_vowel(s: &str) -> bool {
    s.ends_with(['a', 'e', 'i', 'o', 'u'])
}

/// Whether the base letter at `index` is the first of its word.
fn is_first_base(s: &[u32], index: usize) -> bool {
    let mut k = index;
    while k > 0 && is_point(s[k - 1]) {
        k -= 1;
    }
    k == 0 || !is_base(s[k - 1])
}

fn is_base(v: u32) -> bool {
    (0x0621..=0x064A).contains(&v) || v == 0x0671
}

fn is_point(v: u32) -> bool {
    (0x064B..=0x065F).contains(&v) || v == 0x0670
}

/// Whether the base letter at `base_index` carries a shadda.
fn next_has_shadda(s: &[u32], base_index: usize) -> bool {
    s[base_index + 1..]
        .iter()
        .take_while(|&&v| is_point(v))
        .any(|&v| v == 0x0651)
}

/// The 14 sun letters assimilate the definite article's lām.
fn is_sun_letter(v: u32) -> bool {
    matches!(v, 0x062A | 0x062B | 0x062F..=0x0638 | 0x0644 | 0x0646)
}

/// Letter sounds; matres and hamza carriers map to their vowel strings.
#[rustfmt::skip]
fn letter_sound(v: u32) -> &'static str {
    match v {
        0x0621 => "'",  // ء
        0x0622 => "aa", // آ
        0x0623 => "a",  // أ
        0x0624 => "w",  // ؤ
        0x0625 => "i",  // إ
        0x0626 => "y",  // ئ
        0x0627 => "a",  // ا
        0x0628 => "b", 0x0629 => "h", 0x062A => "t", 0x062B => "th", 0x062C => "j",
        0x062D => "h", 0x062E => "kh", 0x062F => "d", 0x0630 => "dh", 0x0631 => "r",
        0x0632 => "z", 0x0633 => "s", 0x0634 => "sh", 0x0635 => "s", 0x0636 => "d",
        0x0637 => "t", 0x0638 => "z",
        0x0639 => "a",  // ع
        0x063A => "gh", 0x0641 => "f", 0x0642 => "q", 0x0643 => "k", 0x0644 => "l",
        0x0645 => "m", 0x0646 => "n", 0x0647 => "h", 0x0648 => "w",
        0x0649 => "a",  // ى
        0x064A => "y",
        0x0671 => "a",  // ٱ alef wasla
        _ => "",
    }
}
