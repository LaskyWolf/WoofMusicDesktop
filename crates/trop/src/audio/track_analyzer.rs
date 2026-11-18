//! `Trop/Audio/TrackAnalyzer.swift`: finds where a stretch of audio is loud,
//! its tempo, its beats and the first beat of each bar (the downbeat). Used
//! to plan crossfades and AutoMix. Works on mono samples at `SAMPLE_RATE`.

use std::ops::Range;

/// Swift `TrackAnalyzer.Result`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackAnalysis {
    /// First and last moment the audio is above the quiet threshold, in seconds.
    pub sound_start: f64,
    pub sound_end: f64,
    /// Beats per minute, `None` when no steady beat was found.
    pub bpm: Option<f64>,
    /// Time of the first beat, in seconds.
    pub first_beat: f64,
    /// Time of the first downbeat (beat 1 of a 4/4 bar), in seconds.
    pub downbeat: f64,
    /// Loudness (RMS) of each bar from the downbeat on, relative to the
    /// loudest bar. Empty without a beat.
    pub bar_energy: Vec<f32>,
    /// How much each bar sounds like a sung line (0...1), see `vocal_presence`.
    pub bar_vocal: Vec<f32>,
    pub key: Option<Key>,
}

impl TrackAnalysis {
    pub fn beat_period(&self) -> Option<f64> {
        self.bpm.map(|bpm| 60.0 / bpm)
    }

    pub fn bar_length(&self) -> Option<f64> {
        self.beat_period().map(|p| p * BEATS_PER_BAR as f64)
    }

    /// Seconds where bar `bar` starts; bars count from the downbeat.
    pub fn bar_time(&self, bar: usize) -> f64 {
        self.downbeat + bar as f64 * self.bar_length().unwrap_or(0.0)
    }

    /// Whether a sung line is heard in any bar overlapping `range` (seconds).
    pub fn has_vocals(&self, range: Range<f64>) -> bool {
        self.last_vocal_end(range).is_some()
    }

    /// Seconds where the last sung bar overlapping `range` ends, `None` when none.
    pub fn last_vocal_end(&self, range: Range<f64>) -> Option<f64> {
        let bar_length = self.bar_length()?;
        // A hair of slack so a bar ending on the range's edge doesn't count.
        let slack = bar_length * 0.01;
        (0..self.bar_vocal.len())
            .rev()
            .filter(|&bar| self.bar_vocal[bar] >= VOCAL_THRESHOLD)
            .map(|bar| self.bar_time(bar))
            .find(|&start| start + slack < range.end && start + bar_length - slack > range.start)
            .map(|start| start + bar_length)
    }
}

/// A musical key, estimated from how much each of the 12 notes sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// 0 = C, 1 = C♯ ... 11 = B.
    pub pitch_class: usize,
    pub is_minor: bool,
}

const NAMES: [&str; 12] = [
    "C", "C♯", "D", "E♭", "E", "F", "F♯", "G", "A♭", "A", "B♭", "B",
];

impl Key {
    /// Position on the Camelot wheel: 1...12 plus A (minor) or B (major).
    /// Neighbours on the wheel share most notes, so they mix well.
    pub fn camelot_number(&self) -> usize {
        let major = if self.is_minor {
            (self.pitch_class + 3) % 12
        } else {
            self.pitch_class
        };
        (major * 7 % 12 + 7) % 12 + 1
    }

    pub fn camelot(&self) -> String {
        let letter = if self.is_minor { "A" } else { "B" };
        format!("{}{letter}", self.camelot_number())
    }

    pub fn name(&self) -> String {
        let mode = if self.is_minor { "minor" } else { "major" };
        format!("{} {mode}", NAMES[self.pitch_class])
    }

    /// Same key, its relative major/minor, or one step around the wheel.
    pub fn is_compatible(&self, other: &Key) -> bool {
        if self.camelot_number() == other.camelot_number() {
            return true;
        }
        let steps = (self.camelot_number() + 12 - other.camelot_number()) % 12;
        self.is_minor == other.is_minor && (steps == 1 || steps == 11)
    }
}

pub const SAMPLE_RATE: usize = 11_025;
const HOP: usize = 256;
pub const BEATS_PER_BAR: usize = 4;
/// `bar_vocal` at or above this counts as a sung bar.
pub const VOCAL_THRESHOLD: f32 = 0.3;

fn rms_of(sum: f32, count: usize) -> f32 {
    (sum / count as f32).sqrt()
}

/// Index of the largest value; the first one on a tie, like Swift `max(by:)`.
fn first_max_index(values: &[f32]) -> Option<usize> {
    (0..values.len()).reduce(|best, i| if values[best] < values[i] { i } else { best })
}

/// Swift `analyze(_:sampleRate:)`. `None` for too little audio or silence.
pub fn analyze(samples: &[f32], sample_rate: usize) -> Option<TrackAnalysis> {
    let frame_count = samples.len() / HOP;
    if frame_count <= 16 {
        return None;
    }
    let frame_seconds = HOP as f64 / sample_rate as f64;

    // Loudness per frame (RMS), for the whole signal and for the bass only.
    // A one-pole low-pass at ~150 Hz keeps the kick drum, which usually marks
    // beat 1 of the bar. The 200 Hz–4 kHz band (voice) is the difference of
    // two more one-poles, for the vocal estimate.
    let pole = |hz: f64| (2.0 * std::f64::consts::PI * hz / sample_rate as f64) as f32;
    let (low_pass, voice_low, voice_high) = (pole(150.0), pole(200.0), pole(4000.0));
    let (mut low, mut under, mut upto) = (0f32, 0f32, 0f32);
    let mut rms = vec![0f32; frame_count];
    let mut low_rms = vec![0f32; frame_count];
    let mut voice_rms = vec![0f32; frame_count];
    for frame in 0..frame_count {
        let (mut sum, mut low_sum, mut voice_sum) = (0f32, 0f32, 0f32);
        for &x in &samples[frame * HOP..(frame + 1) * HOP] {
            sum += x * x;
            low += low_pass * (x - low);
            low_sum += low * low;
            under += voice_low * (x - under);
            upto += voice_high * (x - upto);
            let voice = upto - under;
            voice_sum += voice * voice;
        }
        rms[frame] = rms_of(sum, HOP);
        low_rms[frame] = rms_of(low_sum, HOP);
        voice_rms[frame] = rms_of(voice_sum, HOP);
    }

    // "Quiet" = 26 dB below the loudest frame.
    let threshold = rms.iter().copied().fold(0f32, f32::max) * 0.05;
    if threshold <= 0.0 {
        return None;
    }
    let first = rms.iter().position(|&r| r > threshold)?;
    let last = rms.iter().rposition(|&r| r > threshold)?;

    let onset = onsets(&rms);
    let mut result = TrackAnalysis {
        sound_start: first as f64 * frame_seconds,
        sound_end: (last + 1) as f64 * frame_seconds,
        ..Default::default()
    };
    // The kick (the low band) marks the beat more cleanly in some songs, the
    // full mix in others: keep the tempo whose peak stands out more.
    let low_onset = onsets(&low_rms);
    let Some(grid) = [
        beat_grid(&onset, frame_seconds),
        beat_grid(&low_onset, frame_seconds),
    ]
    .into_iter()
    .flatten()
    .reduce(|best, g| if best.2 < g.2 { g } else { best }) else {
        return Some(result);
    };
    let (period, phase, _) = grid;
    result.bpm = Some(60.0 / (period * frame_seconds));
    result.first_beat = phase * frame_seconds;

    // Downbeat: the beat of the bar where the bass hits hardest.
    let mut accents = [0f32; BEATS_PER_BAR];
    let mut position = phase;
    let mut beat = 0;
    while (position as usize) < low_onset.len() {
        accents[beat % BEATS_PER_BAR] += low_onset[position as usize];
        position += period;
        beat += 1;
    }
    let strongest = first_max_index(&accents).unwrap_or(0);
    result.downbeat = (phase + strongest as f64 * period) * frame_seconds;

    // Per bar: loudness and vocal presence, on the bar grid.
    let bar_frames = period * BEATS_PER_BAR as f64;
    let mut bar_start = phase + strongest as f64 * period;
    let mut energy = Vec::new();
    while (bar_start + bar_frames) as usize <= frame_count {
        let frames = bar_start as usize..(bar_start + bar_frames) as usize;
        let mean = rms[frames.clone()].iter().map(|r| r * r).sum::<f32>() / frames.len() as f32;
        energy.push(mean.sqrt());
        result
            .bar_vocal
            .push(vocal_presence(&voice_rms[frames.clone()], &rms[frames]));
        bar_start += bar_frames;
    }
    let loudest = energy.iter().copied().fold(0f32, f32::max);
    result.bar_energy = if loudest > 0.0 {
        energy.iter().map(|e| e / loudest).collect()
    } else {
        energy
    };
    result.key = key(&chroma(samples, sample_rate));
    Some(result)
}

fn mean4(energy: &[f32], range: Range<usize>) -> f32 {
    energy[range].iter().sum::<f32>() / 4.0
}

/// Bars that start a new phrase: where the next 4 bars are clearly louder or
/// quieter than the 4 before (a drop, a break, a chorus).
pub fn phrase_boundaries(energy: &[f32]) -> Vec<usize> {
    const SPAN: usize = 4;
    if energy.len() < 2 * SPAN {
        return Vec::new();
    }
    let mut boundaries: Vec<usize> = Vec::new();
    for bar in SPAN..=(energy.len() - SPAN) {
        let before = mean4(energy, bar - SPAN..bar);
        let after = mean4(energy, bar..bar + SPAN);
        if before <= 0.0 || after <= 0.0 {
            continue;
        }
        let change = after / before;
        if !(change > 1.4 || change < 0.7) {
            continue;
        }
        // One boundary per change: keep the bar with the sharpest step.
        if let Some(&last) = boundaries.last()
            && bar - last < SPAN
        {
            let last_change = mean4(energy, last..last + SPAN) / mean4(energy, last - SPAN..last);
            if change.ln().abs() <= last_change.ln().abs() {
                continue;
            }
            boundaries.pop();
        }
        boundaries.push(bar);
    }
    boundaries
}

/// The bar where the song's energy falls and stays low: the start of its
/// outro. `None` when the song keeps going until it stops.
pub fn outro_bar(energy: &[f32]) -> Option<usize> {
    phrase_boundaries(energy).into_iter().rev().find(|&bar| {
        let before = mean4(energy, bar - 4..bar);
        energy[bar] < before && energy[bar..].iter().all(|&e| e < before * 0.85)
    })
}

/// The first bar where the song's energy rises sharply: its first strong
/// phrase, after the intro. `None` when it starts at full energy.
pub fn first_rise_bar(energy: &[f32]) -> Option<usize> {
    phrase_boundaries(energy)
        .into_iter()
        .find(|&bar| energy[bar] > mean4(energy, bar - 4..bar))
}

/// How much a bar sounds like singing (0...1): the share of its loudness in
/// the voice band, scaled down when that share barely moves. Sung lines come
/// and go within a bar; pads and drones hold still.
// ponytail: snares and hi-hats live in the same band and fluctuate too, so
// loud drum breaks count as vocals; a learned model would tell them apart.
pub fn vocal_presence(voice: &[f32], total: &[f32]) -> f32 {
    let count = voice.len() as f32;
    if voice.is_empty() {
        return 0.0;
    }
    let energy: f32 = total.iter().map(|x| x * x).sum();
    if energy <= 0.0 {
        return 0.0;
    }
    let share = voice.iter().map(|x| x * x).sum::<f32>() / energy;
    let mean = voice.iter().sum::<f32>() / count;
    if mean <= 0.0 {
        return 0.0;
    }
    let variance = voice.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / count;
    let movement = variance.sqrt() / mean;
    (share * (movement / 0.25).min(1.0)).min(1.0)
}

/// Krumhansl-Schmuckler key profiles: how much each scale degree is heard in
/// major and minor music, from the tonic up.
const MAJOR_PROFILE: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const MINOR_PROFILE: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.6, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

/// How much each of the 12 notes sounds over the whole signal, from C3 to
/// B5. One Goertzel filter per note and second.
pub fn chroma(samples: &[f32], sample_rate: usize) -> [f32; 12] {
    let mut chroma = [0f32; 12];
    let block = sample_rate;
    if samples.len() < block {
        return chroma;
    }
    for tone in 0..36 {
        let frequency = 130.8128 * 2f64.powf(tone as f64 / 12.0);
        let coefficient =
            (2.0 * (2.0 * std::f64::consts::PI * frequency / sample_rate as f64).cos()) as f32;
        for chunk in samples.chunks_exact(block) {
            let (mut s1, mut s2) = (0f32, 0f32);
            for &x in chunk {
                let s = x + coefficient * s1 - s2;
                s2 = s1;
                s1 = s;
            }
            let power = s1 * s1 + s2 * s2 - coefficient * s1 * s2;
            chroma[tone % 12] += power.max(0.0).sqrt();
        }
    }
    chroma
}

/// Swift `key(from:)`: the key whose profile best matches the chroma
/// (Pearson correlation over the 24 keys). `None` for silence.
pub fn key(chroma: &[f32]) -> Option<Key> {
    if chroma.len() != 12 || !chroma.iter().any(|&c| c > 0.0) {
        return None;
    }
    let mut best: Option<(f32, Key)> = None;
    for (is_minor, profile) in [(false, MAJOR_PROFILE), (true, MINOR_PROFILE)] {
        for tonic in 0..12 {
            let rotated: Vec<f32> = (0..12).map(|i| profile[(i + 12 - tonic) % 12]).collect();
            let score = correlation(chroma, &rotated);
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((
                    score,
                    Key {
                        pitch_class: tonic,
                        is_minor,
                    },
                ));
            }
        }
    }
    best.map(|(_, key)| key)
}

fn correlation(a: &[f32], b: &[f32]) -> f32 {
    let count = a.len() as f32;
    let mean_a = a.iter().sum::<f32>() / count;
    let mean_b = b.iter().sum::<f32>() / count;
    let (mut cross, mut var_a, mut var_b) = (0f32, 0f32, 0f32);
    for (x, y) in a.iter().zip(b) {
        cross += (x - mean_a) * (y - mean_b);
        var_a += (x - mean_a) * (x - mean_a);
        var_b += (y - mean_b) * (y - mean_b);
    }
    let denom = (var_a * var_b).sqrt();
    if denom > 0.0 { cross / denom } else { 0.0 }
}

/// Onset strength: how much louder each frame is than the one before. Drum
/// hits make sharp jumps; the beat is the rhythm of those jumps.
fn onsets(levels: &[f32]) -> Vec<f32> {
    let mut onset = vec![0f32; levels.len()];
    for frame in 1..levels.len() {
        onset[frame] = ((levels[frame] + 1e-6).ln() - (levels[frame - 1] + 1e-6).ln()).max(0.0);
    }
    onset
}

/// Beat period and offset in frames, and how far the tempo peak stands above
/// the average lag. `None` when there is no steady beat.
fn beat_grid(onset: &[f32], frame_seconds: f64) -> Option<(f64, f64, f32)> {
    let (min_bpm, max_bpm) = (70.0, 180.0);
    let min_lag = ((60.0 / max_bpm / frame_seconds).floor() as usize).max(1);
    let max_lag = (60.0 / min_bpm / frame_seconds).ceil() as usize;
    // Without clear hits (a 10% jump in loudness) there is no beat to find.
    if onset.len() <= max_lag * 4 || onset.iter().copied().fold(0f32, f32::max) <= 0.1 {
        return None;
    }

    // Autocorrelation: the lag where the onset curve best matches a shifted
    // copy of itself is the beat period.
    let scores: Vec<f32> = (min_lag..=max_lag)
        .map(|lag| {
            let sum: f32 = (lag..onset.len()).map(|i| onset[i] * onset[i - lag]).sum();
            sum / (onset.len() - lag) as f32
        })
        .collect();
    let peak = first_max_index(&scores)?;
    let mean = scores.iter().sum::<f32>() / scores.len() as f32;
    // A flat curve means no steady beat (ambient, speech).
    if mean <= 0.0 || scores[peak] <= mean * 1.3 {
        return None;
    }
    let prominence = scores[peak] / mean;

    // Refine the lag between frames with a parabola through the peak.
    let mut lag = (peak + min_lag) as f64;
    if peak > 0 && peak < scores.len() - 1 {
        let (a, b, c) = (
            scores[peak - 1] as f64,
            scores[peak] as f64,
            scores[peak + 1] as f64,
        );
        let denom = a - 2.0 * b + c;
        if denom != 0.0 {
            lag += 0.5 * (a - c) / denom;
        }
    }
    let mut bpm = 60.0 / (lag * frame_seconds);
    // Fold into the range DJs count in, so 70 and 140 compare as equal.
    while bpm < 85.0 {
        bpm *= 2.0;
    }
    while bpm > 170.0 {
        bpm /= 2.0;
    }

    // Beat grid: the period and offset whose beats land on the most onsets.
    // Fine-tuning the period too matters: a tempo off by 0.5 BPM drifts 70 ms
    // over 40 beats.
    let base_period = 60.0 / bpm / frame_seconds;
    let (mut best_score, mut best_period, mut best_phase) = (-1f32, base_period, 0.0);
    for step in -20..=20 {
        let period = base_period * (1.0 + step as f64 * 0.001);
        let mut phase = 0.0;
        while phase < period {
            let mut sum = 0f32;
            let mut position = phase;
            while (position as usize) < onset.len() {
                sum += onset[position as usize];
                position += period;
            }
            if sum > best_score {
                (best_score, best_period, best_phase) = (sum, period, phase);
            }
            phase += 0.5;
        }
    }
    Some((best_period, best_phase, prominence))
}
