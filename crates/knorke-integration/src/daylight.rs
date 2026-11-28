//! Light by time of day: sun by day, moon by night, and ambient light that fades between night and
//! day. The host owns the clock and passes the time of day; this has no clock of its own (ADR 0003)
//! and only turns a time into light. Every number is a field, so each game sets its own day.
//!
//! Path: the sun (and the moon at night) rises in the east (+x), passes the noon side (`noon_side`:
//! +1 south, -1 north) at `max_elevation` and sets in the west (-x).

use knorke_lighting::Sun;
use knorke_math::{Vec3, det};

use crate::pixel::LitRenderer;

/// Minutes in a day.
const DAY: f32 = 1440.0;

/// The parameters of one day. `Default` is a plain mid-latitude day from 6:00 to 18:00.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayCycle {
    /// Sunrise in minutes after midnight.
    pub sunrise: f32,
    /// Minutes from sunrise to sunset (0..1440).
    pub day_length: f32,
    /// Minutes after sunrise and before sunset over which ambient light fades between night and day.
    pub twilight: f32,
    /// Highest sun and moon, in radians above the horizon.
    pub max_elevation: f32,
    /// Side the sun passes at noon: +1 south (+z), -1 north (-z).
    pub noon_side: f32,
    /// Sun colour near the horizon and high up, `0xRRGGBB`; mixed by height.
    pub sun_low: u32,
    /// Sun colour high up.
    pub sun_high: u32,
    /// Sun strength high up; it fades to 0 at the horizon.
    pub sun_strength: f32,
    /// Moon colour.
    pub moon: u32,
    /// Moon strength high up; 0 = no moon.
    pub moon_strength: f32,
    /// Ambient colour and strength at night.
    pub ambient_night: (u32, f32),
    /// Ambient colour and strength by day.
    pub ambient_day: (u32, f32),
    /// Sun and moon cast shadows.
    pub shadows: bool,
}

impl Default for DayCycle {
    fn default() -> Self {
        Self {
            sunrise: 360.0,
            day_length: 720.0,
            twilight: 60.0,
            max_elevation: 1.0,
            noon_side: 1.0,
            sun_low: 0xffa868,
            sun_high: 0xfff4e0,
            sun_strength: 0.85,
            moon: 0x8898c8,
            moon_strength: 0.3,
            ambient_night: (0x404a78, 0.42),
            ambient_day: (0xd0d4e0, 0.95),
            shadows: true,
        }
    }
}

/// The light of one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Daylight {
    /// Sun by day, moon by night (strength 0 when below the horizon).
    pub sun: Sun,
    /// Ambient colour.
    pub ambient_color: u32,
    /// Ambient strength.
    pub ambient_strength: f32,
    /// 0 at night, 1 by full day, in between during twilight. Hosts use it for their own lights,
    /// e.g. a torch that is strong at night and fades out by day.
    pub day: f32,
}

impl Daylight {
    /// Sets sun and ambient light of `r`.
    pub fn apply(&self, r: &mut LitRenderer) {
        r.set_sun(Some(self.sun));
        r.set_ambient(self.ambient_color, self.ambient_strength);
    }
}

impl DayCycle {
    /// The light at `minute` after midnight (any value; it wraps around the day).
    pub fn at(&self, minute: f32) -> Daylight {
        let day_len = self.day_length.clamp(1.0, DAY - 1.0);
        let t = if minute.is_finite() {
            (minute - self.sunrise).rem_euclid(DAY)
        } else {
            0.0
        };
        let is_day = t < day_len;
        // 0..1 along the arc of the sun (by day) or the moon (by night)
        let s = if is_day {
            t / day_len
        } else {
            (t - day_len) / (DAY - day_len)
        };
        let fade = self.twilight.max(1e-3);
        let day = if is_day {
            smooth(t / fade) * smooth((day_len - t) / fade)
        } else {
            0.0
        };

        let (rise_sin, rise_cos) = det::sin_cos(core::f32::consts::PI * s);
        let height = rise_sin; // 0 at the horizon, 1 at the top of the arc
        let (el_sin, el_cos) = det::sin_cos(self.max_elevation * height);
        let toward = Vec3::new(
            rise_cos * el_cos,
            el_sin,
            self.noon_side.signum() * rise_sin * el_cos,
        );
        let (color, strength) = if is_day {
            (
                mix(self.sun_low, self.sun_high, smooth(height * 2.0)),
                self.sun_strength,
            )
        } else {
            (self.moon, self.moon_strength)
        };
        let (night, full) = (self.ambient_night, self.ambient_day);
        Daylight {
            sun: Sun {
                dir: -toward,
                color,
                strength: strength * smooth(height * 4.0),
                shadows: self.shadows,
            },
            ambient_color: mix(night.0, full.0, day),
            ambient_strength: night.1 + (full.1 - night.1) * day,
            day,
        }
    }
}

/// Smoothstep on 0..1, clamped.
fn smooth(a: f32) -> f32 {
    let a = a.clamp(0.0, 1.0);
    a * a * (3.0 - 2.0 * a)
}

/// Per-channel mix of two `0xRRGGBB` colours.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |c: u32, s: u32| ((c >> s) & 0xff) as f32;
    let one = |s: u32| ((ch(a, s) + (ch(b, s) - ch(a, s)) * t).round() as u32) << s;
    one(16) | one(8) | one(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noon_sun_is_high_on_the_noon_side_and_bright() {
        let c = DayCycle::default();
        let l = c.at(720.0);
        assert!(l.day == 1.0 && l.sun.strength == c.sun_strength && l.sun.color == c.sun_high);
        // light travels down and away from the south (noon side +1)
        assert!(l.sun.dir.y < -0.8 && l.sun.dir.z < 0.0 && l.sun.dir.x.abs() < 1e-4);
        assert_eq!(l.ambient_color, c.ambient_day.0);
        assert!((l.ambient_strength - c.ambient_day.1).abs() < 1e-6);
        let north = DayCycle {
            noon_side: -1.0,
            ..c
        }
        .at(720.0);
        assert!(north.sun.dir.z > 0.0);
    }

    #[test]
    fn sun_rises_in_the_east_and_night_has_the_moon() {
        let c = DayCycle::default();
        let morning = c.at(c.sunrise + 1.0);
        assert!(morning.sun.dir.x < -0.9 && morning.sun.strength < 0.01 && morning.day < 0.01);
        let evening = c.at(c.sunrise + c.day_length - 1.0);
        assert!(evening.sun.dir.x > 0.9);
        let midnight = c.at(0.0);
        assert!(midnight.day == 0.0 && midnight.sun.color == c.moon);
        assert!((midnight.sun.strength - c.moon_strength).abs() < 1e-6);
        assert_eq!(
            (midnight.ambient_color, midnight.ambient_strength),
            c.ambient_night
        );
    }

    #[test]
    fn time_wraps_around_the_day() {
        let c = DayCycle::default();
        assert_eq!(c.at(600.0), c.at(600.0 + 1440.0));
        assert_eq!(c.at(600.0), c.at(600.0 - 1440.0));
        assert_eq!(c.at(f32::NAN), c.at(c.sunrise));
    }
}
