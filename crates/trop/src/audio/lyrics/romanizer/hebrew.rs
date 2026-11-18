//! `LyricsRomanizer+Hebrew.swift`: Hebrew to Latin.
//!
//! Words with niqqud go through the nikud-aware engine. Unpointed modern Hebrew (most lyrics)
//! goes through a common-words dictionary plus ktiv-male letter rules (ו → o/u, י → i,
//! final ה → a, positional ב/כ/פ, ע/א as vowel markers).

use super::lyrics_romanizer::{LyricsRomanizer, Phoneme};

impl LyricsRomanizer {
    pub(crate) fn romanize_hebrew(text: &str) -> String {
        // Split into words and separators; route each word on its own.
        let mut out = String::new();
        let mut word = String::new();
        for c in text.chars() {
            if is_base(c as u32) || is_point(c as u32) {
                word.push(c);
            } else {
                flush_word(&mut out, &mut word);
                out.push(c);
            }
        }
        flush_word(&mut out, &mut word);
        out
    }
}

fn flush_word(out: &mut String, word: &mut String) {
    if word.is_empty() {
        return;
    }
    if word.chars().any(|c| is_point(c as u32)) {
        out.push_str(&romanize_pointed(word));
    } else {
        out.push_str(&romanize_unpointed_word(word));
    }
    word.clear();
}

// MARK: - Unpointed (modern) Hebrew

/// Common lyric words: full-vowel readings the letter rules cannot infer.
#[rustfmt::skip]
fn word_override(word: &str) -> Option<&'static str> {
    Some(match word {
        "אני" => "ani", "את" => "at", "אתה" => "ata", "אנחנו" => "anachnu",
        "הוא" => "hu", "היא" => "hi", "הם" => "hem", "הנה" => "hine",
        "יש" => "yesh", "אין" => "ein", "ישלי" => "yesh li",
        "כן" => "ken", "לא" => "lo", "אולי" => "ulai", "רק" => "rak",
        "אז" => "az", "עכשיו" => "achshav", "עוד" => "od", "כבר" => "kvar",
        "מה" => "ma", "למה" => "lama", "כמה" => "kama", "מי" => "mi",
        "גם" => "gam", "אבל" => "aval", "שלך" => "shelacha", "שלי" => "sheli",
        "שלנו" => "shelanu", "לך" => "lecha", "איתך" => "itach",
        "אותי" => "oti", "אותך" => "otacha", "אותו" => "oto", "אותה" => "ota",
        "לי" => "li", "לנו" => "lanu", "בשבילך" => "bishvilecha",
        "אהבה" => "ahava", "אוהב" => "ohev", "אוהבת" => "ohevet",
        "שלום" => "shalom", "חיים" => "chayim", "החיים" => "hachayim",
        "לב" => "lev", "ליבי" => "libi", "הלב" => "halev",
        "לילה" => "laila", "יום" => "yom", "בוקר" => "boker", "ערב" => "erev",
        "טוב" => "tov", "אמת" => "emet", "שקר" => "sheker",
        "חלום" => "chalom", "חלמתי" => "chalamti", "דמעות" => "dmaot",
        "ודמעת" => "udmaot", "שילמתי" => "shilamti", "מחכתי" => "mechaketi",
        "מחכת" => "mechaka", "מחכות" => "mechakot", "מחכה" => "mechaka",
        "רוצה" => "rotze", "ללכת" => "lalechet", "ביחד" => "beyachad",
        "ושוב" => "veshuv", "שוב" => "shuv", "פשוט" => "pashut",
        "הלך" => "halach", "מזמן" => "mizman", "אמר" => "amar",
        "תן" => "ten", "סליחה" => "slicha", "בבקשה" => "bevakasha",
        "תודה" => "toda", "ולפעמים" => "ulifamim", "לפעמים" => "lifamim",
        "בינינו" => "beineinu", "שנאה" => "sina", "הוספתי" => "hosafati",
        "להודות" => "lehodot", "מנגן" => "menagen", "על" => "al",
        "משוחרר" => "meshucharar", "החרדות" => "hachardot",
        "נשברה" => "nishbera", "חושב" => "choshev", "יודע" => "yodea",
        "בוכה" => "boche", "שיר" => "shir", "לבד" => "levad",
        "בלבד" => "bilvad", "איתי" => "iti", "כאב" => "ke'ev",
        _ => return None,
    })
}

/// `word` holds only base letters: the caller routes words with points to `romanize_pointed`.
fn romanize_unpointed_word(word: &str) -> String {
    if let Some(hit) = word_override(word) {
        return hit.to_string();
    }
    let chars: Vec<char> = word.chars().collect();
    let s: Vec<u32> = chars.iter().map(|&c| c as u32).collect();
    let base = |index: usize| s.get(index).copied().unwrap_or(0);

    let mut phonemes: Vec<Phoneme> = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let value = s[i];
        let is_last = i == s.len() - 1;
        let next = base(i + 1);
        let prev = if i > 0 { s[i - 1] } else { 0 };
        let mut add = |text: &str, vowel: bool| phonemes.push((text.to_string(), vowel));

        match value {
            // א: vowel marker in ktiv male.
            0x05D0 => {
                if i == 0 && next == 0x05D5 {
                    add("o", true);
                    i += 2;
                    continue;
                }
                if i == 0 && next == 0x05D9 {
                    add("ei", true);
                    i += 1;
                    continue;
                }
                add("a", true);
            }
            // ה
            0x05D4 => {
                if i == 0 && next == 0x05D5 {
                    add("ho", true);
                    i += 2;
                    continue;
                }
                if i == 0 && next == 0x05D9 {
                    add("hi", true);
                    i += 2;
                    continue;
                }
                if i == 0 {
                    add("ha", true);
                } else if is_last {
                    add("a", true);
                } else {
                    add("h", false);
                }
            }
            // ו
            0x05D5 => {
                if i == 0 {
                    add("ve", true);
                } else if prev == 0x05D5 {
                    add("v", false);
                } else if is_last {
                    add("u", true);
                } else {
                    add("o", true);
                }
            }
            // י
            0x05D9 => {
                if i == 0 {
                    add("y", false);
                } else {
                    add("i", true);
                }
            }
            // ע: silent glottal, usually a vowel marker. Before a word-initial י, the י carries the vowel.
            0x05E2 => {
                if !(i == 0 && next == 0x05D9) {
                    add("a", true);
                }
            }
            // ב
            0x05D1 => add(if i == 0 { "b" } else { "v" }, false),
            // ך (לך → lech)
            0x05DA => add("ech", true),
            // כ
            0x05DB => add(if i == 0 { "k" } else { "ch" }, false),
            // פ / ף
            0x05E4 | 0x05E3 => add(if i == 0 && value == 0x05E4 { "p" } else { "f" }, false),
            _ => match unpointed_simple(value) {
                Some(mapped) => add(mapped, false),
                None => add(&chars[i].to_string(), false),
            },
        }
        i += 1;
    }

    // Epenthetic "e" between adjacent consonants keeps ktiv-chaser words readable (שמש → shemesh).
    LyricsRomanizer::insert_epenthetic(&phonemes, "e", &[])
        .into_iter()
        .map(|p| p.0)
        .collect()
}

/// Context-free sounds; the position-dependent letters (ב כ ך פ ף ו י ע א ה) are handled above.
#[rustfmt::skip]
fn unpointed_simple(v: u32) -> Option<&'static str> {
    Some(match v {
        0x05D2 => "g", 0x05D3 => "d", 0x05D6 => "z", 0x05D7 => "ch", 0x05D8 => "t",
        0x05DC => "l", 0x05DE => "m", 0x05DD => "m", 0x05E0 => "n", 0x05DF => "n",
        0x05E1 => "s", 0x05E6 => "ts", 0x05E5 => "ts", 0x05E7 => "k", 0x05E8 => "r",
        0x05E9 => "sh", 0x05EA => "t",
        _ => return None,
    })
}

// MARK: - Pointed (nikud) Hebrew

fn romanize_pointed(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let s: Vec<u32> = chars.iter().map(|&c| c as u32).collect();
    let mut out = String::new();
    let mut i = 0;
    let mut at_word_start = true;
    let mut previous_had_sheva = false;
    let mut previous_nikud: Option<u32> = None;

    while i < s.len() {
        let value = s[i];

        if !is_base(value) {
            out.push(chars[i]);
            at_word_start = true;
            i += 1;
            continue;
        }

        // Collect the points attached to this letter.
        let mut j = i + 1;
        let mut has_dagesh = false;
        let mut sin_dot = false;
        let mut nikud: Option<u32> = None;
        while j < s.len() && is_point(s[j]) {
            match s[j] {
                0x05BC => has_dagesh = true,
                0x05C2 => sin_dot = true,
                point @ 0x05B0..=0x05BB => nikud = Some(point),
                _ => {} // cantillation etc.
            }
            j += 1;
        }

        let mut vowel = vowel_sound(
            nikud,
            at_word_start,
            previous_had_sheva,
            has_dagesh,
            is_begadkefat(value),
            j,
            &s,
        );
        let had_sheva = nikud == Some(0x05B0);

        // ו: shuruk, holam male, or consonant vav. A bare vav after o/u is a silent mater.
        if value == 0x05D5 {
            if has_dagesh {
                out.push('u');
            } else if nikud == Some(0x05B9) {
                out.push('o');
            } else if !(nikud.is_none() && ends_with_any(&out, "ou")) {
                out.push('v');
                out.push_str(vowel);
            }
            previous_had_sheva = had_sheva;
            previous_nikud = nikud;
            at_word_start = false;
            i = j;
            continue;
        }

        // יִ: hiriq male; word-initially the yod is a consonant ("yi").
        if value == 0x05D9 && nikud == Some(0x05B4) {
            out.push_str(if at_word_start { "yi" } else { "i" });
            previous_had_sheva = false;
            previous_nikud = nikud;
            at_word_start = false;
            i = j;
            continue;
        }

        let mut consonant = consonant(value, has_dagesh, sin_dot);

        // Alef and ayin are silent, except a hataf vowel right after another vowel keeps
        // its break audible (הַאֲזִין → ha'azin).
        if value == 0x05D0 || value == 0x05E2 {
            consonant = "";
            let is_hataf = matches!(nikud, Some(0x05B1..=0x05B3));
            if is_hataf && !vowel.is_empty() && ends_with_any(&out, "aeiou") {
                out.push('\'');
            }
        }

        // Word-final ה is silent.
        if value == 0x05D4 && (j >= s.len() || !is_base(s[j])) {
            consonant = "";
        }

        // Bare yod: silent after "i", male "i" after tsere/segol.
        if nikud.is_none() && value == 0x05D9 {
            if ends_with_any(&out, "i") {
                consonant = "";
            } else if matches!(previous_nikud, Some(0x05B5 | 0x05B6)) {
                consonant = "";
                vowel = "i";
            }
        }

        out.push_str(consonant);
        out.push_str(vowel);

        previous_had_sheva = had_sheva;
        previous_nikud = nikud;
        at_word_start = false;
        i = j;
    }
    out
}

fn ends_with_any(s: &str, set: &str) -> bool {
    s.chars().last().is_some_and(|c| set.contains(c))
}

/// Vowel sound of a niqqud point on the letter whose points end at `next_base_index`.
fn vowel_sound(
    nikud: Option<u32>,
    at_word_start: bool,
    previous_had_sheva: bool,
    has_dagesh: bool,
    is_begadkefat: bool,
    next_base_index: usize,
    s: &[u32],
) -> &'static str {
    match nikud {
        // Sheva na (vocal): word-initial, second of two shevas, or under a dagesh'd
        // non-begadkefat letter.
        Some(0x05B0) => {
            if at_word_start || previous_had_sheva || (has_dagesh && !is_begadkefat) {
                "e"
            } else {
                ""
            }
        }
        // Qamats qatan (short "o") before a sheva-letter; otherwise long "a".
        Some(0x05B8) => {
            if next_base_has_sheva(next_base_index, s) {
                "o"
            } else {
                "a"
            }
        }
        Some(0x05B3) => "o",          // hataf qamats
        Some(0x05B1) => "e",          // hataf segol
        Some(0x05B2) => "a",          // hataf patah
        Some(0x05B4) => "i",          // hiriq
        Some(0x05B5 | 0x05B6) => "e", // tsere, segol
        Some(0x05B7) => "a",          // patah
        Some(0x05B9) => "o",          // holam
        Some(0x05BB) => "u",          // qubuts
        _ => "",
    }
}

/// Whether the next base letter from `index` carries a sheva.
fn next_base_has_sheva(index: usize, s: &[u32]) -> bool {
    let Some(base) = (index..s.len()).find(|&k| is_base(s[k])) else {
        return false;
    };
    s[base + 1..]
        .iter()
        .take_while(|&&v| is_point(v))
        .any(|&v| v == 0x05B0)
}

fn is_base(v: u32) -> bool {
    (0x05D0..=0x05EA).contains(&v)
}

/// בגדכפת letters, whose dagesh only changes pronunciation.
fn is_begadkefat(v: u32) -> bool {
    matches!(
        v,
        0x05D1 | 0x05D2 | 0x05D3 | 0x05DB | 0x05DA | 0x05E4 | 0x05E3 | 0x05EA
    )
}

/// Niqqud, dagesh, shin/sin dots and cantillation marks.
fn is_point(v: u32) -> bool {
    (0x0591..=0x05C7).contains(&v) && !matches!(v, 0x05BE | 0x05C0 | 0x05C3 | 0x05C6)
}

#[rustfmt::skip]
fn consonant(v: u32, has_dagesh: bool, sin_dot: bool) -> &'static str {
    match v {
        0x05D0 => "'",                                  // א
        0x05D1 => if has_dagesh { "b" } else { "v" },   // ב
        0x05D2 => "g", 0x05D3 => "d", 0x05D4 => "h", 0x05D5 => "v", 0x05D6 => "z",
        0x05D7 => "ch", 0x05D8 => "t", 0x05D9 => "y", 0x05DA => "ch",
        0x05DB => if has_dagesh { "k" } else { "ch" },  // כ
        0x05DC => "l", 0x05DE => "m", 0x05DD => "m", 0x05E0 => "n", 0x05DF => "n",
        0x05E1 => "s",
        0x05E2 => "'",                                  // ע
        0x05E3 => "f",
        0x05E4 => if has_dagesh { "p" } else { "f" },   // פ
        0x05E6 => "ts", 0x05E5 => "ts", 0x05E7 => "k", 0x05E8 => "r",
        0x05E9 => if sin_dot { "s" } else { "sh" },     // ש
        0x05EA => "t",
        _ => "",
    }
}
