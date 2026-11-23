// Origin: chiaro @ 9bc5b9b, src/lighting/LightMath.hpp and LightMath.cpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! CPU reference of the light model (P7). The shaders use the same formulas; the GPU tests
//! compare against these functions.
//!
//! Redesign against the C++ version: vectors are `knorke-math` types, positions are typed
//! ([`World2`], [`Screen`]), and `pow` is `det::pow`, so results are identical on every platform.

use crate::Projection;
use knorke_math::{At, Screen, Vec2, Vec3, World2, det};

/// `saturate(1 - (d/r)^2)^falloff`: 1 at the light, smooth, exactly 0 from `d = r`.
pub fn attenuation(distance: f32, radius: f32, falloff: f32) -> f32 {
    if radius <= 0.0 {
        return 0.0;
    }
    let x = distance / radius;
    let window = (1.0 - x * x).clamp(0.0, 1.0);
    if window <= 0.0 {
        0.0
    } else {
        det::pow(window, falloff)
    }
}

/// Cone factor: 1 inside the inner cone, 0 outside the outer cone, smoothstep between.
pub fn spot_factor(to_pixel: Vec3, direction: Vec3, cos_inner: f32, cos_outer: f32) -> f32 {
    let (lp, ld) = (to_pixel.length(), direction.length());
    if lp <= 1e-6 || ld <= 1e-6 {
        return 1.0;
    }
    let c = to_pixel.dot(direction) / (lp * ld);
    let range = cos_inner - cos_outer;
    if range <= 1e-6 {
        return if c >= cos_inner { 1.0 } else { 0.0 };
    }
    let t = ((c - cos_outer) / range).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Soft near field (`LightDesc::soft_near_field`): the GPU light's direction/type word carries
/// `NEAR_FIELD_SHARE * min(source_radius / radius, 1)` on top of the type.
pub const NEAR_FIELD_SHARE: f32 = 0.49;

/// Lambert term of a sphere light of radius `r`: `max(0, n.v) / sqrt(|v|^2 + r^2)`, `v` from the
/// pixel to the light.
pub fn near_field_lambert(n: Vec3, v: Vec3, r: f32) -> f32 {
    det::max(0.0, n.dot(v)) / (v.dot(v) + r * r).sqrt()
}

/// splitmix64 finalizer over cell and seed; 24 random bits as a float in `[0, 1)`.
fn hash01(cell: i64, seed: u32) -> f32 {
    let mut x = (cell as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((u64::from(seed) << 32) | u64::from(seed));
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// Deterministic 1D value noise in `[0, 1]`; `seed` decorrelates lights.
pub fn flicker_noise(time: f64, speed: f32, seed: u32) -> f32 {
    let x = time * f64::from(speed);
    let cell = x.floor();
    let f = (x - cell) as f32;
    let (a, b) = (hash01(cell as i64, seed), hash01(cell as i64 + 1, seed));
    let u = f * f * (3.0 - 2.0 * f);
    a + (b - a) * u
}

/// `ceil(size * scale)`, at least 1, ignoring float noise in `scale` (100 x 0.3f gives 30, not 31).
pub fn scaled_buffer_size(size: u32, scale: f32) -> u32 {
    let scaled = (f64::from(size) * f64::from(scale) - 1e-3).ceil();
    if scaled < 1.0 { 1 } else { scaled as u32 }
}

/// Radius of the lit area on the ground for a light at `height`: `sqrt(r^2 - z^2)`, 0 if it
/// cannot reach.
pub fn ground_radius(height: f32, radius: f32) -> f32 {
    let squared = radius * radius - height * height;
    if squared > 0.0 { squared.sqrt() } else { 0.0 }
}

/// Scene-space corners of the world square `(x ± r, y ± r)` at ground level: (-,-), (+,-), (-,+), (+,+).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneQuad {
    /// The four corners in scene pixels.
    pub corner: [At<Screen, Vec2>; 4],
}

/// The scene-space quad of a light at `center` with reach `radius`.
pub fn light_quad(p: &Projection, center: At<World2, Vec2>, radius: f32) -> SceneQuad {
    let c = center.raw();
    let at = |dx: f32, dy: f32| p.project(At::new(Vec2::new(c.x + dx, c.y + dy)), 0.0);
    SceneQuad {
        corner: [
            at(-radius, -radius),
            at(radius, -radius),
            at(-radius, radius),
            at(radius, radius),
        ],
    }
}

/// Conservative overlap test of the quad's bounding box with the scene rectangle.
pub fn quad_intersects_scene(quad: &SceneQuad, scene_w: u32, scene_h: u32) -> bool {
    let first = quad.corner[0].raw();
    let (mut min, mut max) = (first, first);
    for c in &quad.corner {
        min = min.min(c.raw());
        max = max.max(c.raw());
    }
    max.x > 0.0 && min.x < scene_w as f32 && max.y > 0.0 && min.y < scene_h as f32
}
