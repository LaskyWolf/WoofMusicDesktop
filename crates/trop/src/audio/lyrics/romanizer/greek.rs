//! `LyricsRomanizer+Greek.swift`: modern Greek to Latin, with common digraphs and accented forms.

use super::lyrics_romanizer::{LyricsRomanizer, capitalized};

impl LyricsRomanizer {
    pub(crate) fn romanize_greek(text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut out = String::new();
        let mut i = 0;
        let mut at_word_start = true;

        while i < chars.len() {
            let c = chars[i];
            let lower = lowercased(c);
            let upper = c.is_uppercase();

            // Common digraphs first.
            if let Some(&next) = chars.get(i + 1) {
                let digraph = match (lower, lowercased(next)) {
                    ('ο', 'υ') => Some(if upper { "OU" } else { "ou" }),
                    ('α', 'υ') => Some(if upper { "Av" } else { "av" }),
                    ('ε', 'υ') => Some(if upper { "Ev" } else { "ev" }),
                    ('μ', 'π') => Some(if upper { "B" } else { "b" }),
                    ('ν', 'τ') => Some(if upper { "D" } else { "d" }),
                    // Double gamma is always the nasal [ŋɡ].
                    ('γ', 'γ') => Some(if upper { "Ng" } else { "ng" }),
                    // Hard g word-initially, nasal cluster medially.
                    ('γ', 'κ') => Some(match (upper, at_word_start) {
                        (true, true) => "G",
                        (true, false) => "Ng",
                        (false, true) => "g",
                        (false, false) => "ng",
                    }),
                    ('γ', 'χ') => Some(if upper { "Nch" } else { "nch" }),
                    ('γ', 'ξ') => Some(if upper { "Nx" } else { "nx" }),
                    _ => None,
                };
                if let Some(digraph) = digraph {
                    out.push_str(digraph);
                    at_word_start = false;
                    i += 2;
                    continue;
                }
            }

            let mapped = greek(lower);
            match mapped {
                Some(mapped) if upper => out.push_str(&capitalized(mapped)),
                Some(mapped) => out.push_str(mapped),
                None => out.push(c),
            }
            at_word_start = mapped.is_none();
            i += 1;
        }
        out
    }
}

fn lowercased(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

#[rustfmt::skip]
fn greek(c: char) -> Option<&'static str> {
    Some(match c {
        'α' => "a", 'β' => "v", 'γ' => "g", 'δ' => "d", 'ε' => "e", 'ζ' => "z",
        'η' => "i", 'θ' => "th", 'ι' => "i", 'κ' => "k", 'λ' => "l", 'μ' => "m",
        'ν' => "n", 'ξ' => "x", 'ο' => "o", 'π' => "p", 'ρ' => "r", 'σ' => "s",
        'ς' => "s", 'τ' => "t", 'υ' => "i", 'φ' => "f", 'χ' => "ch", 'ψ' => "ps",
        'ω' => "o",
        // Accented and diaeresis forms.
        'ά' => "a", 'έ' => "e", 'ή' => "i", 'ί' => "i", 'ό' => "o", 'ύ' => "i",
        'ώ' => "o", 'ϊ' => "i", 'ϋ' => "i", 'ΐ' => "i", 'ΰ' => "i",
        _ => return None,
    })
}
