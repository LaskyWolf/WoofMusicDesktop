//! `LyricsRomanizer+Thai.swift`: Thai to Latin, RTGS-style with syllable-aware parsing: implicit
//! vowels, initial vs final consonant forms, clusters (คร ปล …), preposed vowels and compounds
//! (เ-า, เ-ีย, โ-ะ), carrier อ, and the silencing ์.

use std::ops::Range;

use super::lyrics_romanizer::LyricsRomanizer;

impl LyricsRomanizer {
    pub(crate) fn romanize_thai(text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let s: Vec<u32> = chars.iter().map(|&c| c as u32).collect();
        let mut out = String::new();
        // Text of the last consonant syllable, removed again by a following ์.
        let mut last_consonant: Option<Range<usize>> = None;
        let mut i = 0;
        // The previous syllable ended on an open vowel, so a following vowel-less consonant closes it.
        let mut open_vowel_pending = false;

        while i < s.len() {
            let value = s[i];

            // Tone marks and diacritics carry no sound.
            if is_skippable(value) {
                i += 1;
                continue;
            }
            // ์ silences the preceding consonant.
            if value == 0x0E4C {
                if let Some(range) = last_consonant.take() {
                    out.replace_range(range, "");
                }
                open_vowel_pending = false;
                i += 1;
                continue;
            }
            // ๏ is skipped.
            if value == 0x0E4F {
                i += 1;
                continue;
            }

            // Standalone following vowel (after a finished syllable).
            if let Some(v) = following_vowel(value) {
                emit(&mut out, &mut last_consonant, v, false);
                open_vowel_pending = true;
                i += 1;
                continue;
            }

            // Preposed vowels open their syllable: consonant first, then the vowel (ไทย → thai, เขา → khao).
            if (0x0E40..=0x0E44).contains(&value) {
                let (text, tracked, consumed, ends_open) =
                    preposed_syllable(&s, i + 1, pre_vowel(value));
                emit(&mut out, &mut last_consonant, &text, tracked);
                open_vowel_pending = ends_open;
                i = consumed;
                continue;
            }

            let Some(initial) = initial_form(value) else {
                out.push(chars[i]);
                last_consonant = None;
                open_vowel_pending = false;
                i += 1;
                continue;
            };

            // Collect this consonant's vowel signs, skipping tone marks.
            let (vowel, j) = collect_vowels(&s, i + 1);

            // Next significant token after the vowel.
            let mut k = j;
            while k < s.len() && is_skippable(s[k]) {
                k += 1;
            }
            let has_next_consonant = k < s.len() && initial_form(s[k]).is_some();
            let at_end = k >= s.len() || !is_letterish(s[k]);

            // อ carries vowels silently: with its own vowel it is silent, alone it reads "o".
            if value == 0x0E2D {
                emit(
                    &mut out,
                    &mut last_consonant,
                    if vowel.is_empty() { "o" } else { &vowel },
                    true,
                );
                open_vowel_pending = true;
                i = j;
                continue;
            }

            if !vowel.is_empty() {
                if has_next_consonant && !has_own_vowel(&s, k + 1) {
                    // Explicit vowel + closing consonant (วัส → wat).
                    let closing = final_form(s[k]).unwrap_or("");
                    emit(
                        &mut out,
                        &mut last_consonant,
                        &format!("{initial}{vowel}{closing}"),
                        true,
                    );
                    open_vowel_pending = false;
                    i = k + 1;
                    continue;
                }
                // Word-final long "i" reads "ee" (ดี → dee).
                let rendered = if at_end && vowel == "i" { "ee" } else { &vowel };
                emit(
                    &mut out,
                    &mut last_consonant,
                    &format!("{initial}{rendered}"),
                    true,
                );
                open_vowel_pending = true;
                i = j;
                continue;
            }

            // No written vowel on this consonant.
            if has_next_consonant {
                if open_vowel_pending {
                    // Closes the previous open syllable (ขอบ → khop).
                    emit(
                        &mut out,
                        &mut last_consonant,
                        final_form(value).unwrap_or(""),
                        true,
                    );
                    open_vowel_pending = false;
                    i += 1;
                    continue;
                }
                if has_own_vowel(&s, k + 1) {
                    if is_cluster_lead(value, s[k]) {
                        // Cluster lead: ครับ → kh-…
                        emit(&mut out, &mut last_consonant, initial, true);
                        i = k;
                        continue;
                    }
                    // Isolated leading syllable with implicit "a" (สวัสดี → sa-…).
                    emit(&mut out, &mut last_consonant, &format!("{initial}a"), true);
                    open_vowel_pending = false;
                    i = k;
                    continue;
                }
                // Consonant pair closing a word: implicit "o" between them (ผม → phom, คน → khon).
                // A carrier-อ second letter keeps the syllable open so the next consonant closes it.
                let second_is_carrier = s[k] == 0x0E2D;
                let closing = if second_is_carrier {
                    ""
                } else {
                    final_form(s[k]).unwrap_or("")
                };
                emit(
                    &mut out,
                    &mut last_consonant,
                    &format!("{initial}o{closing}"),
                    true,
                );
                open_vowel_pending = second_is_carrier;
                i = k + 1;
                continue;
            }

            // Trailing bare consonant takes its final form (ไทย → thai).
            emit(
                &mut out,
                &mut last_consonant,
                final_form(value).unwrap_or(initial),
                true,
            );
            open_vowel_pending = false;
            i = j;
        }
        out
    }
}

/// Appends `text`; a tracked emit is the span a following ์ removes.
fn emit(out: &mut String, last_consonant: &mut Option<Range<usize>>, text: &str, tracked: bool) {
    let start = out.len();
    out.push_str(text);
    *last_consonant = tracked.then_some(start..out.len());
}

/// Following vowel signs from `start`, skipping tone marks, and the index after them.
fn collect_vowels(s: &[u32], start: usize) -> (String, usize) {
    let mut vowel = String::new();
    let mut j = start;
    while j < s.len() {
        if is_skippable(s[j]) {
            j += 1;
        } else if let Some(v) = following_vowel(s[j]) {
            vowel.push_str(v);
            j += 1;
        } else {
            break;
        }
    }
    (vowel, j)
}

/// Consonant + vowel of a preposed-vowel syllable whose consonant is at `i`, folding compounds:
/// เ-า → ao, เ-ีย → ia, เ-C-ย → oe, โ-ะ → o.
/// Returns the text, whether it is tracked for ์, the next index and whether the syllable stays open.
/// ะ is collected with the other following vowels, so the โ-ะ branch never matches, as in Swift.
// ponytail: the เ-C-า branch returns one index too far, as in Swift, so the character after า is
// dropped (เขาไป → khaop). Return `j` there once the Swift file is fixed too.
fn preposed_syllable(s: &[u32], i: usize, base: &str) -> (String, bool, usize, bool) {
    let Some(initial) = s.get(i).and_then(|&v| initial_form(v)) else {
        return (base.to_string(), false, i, true);
    };

    let (extra, j) = collect_vowels(s, i + 1);
    let tail_is_sara_a = s.get(j) == Some(&0x0E30);
    let tail_is_yod = s.get(j) == Some(&0x0E22);

    if base == "e" && extra == "aa" {
        return (format!("{initial}ao"), true, j + 1, false);
    }
    if base == "e" && tail_is_yod {
        let vowel = if extra.contains('i') { "ia" } else { "oe" };
        return (format!("{initial}{vowel}"), true, j + 1, false);
    }
    if base == "o" && tail_is_sara_a {
        return (format!("{initial}o"), true, j + 1, false);
    }
    if base == "ai" && tail_is_yod {
        // ไ/ใ-C-ย: the yod is already inside the diphthong.
        return (format!("{initial}ai"), true, j + 1, false);
    }

    (format!("{initial}{base}{extra}"), true, j, true)
}

fn is_cluster_lead(lead: u32, follow: u32) -> bool {
    matches!(
        lead,
        0x0E01 | 0x0E02 | 0x0E03 | 0x0E04 | 0x0E15 | 0x0E17 | 0x0E1B | 0x0E1C | 0x0E1E
    ) && matches!(follow, 0x0E23 | 0x0E25 | 0x0E27)
}

fn is_letterish(v: u32) -> bool {
    initial_form(v).is_some() || following_vowel(v).is_some() || is_skippable(v)
}

fn is_skippable(v: u32) -> bool {
    matches!(v, 0x0E47..=0x0E4B | 0x0E4D | 0x0E3A)
}

/// Whether the consonant before `index` carries its own vowel (a sign or a preposed vowel).
fn has_own_vowel(s: &[u32], index: usize) -> bool {
    let mut k = index;
    while k < s.len() && is_skippable(s[k]) {
        k += 1;
    }
    k < s.len() && (following_vowel(s[k]).is_some() || (0x0E40..=0x0E44).contains(&s[k]))
}

// ponytail: same vowel list as Swift; ิ (U+0E34, short i) is missing and passes through unchanged
// (คิด). Add it here and in Swift together.
fn following_vowel(v: u32) -> Option<&'static str> {
    Some(match v {
        0x0E30 | 0x0E31 => "a",
        0x0E32 => "aa",
        0x0E33 => "am",
        0x0E35 => "i",
        0x0E36 | 0x0E37 => "ue",
        0x0E38 => "u",
        0x0E39 => "uu",
        _ => return None,
    })
}

fn pre_vowel(v: u32) -> &'static str {
    match v {
        0x0E40 => "e",
        0x0E41 => "ae",
        0x0E42 => "o",
        0x0E43 | 0x0E44 => "ai",
        _ => "",
    }
}

// ponytail: rows U+0E05–U+0E0C copy the Swift table, which is shifted there: ง (U+0E07) reads
// "ch"/"t" instead of "ng", ฉ reads "s", ฌ reads "t". Fix both tables here and in Swift together.
#[rustfmt::skip]
fn initial_form(v: u32) -> Option<&'static str> {
    Some(match v {
        0x0E01 => "k", 0x0E02 => "kh", 0x0E03 => "kh", 0x0E04 => "kh", 0x0E05 => "ng",
        0x0E06 => "ch", 0x0E07 => "ch", 0x0E08 => "ch", 0x0E09 => "s", 0x0E0A => "ch",
        0x0E0B => "s", 0x0E0C => "t", 0x0E0D => "y", 0x0E0E => "d", 0x0E0F => "t",
        0x0E10 => "th", 0x0E11 => "th", 0x0E12 => "th", 0x0E13 => "n", 0x0E14 => "d",
        0x0E15 => "t", 0x0E16 => "th", 0x0E17 => "th", 0x0E18 => "th", 0x0E19 => "n",
        0x0E1A => "b", 0x0E1B => "p", 0x0E1C => "ph", 0x0E1D => "f", 0x0E1E => "ph",
        0x0E1F => "f", 0x0E20 => "ph", 0x0E21 => "m", 0x0E22 => "y", 0x0E23 => "r",
        0x0E25 => "l", 0x0E26 => "l", 0x0E27 => "w", 0x0E28 => "s", 0x0E29 => "s",
        0x0E2A => "s", 0x0E2B => "h", 0x0E2D => "o", 0x0E2E => "h",
        _ => return None,
    })
}

#[rustfmt::skip]
fn final_form(v: u32) -> Option<&'static str> {
    Some(match v {
        0x0E01 => "k", 0x0E02 => "k", 0x0E03 => "k", 0x0E04 => "k", 0x0E05 => "ng",
        0x0E06 => "t", 0x0E07 => "t", 0x0E08 => "t", 0x0E09 => "t", 0x0E0A => "t",
        0x0E0B => "t", 0x0E0C => "t", 0x0E0D => "i", 0x0E0E => "t", 0x0E0F => "t",
        0x0E10 => "t", 0x0E11 => "t", 0x0E12 => "t", 0x0E13 => "n", 0x0E14 => "t",
        0x0E15 => "t", 0x0E16 => "t", 0x0E17 => "t", 0x0E18 => "t", 0x0E19 => "n",
        0x0E1A => "p", 0x0E1B => "p", 0x0E1C => "t", 0x0E1D => "t", 0x0E1E => "p",
        0x0E1F => "t", 0x0E20 => "p", 0x0E21 => "m", 0x0E22 => "i", 0x0E23 => "n",
        0x0E25 => "n", 0x0E26 => "n", 0x0E27 => "o", 0x0E28 => "t", 0x0E29 => "t",
        0x0E2A => "t", 0x0E2B => "", 0x0E2D => "", 0x0E2E => "",
        _ => return None,
    })
}
