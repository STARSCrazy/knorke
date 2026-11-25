// Origin: chiaro @ 9bc5b9b, src/shadows/PatternGeometry.hpp and PatternGeometry.cpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Where a shadow ray piece first enters the geometry of a pattern obstacle (P15), and so where
//! it reads the pattern.
//!
//! The piece runs from `a` (the last sample outside) to `b` (the first sample inside) in the
//! obstacle's local units, with ray heights `za` and `zb` at its ends. The same steps run in
//! HLSL in Chiaro's `shaders/src/pattern.hlsli` (`PatternEntry*`) for the `*_pattern` light
//! programs; this CPU version is the reference for them and must stay in step (same tie rules:
//! the first axis, edge or strip wins an exact tie; same fallback). Float math only, no
//! allocation.
//!
//! `u` is a world length along the outline (divide by the repeat width for repeats), chosen so
//! that opposite sides of a thin obstacle show the pattern at the same place:
//!
//! - box: faces along local x ([`SIDE_Y_LOW`], [`SIDE_Y_HIGH`]): `x + half_w`; faces along y:
//!   `y + half_d`;
//! - polygon: `dot(point, edge direction)`, the direction sign-normalised (first non-zero
//!   component positive, an x component below 1e-6 of the length counting as zero); `side` is the
//!   edge index `i` (`v[i] -> v[i + 1]`);
//! - circle: arc length `angle * radius` from local +x, counter-clockwise, in `[0, 2 pi r)`;
//! - tile cell: see [`entry_tile_cell`].

use knorke_math::{Vec2, det};

/// [`EntryHit::side`] of a box or tile cell: `x` = low end, normal `-x` (edge bit 1).
pub const SIDE_X_LOW: i32 = 0;
/// `x` = high end, normal `+x` (edge bit 2).
pub const SIDE_X_HIGH: i32 = 1;
/// `y` = low end, normal `-y` (edge bit 4).
pub const SIDE_Y_LOW: i32 = 2;
/// `y` = high end, normal `+y` (edge bit 8).
pub const SIDE_Y_HIGH: i32 = 3;

/// Where a ray piece enters a pattern obstacle.
///
/// `found`: the piece crosses into the geometry at `t` in `[0, 1]`; `u`, `side` and
/// `world_z = za + t * (zb - za)` belong to that point. Otherwise (both ends inside, a miss
/// through rounding of the mask, a degenerate piece) the nearest point of the outline to `b`
/// gives `u` and `side`, and `world_z` is `zb`. `u` is never NaN for finite `a` and `b`;
/// `world_z` is only meaningful for finite heights (with infinite ones the pattern is read by
/// column mean, without `v`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntryHit {
    /// The piece crosses into the geometry.
    pub found: bool,
    /// World length along the outline.
    pub u: f32,
    /// Ray height at the entry.
    pub world_z: f32,
    /// The face (box, cell) or edge (polygon) crossed; `-1` for none.
    pub side: i32,
}

const TINY: f32 = 1e-12;
/// An x component below this share of the edge length counts as zero, so two near-vertical edges
/// whose x differs only by rounding still agree.
const AXIS_TOLERANCE: f32 = 1e-6;

fn dot(p: Vec2, q: Vec2) -> f32 {
    p.x * q.x + p.y * q.y
}

fn cross(p: Vec2, q: Vec2) -> f32 {
    p.x * q.y - p.y * q.x
}

fn along(a: Vec2, d: Vec2, t: f32) -> Vec2 {
    Vec2::new(a.x + t * d.x, a.y + t * d.y)
}

/// Ternary clamp as in the C++ (`f32::clamp` panics on `lo > hi`).
fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

fn hit(t: f32, u: f32, side: i32, za: f32, zb: f32) -> EntryHit {
    EntryHit {
        found: true,
        u,
        world_z: za + t * (zb - za),
        side,
    }
}

fn fallback(u: f32, side: i32, zb: f32) -> EntryHit {
    EntryHit {
        found: false,
        u,
        world_z: zb,
        side,
    }
}

/// Entry of `a + t (b - a)`, `t` in `[0, 1]`, into the rectangle `[lo, hi]` (slab method): `t`
/// and the face crossed, only if `a` lies outside. On an exact corner the x face wins.
fn rect_entry(lo: Vec2, hi: Vec2, a: Vec2, b: Vec2) -> Option<(f32, i32)> {
    let p = [a.x, a.y];
    let d = [b.x - a.x, b.y - a.y];
    let l = [lo.x, lo.y];
    let h = [hi.x, hi.y];
    let mut t_enter = -1.0f32;
    let mut t_exit = 2.0f32;
    let mut side = -1;
    for k in 0..2 {
        if d[k].abs() < TINY {
            if p[k] < l[k] || p[k] > h[k] {
                return None; // parallel to this slab and outside it
            }
            continue;
        }
        let mut t0 = (l[k] - p[k]) / d[k];
        let mut t1 = (h[k] - p[k]) / d[k];
        let mut face = 2 * k as i32;
        if d[k] < 0.0 {
            core::mem::swap(&mut t0, &mut t1);
            face += 1; // moving down: enters through the high face
        }
        if t0 > t_enter {
            t_enter = t0;
            side = face;
        }
        t_exit = if t1 < t_exit { t1 } else { t_exit };
    }
    if side < 0 || t_enter < 0.0 || t_enter > 1.0 || t_enter > t_exit {
        return None;
    }
    Some((t_enter, side))
}

fn inside(lo: Vec2, hi: Vec2, p: Vec2) -> bool {
    p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y
}

/// The nearest point of the rectangle's outline to `b`, its face and the squared distance.
fn rect_nearest(lo: Vec2, hi: Vec2, b: Vec2) -> (Vec2, i32, f32) {
    if inside(lo, hi, b) {
        let dist = [b.x - lo.x, hi.x - b.x, b.y - lo.y, hi.y - b.y];
        let mut side = 0;
        for k in 1..4 {
            if dist[k] < dist[side] {
                side = k;
            }
        }
        let mut point = b;
        match side as i32 {
            SIDE_X_LOW => point.x = lo.x,
            SIDE_X_HIGH => point.x = hi.x,
            SIDE_Y_LOW => point.y = lo.y,
            _ => point.y = hi.y,
        }
        return (point, side as i32, dist[side] * dist[side]);
    }
    let point = Vec2::new(clamp(b.x, lo.x, hi.x), clamp(b.y, lo.y, hi.y));
    let ex = (b.x - point.x).abs();
    let ey = (b.y - point.y).abs();
    let side = if ex >= ey {
        if b.x < lo.x { SIDE_X_LOW } else { SIDE_X_HIGH }
    } else if b.y < lo.y {
        SIDE_Y_LOW
    } else {
        SIDE_Y_HIGH
    };
    (point, side, ex * ex + ey * ey)
}

fn is_x_side(side: i32) -> bool {
    side == SIDE_X_LOW || side == SIDE_X_HIGH
}

/// `u` on an edge: the point projected on the edge direction, sign-normalised so parallel
/// opposite edges agree.
fn edge_u(p: Vec2, e: Vec2) -> f32 {
    let len = dot(e, e).sqrt();
    if len < TINY {
        return 0.0;
    }
    let mut dir = Vec2::new(e.x / len, e.y / len);
    let vertical = dir.x.abs() < AXIS_TOLERANCE; // dir is unit: the share of the length
    if if vertical { dir.y < 0.0 } else { dir.x < 0.0 } {
        dir = Vec2::new(-dir.x, -dir.y);
    }
    dot(p, dir)
}

/// Arc length from local +x, counter-clockwise, in `[0, 2 pi radius)`.
fn arc_u(p: Vec2, radius: f32) -> f32 {
    let mut angle = det::atan2(p.y, p.x);
    if angle < 0.0 {
        angle += core::f32::consts::TAU;
    }
    angle * radius
}

/// A box of half extents `half_w` (local x) and `half_d` (local y) about the local origin.
pub fn entry_box(half_w: f32, half_d: f32, a: Vec2, b: Vec2, za: f32, zb: f32) -> EntryHit {
    let lo = Vec2::new(-half_w, -half_d);
    let hi = Vec2::new(half_w, half_d);
    let u_of = |p: Vec2, side: i32| {
        if is_x_side(side) {
            p.y + half_d
        } else {
            p.x + half_w
        }
    };
    if let Some((t, side)) = rect_entry(lo, hi, a, b) {
        let p = along(a, b - a, t);
        return hit(t, u_of(p, side), side, za, zb);
    }
    let (p, side, _) = rect_nearest(lo, hi, b);
    fallback(u_of(p, side), side, zb)
}

/// A simple polygon in either winding (at most
/// [`MAX_PATTERN_VERTICES`](crate::obstacle::MAX_PATTERN_VERTICES) corners for pattern
/// obstacles).
pub fn entry_polygon(v: &[Vec2], a: Vec2, b: Vec2, za: f32, zb: f32) -> EntryHit {
    let n = v.len();
    if n < 2 {
        return fallback(0.0, -1, zb);
    }
    let mut area2 = 0.0f32; // twice the signed area: > 0 counter-clockwise
    for i in 0..n {
        area2 += cross(v[i], v[(i + 1) % n]);
    }
    let d = b - a;
    let mut best_t = 2.0f32;
    let mut best_edge = usize::MAX;
    for i in 0..n {
        let v0 = v[i];
        let e = v[(i + 1) % n] - v0;
        let denom = cross(d, e); // = dot(d, outward normal) for counter-clockwise winding
        if denom.abs() < TINY || denom * area2 >= 0.0 {
            continue; // parallel, or leaving the polygon through this edge
        }
        let w = v0 - a;
        let t = cross(w, e) / denom;
        let s = cross(w, d) / denom;
        if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&s) && t < best_t {
            best_t = t;
            best_edge = i;
        }
    }
    if best_edge != usize::MAX {
        let e = v[(best_edge + 1) % n] - v[best_edge];
        return hit(
            best_t,
            edge_u(along(a, d, best_t), e),
            best_edge as i32,
            za,
            zb,
        );
    }
    let mut best_dist = -1.0f32;
    let mut best_u = 0.0f32;
    for i in 0..n {
        let v0 = v[i];
        let e = v[(i + 1) % n] - v0;
        let ee = dot(e, e);
        let s = if ee < TINY {
            0.0
        } else {
            clamp(dot(b - v0, e) / ee, 0.0, 1.0)
        };
        let p = along(v0, e, s);
        let off = b - p;
        let dist = dot(off, off);
        if best_dist < 0.0 || dist < best_dist {
            best_dist = dist;
            best_edge = i;
            best_u = edge_u(p, e);
        }
    }
    fallback(best_u, best_edge as i32, zb)
}

/// A circle of `radius` about the local origin; `side` is 0.
pub fn entry_circle(radius: f32, a: Vec2, b: Vec2, za: f32, zb: f32) -> EntryHit {
    let d = b - a;
    let qa = dot(d, d);
    let qb = dot(a, d);
    let qc = dot(a, a) - radius * radius; // >= 0: a outside (or on) the circle
    if qa >= TINY && qc >= 0.0 {
        let disc = qb * qb - qa * qc;
        if disc >= 0.0 {
            let t = (-qb - disc.sqrt()) / qa;
            if (0.0..=1.0).contains(&t) {
                return hit(t, arc_u(along(a, d, t), radius), 0, za, zb);
            }
        }
    }
    let len = dot(b, b).sqrt();
    let p = if len < TINY {
        Vec2::new(radius, 0.0)
    } else {
        Vec2::new(b.x * radius / len, b.y * radius / len)
    };
    fallback(arc_u(p, radius), 0, zb)
}

/// Cell (`column`, `row`) of a tile grid; `a`, `b` in grid-local units (cell (0, 0) has its
/// corner at the origin), so `u` is continuous across the grid.
///
/// With edge bits (1..15) the obstacle is the union of strips `edge_width` wide (at most the
/// cell) along the named edges; `u` is local x on a y strip and local y on an x strip, whichever
/// face of the strip the ray enters. Edge bits 0 (no edges channel) or above 15 (255: a whole
/// cell under an edges channel) are a whole cell; `u` is then the coordinate along the entered
/// side. `side` is the face crossed.
#[allow(clippy::too_many_arguments)] // the parameters of the C++ function and its HLSL twin
pub fn entry_tile_cell(
    cell_size: Vec2,
    column: u32,
    row: u32,
    edge_bits: u32,
    edge_width: f32,
    a: Vec2,
    b: Vec2,
    za: f32,
    zb: f32,
) -> EntryHit {
    let lo = Vec2::new(column as f32 * cell_size.x, row as f32 * cell_size.y);
    let hi = Vec2::new(lo.x + cell_size.x, lo.y + cell_size.y);
    if edge_bits == 0 || edge_bits > 15 {
        let u_of = |p: Vec2, side: i32| if is_x_side(side) { p.y } else { p.x };
        if let Some((t, side)) = rect_entry(lo, hi, a, b) {
            let p = along(a, b - a, t);
            return hit(t, u_of(p, side), side, za, zb);
        }
        let (p, side, _) = rect_nearest(lo, hi, b);
        return fallback(u_of(p, side), side, zb);
    }

    // Strip k lies along the edge of side k; x strips carry u = y, y strips u = x.
    let wx = clamp(edge_width, 0.0, cell_size.x);
    let wy = clamp(edge_width, 0.0, cell_size.y);
    let strip_lo = [
        lo,
        Vec2::new(hi.x - wx, lo.y),
        lo,
        Vec2::new(lo.x, hi.y - wy),
    ];
    let strip_hi = [
        Vec2::new(lo.x + wx, hi.y),
        hi,
        Vec2::new(hi.x, lo.y + wy),
        hi,
    ];
    let on = |k: usize| (edge_bits >> k) & 1 != 0;
    let a_inside = (0..4).any(|k| on(k) && inside(strip_lo[k], strip_hi[k], a));
    let mut best_t = 2.0f32;
    let mut best_strip = -1i32;
    let mut best_side = -1i32;
    for k in 0..4 {
        if a_inside || !on(k) {
            continue;
        }
        if let Some((t, side)) = rect_entry(strip_lo[k], strip_hi[k], a, b)
            && t < best_t
        {
            best_t = t;
            best_strip = k as i32;
            best_side = side;
        }
    }
    if best_strip >= 0 {
        let p = along(a, b - a, best_t);
        let u = if is_x_side(best_strip) { p.y } else { p.x };
        return hit(best_t, u, best_side, za, zb);
    }
    let mut best_dist = -1.0f32;
    let mut best_u = 0.0f32;
    for k in 0..4 {
        if !on(k) {
            continue;
        }
        let (p, side, dist) = rect_nearest(strip_lo[k], strip_hi[k], b);
        if best_dist < 0.0 || dist < best_dist {
            best_dist = dist;
            best_side = side;
            best_u = if is_x_side(k as i32) { p.y } else { p.x };
        }
    }
    fallback(best_u, best_side, zb)
}
