//! `Trop/Audio/MixPlanner.swift`: decides where a transition between two
//! songs starts and how it runs, from the analysis of the outgoing song's end
//! and the incoming song's start. Pure math.

use std::ops::Range;

use super::track_analyzer::{self, BEATS_PER_BAR, TrackAnalysis};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mix {
    /// Outgoing position where the transition starts, in seconds.
    pub fade_start: f64,
    pub fade_length: f64,
    /// Incoming position the transition starts from.
    pub in_start: f64,
    /// Incoming playback speed during the transition (1 = unchanged).
    pub speed: f64,
    /// Beat-matched mix (AutoMix): its length in beats and the outgoing beat
    /// length in seconds. `None` for a timed crossfade.
    pub beats: Option<usize>,
    pub beat_period: Option<f64>,
    /// Mix progress (0...1) until which the incoming song is held down
    /// because both songs sing over each other. `None` when neither does.
    pub duck_until: Option<f64>,
    /// Whether the two keys mix well; `None` when a key is unknown.
    pub keys_compatible: Option<bool>,
}

impl Mix {
    pub fn is_beat_matched(&self) -> bool {
        self.beats.is_some()
    }
}

/// Largest tempo change a crossfade applies quietly, and the largest an
/// AutoMix applies (about what a DJ pitch fader allows).
pub const MAX_TEMPO_NUDGE: f64 = 0.06;
pub const MAX_TEMPO_MATCH: f64 = 0.08;
pub const MAX_INTRO_SKIP: f64 = 10.0;
/// Intro an AutoMix may skip to land the incoming song's first strong phrase
/// on the end of the mix.
pub const MAX_PHRASE_SKIP: f64 = 30.0;
/// AutoMix lengths to try, longest first. Multiples of 8 so the bass swap
/// halfway lands on a bar line.
pub const MIX_BEATS: [usize; 3] = [32, 16, 8];
/// Bars the mix start may move earlier to begin where nobody sings.
pub const MAX_VOCAL_SHIFT: usize = 4;
/// Incoming level (linear) while both songs sing.
pub const DUCK_LEVEL: f64 = 0.5;

/// Smart Crossfade: a timed equal-power fade that ends where the outgoing
/// song goes quiet and starts on one of its beats. Without analysis it falls
/// back to a plain fade that ends with the song.
pub fn smart(
    duration: f64,
    fade_length: f64,
    tail: Option<&TrackAnalysis>,
    tail_start: f64,
    head: Option<&TrackAnalysis>,
) -> Mix {
    let out_end = duration.min(tail.map_or(duration, |t| tail_start + t.sound_end));
    let earliest = duration * 0.5;
    let mut fade_start = earliest.max(out_end - fade_length);
    if let Some(tail) = tail
        && let Some(period) = tail.beat_period()
    {
        let beat = snap(fade_start, tail_start + tail.first_beat, period, f64::floor);
        if beat >= earliest {
            fade_start = beat;
        }
    }

    let mut in_start = 0.0;
    if let Some(head) = head {
        in_start = head.sound_start;
        if let Some(period) = head.beat_period() {
            in_start = snap(head.sound_start, head.first_beat, period, f64::ceil);
        }
        if in_start > MAX_INTRO_SKIP {
            in_start = 0.0;
        }
    }

    let mut speed = 1.0;
    if let (Some(out_bpm), Some(in_bpm)) = (tail.and_then(|t| t.bpm), head.and_then(|h| h.bpm))
        && (out_bpm / in_bpm - 1.0).abs() <= MAX_TEMPO_NUDGE
    {
        speed = out_bpm / in_bpm;
    }
    Mix {
        fade_start,
        fade_length,
        in_start: in_start.max(0.0),
        speed,
        ..Default::default()
    }
}

/// AutoMix: a beat-matched transition of 8 to 32 beats that starts on an
/// outgoing downbeat, brings the incoming song in on its own downbeat at the
/// outgoing tempo, and ends before the outgoing song goes quiet. It starts
/// where the outgoing song's outro begins when one is heard, avoids starting
/// over a sung bar, and lands the incoming song's first strong phrase on the
/// end of the mix. Clashing keys or vocals on both sides shorten the mix.
/// `None` when either song has no steady beat or the tempos are too far
/// apart; the caller then uses `smart`.
pub fn auto_mix(
    duration: f64,
    tail: Option<&TrackAnalysis>,
    tail_start: f64,
    head: Option<&TrackAnalysis>,
) -> Option<Mix> {
    let (tail, head) = (tail?, head?);
    let (out_bpm, in_bpm) = (tail.bpm?, head.bpm?);
    let speed = out_bpm / in_bpm;
    if (speed - 1.0).abs() > MAX_TEMPO_MATCH {
        return None;
    }

    let period = 60.0 / out_bpm;
    let bar = period * BEATS_PER_BAR as f64;
    let out_end = duration.min(tail_start + tail.sound_end);
    let first_downbeat = tail_start + tail.downbeat;
    let earliest = first_downbeat.max(duration * 0.5);
    let outro = track_analyzer::outro_bar(&tail.bar_energy).map(|b| tail_start + tail.bar_time(b));

    let in_bar = 60.0 / in_bpm * BEATS_PER_BAR as f64;
    let first_bar = snap(head.sound_start, head.downbeat, in_bar, f64::ceil);
    let default_in = if first_bar > MAX_INTRO_SKIP {
        head.downbeat
    } else {
        first_bar
    };
    let rise = track_analyzer::first_rise_bar(&head.bar_energy).map(|b| head.bar_time(b));

    let keys_compatible = match (tail.key, head.key) {
        (Some(out_key), Some(in_key)) => Some(out_key.is_compatible(&in_key)),
        _ => None,
    };
    let options = if keys_compatible == Some(false) {
        &MIX_BEATS[1..]
    } else {
        &MIX_BEATS[..]
    };
    let tail_bar = |start: f64| (start - tail_start)..(start - tail_start + bar);

    for &beats in options {
        let length = beats as f64 * period;
        let mut start = match outro {
            Some(outro) if outro >= earliest && outro + length <= out_end => outro,
            _ => {
                let latest_start = out_end - length;
                if latest_start < earliest {
                    continue;
                }
                let start = snap(latest_start, first_downbeat, bar, f64::floor);
                if start < earliest {
                    continue;
                }
                start
            }
        };
        // Step back up to a few bars to start where the outgoing song doesn't sing.
        if tail.has_vocals(tail_bar(start)) {
            for shift in 1..=MAX_VOCAL_SHIFT {
                let candidate = start - shift as f64 * bar;
                if candidate < earliest {
                    break;
                }
                if !tail.has_vocals(tail_bar(candidate)) {
                    start = candidate;
                    break;
                }
            }
        }
        // The incoming song's first strong phrase lands on the end of the
        // mix, or on its halfway bar when its intro is too short. Never
        // inside the leading silence.
        let mut in_start = default_in;
        if let Some(rise) = rise
            && let Some(fit) = [rise - length * speed, rise - length * speed / 2.0]
                .into_iter()
                .find(|&s| s >= first_bar - 1e-6 && s <= MAX_PHRASE_SKIP)
        {
            in_start = fit;
        }
        in_start = in_start.max(0.0);

        // Both singing over each other: a shorter mix, or the last resort of
        // holding the incoming song down until the outgoing line ends.
        let mut duck_until = None;
        let out_vocals = tail.last_vocal_end((start - tail_start)..(start - tail_start + length));
        if let Some(out_vocals) = out_vocals
            && head.has_vocals(in_start..(in_start + length * speed))
        {
            if Some(&beats) != options.last() {
                continue;
            }
            // Released over 2 beats, so it is back up before the mix ends.
            duck_until =
                Some((1.0 - 2.0 / beats as f64).min((out_vocals + tail_start - start) / length));
        }
        return Some(Mix {
            fade_start: start,
            fade_length: length,
            in_start,
            speed,
            beats: Some(beats),
            beat_period: Some(period),
            duck_until,
            keys_compatible,
        });
    }
    None
}

/// How an AutoMix lets the outgoing song go, on top of the bass swap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Style {
    /// Volume fades only: no effect filters.
    Blend,
    /// The outgoing song thins out through a rising high-pass while the
    /// incoming song's low-pass opens.
    Sweep,
    /// The outgoing song's last bar feeds an echo on the beat, which keeps
    /// repeating after the song is cut.
    Echo,
    /// The outgoing song dissolves into a reverb-like tail.
    Wash,
    /// The outgoing song repeats a bar near its end instead of playing on.
    Loop,
}

impl Style {
    /// Swift `rawValue`.
    pub fn as_str(self) -> &'static str {
        match self {
            Style::Blend => "blend",
            Style::Sweep => "sweep",
            Style::Echo => "echo",
            Style::Wash => "wash",
            Style::Loop => "loop",
        }
    }
}

/// Echo repeats, and how far off the outgoing beat the echo built for the
/// song may be.
pub const ECHO_REPEATS: usize = 4;
pub const MAX_ECHO_DRIFT: f64 = 0.02;
/// Tempo change above which stretched beats smear and a wash hides them.
pub const WASH_TEMPO_CHANGE: f64 = 0.04;
/// Seconds the reverb rings after the outgoing song is cut.
pub const WASH_TAIL: f64 = 1.0;

/// What the music does around the mix, read from the bar energies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cues {
    /// The outgoing song gets quieter through the mix.
    pub out_falling: bool,
    /// The outgoing song stays near its loudest through the mix.
    pub out_high: bool,
    /// The incoming song gets louder from where it comes in to the mix end.
    pub in_rising: bool,
    /// The outgoing song's outro lasts 8 bars or more.
    pub long_outro: bool,
}

/// Mean of `count` bars from `from`, clipped to the bars there are.
fn mean(energy: &[f32], from: i64, count: i64) -> Option<f32> {
    let lower = from.max(0);
    let upper = (energy.len() as i64).min(lower + count);
    (lower < upper).then(|| {
        energy[lower as usize..upper as usize].iter().sum::<f32>() / (upper - lower) as f32
    })
}

/// The cues for `mix` between `tail` and `head`.
pub fn cues(mix: &Mix, tail: &TrackAnalysis, tail_start: f64, head: &TrackAnalysis) -> Cues {
    let (Some(bar), Some(in_bar)) = (tail.bar_length(), head.bar_length()) else {
        return Cues::default();
    };
    if bar <= 0.0 || in_bar <= 0.0 {
        return Cues::default();
    }
    let mut cues = Cues::default();
    let bars = ((mix.fade_length / bar).round() as i64).max(1);
    let start_bar = ((mix.fade_start - tail_start - tail.downbeat) / bar).round() as i64;
    if let (Some(before), Some(during)) = (
        mean(&tail.bar_energy, start_bar - 4, 4),
        mean(&tail.bar_energy, start_bar, bars),
    ) {
        cues.out_falling = during < before * 0.7;
        cues.out_high = during >= 0.75;
    }
    let in_bars = ((mix.fade_length * mix.speed / in_bar).round() as i64).max(1);
    let in_bar0 = ((mix.in_start - head.downbeat) / in_bar).round() as i64;
    if let (Some(entry), Some(after)) = (
        mean(&head.bar_energy, in_bar0, 4),
        mean(&head.bar_energy, in_bar0 + in_bars, 4),
    ) {
        cues.in_rising = after > entry * 1.3;
    }
    if let Some(outro) = track_analyzer::outro_bar(&tail.bar_energy) {
        cues.long_outro = tail.sound_end - tail.bar_time(outro) >= 8.0 * bar;
    }
    cues
}

/// The style of a mix; `Blend` when it isn't beat-matched. `echo_period` is
/// the beat the outgoing song's echo was built for, `room` the seconds the
/// outgoing song plays on after the mix (a tail needs them). The music
/// picks: a falling outgoing song gets an echo or a wash, a loud one meeting
/// a rising one a sweep, a long outro over a steady beat a loop, clashing
/// keys the tails that hide them. `variety` rotates between the styles left.
pub fn style(mix: &Mix, echo_period: Option<f64>, room: f64, cues: Cues, variety: i64) -> Style {
    let Some(period) = mix.beat_period else {
        return Style::Blend;
    };
    let can_wash = room >= WASH_TAIL;
    if (mix.speed - 1.0).abs() > WASH_TEMPO_CHANGE && can_wash {
        return Style::Wash;
    }
    let mut fits = vec![Style::Sweep];
    if loop_bar(mix).is_some() {
        fits.push(Style::Loop);
    }
    if let Some(echo_period) = echo_period
        && (echo_period / period - 1.0).abs() <= MAX_ECHO_DRIFT
        && room >= echo_tail(period)
    {
        fits.push(Style::Echo);
    }
    if can_wash {
        fits.push(Style::Wash);
    }

    let preferred: &[Style] = if mix.keys_compatible == Some(false) {
        &[Style::Wash, Style::Echo]
    } else if cues.out_falling {
        &[Style::Echo, Style::Wash]
    } else if cues.out_high && cues.in_rising {
        &[Style::Sweep]
    } else if cues.long_outro {
        &[Style::Loop]
    } else {
        &[]
    };
    let styles: Vec<Style> = preferred
        .iter()
        .copied()
        .filter(|s| fits.contains(s))
        .collect();
    let choice = if styles.is_empty() { fits } else { styles };
    choice[variety.unsigned_abs() as usize % choice.len()]
}

/// Seconds the echo rings after the outgoing song is cut.
pub fn echo_tail(period: f64) -> f64 {
    ECHO_REPEATS as f64 * period
}

/// The bar the loop style repeats (outgoing seconds) and how many extra
/// times. The repeats replace the mix's last bars, so the mix keeps its
/// length. `None` for mixes shorter than 4 bars.
pub fn loop_bar(mix: &Mix) -> Option<(Range<f64>, usize)> {
    let (beats, period) = (mix.beats?, mix.beat_period?);
    if beats < 4 * BEATS_PER_BAR {
        return None;
    }
    let repeats = if beats >= 8 * BEATS_PER_BAR { 2 } else { 1 };
    let bar = period * BEATS_PER_BAR as f64;
    let start = mix.fade_start + mix.fade_length - bar * (repeats + 1) as f64;
    Some((start..start + bar, repeats))
}

/// `from` at 0 to `to` at 1, evenly in pitch, the way a filter knob sweeps.
pub fn sweep(from: f64, to: f64, progress: f64) -> f64 {
    from * (to / from).powf(progress.clamp(0.0, 1.0))
}

/// The grid point (`grid_start + k * step`) nearest `time` in the direction
/// of `rounding` (`f64::floor` = Swift `.down`, `f64::ceil` = `.up`).
pub fn snap(time: f64, grid_start: f64, step: f64, rounding: fn(f64) -> f64) -> f64 {
    grid_start + rounding((time - grid_start) / step) * step
}
