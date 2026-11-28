//! Retro synthesizer: deterministic, in range, and the knobs do what they say.

use knorke_audio::chip::{self, Chip, Instrument, Note, Wave};

const RATE: u32 = 48_000;

fn one(inst: Instrument, chip: &Chip, key: u8) -> Vec<f32> {
    let mut out = vec![0.0; RATE as usize / 4];
    let note = Note {
        at: 0,
        len: RATE / 5,
        key,
        instrument: 0,
        gain: 1.0,
    };
    chip::render(chip, &[inst], &[note], RATE, &mut out);
    out
}

fn crossings(x: &[f32]) -> usize {
    x.windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

#[test]
fn every_preset_stays_in_range_and_repeats_exactly() {
    let insts = [
        Instrument::pulse(0.125),
        Instrument::nes_triangle(),
        Instrument::noise(false),
        Instrument::noise(true),
        Instrument::sid_lead(),
        Instrument::sid_bass(),
        Instrument::arp_major(),
        Instrument::fm_organ(),
        Instrument::fm_brass(),
        Instrument::laser(),
    ];
    for c in [
        Chip::CLEAN,
        Chip::NES,
        Chip::GAMEBOY,
        Chip::C64,
        Chip::ADLIB,
        Chip::AMIGA,
    ] {
        for i in insts {
            let a = one(i, &c, 60);
            assert!(a.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
            assert!(a.iter().any(|s| *s != 0.0), "{i:?} on {c:?} is silent");
            assert_eq!(a, one(i, &c, 60));
        }
    }
}

#[test]
fn octave_doubles_pitch_and_release_ends_in_silence() {
    let low = crossings(&one(Instrument::pulse(0.5), &Chip::CLEAN, 57));
    let high = crossings(&one(Instrument::pulse(0.5), &Chip::CLEAN, 69));
    assert!(
        (high as f32 / low as f32 - 2.0).abs() < 0.05,
        "{low} {high}"
    );
    let out = one(Instrument::pulse(0.5), &Chip::CLEAN, 69);
    assert!(out[RATE as usize / 5 + 1000..].iter().all(|s| *s == 0.0));
}

#[test]
fn bits_and_levels_quantize() {
    let out = one(
        Instrument::fm_organ(),
        &Chip {
            bits: 4,
            ..Chip::CLEAN
        },
        60,
    );
    assert!(out.iter().all(|s| (s * 8.0).fract() == 0.0));
    let tri = one(
        Instrument {
            gain: 1.0,
            ..Instrument::nes_triangle()
        },
        &Chip::CLEAN,
        45,
    );
    let mut levels: Vec<i32> = tri[..RATE as usize / 5]
        .iter()
        .map(|s| (s * 1000.0) as i32)
        .collect();
    levels.sort_unstable();
    levels.dedup();
    assert!(levels.len() <= 17, "{} levels", levels.len());
}

#[test]
fn short_noise_loops_long_noise_does_not() {
    let fast = |short| Instrument {
        wave: Wave::Noise { short },
        env: knorke_audio::chip::Envelope::GATE,
        ..Instrument::default()
    };
    let s = one(fast(true), &Chip::CLEAN, 120);
    let l = one(fast(false), &Chip::CLEAN, 120);
    // Same shape repeats after 93 register steps in short mode: many equal windows.
    let rep = |x: &[f32]| {
        x.windows(64)
            .skip(1000)
            .take(2000)
            .filter(|w| *w == &x[1000..1064])
            .count()
    };
    assert!(rep(&s) > rep(&l), "{} {}", rep(&s), rep(&l));
}

#[cfg(feature = "opl")]
#[test]
fn opl_plays_in_tune_repeats_exactly_and_stops() {
    let note = Note {
        at: 0,
        len: RATE / 5,
        key: 69,
        instrument: 0,
        gain: 1.0,
    };
    let patch = chip::ChannelPatch {
        modulator: chip::OpPatch {
            mul: 1,
            tl: 63,
            attack: 15,
            release: 15,
            sustaining: true,
            ..Default::default()
        },
        carrier: chip::OpPatch {
            mul: 1,
            attack: 15,
            release: 15,
            sustaining: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let run = || {
        let mut out = vec![0.0; RATE as usize / 2];
        chip::render_opl(&Chip::CLEAN, &[patch], &[note], RATE, &mut out);
        out
    };
    let out = run();
    assert_eq!(out, run());
    assert!(out.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    // A pure carrier sine at A 440: 2 * 440 * 0.2 s crossings within ~2 %.
    let c = crossings(&out[..RATE as usize / 5]) as f32;
    assert!((c / 176.0 - 1.0).abs() < 0.03, "{c}");
    assert!(out[RATE as usize * 2 / 5..].iter().all(|s| s.abs() < 1e-3));
    for p in [
        chip::Patch::brass(),
        chip::Patch::organ(),
        chip::Patch::bass(),
        chip::Patch::bell(),
    ] {
        let mut o = vec![0.0; RATE as usize / 4];
        chip::render_opl(&Chip::ADLIB, &[p], &[note], RATE, &mut o);
        assert!(o.iter().any(|s| s.abs() > 0.01));
    }
}
