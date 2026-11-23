//! Light model: sun, ambient, point lights, shadow casters. Plain data plus the small formulas
//! the light pass shares (falloff, flicker, bias).
//!
//! Origin: pixelraum `src/light.rs` @ 81927d8 (same author, relicensed MIT -> MIT OR Apache-2.0,
//! ADR 0013).

use crate::math::Vec3;
use crate::shadow::ShadowMap;

/// Maximum number of point lights used in one frame (the nearest visible ones).
pub const MAX_LIGHTS: usize = 16;

/// Parallel light (sun or moon).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// Direction the light travels (need not be normalized).
    pub dir: Vec3,
    /// Colour as `0xRRGGBB`.
    pub color: u32,
    /// Strength; 0 or less = no sun.
    pub strength: f32,
    /// Cast shadows (sun shadow map).
    pub shadows: bool,
}

/// A point light (torch, lantern) with quadratic falloff to zero at `radius`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointLight {
    /// World position.
    pub pos: Vec3,
    /// Colour as `0xRRGGBB`.
    pub color: u32,
    /// Strength before flicker.
    pub strength: f32,
    /// Reach in world units; 0 or less = no light.
    pub radius: f32,
    /// Flicker amount 0..1.
    pub flicker: f32,
    /// Seed of the flicker noise.
    pub seed: u32,
    /// Cast shadows (six cube shadow maps).
    pub shadows: bool,
}

/// A world-space triangle that blocks light (recorded before view transform and clipping).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caster {
    /// Corners in world space.
    pub tri: [Vec3; 3],
    /// Texture coordinates of the corners.
    pub uv: [[f32; 2]; 3],
    /// Texture id, or [`SOLID_CASTER`] for an untextured opaque blocker.
    pub texture: usize,
    /// Repeat the texture outside `[0, 1)` instead of clamping.
    pub wrap: bool,
    /// Draw alpha of a see-through caster; 1 = opaque. Below 1 the shadow is stippled.
    pub alpha: f32,
    /// Only the sun map draws it (roofs and ceiling caps that close a volume for the sun).
    pub sun_only: bool,
}

/// [`Caster::texture`] of untextured, fully opaque shadow-only blockers.
pub const SOLID_CASTER: usize = usize::MAX;

/// An upright quad that is turned around the vertical axis towards each light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BillboardCaster {
    /// Bottom centre in world space.
    pub base: Vec3,
    /// Width in world units.
    pub width: f32,
    /// Height in world units.
    pub height: f32,
    /// Texture id.
    pub texture: usize,
    /// Mirror the texture horizontally.
    pub flip: bool,
    /// Texture scroll offset (`None` = no scroll, clamped sampling as on screen).
    pub scroll: Option<[f32; 2]>,
    /// Draw alpha (see [`Caster::alpha`]).
    pub alpha: f32,
}

/// Shadow maps kept between frames, rebuilt only when `key` changes (see `KeyHash`).
#[derive(Clone, Debug)]
pub(crate) struct ShadowCache<M> {
    pub(crate) key: u64,
    /// Changes on every rebuild (`LightState::map_gen`); the light pass cache keys on it.
    pub(crate) generation: u64,
    pub(crate) maps: M,
}

/// A persistent point light with its cached cube shadow maps.
#[derive(Clone, Debug)]
pub(crate) struct LightSlot {
    pub(crate) light: PointLight,
    pub(crate) shadow: Option<ShadowCache<[ShadowMap; 6]>>,
}

/// Deterministic 64-bit hash over 32-bit words (FNV-1a style with a final mix); no RandomState.
pub(crate) struct KeyHash(u64);

impl KeyHash {
    pub(crate) fn new() -> KeyHash {
        KeyHash(0xcbf2_9ce4_8422_2325)
    }
    pub(crate) fn u32(&mut self, v: u32) {
        self.0 = (self.0 ^ v as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    pub(crate) fn u64(&mut self, v: u64) {
        self.u32(v as u32);
        self.u32((v >> 32) as u32);
    }
    pub(crate) fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    pub(crate) fn vec3(&mut self, v: Vec3) {
        self.f32(v.x);
        self.f32(v.y);
        self.f32(v.z);
    }
    /// `opaque`: the caster's texture has no transparent texel (or it is untextured). Then its
    /// texture coordinates cannot change the shadow and are left out, so a scrolling opaque texture
    /// does not rebuild the maps every frame. Stippled casters (alpha < 1) keep them (texel alpha).
    pub(crate) fn caster(&mut self, c: &Caster, opaque: bool) {
        let uv = !(opaque && c.alpha >= 1.0);
        for k in 0..3 {
            self.vec3(c.tri[k]);
            if uv {
                self.f32(c.uv[k][0]);
                self.f32(c.uv[k][1]);
            }
        }
        self.u64(c.texture as u64);
        self.u32(c.wrap as u32);
        self.f32(c.alpha);
        self.u32(c.sun_only as u32);
    }
    /// `opaque` as for `caster`: the scroll offset only counts for textures with transparent texels.
    pub(crate) fn billboard(&mut self, b: &BillboardCaster, opaque: bool) {
        self.vec3(b.base);
        self.f32(b.width);
        self.f32(b.height);
        self.u64(b.texture as u64);
        self.u32(b.flip as u32);
        self.f32(b.alpha);
        if let (Some([du, dv]), false) = (b.scroll, opaque && b.alpha >= 1.0) {
            self.f32(du);
            self.f32(dv);
        }
    }
    pub(crate) fn finish(&self) -> u64 {
        let mut h = self.0;
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^= h >> 33;
        h
    }
}

/// Colour `0xRRGGBB` times strength as 0..1 per channel (may exceed 1 for strength > 1).
pub fn color_scaled(c: u32, strength: f32) -> [f32; 3] {
    let ch = |s: u32| ((c >> s) & 255) as f32 / 255.0 * strength;
    [ch(16), ch(8), ch(0)]
}

/// Shadow bias for a surface: constant part plus a slope part that grows as the light grazes it.
pub(crate) fn slope_bias(bias: f32, n_dot_l: f32) -> f32 {
    bias + 2.0 * bias * (1.0 - n_dot_l.clamp(0.0, 1.0))
}

/// Distance falloff `(1 - (d/r)^2)^2` inside the radius, 0 outside.
pub fn falloff(d: f32, r: f32) -> f32 {
    if !(r > 0.0) || !(d < r) {
        return 0.0;
    }
    let q = d / r;
    let t = 1.0 - q * q;
    t * t
}

/// Integer hash to 0..1.
fn hash01(step: i64, seed: u32) -> f32 {
    let mut h = (step as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (seed as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// Deterministic flicker noise in 0..1: a hashed value per 1/12 s step, blended linearly to the next.
pub fn noise(time: f64, seed: u32) -> f32 {
    if !time.is_finite() {
        return 0.0;
    }
    let s = time * 12.0;
    let step = s.floor();
    let t = (s - step) as f32;
    let a = hash01(step as i64, seed);
    let b = hash01(step as i64 + 1, seed);
    a + (b - a) * t
}

/// Strength of a point light at `time` (seconds), flicker applied.
pub fn flickered_strength(l: &PointLight, time: f64) -> f32 {
    l.strength * (1.0 - l.flicker.clamp(0.0, 1.0) * noise(time, l.seed))
}

/// Alpha in steps of 1/8 (rounded to the nearest step), clamped to 0..1; NaN counts as 0.
/// Same steps as the renderer's blending (pixelraum `blend::step_alpha`), so a stippled shadow
/// covers the share its caster shows on screen.
#[inline]
pub(crate) fn step_alpha(a: f32) -> f32 {
    if a.is_nan() || a <= 0.0 {
        return 0.0;
    }
    (a.min(1.0) * 8.0).round() / 8.0
}

/// 4x4 ordered dither matrix (Bayer), the same one the renderer dithers colours with.
pub(crate) const BAYER4: [[u8; 4]; 4] =
    [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falloff_shape() {
        assert_eq!(falloff(0.0, 2.0), 1.0);
        assert_eq!(falloff(2.0, 2.0), 0.0);
        assert_eq!(falloff(3.0, 2.0), 0.0);
        assert!((falloff(1.0, 2.0) - 0.5625).abs() < 1e-6);
        assert_eq!(falloff(0.5, 0.0), 0.0);
    }

    #[test]
    fn noise_is_deterministic_and_bounded() {
        for k in 0..200 {
            let t = k as f64 * 0.037;
            let n = noise(t, 3);
            assert!((0.0..=1.0).contains(&n));
            assert_eq!(n, noise(t, 3));
        }
        // continuous across a step boundary
        assert!((noise(1.0 / 12.0 - 1e-9, 5) - noise(1.0 / 12.0, 5)).abs() < 1e-4);
        assert_ne!(noise(0.0, 1), noise(0.0, 2));
    }
}
