//! `LyricsRomanizer+Cyrillic.swift`: Cyrillic to Latin (Russian, Ukrainian, Serbian, Bulgarian, …).

use super::lyrics_romanizer::{LyricsRomanizer, capitalized};

impl LyricsRomanizer {
    pub(crate) fn romanize_cyrillic(text: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            let key = c.to_lowercase().next().unwrap_or(c);
            match cyrillic(key) {
                Some(mapped) if c.is_uppercase() => out.push_str(&capitalized(mapped)),
                Some(mapped) => out.push_str(mapped),
                None => out.push(c),
            }
        }
        out
    }
}

#[rustfmt::skip]
fn cyrillic(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a", 'б' => "b", 'в' => "v", 'г' => "g", 'д' => "d", 'е' => "e",
        'ё' => "yo", 'ж' => "zh", 'з' => "z", 'и' => "i", 'й' => "y", 'к' => "k",
        'л' => "l", 'м' => "m", 'н' => "n", 'о' => "o", 'п' => "p", 'р' => "r",
        'с' => "s", 'т' => "t", 'у' => "u", 'ф' => "f", 'х' => "kh", 'ц' => "ts",
        'ч' => "ch", 'ш' => "sh", 'щ' => "shch", 'ъ' => "", 'ы' => "y", 'ь' => "",
        'э' => "e", 'ю' => "yu", 'я' => "ya",
        'і' => "i", 'ї' => "yi", 'є' => "ye", 'ґ' => "g",
        // Serbian / Macedonian / Belarusian.
        'ђ' => "dj", 'ћ' => "c", 'ј' => "j", 'њ' => "nj", 'љ' => "lj",
        'џ' => "dzh", 'ѓ' => "gj", 'ќ' => "kj", 'ў' => "u",
        _ => return None,
    })
}
