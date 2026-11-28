//! Real AdLib / Sound Blaster FM (feature `opl`): notes through the OPL2 emulator `oplon`
//! (MIT, codeberg.org/sbechet/oplon, integer only, so the output is the same everywhere).
//!
//! Unlike [`super::Wave::Fm`], which only imitates the sound, this runs the chip itself: nine
//! two-operator channels with the chip's envelopes, waveforms and feedback.

use super::{Chip, Note, color};
use knorke_math::det;

pub use oplon::{ChannelPatch, OpPatch};

/// The OPL2's native rate in Hz (14.318 MHz / 288); frequency numbers count in it.
const NATIVE: f32 = 49_716.0;
/// Output per channel is up to ±4084; this leaves headroom for a few loud channels.
const SCALE: f32 = 1.0 / 12_000.0;

/// Ready-made OPL instruments in the style of early-90s AdLib game music.
pub struct Patch;

impl Patch {
    fn op(mul: u8, tl: u8, ad: (u8, u8), sr: (u8, u8), wave: u8) -> OpPatch {
        OpPatch {
            mul,
            tl,
            attack: ad.0,
            decay: ad.1,
            sustain: sr.0,
            release: sr.1,
            waveform: wave,
            sustaining: true,
            ..OpPatch::default()
        }
    }

    /// Brass: modulator opens fast, a bit of feedback.
    pub fn brass() -> ChannelPatch {
        ChannelPatch {
            modulator: Self::op(1, 18, (12, 4), (4, 6), 0),
            carrier: Self::op(1, 0, (11, 3), (3, 6), 0),
            feedback: 5,
            additive: false,
        }
    }

    /// Organ: both operators sound (additive), octave apart.
    pub fn organ() -> ChannelPatch {
        ChannelPatch {
            modulator: Self::op(2, 4, (15, 0), (2, 8), 1),
            carrier: Self::op(1, 0, (15, 0), (2, 8), 0),
            feedback: 2,
            additive: true,
        }
    }

    /// Bass: deep, plucked.
    pub fn bass() -> ChannelPatch {
        ChannelPatch {
            modulator: Self::op(1, 14, (15, 5), (8, 8), 0),
            carrier: Self::op(1, 0, (15, 4), (6, 8), 0),
            feedback: 4,
            additive: false,
        }
    }

    /// Bell: odd ratio, long ring.
    pub fn bell() -> ChannelPatch {
        ChannelPatch {
            modulator: Self::op(7, 20, (15, 4), (10, 4), 0),
            carrier: Self::op(2, 0, (15, 2), (12, 4), 0),
            feedback: 0,
            additive: false,
        }
    }
}

/// F-number and block (octave) of a MIDI key.
fn freq(key: u8) -> (u16, u8) {
    let hz = 440.0 * det::exp2((f32::from(key) - 69.0) / 12.0);
    for block in 0..8u8 {
        let f = hz * (1u32 << (20 - u32::from(block))) as f32 / NATIVE;
        if f < 1024.0 {
            return (f as u16, block);
        }
    }
    (1023, 7)
}

/// Plays `notes` (instrument = index into `patches`) on an emulated OPL2 into `out` (mono,
/// `rate` Hz), then applies `chip`'s coloring ([`Chip::CLEAN`] for none). Nine notes sound at
/// once, as on the real card; a tenth takes the channel that started first. `out` is cleared.
pub fn render_opl(
    chip: &Chip,
    patches: &[ChannelPatch],
    notes: &[Note],
    rate: u32,
    out: &mut [f32],
) {
    out.fill(0.0);
    if rate == 0 {
        return;
    }
    // Key-on and key-off as (frame, note index, on), in time order; offline, so a Vec is fine.
    let mut events: Vec<(u32, usize, bool)> = notes
        .iter()
        .enumerate()
        .filter(|(_, n)| usize::from(n.instrument) < patches.len())
        .flat_map(|(i, n)| [(n.at, i, true), (n.at.saturating_add(n.len), i, false)])
        .collect();
    // Offs before ons on the same frame, so a repeated note gets its channel back.
    events.sort_by_key(|&(at, i, on)| (at, on, i));
    let mut opl = oplon::Opl2::new(rate);
    let mut owner = [usize::MAX; 9];
    let mut started = [0u32; 9];
    let mut next = events.iter().peekable();
    for (frame, o) in out.iter_mut().enumerate() {
        while let Some(&&(at, i, on)) = next.peek() {
            if at as usize > frame {
                break;
            }
            next.next();
            let n = &notes[i];
            if on {
                let ch = (0..9)
                    .find(|&c| owner[c] == usize::MAX)
                    .unwrap_or_else(|| (0..9).min_by_key(|&c| started[c]).unwrap_or(0));
                opl.key_off(ch);
                opl.set_patch(ch, &patches[usize::from(n.instrument)]);
                // Note gain as carrier attenuation: 0.75 dB per step.
                let db = -20.0 * det::log10(det::max(n.gain, 0.001));
                opl.set_carrier_tl(ch, det::min(db / 0.75, 63.0) as u8);
                let (fnum, block) = freq(n.key);
                opl.set_frequency(ch, fnum, block);
                opl.key_on(ch);
                owner[ch] = i;
                started[ch] = at;
            } else if let Some(ch) = owner.iter().position(|&w| w == i) {
                opl.key_off(ch);
                owner[ch] = usize::MAX;
            }
        }
        *o = opl.render_frame().0 as f32 * SCALE;
    }
    color(chip, rate, out);
}
