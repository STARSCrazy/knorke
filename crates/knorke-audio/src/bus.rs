//! A mix bus: gain fade, filter and sidechain duck, applied frame by frame in the output.

use crate::ramp::Ramp;
use crate::{Duck, Filter};
use knorke_math::det;

/// Level of the listened-to bus (before its bus gain) at which a duck reaches full depth:
/// -12 dBFS, so normal speech ducks fully and quiet tails let go gradually.
const DUCK_FULL: f32 = 0.25;

#[derive(Clone, Debug)]
pub(crate) struct BusState {
    pub(crate) gain: Ramp,
    /// One-pole coefficients, faded like gains: low-pass 1 and high-pass 0 are open.
    lowpass: Ramp,
    highpass: Ramp,
    /// Two low-pass stages, then two high-pass stages, per channel.
    z: [[f32; 2]; 4],
    duck: Option<Ducking>,
}

#[derive(Clone, Copy, Debug)]
struct Ducking {
    by: usize,
    depth: f32,
    attack: f32,
    release: f32,
    /// Followed level of the `by` bus.
    env: f32,
}

impl Ducking {
    /// Follows the level of the `by` bus at this frame; returns the gain factor for the bus.
    #[inline]
    fn gain(&mut self, by: [f32; 2]) -> f32 {
        let level = det::max(by[0].abs(), by[1].abs());
        let c = if level > self.env {
            self.attack
        } else {
            self.release
        };
        self.env += (level - self.env) * c;
        1.0 - self.depth * (self.env / DUCK_FULL).clamp(0.0, 1.0)
    }
}

/// One-pole low-pass coefficient for `hz`; `None` when off (0) or at/above Nyquist.
fn one_pole(hz: f32, rate: u32) -> Option<f32> {
    let rate = rate as f32;
    (hz > 0.0 && hz < rate * 0.5).then(|| 1.0 - det::exp(-core::f32::consts::TAU * hz / rate))
}

/// Follower coefficient that covers ~63 % of a step in `ms`.
fn follow(ms: u16, rate: u32) -> f32 {
    let frames = (u32::from(ms) * rate / 1000).max(1) as f32;
    1.0 - det::exp(-1.0 / frames)
}

impl BusState {
    pub(crate) fn new() -> Self {
        Self {
            gain: Ramp::at(1.0),
            lowpass: Ramp::at(1.0),
            highpass: Ramp::at(0.0),
            z: [[0.0; 2]; 4],
            duck: None,
        }
    }

    /// Off = settled open, then the bus skips the filter entirely (ADR 0011).
    fn filtering(&self) -> bool {
        !(self.lowpass.settled_at(1.0) && self.highpass.silent())
    }

    pub(crate) fn set_filter(&mut self, f: Filter, rate: u32, fade: u32) {
        if !self.filtering() {
            // State from an earlier filter must not ring into the new one.
            self.z = [[0.0; 2]; 4];
        }
        self.lowpass
            .to(one_pole(f.lowpass_hz, rate).unwrap_or(1.0), fade);
        self.highpass
            .to(one_pole(f.highpass_hz, rate).unwrap_or(0.0), fade);
    }

    /// Sets or clears the duck; a duck on a bus at or above `buses` is cleared.
    pub(crate) fn set_duck(&mut self, duck: Option<Duck>, buses: usize, rate: u32) {
        // A changed duck keeps its level, so it does not jump.
        let env = self.duck.map_or(0.0, |old| old.env);
        self.duck = duck.filter(|d| d.by.index() < buses).map(|d| Ducking {
            by: d.by.index(),
            depth: d.depth.clamp(0.0, 1.0),
            attack: follow(d.attack_ms, rate),
            release: follow(d.release_ms, rate),
            env,
        });
    }

    /// The next output frame of bus `own`; `frame(b)` reads this frame of bus `b`'s mix.
    #[inline]
    pub(crate) fn next(&mut self, own: usize, frame: impl Fn(usize) -> [f32; 2]) -> [f32; 2] {
        let mut g = self.gain.next();
        if let Some(d) = &mut self.duck {
            g *= d.gain(frame(d.by));
        }
        let x = if self.filtering() {
            self.filter(frame(own))
        } else {
            frame(own)
        };
        [x[0] * g, x[1] * g]
    }

    #[inline]
    fn filter(&mut self, x: [f32; 2]) -> [f32; 2] {
        let (a, h) = (self.lowpass.next(), self.highpass.next());
        let z = &mut self.z;
        let mut y = x;
        for ch in 0..2 {
            z[0][ch] += (y[ch] - z[0][ch]) * a;
            z[1][ch] += (z[0][ch] - z[1][ch]) * a;
            z[2][ch] += (z[1][ch] - z[2][ch]) * h;
            let thin = z[1][ch] - z[2][ch];
            z[3][ch] += (thin - z[3][ch]) * h;
            y[ch] = thin - z[3][ch];
        }
        y
    }
}
