//! Where a voice sits in the stereo image: gain, brightness and delay per ear.
//! Model and constants from the Wizardry 7 rewrite (`revised/platform/sdl/audio_sdl.c`,
//! `update_spatial_target` and `mix_positioned`), rebuilt here for any world, not a grid.

use crate::{Config, Listener};
use knorke_math::{Vec2, det};

/// Per-ear values: `[left, right]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Ears {
    pub(crate) gain: [f32; 2],
    /// Coefficient of a one-pole low-pass, 1 = untouched.
    pub(crate) bright: [f32; 2],
    /// Lag in output frames; only the far ear lags.
    pub(crate) delay: [f32; 2],
}

impl Ears {
    pub(crate) fn audible(&self) -> bool {
        self.gain[0] + self.gain[1] > 1e-5
    }

    pub(crate) fn lerp(&self, to: &Self, t: f32) -> Self {
        let l = |a: [f32; 2], b: [f32; 2]| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        Self {
            gain: l(self.gain, to.gain),
            bright: l(self.bright, to.bright),
            delay: l(self.delay, to.delay),
        }
    }
}

/// Equal-power pan, scaled so the centre keeps unit gain.
fn pan_gains(pan: f32) -> [f32; 2] {
    let (s, c) = det::sin_cos((pan.clamp(-1.0, 1.0) + 1.0) * core::f32::consts::FRAC_PI_4);
    [c * core::f32::consts::SQRT_2, s * core::f32::consts::SQRT_2]
}

/// An unplaced voice: only its pan.
pub(crate) fn unplaced(pan: f32) -> Ears {
    Ears {
        gain: pan_gains(pan),
        bright: [1.0; 2],
        delay: [0.0; 2],
    }
}

/// Largest ear delay, 0.65 ms (head width).
const EAR_DELAY_S: f32 = 0.000_65;

/// A voice at `at` behind `walls` walls.
pub(crate) fn placed(
    listener: &Listener,
    at: Vec2,
    walls: f32,
    cfg: &Config,
    headphones: bool,
) -> Ears {
    let rel = at - listener.at.raw();
    let steps = rel.length() / cfg.unit;
    if steps > cfg.range {
        return Ears {
            gain: [0.0; 2],
            ..unplaced(0.0)
        };
    }
    // Below half a step the direction is meaningless; the sound is "here".
    let norm = det::max(steps, 0.5) * cfg.unit;
    let pan = rel.dot(listener.right) / norm;
    let rear = det::max(0.0, -rel.dot(listener.forward) / norm);
    let walls = det::max(walls, 0.0);
    let level = 1.0 / ((1.0 + cfg.rolloff * steps) * (1.0 + 0.6 * walls));
    let bright = 1.0 / (1.0 + cfg.air * steps + 5.0 * walls + 0.55 * rear);
    let g = pan_gains(pan);
    let mut ears = Ears {
        gain: [g[0] * level, g[1] * level],
        bright: [bright; 2],
        delay: [0.0; 2],
    };
    if headphones {
        // The ear away from the source hears it later and duller (head shadow).
        // Source on the right (pan > 0): the left ear (0) is the far one.
        let far = usize::from(pan < 0.0);
        let side = pan.abs().clamp(0.0, 1.0);
        ears.delay[far] = EAR_DELAY_S * cfg.rate as f32 * side;
        ears.bright[far] *= 1.0 - 0.6 * side;
    }
    ears
}

#[cfg(test)]
mod tests {
    use super::*;
    use knorke_math::At;

    fn listener() -> Listener {
        Listener::default()
    }

    #[test]
    fn right_side_is_louder_right_and_lags_left_with_headphones() {
        let cfg = Config::default();
        let e = placed(&listener(), Vec2::new(3.0, 0.0), 0.0, &cfg, true);
        assert!(e.gain[1] > e.gain[0]);
        assert!(e.delay[0] > 30.0 && e.delay[1] == 0.0, "{e:?}");
        assert!(e.bright[0] < e.bright[1]);
    }

    #[test]
    fn behind_and_walls_dull_and_quieten() {
        let cfg = Config::default();
        let front = placed(&listener(), Vec2::new(0.0, 2.0), 0.0, &cfg, false);
        let back = placed(&listener(), Vec2::new(0.0, -2.0), 0.0, &cfg, false);
        let wall = placed(&listener(), Vec2::new(0.0, 2.0), 1.0, &cfg, false);
        assert_eq!(front.gain, back.gain);
        assert!(back.bright[0] < front.bright[0]);
        assert!(wall.gain[0] < front.gain[0] && wall.bright[0] < back.bright[0]);
    }

    #[test]
    fn out_of_range_is_silent_and_centre_keeps_unit_gain() {
        let cfg = Config::default();
        assert!(!placed(&listener(), Vec2::new(0.0, 13.0), 0.0, &cfg, false).audible());
        let here = Listener {
            at: At::new(Vec2::new(5.0, 5.0)),
            ..listener()
        };
        let g = placed(&here, Vec2::new(5.0, 5.0), 0.0, &cfg, false).gain;
        assert!(
            (g[0] - 1.0).abs() < 1e-6 && (g[1] - 1.0).abs() < 1e-6,
            "{g:?}"
        );
    }
}
