//! `LyricsRomanizer.swift`: script detection and the phoneme helper shared by Arabic and Hebrew.

/// Lightweight script transliteration.
pub struct LyricsRomanizer;

/// Emitted text plus whether it carries a vowel sound.
pub(crate) type Phoneme = (String, bool);

/// The script that decides which romanizer handles a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Kana,
    Hangul,
    Han,
    Devanagari,
    Greek,
    Cyrillic,
    Thai,
    Hebrew,
    Arabic,
    Georgian,
    Armenian,
}

impl LyricsRomanizer {
    /// Romanized line, or `None` when the line has no supported script.
    pub fn romanize(text: &str) -> Option<String> {
        Some(match Self::detect_script(text)? {
            Script::Kana => Self::romanize_japanese(text),
            Script::Hangul => Self::romanize_korean(text),
            Script::Han => Self::romanize_chinese(text),
            Script::Devanagari => Self::romanize_devanagari(text),
            Script::Greek => Self::romanize_greek(text),
            Script::Cyrillic => Self::romanize_cyrillic(text),
            Script::Thai => Self::romanize_thai(text),
            Script::Hebrew => Self::romanize_hebrew(text),
            Script::Arabic => Self::romanize_arabic(text),
            Script::Georgian => Self::romanize_georgian(text),
            Script::Armenian => Self::romanize_armenian(text),
        })
    }

    /// Interleaves an epenthetic vowel between adjacent consonant phonemes
    /// so unvowelled words stay pronounceable.
    pub(crate) fn insert_epenthetic(
        phonemes: &[Phoneme],
        vowel: &str,
        starts_word: &[bool],
    ) -> Vec<Phoneme> {
        let mut fixed = Vec::with_capacity(phonemes.len() * 2);
        for (index, phoneme) in phonemes.iter().enumerate() {
            let begins_word = starts_word.get(index).copied().unwrap_or(false);
            if index > 0 && !begins_word {
                let previous = &phonemes[index - 1];
                if !previous.1 && !phoneme.1 && !previous.0.is_empty() && !phoneme.0.is_empty() {
                    fixed.push((vowel.to_string(), true));
                }
            }
            fixed.push(phoneme.clone());
        }
        fixed
    }

    /// The highest-ranked script in the line. Kana outranks han so Japanese lines stay Japanese.
    pub fn detect_script(text: &str) -> Option<Script> {
        let mut best = None;
        let mut best_rank = 0;
        for c in text.chars() {
            let (script, rank) = match c as u32 {
                0x3040..=0x30FF | 0x31F0..=0x31FF => (Script::Kana, 50),
                0xAC00..=0xD7AF | 0x1100..=0x11FF | 0x3130..=0x318F => (Script::Hangul, 40),
                0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => (Script::Han, 30),
                0x0900..=0x097F => (Script::Devanagari, 28),
                0x0370..=0x03FF | 0x1F00..=0x1FFF => (Script::Greek, 26),
                0x0400..=0x04FF => (Script::Cyrillic, 20),
                0x0E00..=0x0E7F => (Script::Thai, 18),
                0x0590..=0x05FF => (Script::Hebrew, 14),
                0x0600..=0x06FF | 0x0750..=0x077F => (Script::Arabic, 12),
                0x10A0..=0x10FF | 0x1C90..=0x1CBF => (Script::Georgian, 10),
                0x0530..=0x058F => (Script::Armenian, 8),
                _ => continue,
            };
            if rank > best_rank {
                best_rank = rank;
                best = Some(script);
            }
        }
        best
    }
}

/// Swift's `String.capitalized` for the one-word lowercase readings in the tables.
pub(crate) fn capitalized(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
