// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/core/Shockwave.hpp and Shockwave.cpp), zlib licence:
//
// Copyright (C) 2026 Manuel Vögele
//
// This software is provided 'as-is', without any express or implied
// warranty.  In no event will the authors be held liable for any damages
// arising from the use of this software.
//
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not
//    claim that you wrote the original software. If you use this software
//    in a product, an acknowledgment in the product documentation would be
//    appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be
//    misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.

//! Shockwaves (E140 to E144): the wave list, its validation, the time and size helpers, the culling and
//! the CPU mirrors of the two shader steps (displacement and ring light).
//!
//! Differences to the C++ original: the light colour is a [`LinearRgba`] that the caller passes in
//! (`CHI_Kelvin` is not ported, so the orange default of `CHI_InitShockwaveDesc()` is not built in),
//! [`ExplosionParams::kelvin`] carries the colour temperature instead of a colour, and validation errors
//! are a [`ShockwaveError`] value instead of an SDL error string.

use knorke_math::{LinearRgba, Vec2, det};

use core::f32::consts::PI;

use crate::clamp;

/// Capacity of the wave list (`CHI_MAX_SHOCKWAVES`).
pub const MAX_SHOCKWAVES: usize = 16;
/// Largest power of an explosion; above it the blast stays the same.
pub const MAX_EXPLOSION_POWER: f32 = 4.0;
/// `shockwave` keeps `|strength| <= SHOCKWAVE_FOLD_LIMIT * width`; the band folds above `width / pi`.
pub const SHOCKWAVE_FOLD_LIMIT: f32 = 0.28;
/// Part of the life over which a wave of [`make_shockwave`] fades in.
pub const SHOCKWAVE_FADE_IN: f32 = 0.05;

/// Scene projection of the world: where the origin and the unit axes land, in scene pixels (the same
/// layout as `CHI_Projection`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    /// Scene pixel of the world origin.
    pub origin: Vec2,
    /// Scene pixels per world unit along x.
    pub x_axis: Vec2,
    /// Scene pixels per world unit along y; must not be collinear with `x_axis`.
    pub y_axis: Vec2,
    /// Scene pixels per world unit of height; zero for top-down views.
    pub z_axis: Vec2,
}

/// One shockwave (the same fields as `CHI_ShockwaveDesc`, without the flags).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shockwave {
    /// Centre, world x.
    pub x: f32,
    /// Centre, world y.
    pub y: f32,
    /// Radius of the band centre on the ground.
    pub radius: f32,
    /// Band width.
    pub width: f32,
    /// Displacement at the ring, world units; negative pushes inwards.
    pub strength: f32,
    /// Colour of the ring light, linear, each channel `>= 0`.
    pub light: LinearRgba,
    /// Factor of the ring light, `>= 0`.
    pub light_intensity: f32,
}

impl Default for Shockwave {
    /// No distortion, no ring light, width 1. The light colour is black (see the module docs).
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            radius: 0.0,
            width: 1.0,
            strength: 0.0,
            light: LinearRgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            light_intensity: 0.0,
        }
    }
}

/// The size law of an explosion for one power and reach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExplosionParams {
    /// Final radius.
    pub max_radius: f32,
    /// Life in seconds; 0 for power `<= 0`.
    pub duration: f32,
    /// Displacement at the ring.
    pub strength: f32,
    /// Factor of the ring light.
    pub light_intensity: f32,
    /// Colour temperature of the ring light in kelvin (to be turned into a colour by the caller).
    pub kelvin: f32,
}

/// `S(p) = (1 - e^-p) / (1 - e^-1)`, 1 at power 1, saturating at 1.58. `p` is clamped to
/// `[0, MAX_EXPLOSION_POWER]`.
pub fn explosion_saturation(power: f32) -> f32 {
    let p = clamp(power, 0.0, MAX_EXPLOSION_POWER);
    (1.0 - det::exp(-p)) / (1.0 - det::exp(-1.0))
}

/// The size law of an explosion for `power` (clamped to `[0, MAX_EXPLOSION_POWER]`) and `reach`.
pub fn explosion_law(power: f32, reach: f32) -> ExplosionParams {
    let p = if power > 0.0 {
        det::min(power, MAX_EXPLOSION_POWER)
    } else {
        0.0
    };
    let s = explosion_saturation(p);
    ExplosionParams {
        max_radius: reach * p,
        duration: 0.6 * p.sqrt(),
        strength: 0.1 * reach * s,
        light_intensity: 4.0 * s,
        kelvin: 1800.0 + 1200.0 * s,
    }
}

/// Band profile `P(s) = (1 + cos(pi s)) / 2` for `|s| < 1`, else 0.
pub fn shockwave_profile(s: f32) -> f32 {
    if s.abs() < 1.0 {
        0.5 + 0.5 * det::cos(PI * s)
    } else {
        0.0
    }
}

/// The wave `age` seconds into a life of `duration` (`CHI_Shockwave()`). Before the start, after the end
/// or without a duration the wave has no effect and sits at its final size.
pub fn make_shockwave(
    origin: Vec2,
    age: f32,
    duration: f32,
    max_radius: f32,
    strength: f32,
    light_intensity: f32,
    light: LinearRgba,
) -> Shockwave {
    let t = if duration > 0.0 { age / duration } else { -1.0 };
    let mut d = Shockwave {
        x: origin.x,
        y: origin.y,
        light,
        ..Shockwave::default()
    };
    // Width of the band: `scale` times the max radius; 1 for a zero radius, the radius itself if negative.
    let width_of = |scale: f32| {
        if max_radius > 0.0 {
            max_radius * scale
        } else if max_radius == 0.0 {
            1.0
        } else {
            max_radius
        }
    };
    if !(0.0..1.0).contains(&t) {
        // Before the start, after the end, or no duration: a wave without effect at its final size.
        d.radius = max_radius;
        d.width = width_of(0.35);
        return d;
    }
    let rest = 1.0 - t;
    d.radius = max_radius * (1.0 - rest * rest * rest);
    d.width = width_of(0.2 + 0.2 * t);
    let fade = det::min(1.0, t / SHOCKWAVE_FADE_IN) * rest * rest.sqrt();
    let limit = SHOCKWAVE_FOLD_LIMIT * d.width;
    d.strength = clamp(strength * fade, -limit, limit);
    if !strength.is_finite() {
        d.strength = strength; // rejected by the validation, with the field named
    }
    d.light_intensity = light_intensity * fade;
    d
}

/// The wave of an explosion (`CHI_ExplosionShockwave()`), with the size law of [`explosion_law`].
/// `light` is the colour of the ring light, built by the caller from [`ExplosionParams::kelvin`].
pub fn make_explosion_shockwave(
    origin: Vec2,
    age: f32,
    power: f32,
    reach: f32,
    light: LinearRgba,
) -> Shockwave {
    let e = explosion_law(power, reach);
    let mut d = make_shockwave(
        origin,
        age,
        e.duration,
        e.max_radius,
        e.strength,
        e.light_intensity,
        light,
    );
    if e.max_radius == 0.0 && reach.is_finite() {
        d.width = if reach > 0.0 { reach * 0.35 } else { 1.0 }; // power 0: a valid wave without effect
    }
    d
}

/// Why a wave or a wave list was rejected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShockwaveError {
    /// More waves than [`MAX_SHOCKWAVES`].
    TooMany {
        /// Number of waves given.
        count: usize,
    },
    /// A field of one wave is out of its range.
    Invalid {
        /// Index of the wave in the list.
        index: usize,
        /// Name of the field, as in `CHI_ShockwaveDesc` (`"light.g"` for the green light channel).
        field: &'static str,
        /// The rule the field broke, e.g. `"finite and >= 0"`.
        rule: &'static str,
    },
}

/// Checks one wave: finite positions and strength, `radius >= 0`, `width > 0`, light `>= 0`.
pub fn validate_shockwave(wave: &Shockwave, index: usize) -> Result<(), ShockwaveError> {
    let invalid = |field: &'static str, rule: &'static str| {
        Err(ShockwaveError::Invalid { index, field, rule })
    };
    let finite_non_negative = "finite and >= 0";
    if !wave.x.is_finite() {
        return invalid("x", "finite");
    }
    if !wave.y.is_finite() {
        return invalid("y", "finite");
    }
    if !wave.radius.is_finite() || wave.radius < 0.0 {
        return invalid("radius", finite_non_negative);
    }
    if !wave.width.is_finite() || wave.width <= 0.0 {
        return invalid("width", "finite and > 0");
    }
    if !wave.strength.is_finite() {
        return invalid("strength", "finite");
    }
    let channels = [
        ("light.r", wave.light.r),
        ("light.g", wave.light.g),
        ("light.b", wave.light.b),
    ];
    for (field, value) in channels {
        if !value.is_finite() || value < 0.0 {
            return invalid(field, finite_non_negative);
        }
    }
    if !wave.light_intensity.is_finite() || wave.light_intensity < 0.0 {
        return invalid("light_intensity", finite_non_negative);
    }
    Ok(())
}

/// The shockwaves of the next frame, fixed storage, no allocation.
#[derive(Clone, Debug)]
pub struct ShockwaveList {
    waves: [Shockwave; MAX_SHOCKWAVES],
    count: usize,
}

impl Default for ShockwaveList {
    fn default() -> Self {
        Self {
            waves: [Shockwave::default(); MAX_SHOCKWAVES],
            count: 0,
        }
    }
}

impl ShockwaveList {
    /// Replaces the list. All or nothing: on error the old list stays. An empty slice clears the list.
    pub fn set(&mut self, waves: &[Shockwave]) -> Result<(), ShockwaveError> {
        if waves.len() > MAX_SHOCKWAVES {
            return Err(ShockwaveError::TooMany { count: waves.len() });
        }
        for (index, wave) in waves.iter().enumerate() {
            validate_shockwave(wave, index)?;
        }
        self.waves[..waves.len()].copy_from_slice(waves);
        self.count = waves.len();
        Ok(())
    }

    /// Removes all waves.
    pub fn clear(&mut self) {
        self.count = 0;
    }

    /// Number of waves in the list.
    pub fn count(&self) -> usize {
        self.count
    }

    /// The waves, in list order.
    pub fn waves(&self) -> &[Shockwave] {
        &self.waves[..self.count]
    }
}

/// A wave of the output pass: centre, radius and `2 / width`, then the strength (layout of the shader).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DistortionWave {
    /// Centre x, y (world), radius, `2 / width`.
    pub ring: [f32; 4],
    /// x strength (world units), rest 0.
    pub strength: [f32; 4],
}

/// A ring light of the light buffer pass (layout of the shader).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RingLight {
    /// `x0, y0, x1, y1` in scene pixels.
    pub scene_rect: [f32; 4],
    /// Centre x, y (world), radius, `2 / width`.
    pub ring: [f32; 4],
    /// `rgb` light times light intensity, `a` 0.
    pub color: [f32; 4],
}

/// The waves of one frame that have an effect in view.
#[derive(Clone, Debug)]
pub struct ShockwaveFrame {
    /// Distorting waves; the first `distortion_count` are valid.
    pub distortion: [DistortionWave; MAX_SHOCKWAVES],
    /// Number of valid entries in `distortion`.
    pub distortion_count: usize,
    /// Ring lights; the first `ring_count` are valid.
    pub rings: [RingLight; MAX_SHOCKWAVES],
    /// Number of valid entries in `rings`.
    pub ring_count: usize,
}

impl Default for ShockwaveFrame {
    fn default() -> Self {
        Self {
            distortion: [DistortionWave::default(); MAX_SHOCKWAVES],
            distortion_count: 0,
            rings: [RingLight::default(); MAX_SHOCKWAVES],
            ring_count: 0,
        }
    }
}

/// Culls the list for a scene of `width` x `height` under `projection`: a wave distorts only with
/// `strength != 0` and its outer ground circle in view; it lights only with a positive light and its torus
/// in view. Keeps the list order.
pub fn collect_shockwaves(
    list: &ShockwaveList,
    p: &Projection,
    width: u32,
    height: u32,
    out: &mut ShockwaveFrame,
) {
    out.distortion_count = 0;
    out.ring_count = 0;
    for w in list.waves() {
        let half = 0.5 * w.width;
        let outer = w.radius + half;
        let ring = [w.x, w.y, w.radius, 2.0 / w.width];
        let ground = scene_bounds(p, w.x - outer, w.y - outer, w.x + outer, w.y + outer, 0.0);
        if w.strength != 0.0 && overlaps(&ground, width, height) {
            let index = out.distortion_count;
            out.distortion_count += 1;
            out.distortion[index] = DistortionWave {
                ring,
                strength: [w.strength, 0.0, 0.0, 0.0],
            };
        }
        let r = w.light.r * w.light_intensity;
        let g = w.light.g * w.light_intensity;
        let b = w.light.b * w.light_intensity;
        if r > 0.0 || g > 0.0 || b > 0.0 {
            let mut rect =
                scene_bounds(p, w.x - outer, w.y - outer, w.x + outer, w.y + outer, half);
            rect[0] -= 1.0;
            rect[1] -= 1.0;
            rect[2] += 1.0;
            rect[3] += 1.0;
            if overlaps(&rect, width, height) {
                let index = out.ring_count;
                out.ring_count += 1;
                out.rings[index] = RingLight {
                    scene_rect: rect,
                    ring,
                    color: [r, g, b, 0.0],
                };
            }
        }
    }
}

/// Scene rectangle `(x0, y0, x1, y1)` of the world box `[x0, x1] x [y0, y1] x [0, top]` under `p`.
fn scene_bounds(p: &Projection, x0: f32, y0: f32, x1: f32, y1: f32, top: f32) -> [f32; 4] {
    let mut r = [f32::MAX, f32::MAX, -f32::MAX, -f32::MAX];
    for i in 0..8u32 {
        let wx = if (i & 1) != 0 { x1 } else { x0 };
        let wy = if (i & 2) != 0 { y1 } else { y0 };
        let wz = if (i & 4) != 0 { top } else { 0.0 };
        let sx = p.origin.x + wx * p.x_axis.x + wy * p.y_axis.x + wz * p.z_axis.x;
        let sy = p.origin.y + wx * p.x_axis.y + wy * p.y_axis.y + wz * p.z_axis.y;
        r[0] = det::min(r[0], sx);
        r[1] = det::min(r[1], sy);
        r[2] = det::max(r[2], sx);
        r[3] = det::max(r[3], sy);
    }
    r
}

/// Whether a scene rectangle touches the scene of `width` x `height`.
fn overlaps(r: &[f32; 4], width: u32, height: u32) -> bool {
    r[2] >= 0.0 && r[3] >= 0.0 && r[0] <= width as f32 && r[1] <= height as f32
}

/// Rows of the inverse of `[x_axis y_axis]`: scene offset from the origin to the ground point.
pub fn ground_inverse(p: &Projection) -> [f32; 4] {
    let (x0, x1) = (f64::from(p.x_axis.x), f64::from(p.x_axis.y));
    let (y0, y1) = (f64::from(p.y_axis.x), f64::from(p.y_axis.y));
    let denom = x0 * y1 - x1 * y0;
    [
        (y1 / denom) as f32,
        (-y0 / denom) as f32,
        (-x1 / denom) as f32,
        (x0 / denom) as f32,
    ]
}

/// Scene pixel offset (to subtract from the scene position `(qx, qy)`) of all `waves` at that position,
/// with the ground point found by inverting the projection at height 0.
pub fn shockwave_scene_offset(waves: &[DistortionWave], p: &Projection, qx: f32, qy: f32) -> Vec2 {
    let inv = ground_inverse(p);
    let vx = qx - p.origin.x;
    let vy = qy - p.origin.y;
    let gx = inv[0] * vx + inv[1] * vy;
    let gy = inv[2] * vx + inv[3] * vy;
    let mut tx = 0.0f32;
    let mut ty = 0.0f32;
    for w in waves {
        let dx = gx - w.ring[0];
        let dy = gy - w.ring[1];
        let dist = (dx * dx + dy * dy).sqrt();
        let s = (dist - w.ring[2]) * w.ring[3];
        if s.abs() < 1.0 && dist > 0.0 {
            let d = w.strength[0] * shockwave_profile(s) / dist;
            tx += dx * d;
            ty += dy * d;
        }
    }
    Vec2::new(
        tx * p.x_axis.x + ty * p.y_axis.x,
        tx * p.x_axis.y + ty * p.y_axis.y,
    )
}

/// The factor of `light * light_intensity` at the world point `(x, y, z)`.
pub fn ring_light_weight(w: &Shockwave, x: f32, y: f32, z: f32) -> f32 {
    let dx = x - w.x;
    let dy = y - w.y;
    let a = (dx * dx + dy * dy).sqrt() - w.radius;
    shockwave_profile((a * a + z * z).sqrt() * 2.0 / w.width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{approx, near};

    // Port of tests/unit/ShockwaveTests.cpp. Not ported: the `flags` checks and the sizeof check (no flags
    // field), the colour-temperature checks (CHI_Kelvin is not ported) and the C++ wrapper test.

    fn white() -> LinearRgba {
        LinearRgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }
    }

    fn origin(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y)
    }

    /// Largest strength and light intensity of a wave over its life, sampled finely.
    fn peak_of(power: f32, reach: f32) -> (f32, f32) {
        let duration = explosion_law(power, reach).duration;
        let mut strength = 0.0f32;
        let mut light = 0.0f32;
        for i in 0..=2000 {
            let age = duration * i as f32 / 2000.0;
            let d = make_explosion_shockwave(origin(0.0, 0.0), age, power, reach, white());
            strength = det::max(strength, d.strength);
            light = det::max(light, d.light_intensity);
        }
        (strength, light)
    }

    /// Every field of a valid wave set to a bad value must be rejected with its name.
    fn rejects(edit: impl FnOnce(&mut Shockwave), field: &str) {
        let mut w = Shockwave {
            light: white(),
            ..Shockwave::default()
        };
        edit(&mut w);
        match validate_shockwave(&w, 3) {
            Err(ShockwaveError::Invalid {
                index, field: f, ..
            }) => {
                assert_eq!(index, 3);
                assert_eq!(f, field);
            }
            other => panic!("expected rejection of {field}, got {other:?}"),
        }
    }

    #[test]
    fn shockwave_band_profile_is_1_at_the_ring_and_smooth_to_0_at_the_edges() {
        assert!(approx(shockwave_profile(0.0), 1.0));
        assert!(approx(shockwave_profile(0.5), 0.5));
        assert!(approx(shockwave_profile(-0.5), 0.5));
        assert!(near(shockwave_profile(0.999), 0.0, 1e-4));
        assert_eq!(shockwave_profile(1.0), 0.0);
        assert_eq!(shockwave_profile(-1.5), 0.0);
        assert_eq!(shockwave_profile(f32::NAN), 0.0);
    }

    #[test]
    fn default_wave_is_valid_without_effect() {
        let d = Shockwave::default();
        assert_eq!(d.radius, 0.0);
        assert_eq!(d.width, 1.0);
        assert_eq!(d.strength, 0.0);
        assert_eq!(d.light_intensity, 0.0);
        assert!(validate_shockwave(&d, 0).is_ok());
    }

    #[test]
    fn shockwave_radius_grows_and_slows_the_band_thickens_strength_and_light_fade_in_and_out() {
        let mut d = make_shockwave(origin(5.0, 6.0), 0.0, 2.0, 100.0, 10.0, 2.0, white());
        assert_eq!(d.x, 5.0);
        assert_eq!(d.y, 6.0);
        assert_eq!(d.radius, 0.0);
        assert!(approx(d.width, 20.0));
        assert_eq!(d.strength, 0.0); // fade in from 0
        assert_eq!(d.light_intensity, 0.0);

        // Half way: t = 0.5, radius = 100 (1 - 0.125), width 100 (0.2 + 0.1), fade 0.5^1.5.
        d = make_shockwave(origin(5.0, 6.0), 1.0, 2.0, 100.0, 10.0, 2.0, white());
        assert!(near(d.radius, 87.5, 1e-4));
        assert!(near(d.width, 30.0, 1e-4));
        assert!(near(d.strength, 3.5355, 1e-4));
        assert!(near(
            d.light_intensity,
            core::f32::consts::FRAC_1_SQRT_2,
            1e-4
        ));
        assert!(validate_shockwave(&d, 0).is_ok());

        let mut last = -1.0f32;
        let mut last_step = 1e9f32;
        for i in 1..100 {
            let age = 2.0 * i as f32 / 100.0;
            d = make_shockwave(origin(0.0, 0.0), age, 2.0, 100.0, 1000.0, 2.0, white());
            assert!(d.radius > last, "age {age}");
            assert!(
                d.radius - last <= last_step + 1e-3,
                "slowing down at age {age}"
            ); // slowing down
            last_step = d.radius - last;
            last = d.radius;
            // A huge strength is held below the fold of the band.
            assert!(d.strength.abs() <= SHOCKWAVE_FOLD_LIMIT * d.width * 1.0001);
            assert!(d.strength.abs() < d.width / core::f32::consts::PI);
        }
        d = make_shockwave(origin(0.0, 0.0), 0.5, 2.0, 100.0, -1000.0, 0.0, white());
        assert!(d.strength < 0.0); // inward stays inward

        // Outside the life: no effect, still valid.
        for age in [-0.1f32, 2.0, 5.0, f32::NAN] {
            d = make_shockwave(origin(0.0, 0.0), age, 2.0, 100.0, 10.0, 2.0, white());
            assert_eq!(d.strength, 0.0, "age {age}");
            assert_eq!(d.light_intensity, 0.0, "age {age}");
            assert!(validate_shockwave(&d, 0).is_ok());
        }
        d = make_shockwave(origin(0.0, 0.0), 0.5, 0.0, 100.0, 10.0, 2.0, white()); // no duration
        assert_eq!(d.strength, 0.0);
        assert!(validate_shockwave(&d, 0).is_ok());
        d = make_shockwave(origin(0.0, 0.0), 0.5, 1.0, 0.0, 10.0, 2.0, white()); // no radius: valid
        assert!(validate_shockwave(&d, 0).is_ok());
    }

    #[test]
    fn explosion_follows_the_documented_size_law() {
        let one = explosion_law(1.0, 10.0);
        assert!(approx(one.max_radius, 10.0));
        assert!(approx(one.duration, 0.6));
        assert!(approx(one.strength, 1.0));
        assert!(approx(one.light_intensity, 4.0));
        assert!(approx(explosion_saturation(1.0), 1.0));
        assert!(near(
            explosion_saturation(100.0),
            1.0 / (1.0 - det::exp(-1.0)),
            0.02
        ));

        // Syndicate's sizes: small 0.4, medium 1, large 1.8.
        let small = explosion_law(0.4, 10.0);
        let large = explosion_law(1.8, 10.0);
        assert!(approx(small.max_radius, 4.0));
        assert!(approx(large.max_radius, 18.0));
        assert!(near(small.duration, 0.379, 0.01));
        assert!(near(large.duration, 0.805, 0.01));
        // The radius grows faster than the duration.
        assert!(large.max_radius / small.max_radius > large.duration / small.duration);
        // Large blasts are hotter: a higher colour temperature than small ones.
        assert!(large.kelvin > small.kelvin);
    }

    #[test]
    fn explosion_grows_with_power_and_is_clamped() {
        let mut last = [0.0f32; 4];
        for i in 1..=40 {
            let p = 0.1 * i as f32;
            let e = explosion_law(p, 10.0);
            let (peak_strength, peak_light) = peak_of(p, 10.0);
            assert!(e.max_radius > last[0], "power {p}");
            assert!(e.duration > last[1], "power {p}");
            assert!(peak_strength > last[2], "power {p}");
            assert!(peak_light > last[3], "power {p}");
            last = [e.max_radius, e.duration, peak_strength, peak_light];
        }
        // Saturation: power 4 bends and shines at most 1.6 times as strongly as power 1.
        assert!(explosion_law(4.0, 10.0).strength / explosion_law(1.0, 10.0).strength < 1.6);
        assert!(
            explosion_law(4.0, 10.0).light_intensity / explosion_law(1.0, 10.0).light_intensity
                < 1.6
        );
        // Clamped at 4.
        let four = explosion_law(4.0, 10.0);
        let huge = explosion_law(50.0, 10.0);
        assert_eq!(huge, four);

        // The wave of an explosion is the wave of make_shockwave with the law's numbers.
        let a = make_explosion_shockwave(origin(1.0, 2.0), 0.3, 1.8, 10.0, white());
        let e = explosion_law(1.8, 10.0);
        let b = make_shockwave(
            origin(1.0, 2.0),
            0.3,
            e.duration,
            e.max_radius,
            e.strength,
            e.light_intensity,
            white(),
        );
        assert_eq!(a.radius, b.radius);
        assert_eq!(a.width, b.width);
        assert_eq!(a.strength, b.strength);
        assert_eq!(a.light_intensity, b.light_intensity);

        // No power, no effect; still valid.
        for p in [0.0f32, -1.0, f32::NAN] {
            let a = make_explosion_shockwave(origin(1.0, 2.0), 0.1, p, 10.0, white());
            assert_eq!(a.strength, 0.0, "power {p}");
            assert_eq!(a.light_intensity, 0.0, "power {p}");
            assert!(validate_shockwave(&a, 0).is_ok());
        }
    }

    #[test]
    fn validation_names_the_element_and_the_field() {
        rejects(|w| w.x = f32::NAN, "x");
        rejects(|w| w.y = f32::INFINITY, "y");
        rejects(|w| w.radius = -1.0, "radius");
        rejects(|w| w.width = 0.0, "width");
        rejects(|w| w.strength = f32::NAN, "strength");
        rejects(|w| w.light.g = -0.5, "light.g");
        rejects(|w| w.light_intensity = -1.0, "light_intensity");
    }

    #[test]
    fn wave_list_takes_at_most_sixteen_all_or_nothing() {
        let mut list = ShockwaveList::default();
        let mut waves = [Shockwave::default(); MAX_SHOCKWAVES + 1];
        for w in waves.iter_mut() {
            *w = make_shockwave(origin(0.0, 0.0), 0.2, 1.0, 10.0, 1.0, 1.0, white());
        }
        assert!(list.set(&waves[..MAX_SHOCKWAVES]).is_ok());
        assert_eq!(list.count(), MAX_SHOCKWAVES);
        assert_eq!(
            list.set(&waves),
            Err(ShockwaveError::TooMany {
                count: MAX_SHOCKWAVES + 1
            })
        );
        assert_eq!(list.count(), MAX_SHOCKWAVES); // unchanged
        waves[2].width = -1.0;
        assert_eq!(
            list.set(&waves[..3]),
            Err(ShockwaveError::Invalid {
                index: 2,
                field: "width",
                rule: "finite and > 0"
            })
        );
        assert_eq!(list.count(), MAX_SHOCKWAVES);
        assert!(list.set(&[]).is_ok());
        assert_eq!(list.count(), 0);
    }

    #[test]
    fn culling_keeps_only_waves_with_an_effect_in_view() {
        let p = Projection {
            origin: origin(0.0, 0.0),
            x_axis: origin(1.0, 0.0),
            y_axis: origin(0.0, 1.0),
            z_axis: origin(0.0, 0.0),
        };
        let base = Shockwave {
            x: 50.0,
            y: 50.0,
            radius: 10.0,
            width: 4.0,
            strength: 1.0,
            light: white(),
            light_intensity: 0.0,
        };
        let no_effect = Shockwave::default();
        let light_only = Shockwave {
            strength: 0.0,
            light_intensity: 2.0,
            ..base
        };
        let out_of_view = Shockwave {
            x: 500.0,
            light_intensity: 1.0,
            ..base
        };
        let mut list = ShockwaveList::default();
        list.set(&[no_effect, base, light_only, out_of_view])
            .unwrap();
        let mut f = ShockwaveFrame::default();
        collect_shockwaves(&list, &p, 100, 100, &mut f);
        assert_eq!(f.distortion_count, 1);
        assert_eq!(f.distortion[0].ring[2], 10.0);
        assert!(approx(f.distortion[0].ring[3], 0.5));
        assert_eq!(f.ring_count, 1);
        assert!(approx(f.rings[0].scene_rect[0], 37.0)); // 50 - 12 - 1 pixel
        assert!(approx(f.rings[0].color[0], 2.0));
    }

    #[test]
    fn mirrors_offset_along_the_world_direction_and_ring_light_is_a_torus() {
        // Isometric-like projection: the offset follows the projected world direction.
        let p = Projection {
            origin: origin(100.0, 50.0),
            x_axis: origin(2.0, 1.0),
            y_axis: origin(-2.0, 1.0),
            z_axis: origin(0.0, -2.0),
        };
        let g = DistortionWave {
            ring: [0.0, 0.0, 10.0, 0.5],
            strength: [3.0, 0.0, 0.0, 0.0],
        };
        // The scene point of the world point (10, 0) on the ring.
        let qx = 100.0 + 10.0 * 2.0;
        let qy = 50.0 + 10.0 * 1.0;
        let o = shockwave_scene_offset(&[g], &p, qx, qy);
        assert!(approx(o.x, 6.0)); // 3 world units along +x -> (6, 3) scene pixels
        assert!(approx(o.y, 3.0));
        let inside = shockwave_scene_offset(&[g], &p, 100.0 + 7.0 * 2.0, 50.0 + 7.0);
        assert_eq!(inside.x, 0.0);
        assert_eq!(inside.y, 0.0);
        let half = shockwave_scene_offset(&[g], &p, 100.0 + 11.0 * 2.0, 50.0 + 11.0);
        assert!(approx(half.x, 3.0)); // s = 0.5: half the strength
        // Two waves add.
        assert!(approx(shockwave_scene_offset(&[g, g], &p, qx, qy).x, 12.0));

        let w = Shockwave {
            radius: 10.0,
            width: 4.0,
            ..Shockwave::default()
        };
        assert!(approx(ring_light_weight(&w, 10.0, 0.0, 0.0), 1.0));
        assert!(approx(ring_light_weight(&w, 0.0, 11.0, 0.0), 0.5));
        assert!(approx(ring_light_weight(&w, 10.0, 0.0, 1.0), 0.5)); // up a wall at the ring
        assert_eq!(ring_light_weight(&w, 7.0, 0.0, 0.0), 0.0);
        assert_eq!(ring_light_weight(&w, 13.0, 0.0, 0.0), 0.0);
    }
}
