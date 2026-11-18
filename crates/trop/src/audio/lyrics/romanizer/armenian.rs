//! `LyricsRomanizer+Armenian.swift`: Armenian to Latin, with the word-initial ye- rule and the ու digraph.

use super::lyrics_romanizer::LyricsRomanizer;

impl LyricsRomanizer {
    /// The whole line is lowercased first, Latin text included, as in Swift.
    /// ու reads "ou" ("vou" word-initially): ո emits "o"/"vo", then ւ after an "o" emits "u".
    pub(crate) fn romanize_armenian(text: &str) -> String {
        let chars: Vec<char> = text.to_lowercase().chars().collect();
        let mut out = String::new();
        let mut at_word_start = true;

        for (index, &c) in chars.iter().enumerate() {
            let Some(mapped) = armenian(c) else {
                out.push(c);
                at_word_start = true;
                continue;
            };
            if c == 'ո' && chars.get(index + 1) == Some(&'ւ') {
                out.push_str(if at_word_start { "vo" } else { "o" });
            } else if c == 'ւ' && out.ends_with('o') {
                out.push('u');
            } else if c == 'ե' && at_word_start {
                out.push_str("ye");
            } else {
                out.push_str(mapped);
            }
            at_word_start = false;
        }
        out
    }
}

#[rustfmt::skip]
fn armenian(c: char) -> Option<&'static str> {
    Some(match c {
        'ա' => "a", 'բ' => "b", 'գ' => "g", 'դ' => "d", 'ե' => "e", 'զ' => "z",
        'է' => "e", 'ը' => "y", 'թ' => "t", 'ժ' => "zh", 'ի' => "i", 'լ' => "l",
        'խ' => "kh", 'ծ' => "ts", 'կ' => "k", 'հ' => "h", 'ձ' => "dz", 'ղ' => "gh",
        'ճ' => "ch", 'մ' => "m", 'յ' => "y", 'ն' => "n", 'շ' => "sh", 'ո' => "o",
        'չ' => "ch", 'պ' => "p", 'ջ' => "j", 'ռ' => "r", 'ս' => "s", 'վ' => "v",
        'տ' => "t", 'ր' => "r", 'ց' => "ts", 'ւ' => "w", 'փ' => "p", 'ք' => "q",
        'օ' => "o", 'ֆ' => "f", 'և' => "ev",
        _ => return None,
    })
}
