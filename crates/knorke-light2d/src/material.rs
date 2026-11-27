// Origin: chiaro @ 9bc5b9b, src/lighting/MaterialMath.hpp and MaterialMath.cpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! CPU model of the material shader math (P6): normal tangent frame, material bits, Toksvig
//! exponent and the normalized specular lobes. Test oracle for the shaders and per-frame helper
//! for the lighting stage.
//!
//! Redesign against the C++ version: `Vec3` is the `knorke-math` type, and the transcendental
//! functions (`exp`, `exp2`, `pow`) are the deterministic `det::*` ones, so the model gives the
//! same bits on every platform.

use crate::Projection;
use crate::light_math::smoothstep;
use knorke_math::{Vec3, det};

/// An orthonormal frame: tangent (maps to screen-right), bitangent (maps to screen-up), normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TangentFrame {
    /// Tangent.
    pub t: Vec3,
    /// Bitangent.
    pub b: Vec3,
    /// Normal.
    pub n: Vec3,
}

/// The decoded material B byte.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialBits {
    /// Specular strength `0..=1` (3 bits).
    pub strength: f32,
    /// Metal bit.
    pub metal: bool,
    /// Blinn-Phong exponent `2^(1 + gloss * 7/15)` (4 bits).
    pub exponent: f32,
}

fn normalize(a: Vec3) -> Vec3 {
    let len = a.length();
    if len > 0.0 { a * (1.0 / len) } else { a }
}

/// Unit direction from the scene towards the viewer in world space (`z >= 0`).
pub fn view_direction(p: &Projection) -> Vec3 {
    let row_x = Vec3::new(p.x_axis.x, p.y_axis.x, p.z_axis.x);
    let row_y = Vec3::new(p.x_axis.y, p.y_axis.y, p.z_axis.y);
    let v = normalize(row_x.cross(row_y));
    if v.z < 0.0 { v * -1.0 } else { v }
}

/// Unit world-space direction (`z = 0`) that maps to screen-right.
pub fn screen_right(p: &Projection) -> Vec3 {
    let det = p.x_axis.x * p.y_axis.y - p.x_axis.y * p.y_axis.x;
    let sign = if det < 0.0 { -1.0 } else { 1.0 };
    normalize(Vec3::new(p.y_axis.y, -p.x_axis.y, 0.0)) * sign
}

/// Tangent frame of a base surface normal; `t` maps to screen-right, `b` to screen-up.
pub fn tangent_frame(base_normal: Vec3, right: Vec3, p: &Projection) -> TangentFrame {
    let n = base_normal;
    let proj = right - n * right.dot(n);
    let t = if proj.length() < 1e-4 {
        let t = normalize(Vec3::new(0.0, 0.0, 1.0).cross(n));
        if p.image_of(t).x < 0.0 { t * -1.0 } else { t }
    } else {
        normalize(proj)
    };
    let mut b = n.cross(t);
    if p.image_of(b).y > 0.0 {
        b *= -1.0;
    }
    TangentFrame { t, b, n }
}

/// Detail normal (tangent space: x = T, y = B, z = N) to world space.
pub fn detail_to_world(f: &TangentFrame, d: Vec3) -> Vec3 {
    Vec3::new(
        f.t.x * d.x + f.b.x * d.y + f.n.x * d.z,
        f.t.y * d.x + f.b.y * d.y + f.n.y * d.z,
        f.t.z * d.x + f.b.z * d.y + f.n.z * d.z,
    )
}

/// Decodes the packed material byte: 3 bits strength, 1 bit metal, 4 bits exponent.
pub fn decode_material_bits(b: u8) -> MaterialBits {
    MaterialBits {
        strength: f32::from(b >> 5) / 7.0,
        metal: (b >> 4) & 1 != 0,
        exponent: det::exp2(1.0 + f32::from(b & 15) * 7.0 / 15.0),
    }
}

/// Specular exponent reduced by the average normal length of a mip texel.
pub fn toksvig_exponent(e: f32, avg_normal_length: f32) -> f32 {
    let len = avg_normal_length;
    let ft = len / (len + e * (1.0 - len));
    det::max(1.0, ft * e)
}

/// Energy normalization of the Blinn-Phong lobe (E49): `(e + 2) / (8 (1 - 2^(-(e + 2) / 2)))`,
/// chosen so that the lobe `N(e) (n.h)^e` seen straight along the normal sums to pi over the
/// hemisphere of light directions for every `e`, the same sum as the Lambert term. So
/// `Material::specular` is the highlight's share of the light; gloss only changes the shape.
/// The wet lobe divides by `sqrt(stretch)`, its area grows by that factor (exact where the
/// streak fits in the hemisphere).
pub fn specular_normalization(e: f32) -> f32 {
    (e + 2.0) / (8.0 * (1.0 - det::exp2(-0.5 * (e + 2.0))))
}

/// Normalized Blinn-Phong term `N(e) (n.h)^e` (without light colour and N.L diffuse factor).
pub fn specular_term(n: Vec3, l: Vec3, v: Vec3, e: f32) -> f32 {
    if n.dot(l) <= 0.0 {
        return 0.0;
    }
    let h = normalize(l + v);
    specular_normalization(e) * det::pow(det::max(0.0, n.dot(h)), e)
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

fn wet_hash(px: f32, py: f32) -> f32 {
    let mut q = [fract(px * 0.1031), fract(py * 0.1031), fract(px * 0.1031)];
    let d = q[0] * (q[1] + 33.33) + q[1] * (q[2] + 33.33) + q[2] * (q[0] + 33.33);
    for v in &mut q {
        *v += d;
    }
    fract((q[0] + q[1]) * q[2])
}

/// Wet ground value noise in `[0, 1]` (`WetNoise` in `shaders/src/surface.hlsli`).
pub fn wet_noise(x: f32, y: f32) -> f32 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (a, b) = (wet_hash(ix, iy), wet_hash(ix + 1.0, iy));
    let (c, d) = (wet_hash(ix, iy + 1.0), wet_hash(ix + 1.0, iy + 1.0));
    let (lo, hi) = (a + (b - a) * ux, c + (d - c) * ux);
    lo + (hi - lo) * uy
}

/// [`specular_term`] with the wet ground lobe (`WetSpecularFactor` in `surface.hlsli`): the
/// tangent of `h` across `right` on the ground is divided by `stretch`, the one along `right`
/// multiplied by `sqrt(stretch)`, and with `breakup > 0` two octaves of value noise at
/// `(ground . right, ground . across) * inv_scale`, cells long along `right`, scale it by
/// `lerp(1, 2 smoothstep(0.4, 0.65, noise), breakup)`.
#[allow(clippy::too_many_arguments)]
pub fn wet_specular_term(
    n: Vec3,
    l: Vec3,
    v: Vec3,
    e: f32,
    right: Vec3,
    ground_x: f32,
    ground_y: f32,
    stretch: f32,
    breakup: f32,
    inv_scale: f32,
) -> f32 {
    if n.dot(l) <= 0.0 {
        return 0.0;
    }
    let mut h = l + v;
    let hl = h.length();
    if hl <= 0.0 {
        return 0.0;
    }
    h *= 1.0 / hl;
    let hn = n.dot(h);
    if hn <= 0.0 {
        return 0.0;
    }
    let across = Vec3::new(-right.y, right.x, 0.0);
    let x = h.dot(right) / hn * stretch.sqrt();
    let y = h.dot(across) / hn / stretch;
    let mut f = specular_normalization(e) / stretch.sqrt() * det::exp(-0.5 * e * (x * x + y * y));
    if breakup > 0.0 {
        let qx = (ground_x * right.x + ground_y * right.y) * inv_scale;
        let qy = (ground_x * across.x + ground_y * across.y) * inv_scale;
        let noise = 0.6 * wet_noise(qx * 0.2, qy) + 0.4 * wet_noise(qx * 0.5 + 17.0, qy * 2.3);
        f *= 1.0 + (2.0 * smoothstep(0.4, 0.65, noise) - 1.0) * breakup;
    }
    f
}
