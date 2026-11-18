//! `Trop/Audio/Equalizer.swift`: the bands, the gain range and the presets,
//! and the mpv filter that applies them (Swift `PlayerController.filterChain`).

use crate::audio::crossfade::fmt;

/// Band centre frequencies in Hz.
pub const FREQUENCIES: [f64; 10] = [
    60.0, 120.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 12000.0, 16000.0,
];

/// Gain bounds in dB, Spotify's ±12 dB.
pub const MIN_GAIN: f64 = -12.0;
pub const MAX_GAIN: f64 = 12.0;

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub gains: [f64; 10],
}

const fn preset(id: &'static str, name: &'static str, gains: [f64; 10]) -> Preset {
    Preset { id, name, gains }
}

/// The id Swift stores when the user dragged a band away from every preset.
pub const CUSTOM: &str = "custom";

pub const PRESETS: [Preset; 15] = [
    preset(
        "flat",
        "Flat",
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    ),
    preset(
        "bassBoost",
        "Bass Boost",
        [7.0, 6.0, 4.0, 2.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    ),
    preset(
        "bassReducer",
        "Bass Reducer",
        [-7.0, -6.0, -4.0, -2.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    ),
    preset(
        "trebleBoost",
        "Treble Boost",
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 6.0, 7.0],
    ),
    preset(
        "trebleReducer",
        "Treble Reducer",
        [0.0, 0.0, 0.0, 0.0, 0.0, -1.0, -2.0, -4.0, -6.0, -7.0],
    ),
    preset(
        "vShape",
        "V-Shape",
        [6.0, 5.0, 3.0, 1.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0],
    ),
    preset(
        "rock",
        "Rock",
        [5.0, 4.0, 2.0, 0.0, -1.0, -1.0, 0.0, 2.0, 4.0, 5.0],
    ),
    preset(
        "pop",
        "Pop",
        [-1.0, 0.0, 1.0, 3.0, 3.0, 2.0, 1.0, 0.0, -1.0, -1.0],
    ),
    preset(
        "hiphop",
        "Hip-Hop",
        [6.0, 5.0, 3.0, 1.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0],
    ),
    preset(
        "electronic",
        "Electronic",
        [5.0, 4.0, 3.0, 1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
    ),
    preset(
        "jazz",
        "Jazz",
        [3.0, 2.0, 1.0, 2.0, 1.0, 0.0, 1.0, 2.0, 3.0, 4.0],
    ),
    preset(
        "classical",
        "Classical",
        [4.0, 3.0, 2.0, 1.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0],
    ),
    preset(
        "acoustic",
        "Acoustic",
        [4.0, 3.0, 2.0, 1.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0],
    ),
    preset(
        "vocal",
        "Vocal",
        [-2.0, -1.0, 0.0, 1.0, 3.0, 4.0, 3.0, 1.0, 0.0, -2.0],
    ),
    preset(
        "loudness",
        "Loudness",
        [4.0, 3.0, 2.0, 1.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0],
    ),
];

/// The `firequalizer` filter for `gains` (one per band), as Swift writes it.
pub fn filter(gains: &[f64]) -> Option<String> {
    let entries: Vec<String> = FREQUENCIES
        .iter()
        .zip(gains)
        .map(|(frequency, gain)| format!("entry({},{})", fmt(*frequency), fmt(*gain)))
        .collect();
    (!entries.is_empty()).then(|| format!("firequalizer=gain_entry=\"{}\"", entries.join(";")))
}
