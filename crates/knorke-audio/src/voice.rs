//! One playing voice: where it is in its sound, how it fades and how it reaches the ears.

use crate::ramp::Ramp;
use crate::spatial::{self, Ears};
use crate::{BLOCK, Config, Listener, Pcm, Play, SoundId, VoiceId};
use knorke_math::{Vec2, det};

#[derive(Debug)]
pub(crate) struct Voice {
    pub(crate) id: VoiceId,
    pub(crate) sound: SoundId,
    pub(crate) bus: usize,
    pub(crate) priority: u8,
    looped: bool,
    pub(crate) stopping: bool,
    /// Play position in source frames.
    pub(crate) pos: f64,
    pub(crate) pitch: f32,
    pub(crate) gain: Ramp,
    pub(crate) at: Option<Vec2>,
    pub(crate) walls: f32,
    pan: f32,
    /// Ear values at the start and end of the current block.
    from: Ears,
    to: Ears,
    /// Low-pass state per ear.
    lp: [f32; 2],
    /// Output frames until the voice starts ([`Play::start`], sound travel time).
    pub(crate) wait: u64,
}

/// What a voice did in one stretch of a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Mixed,
    /// Out of earshot: time kept, nothing mixed.
    Virtual,
    Ended,
}

impl Voice {
    pub(crate) fn new(id: VoiceId, play: &Play, wait: u64, ears: impl Fn(&Self) -> Ears) -> Self {
        let mut gain = Ramp::at(0.0);
        gain.to(det::max(play.gain, 0.0), play.fade_in);
        let mut v = Self {
            id,
            sound: play.sound,
            bus: play.bus.index(),
            priority: play.priority,
            looped: play.looped,
            stopping: false,
            pos: 0.0,
            pitch: det::max(play.pitch, 0.0),
            gain,
            at: play.at.map(|a| a.raw()),
            walls: play.walls,
            pan: play.pan,
            from: spatial::unplaced(0.0),
            to: spatial::unplaced(0.0),
            lp: [0.0; 2],
            wait,
        };
        v.to = ears(&v);
        v.from = v.to;
        v
    }

    /// Ear values for the current listener and position.
    pub(crate) fn ears(&self, listener: &Listener, cfg: &Config, headphones: bool) -> Ears {
        match self.at {
            Some(at) => spatial::placed(listener, at, self.walls, cfg, headphones),
            None => spatial::unplaced(self.pan),
        }
    }

    /// At a block start: the last target becomes the start of the new ramp.
    pub(crate) fn retarget(&mut self, to: Ears) {
        self.from = self.to;
        self.to = to;
    }

    /// Moves the play position by `by`; `true` once a one-shot voice ran past its end.
    #[inline]
    fn advance(&mut self, by: f64, frames: f64) -> bool {
        self.pos += by;
        if self.pos < frames {
            false
        } else if self.looped {
            self.pos %= frames;
            false
        } else {
            true
        }
    }

    /// Renders `send.len()` frames from block offset `phase` into `out` (stereo) and `send`
    /// (mono echo send), or only keeps time when out of earshot.
    pub(crate) fn render(
        &mut self,
        pcm: &Pcm<'_>,
        rate: u32,
        phase: usize,
        out: &mut [f32],
        send: &mut [f32],
    ) -> Step {
        let step = f64::from(pcm.rate) / f64::from(rate) * f64::from(self.pitch);
        let frames = pcm.frames() as f64;
        if !(self.from.audible() || self.to.audible()) {
            let n = send.len();
            self.gain.skip(n as u32);
            return if self.advance(step * n as f64, frames) {
                Step::Ended
            } else {
                Step::Virtual
            };
        }
        let placed = self.at.is_some();
        for (i, (o, s)) in out
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .zip(send.iter_mut())
            .enumerate()
        {
            let e = self.from.lerp(&self.to, (phase + i) as f32 / BLOCK as f32);
            let g = self.gain.next();
            let a = fetch(pcm, self.pos - f64::from(e.delay[0]) * step, self.looped);
            let b = if e.delay[1] == e.delay[0] {
                a
            } else {
                fetch(pcm, self.pos - f64::from(e.delay[1]) * step, self.looped)
            };
            // A placed source is a point: mono. An unplaced stereo source keeps its channels.
            let x = if placed {
                [(a.0 + a.1) * 0.5, (b.0 + b.1) * 0.5]
            } else {
                [a.0, b.1]
            };
            for ear in 0..2 {
                self.lp[ear] += (x[ear] - self.lp[ear]) * e.bright[ear];
                o[ear] += self.lp[ear] * e.gain[ear] * g;
            }
            if placed {
                *s += (o[0] + o[1]) * 0.5;
            }
            if self.advance(step, frames) {
                return Step::Ended;
            }
        }
        Step::Mixed
    }
}

/// Frame `p` of `pcm`, linearly interpolated; silence outside unless `looped`.
/// Returns a tuple, not `[f32; 2]`: with the array the voice loop measured ~12 % slower
/// (`--bench audio`, 2026-11-27).
#[inline]
fn fetch(pcm: &Pcm<'_>, p: f64, looped: bool) -> (f32, f32) {
    let frames = pcm.frames();
    // Only an ear delay reaches before the start; the play position itself stays in range.
    let p = if p >= 0.0 {
        p
    } else if looped {
        p + frames as f64
    } else {
        return (0.0, 0.0);
    };
    let get = |k: usize| {
        let k = match k {
            k if k < frames => k,
            k if looped => k % frames,
            _ => return (0.0, 0.0),
        };
        let s = pcm.samples;
        if pcm.stereo {
            (s[2 * k], s[2 * k + 1])
        } else {
            (s[k], s[k])
        }
    };
    let i = p as usize;
    let f = (p - i as f64) as f32;
    let (a, b) = (get(i), get(i + 1));
    (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
}
