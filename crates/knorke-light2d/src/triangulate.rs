// Origin: chiaro @ 9bc5b9b, src/core/Triangulate.hpp and Triangulate.cpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Ear-clipping triangulation of simple polygons (P5).
//!
//! Redesign against the C++ version: the polygon comes in as a [`Column`] of [`Vec2`] and the
//! triangles go into a host-owned `Vec<Vec2>` (three vertices per triangle) instead of flat
//! float pairs; the working memory lives in a reusable [`Triangulator`], so triangulating again
//! allocates nothing once its buffers and the host's output buffer have grown. The output
//! stays untouched on failure. The arithmetic is the same (double precision for the tests,
//! single precision in the output).

use knorke_math::{Vec2, det};
use knorke_types::{Column, Diagnostic, DiagnosticCode};

/// Why a polygon cannot be triangulated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriangulateError {
    /// A vertex is NaN or infinite.
    NotFinite,
    /// Fewer than three distinct, non-collinear vertices remain after cleaning.
    TooFewVertices,
    /// The polygon's area is below `1e-6`.
    NoArea,
    /// Two non-adjacent edges touch or cross.
    SelfIntersecting,
    /// Ear clipping found no ear (numerically degenerate input).
    NotTriangulable,
}

impl TriangulateError {
    fn what(&self) -> &'static str {
        match self {
            Self::NotFinite => "vertices must be finite",
            Self::TooFewVertices => "has fewer than 3 distinct, non-collinear vertices",
            Self::NoArea => "has no area",
            Self::SelfIntersecting => "is self-intersecting",
            Self::NotTriangulable => "could not be triangulated",
        }
    }

    /// As a diagnostic with a stable code.
    pub fn to_diagnostic(&self) -> Diagnostic {
        let code = match self {
            Self::NotFinite => DiagnosticCode::new("knorke.light2d.polygon.not_finite"),
            Self::TooFewVertices => DiagnosticCode::new("knorke.light2d.polygon.too_few_vertices"),
            Self::NoArea => DiagnosticCode::new("knorke.light2d.polygon.no_area"),
            Self::SelfIntersecting => {
                DiagnosticCode::new("knorke.light2d.polygon.self_intersecting")
            }
            Self::NotTriangulable => DiagnosticCode::new("knorke.light2d.polygon.not_triangulable"),
        };
        Diagnostic::error(code, self.to_string())
    }
}

impl core::fmt::Display for TriangulateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "polygon obstacle {}", self.what())
    }
}

impl std::error::Error for TriangulateError {}

type P = [f64; 2];

fn cross(o: P, a: P, b: P) -> f64 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn sign(v: f64) -> i32 {
    i32::from(v > 0.0) - i32::from(v < 0.0)
}

fn on_segment(a: P, b: P, p: P) -> bool {
    let (lo_x, hi_x) = if a[0] <= b[0] {
        (a[0], b[0])
    } else {
        (b[0], a[0])
    };
    let (lo_y, hi_y) = if a[1] <= b[1] {
        (a[1], b[1])
    } else {
        (b[1], a[1])
    };
    lo_x <= p[0] && p[0] <= hi_x && lo_y <= p[1] && p[1] <= hi_y
}

/// Closed segments ab and cd share at least one point.
fn segments_touch(a: P, b: P, c: P, d: P) -> bool {
    let d1 = sign(cross(a, b, c));
    let d2 = sign(cross(a, b, d));
    let d3 = sign(cross(c, d, a));
    let d4 = sign(cross(c, d, b));
    if d1 * d2 < 0 && d3 * d4 < 0 {
        return true;
    }
    (d1 == 0 && on_segment(a, b, c))
        || (d2 == 0 && on_segment(a, b, d))
        || (d3 == 0 && on_segment(c, d, a))
        || (d4 == 0 && on_segment(c, d, b))
}

/// `p` inside or on the border of the counter-clockwise triangle abc.
fn in_triangle(a: P, b: P, c: P, p: P) -> bool {
    cross(a, b, p) >= 0.0 && cross(b, c, p) >= 0.0 && cross(c, a, p) >= 0.0
}

fn signed_area(poly: &[P]) -> f64 {
    let mut sum = 0.0;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        sum += a[0] * b[1] - b[0] * a[1];
    }
    0.5 * sum
}

/// Reusable working memory of the ear-clipping triangulation.
#[derive(Clone, Debug, Default)]
pub struct Triangulator {
    poly: Vec<P>,
    idx: Vec<usize>,
    out: Vec<Vec2>,
}

impl Triangulator {
    /// A triangulator with empty buffers; they grow on first use and are then reused.
    pub fn new() -> Self {
        Self::default()
    }

    /// Triangulates a simple polygon. Removes duplicate and collinear neighbours and
    /// normalizes the winding, so the triangles are counter-clockwise. On success `triangles`
    /// is replaced by the triangle list (three [`Vec2`] per triangle); on failure it is left
    /// untouched.
    ///
    /// # Errors
    /// See [`TriangulateError`].
    pub fn triangulate(
        &mut self,
        polygon: impl Column<Vec2>,
        triangles: &mut Vec<Vec2>,
    ) -> Result<(), TriangulateError> {
        self.poly.clear();
        for v in polygon.iter() {
            if !v.x.is_finite() || !v.y.is_finite() {
                return Err(TriangulateError::NotFinite);
            }
            self.poly.push([f64::from(v.x), f64::from(v.y)]);
        }
        let poly = &mut self.poly;

        // Drop duplicate and collinear neighbours until stable (spikes have a zero cross product too).
        let mut changed = true;
        while changed && poly.len() >= 3 {
            changed = false;
            let mut i = 0;
            while i < poly.len() && poly.len() >= 3 {
                let n = poly.len();
                let (prev, cur, next) = (poly[(i + n - 1) % n], poly[i], poly[(i + 1) % n]);
                let duplicate = cur[0] == prev[0] && cur[1] == prev[1];
                if duplicate || cross(prev, cur, next) == 0.0 {
                    poly.remove(i);
                    changed = true;
                } else {
                    i += 1;
                }
            }
        }
        if poly.len() < 3 {
            return Err(TriangulateError::TooFewVertices);
        }

        let area = signed_area(poly);
        if area.abs() < 1e-6 {
            return Err(TriangulateError::NoArea);
        }

        let n = poly.len();
        for i in 0..n {
            for j in (i + 1)..n {
                if j == i + 1 || (i == 0 && j == n - 1) {
                    continue; // adjacent edges share a vertex by construction
                }
                if segments_touch(poly[i], poly[(i + 1) % n], poly[j], poly[(j + 1) % n]) {
                    return Err(TriangulateError::SelfIntersecting);
                }
            }
        }

        if area < 0.0 {
            poly.reverse();
        }

        self.idx.clear();
        self.idx.extend(0..n);
        self.out.clear();
        let push = |out: &mut Vec<Vec2>, p: P| out.push(Vec2::new(p[0] as f32, p[1] as f32));
        while self.idx.len() > 3 {
            let m = self.idx.len();
            let mut clipped = false;
            for k in 0..m {
                let (kp, kn) = ((k + m - 1) % m, (k + 1) % m);
                let (a, b, c) = (poly[self.idx[kp]], poly[self.idx[k]], poly[self.idx[kn]]);
                if cross(a, b, c) <= 0.0 {
                    continue; // reflex
                }
                let empty = (0..m).all(|q| {
                    q == k || q == kp || q == kn || !in_triangle(a, b, c, poly[self.idx[q]])
                });
                if !empty {
                    continue;
                }
                push(&mut self.out, a);
                push(&mut self.out, b);
                push(&mut self.out, c);
                self.idx.remove(k);
                clipped = true;
                break;
            }
            if !clipped {
                return Err(TriangulateError::NotTriangulable);
            }
        }
        for &id in &self.idx {
            push(&mut self.out, poly[id]);
        }
        triangles.clear();
        triangles.extend_from_slice(&self.out);
        Ok(())
    }
}

/// A circle of `radius` around (0, 0) as a fan of 48 triangles (three [`Vec2`] each), written
/// to `triangles` (replacing its content, reusing its capacity).
pub fn circle_triangles(radius: f32, triangles: &mut Vec<Vec2>) {
    const SEGMENTS: usize = 48;
    let step = 2.0 * core::f32::consts::PI / SEGMENTS as f32;
    triangles.clear();
    triangles.reserve(SEGMENTS * 3);
    for i in 0..SEGMENTS {
        let (s0, c0) = det::sin_cos(step * i as f32);
        let (s1, c1) = det::sin_cos(step * (i + 1) as f32);
        triangles.push(Vec2::ZERO);
        triangles.push(Vec2::new(radius * c0, radius * s0));
        triangles.push(Vec2::new(radius * c1, radius * s1));
    }
}
