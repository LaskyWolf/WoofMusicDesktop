//! `LyricsRomanizer+Devanagari.swift`: Devanagari to Latin (Hindi, Marathi, Nepali): matras,
//! virama conjuncts, nukta variants, inherent "a" with word-final deletion.

use super::lyrics_romanizer::LyricsRomanizer;

impl LyricsRomanizer {
    pub(crate) fn romanize_devanagari(text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let s: Vec<u32> = chars.iter().map(|&c| c as u32).collect();
        let mut out = String::new();
        let mut i = 0;

        let is_letterish = |v: u32| {
            is_consonant(v) || independent_vowel(v).is_some() || (0x0900..=0x094D).contains(&v)
        };

        while i < s.len() {
            let value = s[i];

            if is_consonant(value) {
                let mut j = i + 1;
                let mut has_nukta = false;
                let mut has_virama = false;
                let mut matra = None;

                while j < s.len() && is_sign(s[j]) {
                    let point = s[j];
                    if point == 0x093C {
                        has_nukta = true;
                    } else if point == 0x094D {
                        has_virama = true;
                        j += 1;
                        break;
                    } else if matra.is_none() && matra_vowel(point).is_some() {
                        matra = matra_vowel(point);
                    } else if matches!(point, 0x0900..=0x0902) {
                        break;
                    }
                    j += 1;
                }

                out.push_str(consonant(value, has_nukta));

                // A virama makes a conjunct: no vowel at all.
                if !has_virama {
                    if let Some(matra) = matra {
                        out.push_str(matra);
                    } else if j < s.len() && is_letterish(s[j]) {
                        // Inherent "a", dropped word-finally.
                        out.push('a');
                    }
                }

                // Trailing anusvara/visarga after the vowel.
                while j < s.len() && (0x0900..=0x0903).contains(&s[j]) {
                    out.push(if s[j] == 0x0903 { 'h' } else { 'n' });
                    j += 1;
                }
                i = j;
                continue;
            }

            if let Some(vowel) = independent_vowel(value) {
                out.push_str(vowel);
                i += 1;
                continue;
            }

            out.push(chars[i]);
            i += 1;
        }
        out
    }
}

fn is_consonant(v: u32) -> bool {
    (0x0915..=0x0939).contains(&v)
}

fn consonant(v: u32, has_nukta: bool) -> &'static str {
    // Nukta variants (क़ ज़ ड़ फ़).
    if has_nukta {
        match v {
            0x0915 => return "q",
            0x091C => return "z",
            0x0921 => return "r",
            0x092B => return "f",
            _ => {}
        }
    }
    plain_consonant(v)
}

#[rustfmt::skip]
fn plain_consonant(v: u32) -> &'static str {
    match v {
        0x0915 => "k", 0x0916 => "kh", 0x0917 => "g", 0x0918 => "gh", 0x0919 => "ng",
        0x091A => "ch", 0x091B => "chh", 0x091C => "j", 0x091D => "jh", 0x091E => "ny",
        0x091F => "t", 0x0920 => "th", 0x0921 => "d", 0x0922 => "dh", 0x0923 => "n",
        0x0924 => "t", 0x0925 => "th", 0x0926 => "d", 0x0927 => "dh", 0x0928 => "n",
        0x092A => "p", 0x092B => "ph", 0x092C => "b", 0x092D => "bh", 0x092E => "m",
        0x092F => "y", 0x0930 => "r", 0x0932 => "l", 0x0933 => "l", 0x0935 => "v",
        0x0936 => "sh", 0x0937 => "sh", 0x0938 => "s", 0x0939 => "h",
        _ => "",
    }
}

#[rustfmt::skip]
fn matra_vowel(v: u32) -> Option<&'static str> {
    Some(match v {
        0x093E => "aa", 0x093F => "i", 0x0940 => "ii", 0x0941 => "u", 0x0942 => "uu",
        0x0943 => "ri", 0x0944 => "rii",
        0x0945..=0x0947 => "e", 0x0948 => "ai", 0x0949 | 0x094A => "o", 0x094B | 0x094C => "au",
        _ => return None,
    })
}

#[rustfmt::skip]
fn independent_vowel(v: u32) -> Option<&'static str> {
    Some(match v {
        0x0905 => "a", 0x0906 => "aa", 0x0907 => "i", 0x0908 => "ii", 0x0909 => "u",
        0x090A => "uu", 0x090B => "ri", 0x090C => "lri",
        0x090D | 0x090F => "e", 0x090E | 0x0910 => "ai", 0x0911 | 0x0912 => "o", 0x0913 | 0x0914 => "au",
        _ => return None,
    })
}

/// Independent vowels end a consonant's point run: they are syllables of their own.
fn is_sign(v: u32) -> bool {
    !(0x0904..=0x0914).contains(&v) && (0x0900..=0x094D).contains(&v) && !is_consonant(v)
}
