// Origin: chiaro @ 9bc5b9b, src/lighting/LightMask.hpp and LightMask.cpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Placement of a light mask (P10): the CPU form of `light_mask.hlsli` and the oracle of the
//! tests.
//!
//! Redesign against the C++ version: vectors are [`Vec3`], "no placement" is `None` instead of a
//! `false` return with an out parameter, the angle "auto" sentinel is `Option` (see
//! [`LightDesc::mask_angle`]), and `tan` is [`det::tan`], so results are identical on every
//! platform.

use crate::gpu_light::GpuMask;
use crate::light::{LightDesc, LightType, MaskMode, StoredLight};
use crate::mask::sample_mask_texels;
use knorke_math::{Vec3, det};

use crate::light_math::DEG_TO_RAD;
/// Projector: a point light's default half angle.
const POINT_MASK_ANGLE: f32 = 45.0;

/// A mask placed for one light in one frame: for `w = pixel - light`,
/// `u = (w . p) / (w . n)`, `v = (w . q) / (w . n)`. Projector: `n` is the unit direction, the
/// pixel must lie in front (`w . n > 0`). Window: `n` is the window's normal away from the light
/// divided by the light's distance to the window plane, the pixel must lie beyond the plane
/// (`w . n >= 1`), and the ray reaches the window at the share `t = 1 / (w . n)` of the way from
/// the light.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MaskPlacement {
    /// Plane vector.
    pub n: Vec3,
    /// Vector of the `u` coordinate.
    pub p: Vec3,
    /// Vector of the `v` coordinate.
    pub q: Vec3,
    /// A window (`true`) or a projector.
    pub window: bool,
}

/// The mask coordinates of one pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaskCoords {
    /// Horizontal mask coordinate in `[-1, 1]`.
    pub u: f32,
    /// Vertical mask coordinate in `[-1, 1]`.
    pub v: f32,
    /// Window: the share of the way from the light at which the ray passes the window; 0 for a
    /// projector.
    pub t: f32,
}

/// `true` if a window's half axes span a plane: `|u x v|^2 > 1e-6 |u|^2 |v|^2` (sin of the angle
/// between them above 0.001). The one criterion of the light's validation and of
/// [`make_mask_placement`].
pub fn window_axes_span_plane(u: Vec3, v: Vec3) -> bool {
    let c = u.cross(v);
    c.length_squared() > 1e-6 * u.length_squared() * v.length_squared()
}

/// The projector's half angle in degrees: `mask_angle`, or with `None` the outer angle (spot) or
/// 45.
pub fn mask_angle_of(d: &LightDesc) -> f32 {
    d.mask_angle
        .unwrap_or(if d.kind == LightType::Spot && d.outer_angle > 0.0 {
            d.outer_angle
        } else {
            POINT_MASK_ANGLE
        })
}

/// The placement of a masked light at height `z` (the resolved light height). `stored.direction`
/// and `stored.mask_reference` are the ones of this frame (a beacon turns them). `None` if the
/// light lies in its window's plane (then it lights nothing) or the window axes do not span a
/// plane.
pub fn make_mask_placement(stored: &StoredLight, z: f32) -> Option<MaskPlacement> {
    let d = &stored.desc;
    if d.mask_mode == MaskMode::Projector {
        let k = 1.0 / det::tan(mask_angle_of(d) * DEG_TO_RAD);
        let c90 = stored.mask_reference.cross(stored.direction);
        return Some(MaskPlacement {
            n: stored.direction,
            p: stored.mask_reference * k,
            q: c90 * k,
            window: false,
        });
    }
    // Window: the dual basis of (U, V) in the plane, so (I - C) . pu = u and (I - C) . pv = v for
    // a point I of it.
    let (u, v) = (d.mask_u, d.mask_v);
    if !window_axes_span_plane(u, v) {
        return None; // rejected by the light's validation; kept for callers that skip it
    }
    let mut normal = u.cross(v);
    normal = normal / normal.length();
    let vn = v.cross(normal);
    let nu = normal.cross(u);
    let (su, sv) = (u.dot(vn), v.dot(nu));
    let (pu, pv) = (vn / su, nu / sv);
    let pos = d.position.raw();
    let light = Vec3::new(pos.x, pos.y, z);
    let to_center = d.mask_center - light;
    let mut dist = to_center.dot(normal);
    if dist < 0.0 {
        normal = -normal;
        dist = -dist;
    }
    let size = u.length() + v.length();
    if !(dist > 1e-4 * size) {
        return None; // the light lies in the window's plane
    }
    let (cu, cv) = (to_center.dot(pu), to_center.dot(pv));
    let n = normal / dist;
    Some(MaskPlacement {
        n,
        p: pu - n * cu,
        q: pv - n * cv,
        window: true,
    })
}

/// The mask coordinates of the pixel at `to_pixel = pixel - light` and, for a window, the share
/// `t` of the way from the light at which the ray passes the window (0 for a projector). `None`
/// if the pixel gets no light through the mask: behind the projector, on the light's side of the
/// window, or outside the square.
pub fn mask_coordinates(m: &MaskPlacement, to_pixel: Vec3) -> Option<MaskCoords> {
    let wn = to_pixel.dot(m.n);
    let in_front = if m.window { wn >= 1.0 } else { wn > 0.0 };
    if !in_front {
        return None;
    }
    let u = to_pixel.dot(m.p) / wn;
    let v = to_pixel.dot(m.q) / wn;
    if !(u.abs() <= 1.0 && v.abs() <= 1.0) {
        return None;
    }
    Some(MaskCoords {
        u,
        v,
        t: if m.window { 1.0 / wn } else { 0.0 },
    })
}

/// The colour factor of the mask `texels` toward `to_pixel` (zero where [`mask_coordinates`] is
/// `None`).
pub fn mask_factor(m: &MaskPlacement, texels: &[u8], to_pixel: Vec3) -> Vec3 {
    match mask_coordinates(m, to_pixel) {
        Some(c) => sample_mask_texels(texels, c.u, c.v),
        None => Vec3::ZERO,
    }
}

/// The GPU form for atlas block `block` of `blocks`.
pub fn make_gpu_mask(m: &MaskPlacement, block: u32, blocks: u32) -> GpuMask {
    let scale = if blocks > 0 { 1.0 / blocks as f32 } else { 0.0 };
    GpuMask {
        n: [m.n.x, m.n.y, m.n.z, if m.window { 1.0 } else { 0.0 }],
        p: [m.p.x, m.p.y, m.p.z, block as f32 * scale],
        q: [m.q.x, m.q.y, m.q.z, scale],
    }
}
