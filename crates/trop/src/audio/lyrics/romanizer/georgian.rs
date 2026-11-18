//! `LyricsRomanizer+Georgian.swift`: Georgian Mkhedruli to Latin.

use super::lyrics_romanizer::LyricsRomanizer;

impl LyricsRomanizer {
    pub(crate) fn romanize_georgian(text: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            match georgian(c) {
                Some(mapped) => out.push_str(mapped),
                None => out.push(c),
            }
        }
        out
    }
}

#[rustfmt::skip]
fn georgian(c: char) -> Option<&'static str> {
    Some(match c {
        'ა' => "a", 'ბ' => "b", 'გ' => "g", 'დ' => "d", 'ე' => "e", 'ვ' => "v",
        'ზ' => "z", 'თ' => "t", 'ი' => "i", 'კ' => "k", 'ლ' => "l", 'მ' => "m",
        'ნ' => "n", 'ო' => "o", 'პ' => "p", 'ჟ' => "zh", 'რ' => "r", 'ს' => "s",
        'ტ' => "t", 'უ' => "u", 'ფ' => "p", 'ქ' => "k", 'ღ' => "gh", 'ყ' => "q",
        'შ' => "sh", 'ჩ' => "ch", 'ც' => "ts", 'ძ' => "dz", 'წ' => "ts", 'ჭ' => "ch",
        'ხ' => "kh", 'ჯ' => "j", 'ჰ' => "h",
        _ => return None,
    })
}
