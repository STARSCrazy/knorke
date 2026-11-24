// Origin: chiaro @ 9bc5b9b, src/lighting/AreaLight.hpp and AreaLight.cpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! CPU model of the area and line lights (P8): the segment `center ± u` and the rectangle
//! `center ± u ± v`. The shaders mirror these functions (`LightReachAt`, `AreaLambert`,
//! `AreaSpecularPoint`); the GPU tests compare against them.
//!
//! Redesign against the C++ version: vectors are [`Vec3`], the reach returns its by-products in a
//! struct instead of out parameters, and `min`/`max` are [`det::min`]/[`det::max`], so results are
//! identical on every platform.

use crate::light_math::attenuation;
use knorke_math::{Vec3, det};

/// Below this squared distance a Lambert sample counts as touching the pixel.
const MIN_SAMPLE_DISTANCE_2: f32 = 1e-10;

/// What kind of emitter an [`AreaEmitter`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AreaKind {
    /// The segment `center ± u`.
    Line,
    /// The rectangle `center ± u ± v`, emitting towards `normal` only.
    Rect,
    /// The rectangle, emitting from both sides.
    RectTwoSided,
}

/// An emitter: the segment `center ± u`, or the rectangle `center ± u ± v` with the emitting
/// side `normal`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaEmitter {
    /// Which emitter.
    pub kind: AreaKind,
    /// Centre.
    pub center: Vec3,
    /// Half extent along the length, non-zero.
    pub u: Vec3,
    /// Rect: half extent across, perpendicular to `u` and `normal`; line: zero.
    pub v: Vec3,
    /// Rect: unit normal of the emitting side.
    pub normal: Vec3,
}

impl AreaEmitter {
    fn is_rect(&self) -> bool {
        self.kind != AreaKind::Line
    }
}

/// `v` of a rectangle: `half_width * normalize(normal x u)`; zero if they are parallel.
pub fn rect_across(normal: Vec3, u: Vec3, half_width: f32) -> Vec3 {
    let c = normal.cross(u);
    let l = c.length();
    if l > 0.0 {
        c * (half_width / l)
    } else {
        Vec3::ZERO
    }
}

/// The point of the emitter nearest to a world point, with its emitter coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmitterPoint {
    /// The point.
    pub point: Vec3,
    /// Coordinate along `u` in `[-1, 1]`.
    pub t: f32,
    /// Coordinate along `v` in `[-1, 1]` (0 for a line).
    pub s: f32,
}

/// Coordinate of `p` along the half extent `axis`, clamped to `[-1, 1]`.
fn coordinate(p: Vec3, axis: Vec3) -> f32 {
    let aa = axis.dot(axis);
    if aa > 0.0 {
        (p.dot(axis) / aa).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// The point of the emitter nearest to `world`.
pub fn nearest_emitter_point(e: &AreaEmitter, world: Vec3) -> EmitterPoint {
    let rel = world - e.center;
    let t = coordinate(rel, e.u);
    let s = if e.is_rect() {
        coordinate(rel, e.v)
    } else {
        0.0
    };
    EmitterPoint {
        point: e.center + e.u * t + e.v * s,
        t,
        s,
    }
}

/// Result of [`area_reach`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaReach {
    /// The reach, without the receiver's Lambert term.
    pub reach: f32,
    /// Unit vector from `world` to the nearest point, zero on the emitter.
    pub to_light: Vec3,
    /// The nearest point.
    pub nearest: EmitterPoint,
}

/// Reach of the emitter at `world` without the receiver's Lambert term: the attenuation of the
/// distance to the nearest point times, for a rectangle, the emitter cosine (one-sided
/// `max(0, cos)`, two-sided `|cos|`, 1 on the emitter).
pub fn area_reach(e: &AreaEmitter, radius: f32, falloff: f32, world: Vec3) -> AreaReach {
    let nearest = nearest_emitter_point(e, world);
    let from_emitter = world - nearest.point;
    let d = from_emitter.length();
    let to_light = if d > 1e-6 {
        from_emitter * (-1.0 / d)
    } else {
        Vec3::ZERO
    };
    let mut emit = 1.0;
    if e.is_rect() && d > 1e-6 {
        let c = e.normal.dot(from_emitter) / d;
        emit = if e.kind == AreaKind::Rect {
            det::max(0.0, c)
        } else {
            c.abs()
        };
    }
    AreaReach {
        reach: attenuation(d, radius, falloff) * emit,
        to_light,
        nearest,
    }
}

/// Adds the Lambert term toward sample `q`, weighted by `1 / d^2` (its share of the irradiance),
/// to `sum` and the weight to `weight`. A pixel on the sample counts cos 1 with a weight that
/// outweighs every other.
fn add_lambert(n: Vec3, world: Vec3, q: Vec3, sum: &mut f32, weight: &mut f32) {
    let l = q - world;
    let dd = l.dot(l);
    let w = 1.0 / det::max(dd, MIN_SAMPLE_DISTANCE_2);
    *sum += w * if dd > MIN_SAMPLE_DISTANCE_2 {
        det::max(0.0, n.dot(l) / dd.sqrt())
    } else {
        1.0
    };
    *weight += w;
}

/// Receiver Lambert term: the mean of `max(0, n . L)` over the nearest point and the ends (line,
/// 3 points) or the corners (rect, 5 points), each weighted by `1 / d^2`, its share of the
/// irradiance. Far away all weights are equal and the term tends to the point light's `n . L`;
/// right at a large emitter the nearest point dominates. A point the receiver lies on counts 1.
pub fn area_lambert(e: &AreaEmitter, n: Vec3, world: Vec3, nearest: Vec3) -> f32 {
    let (mut sum, mut weight) = (0.0, 0.0);
    add_lambert(n, world, nearest, &mut sum, &mut weight);
    if !e.is_rect() {
        add_lambert(n, world, e.center + e.u, &mut sum, &mut weight);
        add_lambert(n, world, e.center - e.u, &mut sum, &mut weight);
        return sum / weight;
    }
    for a in [-1.0f32, 1.0] {
        for b in [-1.0f32, 1.0] {
            add_lambert(
                n,
                world,
                e.center + e.u * a + e.v * b,
                &mut sum,
                &mut weight,
            );
        }
    }
    sum / weight
}

/// Brightness normalization of line and rect lights with normals (E51): the factor on the light's
/// colour that makes it light a receiver straight in front of it at distance `radius / 2`,
/// facing it, as brightly as a point light of the same intensity at the same distance does:
/// `1 / area_lambert` there. "In front": along the rect's normal; for a line the direction down
/// (-z) made perpendicular to the axis (+x for a vertical tube). At least 1, at most 4.
pub fn area_normalization(e: &AreaEmitter, radius: f32) -> f32 {
    let mut front = e.normal;
    if !e.is_rect() {
        let uu = e.u.dot(e.u);
        let perpendicular = |d: Vec3| d - e.u * (d.dot(e.u) / uu);
        front = perpendicular(Vec3::new(0.0, 0.0, -1.0));
        if front.length() < 1e-3 {
            front = perpendicular(Vec3::new(1.0, 0.0, 0.0));
        }
        front = front * (1.0 / front.length());
    }
    let world = e.center + front * (0.5 * radius);
    let lambert = area_lambert(e, front * -1.0, world, e.center);
    if lambert > 0.25 {
        det::min(1.0 / lambert, 4.0)
    } else {
        4.0
    }
}

/// Representative point of the specular highlight (Karis 2013) for the view direction `v` and
/// the shading normal `n`: the point of the segment nearest to the reflected ray, or the
/// reflected ray's hit on the rectangle's plane clamped into the rectangle; `nearest` where
/// there is no hit.
pub fn area_specular_point(e: &AreaEmitter, world: Vec3, n: Vec3, v: Vec3, nearest: Vec3) -> Vec3 {
    let r = n * (2.0 * n.dot(v)) - v; // reflect(-v, n)
    if !e.is_rect() {
        let l0 = e.center - e.u - world;
        let ld = e.u * 2.0;
        let rd = r.dot(ld);
        let denom = ld.dot(ld) - rd * rd;
        if denom <= 1e-6 * ld.dot(ld) {
            return nearest; // reflected ray along the segment
        }
        let t = ((r.dot(l0) * rd - l0.dot(ld)) / denom).clamp(0.0, 1.0);
        return world + l0 + ld * t;
    }
    let rn = r.dot(e.normal);
    if rn.abs() <= 1e-4 {
        return nearest;
    }
    let x = (e.center - world).dot(e.normal) / rn;
    if x <= 0.0 {
        return nearest;
    }
    let rel = world + r * x - e.center;
    e.center + e.u * coordinate(rel, e.u) + e.v * coordinate(rel, e.v)
}
