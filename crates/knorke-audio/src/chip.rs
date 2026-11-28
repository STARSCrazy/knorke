//! Retro synthesizer: 8- and 16-bit console and home computer sound (Plan 17.6).
//!
//! The sound chips of the 1980s had a few voices with simple waves: NES two pulses, a stepped
//! triangle and noise; C64 (SID) pulse with moving width, saw, triangle and noise; AdLib and
//! Sound Blaster (OPL) frequency modulation. [`render`] plays [`Note`]s with [`Instrument`]s in
//! the style of a [`Chip`] into a buffer the host lends. The result is ordinary mono PCM for
//! the [`crate::Mixer`] (`Pcm { rate, stereo: false, .. }`), so jingles and sound effects are
//! made once, at load time, not on the audio thread.
//!
//! The tricks of the old music are here too: arpeggio (a fast cycle through chord notes so
//! one voice sounds like a chord), vibrato, pitch slides for lasers and drops, pulse-width
//! sweeps for the SID's fat lead. [`Chip`] adds the hardware's character: few volume steps,
//! a low sample rate (sample and hold), few bits and a dull output.
//!
//! All math goes through `knorke_math::det`, so the same notes give the same samples on every
//! platform.

use core::f32::consts::TAU;

use knorke_math::det;

#[cfg(feature = "opl")]
mod opl;
#[cfg(feature = "opl")]
pub use opl::{ChannelPatch, OpPatch, Patch, render_opl};

/// The shape of a voice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wave {
    /// Square with adjustable width ([`Instrument::duty`]); NES and Game Boy: 0.125, 0.25,
    /// 0.5, 0.75.
    Pulse,
    /// Triangle; with [`Instrument::steps`] = 16 the stepped one of the NES.
    Triangle,
    /// Sawtooth (C64 SID).
    Saw,
    /// Noise from a 15-bit shift register like the NES; the note sets how fast it ticks.
    Noise {
        /// Short 93-step loop: metallic, buzzing (NES "mode 1").
        short: bool,
    },
    /// Two-operator FM as on AdLib/Sound Blaster (OPL2): a sine bends the phase of another.
    Fm {
        /// Modulator frequency as a multiple of the note.
        ratio: f32,
        /// Modulation strength in radians; falls with the envelope, so notes start bright.
        depth: f32,
    },
}

/// Attack, decay, sustain, release.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
    /// Rise to full level, in milliseconds.
    pub attack_ms: f32,
    /// Fall to `sustain`, in milliseconds.
    pub decay_ms: f32,
    /// Level held while the note is down, 0..=1.
    pub sustain: f32,
    /// Fade after the note ends, in milliseconds (the buffer must leave room for it).
    pub release_ms: f32,
}

impl Envelope {
    /// Hard on, hard off: organ-like.
    pub const GATE: Self = Self {
        attack_ms: 0.0,
        decay_ms: 0.0,
        sustain: 1.0,
        release_ms: 5.0,
    };
    /// Short hit that dies away: drums, plucks, blips.
    pub const PLUCK: Self = Self {
        attack_ms: 0.0,
        decay_ms: 180.0,
        sustain: 0.0,
        release_ms: 5.0,
    };

    /// Level `t` seconds after note on, for a note held `held` seconds.
    fn level(&self, t: f32, held: f32) -> f32 {
        let down = |t: f32| {
            let a = self.attack_ms * 0.001;
            let d = self.decay_ms * 0.001;
            if t < a {
                t / a
            } else if t < a + d {
                1.0 - (1.0 - self.sustain) * (t - a) / d
            } else {
                self.sustain
            }
        };
        if t < held {
            return down(t);
        }
        let r = self.release_ms * 0.001;
        let after = t - held;
        if after >= r {
            0.0
        } else {
            down(held) * (1.0 - after / r)
        }
    }
}

/// How a note sounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instrument {
    /// Wave shape.
    pub wave: Wave,
    /// Pulse width, 0..1 (only [`Wave::Pulse`]).
    pub duty: f32,
    /// Pulse width swing (amount, Hz): the C64 "PWM" lead.
    pub pwm: (f32, f32),
    /// Quantize the wave to this many levels (0: smooth). NES triangle: 16.
    pub steps: u8,
    /// Envelope.
    pub env: Envelope,
    /// Vibrato (semitones, Hz), starting after a short delay like on the old trackers.
    pub vibrato: (f32, f32),
    /// Arpeggio: semitone offsets cycled at `arp_hz`; all zero for none.
    pub arp: [i8; 4],
    /// Arpeggio steps per second (50 or 60 on the old machines: one per video frame).
    pub arp_hz: f32,
    /// Pitch slide in semitones per second (negative falls: lasers, drops, kicks).
    pub slide: f32,
    /// Linear level.
    pub gain: f32,
}

impl Default for Instrument {
    fn default() -> Self {
        Self {
            wave: Wave::Pulse,
            duty: 0.5,
            pwm: (0.0, 0.0),
            steps: 0,
            env: Envelope::GATE,
            vibrato: (0.0, 0.0),
            arp: [0; 4],
            arp_hz: 50.0,
            slide: 0.0,
            gain: 0.3,
        }
    }
}

impl Instrument {
    /// NES/Game Boy pulse with width `duty` (0.125, 0.25 or 0.5).
    pub fn pulse(duty: f32) -> Self {
        Self {
            duty,
            ..Self::default()
        }
    }

    /// NES triangle: stepped, used for bass; it had no volume, so it is always loud.
    pub fn nes_triangle() -> Self {
        Self {
            wave: Wave::Triangle,
            steps: 16,
            gain: 0.35,
            ..Self::default()
        }
    }

    /// Noise drum: long noise for snare and hi-hat, short for metal.
    pub fn noise(short: bool) -> Self {
        Self {
            wave: Wave::Noise { short },
            env: Envelope::PLUCK,
            gain: 0.25,
            ..Self::default()
        }
    }

    /// C64 lead: pulse whose width swings slowly, with vibrato.
    pub fn sid_lead() -> Self {
        Self {
            duty: 0.5,
            pwm: (0.35, 1.5),
            vibrato: (0.25, 6.0),
            env: Envelope {
                attack_ms: 5.0,
                decay_ms: 300.0,
                sustain: 0.7,
                release_ms: 120.0,
            },
            ..Self::default()
        }
    }

    /// C64 bass: sawtooth with a quick decay.
    pub fn sid_bass() -> Self {
        Self {
            wave: Wave::Saw,
            env: Envelope {
                attack_ms: 0.0,
                decay_ms: 250.0,
                sustain: 0.5,
                release_ms: 40.0,
            },
            ..Self::default()
        }
    }

    /// Chord from one voice: root, major third, fifth (C64 and NES classic).
    pub fn arp_major() -> Self {
        Self {
            arp: [0, 4, 7, 12],
            duty: 0.25,
            ..Self::default()
        }
    }

    /// AdLib/OPL organ-ish FM tone.
    pub fn fm_organ() -> Self {
        Self {
            wave: Wave::Fm {
                ratio: 2.0,
                depth: 1.5,
            },
            env: Envelope {
                attack_ms: 10.0,
                decay_ms: 400.0,
                sustain: 0.6,
                release_ms: 200.0,
            },
            ..Self::default()
        }
    }

    /// AdLib/OPL brass: bright attack that softens.
    pub fn fm_brass() -> Self {
        Self {
            wave: Wave::Fm {
                ratio: 1.0,
                depth: 4.0,
            },
            env: Envelope {
                attack_ms: 30.0,
                decay_ms: 500.0,
                sustain: 0.5,
                release_ms: 150.0,
            },
            vibrato: (0.15, 5.0),
            ..Self::default()
        }
    }

    /// Falling laser blip.
    pub fn laser() -> Self {
        Self {
            duty: 0.125,
            slide: -60.0,
            env: Envelope::PLUCK,
            ..Self::default()
        }
    }
}

/// The hardware's character, applied to the whole mix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chip {
    /// Volume steps per voice (NES: 16); 0 for smooth.
    pub levels: u8,
    /// Output bits (8 for the old DACs); 0 for full resolution.
    pub bits: u8,
    /// Output rate of the old machine in Hz; each sample is held until the next (0: off).
    pub hold_hz: u32,
    /// Dull output above this frequency (old speakers and filters); 0: off.
    pub lowpass_hz: f32,
}

impl Chip {
    /// No coloring: clean synthesis.
    pub const CLEAN: Self = Self {
        levels: 0,
        bits: 0,
        hold_hz: 0,
        lowpass_hz: 0.0,
    };
    /// Nintendo Entertainment System.
    pub const NES: Self = Self {
        levels: 16,
        bits: 8,
        hold_hz: 0,
        lowpass_hz: 14_000.0,
    };
    /// Game Boy: rougher, duller.
    pub const GAMEBOY: Self = Self {
        levels: 16,
        bits: 6,
        hold_hz: 32_768,
        lowpass_hz: 8_000.0,
    };
    /// Commodore 64 (SID): warm, dull filter.
    pub const C64: Self = Self {
        levels: 16,
        bits: 0,
        hold_hz: 0,
        lowpass_hz: 6_000.0,
    };
    /// AdLib / Sound Blaster (OPL2 at about 49.7 kHz, 13-bit DAC).
    pub const ADLIB: Self = Self {
        levels: 64,
        bits: 13,
        hold_hz: 0,
        lowpass_hz: 12_000.0,
    };
    /// Amiga: 8-bit samples at a low rate.
    pub const AMIGA: Self = Self {
        levels: 64,
        bits: 8,
        hold_hz: 22_050,
        lowpass_hz: 4_500.0,
    };
}

/// One note.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    /// Start in output frames.
    pub at: u32,
    /// How long it is held, in frames (the release follows).
    pub len: u32,
    /// MIDI key: 60 is middle C, 69 is A 440 Hz.
    pub key: u8,
    /// Index into the instrument list.
    pub instrument: u8,
    /// Linear level of this note.
    pub gain: f32,
}

/// Plays `notes` with `instruments` in the style of `chip` into `out` (mono, `rate` Hz).
/// `out` is cleared first; notes outside it are cut, unknown instruments skipped.
pub fn render(chip: &Chip, instruments: &[Instrument], notes: &[Note], rate: u32, out: &mut [f32]) {
    out.fill(0.0);
    if rate == 0 {
        return;
    }
    for note in notes {
        if let Some(inst) = instruments.get(usize::from(note.instrument)) {
            add_note(chip, inst, note, rate, out);
        }
    }
    color(chip, rate, out);
}

fn add_note(chip: &Chip, inst: &Instrument, note: &Note, rate: u32, out: &mut [f32]) {
    let dt = 1.0 / rate as f32;
    let held = note.len as f32 * dt;
    let total = note.len as usize + (inst.env.release_ms * 0.001 * rate as f32) as usize;
    let start = note.at as usize;
    let Some(span) = out.get_mut(start..(start + total).min(out.len())) else {
        return;
    };
    let arp_len = inst.arp.iter().rposition(|&s| s != 0).map_or(1, |i| i + 1);
    let mut phase = 0.0_f32;
    let mut noise = Lfsr::new();
    for (i, o) in span.iter_mut().enumerate() {
        let t = i as f32 * dt;
        let env = inst.env.level(t, held);
        let env = if chip.levels > 0 {
            let l = f32::from(chip.levels - 1);
            (env * l).round() / l
        } else {
            env
        };
        let arp = f32::from(inst.arp[(t * inst.arp_hz) as usize % arp_len]);
        // Vibrato fades in over the first quarter second, as the trackers did it.
        let vib_in = det::min(t * 4.0, 1.0);
        let vib = inst.vibrato.0 * vib_in * det::sin(TAU * inst.vibrato.1 * t);
        let semis = f32::from(note.key) - 69.0 + arp + vib + inst.slide * t;
        let hz = 440.0 * det::exp2(semis / 12.0);
        let step = hz * dt;
        let x = match inst.wave {
            Wave::Pulse => {
                let duty = inst.duty + inst.pwm.0 * 0.5 * det::sin(TAU * inst.pwm.1 * t);
                let duty = duty.clamp(0.02, 0.98);
                if phase < duty { 1.0 } else { -1.0 }
            }
            Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
            Wave::Saw => 2.0 * phase - 1.0,
            Wave::Noise { short } => noise.tick(step * 16.0, short),
            Wave::Fm { ratio, depth } => {
                let m = det::sin(TAU * phase * ratio) * depth * env;
                det::sin(TAU * phase + m)
            }
        };
        let x = if inst.steps > 1 {
            let s = f32::from(inst.steps - 1) * 0.5;
            ((x + 1.0) * s).round() / s - 1.0
        } else {
            x
        };
        *o += x * env * inst.gain * note.gain;
        phase += step;
        phase -= phase.floor();
    }
}

/// The 15-bit noise register of the NES APU: tap 1 for the long, tap 6 for the short loop.
struct Lfsr {
    reg: u16,
    /// Fractional register ticks not yet taken.
    acc: f32,
}

impl Lfsr {
    fn new() -> Self {
        Self { reg: 1, acc: 0.0 }
    }

    fn tick(&mut self, ticks: f32, short: bool) -> f32 {
        self.acc += ticks;
        while self.acc >= 1.0 {
            self.acc -= 1.0;
            let tap = if short { 6 } else { 1 };
            let bit = (self.reg ^ (self.reg >> tap)) & 1;
            self.reg = (self.reg >> 1) | (bit << 14);
        }
        if self.reg & 1 == 0 { 1.0 } else { -1.0 }
    }
}

/// Mix gain, low rate, low pass, few bits: what the old output stage did.
fn color(chip: &Chip, rate: u32, out: &mut [f32]) {
    let hold = if chip.hold_hz > 0 && chip.hold_hz < rate {
        rate as f32 / chip.hold_hz as f32
    } else {
        0.0
    };
    let lp = if chip.lowpass_hz > 0.0 {
        1.0 - det::exp(-TAU * chip.lowpass_hz / rate as f32)
    } else {
        1.0
    };
    let q = if chip.bits > 0 {
        f32::from(1u16 << (chip.bits.min(15) - 1))
    } else {
        0.0
    };
    let (mut held, mut next, mut y) = (0.0_f32, 0.0_f32, 0.0_f32);
    for (i, s) in out.iter_mut().enumerate() {
        if hold == 0.0 || i as f32 >= next {
            held = *s;
            next += hold;
        }
        y += (held - y) * lp;
        let v = y.clamp(-1.0, 1.0);
        *s = if q > 0.0 { (v * q).round() / q } else { v };
    }
}

/// Frames for `beats` at `bpm` beats per minute, for laying out notes.
pub fn beats(beats: f32, bpm: f32, rate: u32) -> u32 {
    (beats * 60.0 / bpm * rate as f32) as u32
}
