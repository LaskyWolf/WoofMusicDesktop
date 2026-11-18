//! Crossfade, MixPlanner and TrackAnalyzer. The analyzer and planner cases
//! port `scripts/check_track_analyzer.py` from the WoofMusic repo.

use std::f64::consts::FRAC_1_SQRT_2;

use trop::audio::crossfade::{
    self, Action, BeatMix, Crossfade, Deck, Finish, NowPlayingState, Phase, Plan,
    TransitionSettings,
};
use trop::audio::mix_planner::{self, Cues, Mix, Style};
use trop::audio::track_analyzer::{self, Key, SAMPLE_RATE, TrackAnalysis, VOCAL_THRESHOLD};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn analysis(
    sound_start: f64,
    sound_end: f64,
    bpm: f64,
    first_beat: f64,
    downbeat: f64,
) -> TrackAnalysis {
    TrackAnalysis {
        sound_start,
        sound_end,
        bpm: Some(bpm),
        first_beat,
        downbeat,
        ..Default::default()
    }
}

const PERIOD: f64 = 60.0 / 128.0;

/// A 200 s song at 128 BPM whose last 60 s are analysed, going quiet at
/// 196 s; the next song at 124 BPM starts after 1.5 s of silence.
fn tail() -> TrackAnalysis {
    analysis(0.0, 56.0, 128.0, 0.2, 0.2 + PERIOD)
}

fn head() -> TrackAnalysis {
    analysis(1.5, 45.0, 124.0, 0.1, 0.1)
}

fn auto(tail: &TrackAnalysis, head: &TrackAnalysis) -> Option<Mix> {
    mix_planner::auto_mix(200.0, Some(tail), 140.0, Some(head))
}

fn key(pitch_class: usize, is_minor: bool) -> Key {
    Key {
        pitch_class,
        is_minor,
    }
}

// MARK: - TrackAnalyzer

/// `lead` s of silence, clicks at `bpm` for `length` s, then `tail` s of
/// silence. Every 4th click, starting with the first, is twice as loud.
fn click_track(bpm: f64, lead: f64, length: f64, tail: f64) -> Vec<f32> {
    let sr = SAMPLE_RATE as f64;
    let mut samples = vec![0f32; ((lead + length + tail) * sr) as usize];
    let (mut beat, mut seed, mut count) = (lead, 1u32, 0);
    while beat < lead + length - 0.1 {
        let accent = if count % 4 == 0 { 1.0 } else { 0.5 };
        count += 1;
        let start = (beat * sr) as usize;
        for i in 0..(0.05 * sr) as usize {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let noise = ((seed >> 1) as i32 % 1000) as f32 / 1000.0;
            samples[start + i] = accent * noise * (-(i as f32) / 150.0).exp();
        }
        beat += 60.0 / bpm;
    }
    // A quiet pad under the clicks, like the rest of a real mix.
    for (i, sample) in samples
        .iter_mut()
        .enumerate()
        .take(((lead + length) * sr) as usize)
        .skip((lead * sr) as usize)
    {
        *sample += 0.05 * (i as f32 * 0.3).sin();
    }
    samples
}

#[test]
fn click_tracks_give_tempo_beats_downbeats_and_silence_edges() {
    for bpm in [120.0, 128.0, 96.0] {
        let result = track_analyzer::analyze(&click_track(bpm, 2.0, 20.0, 3.0), SAMPLE_RATE)
            .unwrap_or_else(|| panic!("{bpm} BPM: no result"));
        assert!(
            (result.sound_start - 2.0).abs() < 0.1,
            "{bpm}: soundStart {}",
            result.sound_start
        );
        assert!(
            (result.sound_end - 22.0).abs() < 0.2,
            "{bpm}: soundEnd {}",
            result.sound_end
        );
        let found = result.bpm.expect("tempo");
        assert!((found - bpm).abs() < 2.0, "{bpm}: found {found}");
        let period = 60.0 / bpm;
        let offset = (result.first_beat - 2.0) % period;
        let phase_error = offset.abs().min((period - offset.abs()).abs());
        assert!(phase_error < 0.05, "{bpm}: beat phase off by {phase_error}");
        let bar = 4.0 * period;
        let bar_offset = (result.downbeat - 2.0) % bar;
        let downbeat_error = bar_offset.abs().min((bar - bar_offset.abs()).abs());
        assert!(
            downbeat_error < 0.05,
            "{bpm}: downbeat off by {downbeat_error}"
        );
    }
}

#[test]
fn tone_has_no_beat_and_silence_no_result() {
    let tone: Vec<f32> = (0..10 * SAMPLE_RATE)
        .map(|i| (i as f64 * 0.2).sin() as f32 * 0.5)
        .collect();
    assert_eq!(
        track_analyzer::analyze(&tone, SAMPLE_RATE).unwrap().bpm,
        None
    );
    assert_eq!(
        track_analyzer::analyze(&vec![0.0; SAMPLE_RATE], SAMPLE_RATE),
        None
    );
    assert_eq!(track_analyzer::analyze(&[0.5; 100], SAMPLE_RATE), None);
}

#[test]
fn keys_from_chroma_camelot_and_a_real_chord() {
    let c_major_chroma = [1.0, 0.0, 0.5, 0.0, 1.0, 0.5, 0.0, 1.0, 0.0, 0.5, 0.0, 0.5];
    assert_eq!(track_analyzer::key(&c_major_chroma), Some(key(0, false)));
    let a_minor_chroma = [0.8, 0.0, 0.4, 0.0, 0.8, 0.4, 0.0, 0.4, 0.0, 1.0, 0.0, 0.3];
    assert_eq!(track_analyzer::key(&a_minor_chroma), Some(key(9, true)));
    assert_eq!(track_analyzer::key(&[0.0; 12]), None);

    let (c_major, a_minor, g_major, e_minor, f_sharp_major) = (
        key(0, false),
        key(9, true),
        key(7, false),
        key(4, true),
        key(6, false),
    );
    assert_eq!(
        [c_major, a_minor, g_major, e_minor].map(|k| k.camelot()),
        ["8B", "8A", "9B", "9A"]
    );
    assert_eq!(key(11, false).camelot(), "1B");
    assert_eq!(a_minor.name(), "A minor");
    assert!(
        c_major.is_compatible(&a_minor)
            && c_major.is_compatible(&g_major)
            && a_minor.is_compatible(&e_minor)
    );
    assert!(!c_major.is_compatible(&f_sharp_major) && !c_major.is_compatible(&e_minor));

    let sr = SAMPLE_RATE as f64;
    let chord: Vec<f32> = (0..2 * SAMPLE_RATE)
        .map(|i| {
            let t = i as f64 / sr;
            let tau = std::f64::consts::TAU;
            (((tau * 261.63 * t).sin() + (tau * 329.63 * t).sin() + (tau * 392.0 * t).sin()) / 3.0)
                as f32
        })
        .collect();
    let chroma = track_analyzer::chroma(&chord, SAMPLE_RATE);
    let mut loudest: Vec<usize> = (0..12).collect();
    loudest.sort_by(|a, b| chroma[*b].total_cmp(&chroma[*a]));
    loudest.truncate(3);
    loudest.sort();
    assert_eq!(loudest, [0, 4, 7], "C-E-G chroma {chroma:?}");
    assert_eq!(track_analyzer::key(&chroma), Some(c_major));
}

fn curve(parts: &[(f32, usize)]) -> Vec<f32> {
    parts
        .iter()
        .flat_map(|&(v, n)| std::iter::repeat_n(v, n))
        .collect()
}

#[test]
fn phrases_outro_and_first_rise() {
    let drop = curve(&[(1.0, 24), (0.4, 8)]);
    assert_eq!(track_analyzer::phrase_boundaries(&drop), [24]);
    assert_eq!(track_analyzer::outro_bar(&drop), Some(24));
    assert_eq!(track_analyzer::first_rise_bar(&drop), None);
    let rise = curve(&[(0.3, 8), (1.0, 16)]);
    assert_eq!(track_analyzer::first_rise_bar(&rise), Some(8));
    assert_eq!(track_analyzer::outro_bar(&rise), None);
    assert!(track_analyzer::phrase_boundaries(&[0.8; 32]).is_empty());
    let breakdown = curve(&[(1.0, 16), (0.5, 8), (1.0, 8)]);
    assert_eq!(track_analyzer::phrase_boundaries(&breakdown), [16, 24]);
    assert_eq!(
        track_analyzer::outro_bar(&breakdown),
        None,
        "a break isn't an outro"
    );
}

#[test]
fn vocal_presence_moves_with_a_sung_line() {
    let pad = [0.5f32; 80];
    assert_eq!(track_analyzer::vocal_presence(&pad, &pad), 0.0);
    let line: Vec<f32> = (0..80)
        .map(|i| if i % 8 < 4 { 0.5 } else { 0.05 })
        .collect();
    assert!(track_analyzer::vocal_presence(&line, &line) >= VOCAL_THRESHOLD);
    let quiet: Vec<f32> = line.iter().map(|x| x * 0.1).collect();
    assert!(track_analyzer::vocal_presence(&quiet, &line) < VOCAL_THRESHOLD);
    assert_eq!(track_analyzer::vocal_presence(&[], &[]), 0.0);
}

// MARK: - MixPlanner

#[test]
fn auto_mix_on_compatible_songs() {
    let mix = auto(&tail(), &head()).expect("AutoMix for compatible songs");
    assert_eq!(mix.beats, Some(32));
    assert!(
        mix.fade_start + mix.fade_length <= 196.0 + 1e-9,
        "ends after the song goes quiet"
    );
    let bars = (mix.fade_start - (140.0 + tail().downbeat)) / (4.0 * PERIOD);
    assert!(close(bars, bars.round()), "starts off a downbeat");
    assert!(
        196.0 - (mix.fade_start + mix.fade_length) < 4.0 * PERIOD,
        "starts more than a bar early"
    );
    assert!(
        close(mix.in_start, 0.1 + 4.0 * 60.0 / 124.0),
        "incoming starts at {}",
        mix.in_start
    );
    assert!(close(mix.speed, 128.0 / 124.0));
}

#[test]
fn auto_mix_refuses_tempo_gaps_and_beatless_songs() {
    let far = TrackAnalysis {
        bpm: Some(100.0),
        ..head()
    };
    assert_eq!(auto(&tail(), &far), None);
    let beatless = TrackAnalysis {
        bpm: None,
        ..head()
    };
    assert_eq!(auto(&tail(), &beatless), None);
    assert_eq!(
        mix_planner::auto_mix(200.0, None, 140.0, Some(&head())),
        None
    );
    assert_eq!(
        crossfade::auto_mix_blocker(Some(&tail()), Some(&far)),
        "tempo gap of 28%"
    );
    assert_eq!(
        crossfade::auto_mix_blocker(None, Some(&far)),
        "the outgoing song could not be analysed"
    );
}

#[test]
fn smart_fade_placement() {
    let far = TrackAnalysis {
        bpm: Some(100.0),
        ..head()
    };
    let fade = mix_planner::smart(200.0, 8.0, Some(&tail()), 140.0, Some(&far));
    assert!(
        fade.fade_start + 8.0 <= 196.0 && fade.fade_start > 196.0 - 8.0 - PERIOD,
        "at {}",
        fade.fade_start
    );
    assert_eq!(fade.speed, 1.0, "no nudge across a 28% tempo gap");
    // Without analysis the fade ends with the song.
    let plain = mix_planner::smart(200.0, 8.0, None, 140.0, None);
    assert_eq!(
        plain,
        Mix {
            fade_start: 192.0,
            fade_length: 8.0,
            in_start: 0.0,
            speed: 1.0,
            ..Default::default()
        }
    );
    // A short song: the fade ends with it.
    assert_eq!(
        mix_planner::smart(40.0, 6.0, None, 0.0, None).fade_start,
        34.0
    );
    // A song shorter than the fade: it never starts before the song's half.
    assert_eq!(
        mix_planner::smart(8.0, 12.0, None, 0.0, None).fade_start,
        4.0
    );
    // A close tempo is nudged; the incoming song starts on its first beat after the silence.
    let near = mix_planner::smart(200.0, 8.0, Some(&tail()), 140.0, Some(&head()));
    assert!(close(near.speed, 128.0 / 124.0));
    assert!(
        close(near.in_start, 0.1 + 3.0 * 60.0 / 124.0),
        "first beat, not bar"
    );
}

#[test]
fn styles_rotate_and_follow_the_music() {
    let mix = auto(&tail(), &head()).unwrap();
    let plain = mix_planner::smart(200.0, 8.0, None, 140.0, None);
    let flat = Cues::default();
    let styles = |mix: &Mix, echo: Option<f64>, room: f64, cues: Cues| {
        let mut all: Vec<Style> = (0..12)
            .map(|v| mix_planner::style(mix, echo, room, cues, v))
            .collect();
        all.sort_by_key(|s| s.as_str());
        all.dedup();
        all
    };
    assert_eq!(
        mix_planner::style(&plain, None, 10.0, flat, 0),
        Style::Blend
    );
    assert_eq!(
        styles(&mix, Some(PERIOD), 10.0, flat),
        [Style::Echo, Style::Loop, Style::Sweep, Style::Wash]
    );
    assert!(
        !styles(&mix, Some(PERIOD * 1.05), 10.0, flat).contains(&Style::Echo),
        "echo built for another tempo"
    );
    assert_eq!(
        styles(&mix, Some(PERIOD), 0.5, flat),
        [Style::Loop, Style::Sweep],
        "tails without room"
    );
    let stretched = Mix {
        speed: 1.06,
        ..mix.clone()
    };
    assert_eq!(
        mix_planner::style(&stretched, None, 10.0, flat, 0),
        Style::Wash
    );

    let falling = Cues {
        out_falling: true,
        ..flat
    };
    assert_eq!(
        mix_planner::style(&mix, Some(PERIOD), 10.0, falling, 0),
        Style::Echo
    );
    assert_eq!(
        mix_planner::style(&mix, Some(PERIOD), 10.0, falling, 1),
        Style::Wash
    );
    assert_eq!(
        mix_planner::style(&mix, None, 0.5, falling, 0),
        Style::Sweep
    );
    let peak = Cues {
        out_high: true,
        in_rising: true,
        ..flat
    };
    assert_eq!(
        mix_planner::style(&mix, Some(PERIOD), 10.0, peak, 3),
        Style::Sweep
    );
    let outro = Cues {
        long_outro: true,
        ..flat
    };
    assert_eq!(
        mix_planner::style(&mix, Some(PERIOD), 10.0, outro, 3),
        Style::Loop
    );
    let clash = Mix {
        keys_compatible: Some(false),
        ..mix.clone()
    };
    assert_eq!(
        mix_planner::style(&clash, Some(PERIOD), 10.0, peak, 0),
        Style::Wash
    );
}

#[test]
fn loop_bar_and_sweep() {
    let mix = auto(&tail(), &head()).unwrap();
    let short = Mix {
        beats: Some(8),
        fade_length: 8.0 * PERIOD,
        ..mix.clone()
    };
    assert_eq!(mix_planner::loop_bar(&short), None, "loop in a 2-bar mix");
    let (bar, repeats) = mix_planner::loop_bar(&mix).expect("loop bar in a 32-beat mix");
    assert_eq!(repeats, 2);
    assert!(
        close(
            bar.end + 2.0 * 4.0 * PERIOD,
            mix.fade_start + mix.fade_length
        ),
        "repeats end with the mix"
    );
    assert!(close(bar.end - bar.start, 4.0 * PERIOD), "one bar");

    assert_eq!(mix_planner::sweep(300.0, 16_000.0, 0.0), 300.0);
    assert!((mix_planner::sweep(300.0, 16_000.0, 1.0) - 16_000.0).abs() < 1e-6);
    assert!(
        (mix_planner::sweep(100.0, 10_000.0, 0.5) - 1_000.0).abs() < 1e-6,
        "even in pitch"
    );
    assert_eq!(
        mix_planner::sweep(100.0, 10_000.0, 2.0),
        mix_planner::sweep(100.0, 10_000.0, 1.0)
    );
}

fn phrased() -> TrackAnalysis {
    TrackAnalysis {
        bar_energy: curve(&[(1.0, 16), (0.5, 12)]),
        ..tail()
    }
}

fn rising() -> TrackAnalysis {
    TrackAnalysis {
        bar_energy: curve(&[(0.3, 8), (1.0, 12)]),
        ..head()
    }
}

#[test]
fn phrase_aware_auto_mix() {
    let outro_time = 140.0 + tail().bar_time(16);
    let mix = auto(&phrased(), &rising()).expect("phrase mix");
    assert_eq!(mix.beats, Some(32));
    assert!(
        close(mix.fade_start, outro_time),
        "starts at the outro: {mix:?}"
    );
    // Landing the rise on the mix end would start inside the 1.5 s of silence,
    // so it lands on the halfway bar.
    let rise_time = head().bar_time(8);
    assert!(close(
        mix.in_start,
        rise_time - mix.fade_length * mix.speed / 2.0
    ));
    let no_silence = TrackAnalysis {
        sound_start: 0.0,
        ..rising()
    };
    let full = auto(&phrased(), &no_silence).expect("full mix");
    assert!(
        close(full.in_start, rise_time - full.fade_length * full.speed),
        "rise lands on the mix end"
    );
    assert_eq!((mix.duck_until, mix.keys_compatible), (None, None));
    assert_eq!(
        mix_planner::cues(&mix, &phrased(), 140.0, &rising()),
        Cues {
            out_falling: true,
            out_high: false,
            in_rising: true,
            long_outro: true
        }
    );
}

#[test]
fn vocals_shift_shorten_and_duck() {
    let outro_time = 140.0 + tail().bar_time(16);
    let bar = 4.0 * PERIOD;
    let mut sung = phrased();
    sung.bar_vocal = vec![0.0; 28];
    sung.bar_vocal[16] = 1.0;
    let shifted = auto(&sung, &rising()).expect("shifted mix");
    assert!(
        close(shifted.fade_start, outro_time - bar) && shifted.duck_until.is_none(),
        "{shifted:?}"
    );

    sung.bar_vocal = vec![1.0; 28];
    let singing = TrackAnalysis {
        bar_vocal: vec![1.0; 20],
        ..rising()
    };
    let ducked = auto(&sung, &singing).expect("ducked mix");
    assert_eq!(ducked.beats, Some(8));
    assert!(close(ducked.duck_until.unwrap(), 0.75), "{ducked:?}");
}

#[test]
fn key_clash_drops_the_longest_mix() {
    let keyed = TrackAnalysis {
        key: Some(key(0, false)),
        ..phrased()
    };
    let mut far_key = TrackAnalysis {
        key: Some(key(6, false)),
        ..rising()
    };
    let clash = auto(&keyed, &far_key).expect("key clash mix");
    assert_eq!(
        (clash.beats, clash.keys_compatible),
        (Some(16), Some(false))
    );
    far_key.key = Some(key(7, false));
    assert_eq!(auto(&keyed, &far_key).unwrap().keys_compatible, Some(true));
}

// MARK: - Fade math

#[test]
fn equal_power_fade_curve() {
    assert_eq!(crossfade::fade_gains(0.0), (1.0, 0.0));
    let (out, inc) = crossfade::fade_gains(0.5);
    assert!(close(out, FRAC_1_SQRT_2) && close(inc, FRAC_1_SQRT_2));
    let (out, inc) = crossfade::fade_gains(1.0);
    assert!(close(out, 0.0) && close(inc, 1.0));
    assert_eq!(crossfade::fade_gains(-1.0), crossfade::fade_gains(0.0));
    assert_eq!(crossfade::fade_gains(2.0), crossfade::fade_gains(1.0));
    assert_eq!(crossfade::fade_progress(3.0, 6.0), 0.5);
    assert_eq!(crossfade::fade_progress(10.0, 6.0), 1.0);
}

#[test]
fn mpv_volume_is_cube_root() {
    assert_eq!(crossfade::mpv_volume(1.0), "100.0000");
    assert_eq!(crossfade::mpv_volume(0.125), "50.0000");
    assert_eq!(crossfade::mpv_volume(-1.0), "0.0000");
}

#[test]
fn beat_mix_frame_gains_and_effects() {
    let at = |t| crossfade::frame(Style::Blend, t, 32, 1.0, 1.0);
    assert_eq!((at(0.0).outgoing, at(0.0).incoming), (1.0, 0.0));
    assert!(close(at(0.25).incoming, FRAC_1_SQRT_2) && at(0.25).outgoing == 1.0);
    assert!(close(at(0.5).outgoing, 1.0) && close(at(0.5).incoming, 1.0));
    assert!(close(at(0.75).outgoing, FRAC_1_SQRT_2));
    assert!(close(at(1.0).outgoing, 0.0) && at(1.0).effects.is_empty());

    // Echo: the deck stays up, the dry path fades, the send peaks mid last bar.
    let echo = crossfade::frame(Style::Echo, 1.0 - 2.0 / 32.0, 32, 1.0, 0.5);
    assert_eq!(echo.outgoing, 1.0);
    assert_eq!(echo.effects[1].target, "volume@echosend");
    assert_eq!(echo.effects[1].value, "0.5000");
    assert_eq!(
        echo.effects[1].args(),
        ["automixfx", "volume", "0.5000", "volume@echosend"]
    );
    // Sweep at the start: the low-pass sits at 300 Hz, the high-pass mix at 0.
    let sweep = crossfade::frame(Style::Sweep, 0.0, 32, 1.0, 1.0);
    assert_eq!(sweep.effects[0].value, "300.0000");
    assert_eq!(
        (sweep.effects[1].deck, sweep.effects[1].value.as_str()),
        (Deck::Outgoing, "0.0000")
    );
    assert_eq!(
        crossfade::low_pass_start(0.0),
        16_000.0,
        "no strength: already open"
    );
    assert_eq!(crossfade::high_pass_end(0.0), 20.0);
}

#[test]
fn duck_lock_and_speed_ease() {
    let mix = Mix {
        fade_start: 100.0,
        in_start: 4.0,
        speed: 1.02,
        beats: Some(8),
        duck_until: Some(0.75),
        ..Default::default()
    };
    assert_eq!(crossfade::duck(&mix, 0.5), 0.5);
    assert_eq!(crossfade::duck(&mix, 0.75), 0.5);
    assert_eq!(crossfade::duck(&mix, 0.8125), 0.625);
    assert_eq!(crossfade::duck(&mix, 1.0), 1.0);
    assert_eq!(crossfade::duck(&Mix::default(), 0.1), 1.0);

    // On the beat: no correction; far ahead: the largest slow-down.
    assert!(close(
        crossfade::lock_speed(&mix, 110.0, 4.0 + 10.0 * 1.02),
        1.02
    ));
    assert!(close(crossfade::lock_speed(&mix, 110.0, 20.0), 1.02 * 0.98));

    assert_eq!(crossfade::eased_speed(1.04, 0.0), 1.04);
    assert!(close(crossfade::eased_speed(1.04, 4.0), 1.02));
    assert_eq!(crossfade::eased_speed(1.04, 20.0), 1.0);
}

// MARK: - Crossfade state machine

fn crossfade_on() -> TransitionSettings {
    TransitionSettings {
        crossfade_enabled: true,
        ..Default::default()
    }
}

fn playing(position: f64) -> NowPlayingState<'static> {
    NowPlayingState {
        video_id: Some("a"),
        next_video_id: Some("b"),
        is_playing: true,
        current_time: position,
        duration: 200.0,
        ..Default::default()
    }
}

/// A crossfade planned without analysis: 6 s ending with the 200 s song, at 194 s.
fn planned(cf: &mut Crossfade, settings: &TransitionSettings) -> Plan {
    let Action::Plan {
        token,
        from,
        to,
        duration,
    } = cf.tick(&playing(10.0), settings)
    else {
        panic!("no planning at 10 s");
    };
    assert_eq!((from.as_str(), to.as_str(), duration), ("a", "b", 200.0));
    cf.plan_ready(token, "url-b", Some(-9.0), None, None, 10.0, true, None)
        .cloned()
        .expect("plan")
}

#[test]
fn crossfade_off_or_unfit_songs_never_plan() {
    let mut cf = Crossfade::new();
    assert_eq!(
        cf.tick(&playing(50.0), &TransitionSettings::default()),
        Action::None
    );
    let short = NowPlayingState {
        duration: 30.0,
        ..playing(20.0)
    };
    let long = NowPlayingState {
        duration: 600.0,
        ..playing(20.0)
    };
    let last = NowPlayingState {
        next_video_id: None,
        ..playing(20.0)
    };
    let repeat = NowPlayingState {
        is_repeat_on: true,
        ..playing(20.0)
    };
    for np in [short, long, last, repeat] {
        let mut cf = Crossfade::new();
        assert_eq!(cf.tick(&np, &crossfade_on()), Action::None);
        assert_eq!(cf.phase(), Phase::Idle);
    }
}

#[test]
fn settings_are_read_when_the_song_starts() {
    let mut cf = Crossfade::new();
    assert_eq!(
        cf.tick(&playing(5.0), &TransitionSettings::default()),
        Action::None
    );
    // Turned on mid-song: takes effect from the next song.
    assert_eq!(cf.tick(&playing(20.0), &crossfade_on()), Action::None);
}

#[test]
fn full_crossfade_flow() {
    let settings = crossfade_on();
    let mut cf = Crossfade::new();
    assert_eq!(
        cf.tick(&playing(5.0), &settings),
        Action::None,
        "plans from 10 s"
    );
    let plan = planned(&mut cf, &settings);
    assert_eq!((plan.mix.fade_start, plan.mix.fade_length), (194.0, 6.0));
    assert_eq!(plan.style, Style::Blend);
    assert_eq!(plan.incoming_bass_gain(), None);
    assert_eq!(cf.phase(), Phase::Planned);

    assert_eq!(cf.tick(&playing(181.0), &settings), Action::None);
    let Action::LoadIncoming(load) = cf.tick(&playing(182.0), &settings) else {
        panic!("load 12 s before the fade");
    };
    assert_eq!(load.url, "url-b");
    assert_eq!(
        crossfade::incoming_properties(&load)[2],
        ("speed", "1.0000".to_string())
    );
    assert_eq!(
        cf.tick(&playing(193.7), &settings),
        Action::None,
        "still loading"
    );
    assert!(cf.deck_loaded());
    let Action::StartMix { wait, .. } = cf.tick(&playing(193.7), &settings) else {
        panic!("no mix start");
    };
    assert!(close(wait, 0.3));
    assert_eq!(cf.mixing_into(), Some("b"));

    cf.begin();
    assert!(cf.promote());
    assert!(!cf.promote(), "promotes once");
    assert_eq!(cf.complete(None), 1.0);
    assert_eq!(cf.phase(), Phase::Idle);
}

#[test]
fn seeking_into_the_fade_before_the_song_loads_gives_up() {
    let settings = crossfade_on();
    let mut cf = Crossfade::new();
    planned(&mut cf, &settings);
    // A seek past the start of the fade: the next song loads at once.
    assert!(matches!(
        cf.tick(&playing(195.5), &settings),
        Action::LoadIncoming(_)
    ));
    let Action::Reset { was, .. } = cf.tick(&playing(195.5), &settings) else {
        panic!("no give-up");
    };
    assert_eq!(was, Phase::Prepared);
    assert_eq!(
        cf.tick(&playing(196.0), &settings),
        Action::None,
        "no retry for this song"
    );
    // The next song plans again.
    let next = NowPlayingState {
        video_id: Some("b"),
        next_video_id: Some("c"),
        ..playing(12.0)
    };
    assert!(matches!(cf.tick(&next, &settings), Action::Plan { .. }));
}

#[test]
fn a_plan_that_comes_too_late_lets_the_song_end() {
    let settings = crossfade_on();
    let mut cf = Crossfade::new();
    let Action::Plan { token, .. } = cf.tick(&playing(10.0), &settings) else {
        panic!("no planning");
    };
    assert!(
        cf.plan_ready(token, "u", None, None, None, 192.0, true, None)
            .is_none()
    );
    assert_eq!(cf.phase(), Phase::Idle);
    assert_eq!(cf.tick(&playing(192.5), &settings), Action::None);
}

#[test]
fn stale_and_failed_planning_runs() {
    let settings = crossfade_on();
    let mut cf = Crossfade::new();
    let Action::Plan { token, .. } = cf.tick(&playing(10.0), &settings) else {
        panic!("no planning");
    };
    assert_eq!(cf.reset("a new song started"), Phase::Planning);
    assert!(
        cf.plan_ready(token, "u", None, None, None, 10.0, true, None)
            .is_none(),
        "stale run"
    );

    let Action::Plan { token, .. } = cf.tick(&playing(11.0), &settings) else {
        panic!("no planning");
    };
    cf.plan_failed(token);
    assert_eq!(cf.phase(), Phase::Idle);
    assert_eq!(cf.tick(&playing(12.0), &settings), Action::None, "no retry");
}

#[test]
fn queue_changes_and_repeat_cancel_a_plan() {
    let settings = crossfade_on();
    let mut cf = Crossfade::new();
    planned(&mut cf, &settings);
    let reordered = NowPlayingState {
        next_video_id: Some("c"),
        ..playing(100.0)
    };
    assert_eq!(
        cf.tick(&reordered, &settings),
        Action::Reset {
            was: Phase::Planned,
            reason: "queue changed".into()
        }
    );

    let mut cf = Crossfade::new();
    planned(&mut cf, &settings);
    let repeat = NowPlayingState {
        is_repeat_on: true,
        ..playing(100.0)
    };
    assert!(matches!(
        cf.tick(&repeat, &settings),
        Action::Reset {
            was: Phase::Planned,
            ..
        }
    ));
}

#[test]
fn finish_now_and_outgoing_end_during_a_mix() {
    let settings = crossfade_on();
    let mut cf = Crossfade::new();
    planned(&mut cf, &settings);
    cf.tick(&playing(182.0), &settings);
    cf.deck_loaded();
    cf.tick(&playing(194.0), &settings);
    assert_eq!(
        cf.finish_now(),
        Finish::Reset { was: Phase::Fading },
        "not begun"
    );

    let mut cf = Crossfade::new();
    planned(&mut cf, &settings);
    cf.tick(&playing(182.0), &settings);
    cf.deck_loaded();
    cf.tick(&playing(194.0), &settings);
    cf.begin();
    assert!(cf.outgoing_ended());
    assert!(cf.outgoing_finished() && cf.has_promoted());
    assert_eq!(cf.deck_failed(), None, "promoted: the mix goes on");
    assert_eq!(
        cf.finish_now(),
        Finish::Complete {
            promote: false,
            final_speed: 1.0
        }
    );
    assert_eq!(cf.finish_now(), Finish::Nothing);
}

#[test]
fn auto_mix_plans_and_falls_back() {
    let settings = TransitionSettings {
        auto_mix_enabled: true,
        ..Default::default()
    };
    let mut cf = Crossfade::new();
    let Action::Plan { token, .. } = cf.tick(&playing(10.0), &settings) else {
        panic!("no planning");
    };
    let plan = cf
        .plan_ready(
            token,
            "u",
            None,
            Some(&tail()),
            Some(&head()),
            10.0,
            true,
            None,
        )
        .cloned()
        .unwrap();
    assert_eq!(plan.mix.beats, Some(32));
    assert_eq!(plan.fallback, None);
    // First mix, no echo built: sweep, loop, wash; variety 1 picks the loop.
    assert_eq!(plan.style, Style::Loop);
    assert_eq!(plan.incoming_bass_gain(), Some(crossfade::BASS_CUT));
    assert!(close(plan.echo_delay.unwrap(), 60.0 / 124.0));

    let mut cf = Crossfade::new();
    let Action::Plan { token, .. } = cf.tick(&playing(10.0), &settings) else {
        panic!("no planning");
    };
    let far = TrackAnalysis {
        bpm: Some(100.0),
        ..head()
    };
    let plan = cf
        .plan_ready(
            token,
            "u",
            None,
            Some(&tail()),
            Some(&far),
            10.0,
            true,
            None,
        )
        .cloned()
        .unwrap();
    assert_eq!(plan.fallback.as_deref(), Some("tempo gap of 28%"));
    assert!(!plan.mix.is_beat_matched());
    assert_eq!(plan.mix.fade_length, 6.0);
}

fn beat_plan(style: Style) -> Plan {
    let mix = auto(&tail(), &head()).unwrap();
    Plan {
        from_id: "a".into(),
        to_id: "b".into(),
        url: "u".into(),
        loudness: None,
        mix,
        style,
        echo_delay: None,
        duration: 200.0,
        fallback: None,
    }
}

#[test]
fn beat_mix_steps_swap_bass_and_lock() {
    let plan = beat_plan(Style::Blend);
    let mix = plan.mix.clone();
    let mut run = BeatMix::new(&plan).unwrap();
    assert!(run.start_effects(1.0).is_empty());
    let on_beat = |out: f64| (out, mix.in_start + (out - mix.fade_start) * mix.speed);

    let first = run.step(0.0, Some(on_beat(mix.fade_start)), true, 1.0);
    assert_eq!(
        (first.t, first.incoming_volume, first.speed, first.done),
        (0.0, 0.0, None, false)
    );
    let half = mix.fade_start + mix.fade_length / 2.0;
    let middle = run.step(7.0, Some(on_beat(half + 0.01)), true, 1.0);
    assert!(middle.t >= 0.5 && middle.t < 0.501);
    assert_eq!(middle.effects.len(), 2, "bass swap");
    assert_eq!(
        middle.effects[1].args(),
        ["automixbass", "g", "-24.0000", "bass"]
    );
    let (out, inc) = on_beat(half + 1.0);
    let late = run.step(8.0, Some((out, inc - 0.1)), true, 1.0);
    assert!(late.effects.is_empty(), "swapped once");
    assert!(
        late.speed.unwrap() > mix.speed,
        "catches up a late incoming song"
    );
    let end = run.step(
        15.0,
        Some(on_beat(mix.fade_start + mix.fade_length)),
        true,
        1.0,
    );
    assert!(end.done && close(end.outgoing_volume, 0.0));
    assert_eq!(run.tail(Some(196.0), 200.0), None, "blend has no tail");
    assert!(
        close(run.final_speed(), mix.speed),
        "locked back on the beat"
    );
}

#[test]
fn beat_mix_counts_by_the_clock_once_the_outgoing_song_ends() {
    let plan = beat_plan(Style::Blend);
    let mut run = BeatMix::new(&plan).unwrap();
    let period = plan.mix.beat_period.unwrap();
    let step = run.step(16.0 * period, None, true, 1.0);
    assert!(close(step.t, 0.5));
    assert!(
        run.step(plan.mix.fade_length / plan.mix.speed + 5.1, None, true, 1.0)
            .done,
        "safety net"
    );
}

#[test]
fn sweep_and_echo_effects_are_sent_once_and_tails_fit_the_song() {
    let mut sweep = BeatMix::new(&beat_plan(Style::Sweep)).unwrap();
    assert_eq!(sweep.start_effects(1.0).len(), 3);
    let start = beat_plan(Style::Sweep).mix.fade_start;
    let first = sweep.step(0.0, Some((start, 0.0)), true, 1.0);
    assert_eq!(
        first.effects.len(),
        1,
        "low-pass already at 300 Hz, high-pass mix 0 is new"
    );
    assert!(
        sweep
            .step(0.01, Some((start, 0.0)), true, 1.0)
            .effects
            .is_empty()
    );
    // mpv dropped the filters: the mix goes on by volume alone.
    let degraded = sweep.step(0.02, Some((start + 1.0, 0.0)), false, 1.0);
    assert!(degraded.effects.is_empty() && sweep.style() == Style::Blend);

    let echo = BeatMix::new(&beat_plan(Style::Echo)).unwrap();
    assert!(close(echo.tail(Some(190.0), 200.0).unwrap(), 4.0 * PERIOD));
    assert!(
        close(echo.tail(Some(198.0), 200.0).unwrap(), 1.5),
        "cut at the song's end"
    );
    assert_eq!(echo.tail(None, 200.0), Some(0.0));
    assert_eq!(crossfade::tail_cut_effects()[0].target, "volume@dry");
    assert_eq!(
        crossfade::clear_effects(Deck::Incoming, true, false).len(),
        1
    );
    assert_eq!(
        crossfade::clear_effects(Deck::Incoming, true, true).len(),
        6
    );

    let looped = BeatMix::new(&beat_plan(Style::Loop)).unwrap();
    assert_eq!(looped.loop_bar.as_ref().map(|(_, r)| *r), Some(2));
    assert!(
        BeatMix::new(&Plan {
            mix: Mix::default(),
            ..beat_plan(Style::Blend)
        })
        .is_none()
    );
}

#[test]
fn loop_jumps_keep_the_mix_counting_forward() {
    let plan = beat_plan(Style::Loop);
    let mut run = BeatMix::new(&plan).unwrap();
    let (bar, _) = run.loop_bar.clone().unwrap();
    let before = run.step(0.0, Some((bar.end - 0.01, 0.0)), true, 1.0);
    let after = run.step(0.05, Some((bar.start + 0.04, 0.0)), true, 1.0);
    assert!(after.t > before.t, "{} then {}", before.t, after.t);
}

#[test]
fn ffmpeg_decode_args_and_samples() {
    let args = crossfade::ffmpeg_args("https://x/y", 60.0, true, "pipe:1");
    let joined = args.join(" ");
    assert!(joined.contains("-sseof -60 -i https://x/y -t 60"));
    assert!(joined.ends_with("-ar 11025 -f f32le pipe:1"));
    assert!(args.contains(&"-headers".to_string()));
    assert!(
        !crossfade::ffmpeg_args("/music/a.m4a", 45.0, false, "o").contains(&"-sseof".to_string())
    );

    let bytes: Vec<u8> = [0.5f32, -1.0]
        .iter()
        .flat_map(|x| x.to_le_bytes())
        .chain([7])
        .collect();
    assert_eq!(crossfade::samples_from_f32le(&bytes), [0.5, -1.0]);
}
