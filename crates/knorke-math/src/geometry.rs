//! 2D geometry primitives shared by physics, navigation (line of sight), perception (view
//! cones, occlusion), lighting (shadow casters), culling and picking (plan section 17.8).
//!
//! All types are `#[repr(C)]` `f32` structs without padding, castable like the vectors.
//! Space tagging works as for vectors: `At<World2, Aabb2>`.

use crate::Vec2;

/// Axis-aligned box. Invariant: `min <= max` component-wise (use [`Aabb2::from_corners`]
/// for unsorted input). Layout: `[min.x, min.y, max.x, max.y]`, 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Aabb2 {
    /// Lower corner.
    pub min: Vec2,
    /// Upper corner.
    pub max: Vec2,
}

/// Circle. Layout: `[center.x, center.y, radius]`, 12 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Circle {
    /// Center.
    pub center: Vec2,
    /// Radius (≥ 0).
    pub radius: f32,
}

/// Line segment from `a` to `b`. Layout: `[a.x, a.y, b.x, b.y]`, 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Segment2 {
    /// Start.
    pub a: Vec2,
    /// End.
    pub b: Vec2,
}

/// Capsule: all points within `radius` of a segment (typical character collider).
/// Layout: `[a.x, a.y, b.x, b.y, radius]`, 20 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Capsule2 {
    /// Core segment.
    pub segment: Segment2,
    /// Radius (≥ 0).
    pub radius: f32,
}

/// Ray `origin + t · dir`, `t ≥ 0`. `dir` need not be normalised; returned `t` values are in
/// units of `dir`. Layout: `[origin.x, origin.y, dir.x, dir.y]`, 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ray2 {
    /// Start point.
    pub origin: Vec2,
    /// Direction.
    pub dir: Vec2,
}

impl Aabb2 {
    /// Box from two arbitrary corners.
    #[inline]
    pub fn from_corners(a: Vec2, b: Vec2) -> Self {
        Self {
            min: a.min(b),
            max: a.max(b),
        }
    }

    /// Box from center and half extents (`half` ≥ 0).
    #[inline]
    pub fn from_center_half(center: Vec2, half: Vec2) -> Self {
        Self {
            min: center - half,
            max: center + half,
        }
    }

    /// Center point.
    #[inline]
    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }

    /// Half extents.
    #[inline]
    pub fn half_extents(&self) -> Vec2 {
        (self.max - self.min) * 0.5
    }

    /// `true` if `p` is inside or on the border.
    #[inline]
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    /// `true` if the boxes overlap or touch.
    #[inline]
    pub fn intersects(&self, o: &Aabb2) -> bool {
        self.min.x <= o.max.x
            && o.min.x <= self.max.x
            && self.min.y <= o.max.y
            && o.min.y <= self.max.y
    }

    /// Smallest box containing both.
    #[inline]
    pub fn union(&self, o: &Aabb2) -> Aabb2 {
        Aabb2 {
            min: self.min.min(o.min),
            max: self.max.max(o.max),
        }
    }

    /// Grown by `margin` on every side.
    #[inline]
    pub fn expanded(&self, margin: f32) -> Aabb2 {
        Aabb2 {
            min: self.min - Vec2::splat(margin),
            max: self.max + Vec2::splat(margin),
        }
    }

    /// Point of the box closest to `p` (`p` itself if inside).
    #[inline]
    pub fn closest_point(&self, p: Vec2) -> Vec2 {
        p.max(self.min).min(self.max)
    }

    /// Squared distance from `p` to the box (0 inside).
    #[inline]
    pub fn distance_squared(&self, p: Vec2) -> f32 {
        (self.closest_point(p) - p).length_squared()
    }
}

impl Circle {
    /// Creates a circle.
    #[inline]
    pub const fn new(center: Vec2, radius: f32) -> Self {
        Self { center, radius }
    }

    /// `true` if `p` is inside or on the border.
    #[inline]
    pub fn contains(&self, p: Vec2) -> bool {
        (p - self.center).length_squared() <= self.radius * self.radius
    }

    /// `true` if the circles overlap or touch.
    #[inline]
    pub fn intersects_circle(&self, o: &Circle) -> bool {
        let r = self.radius + o.radius;
        (o.center - self.center).length_squared() <= r * r
    }

    /// `true` if the circle overlaps or touches the box.
    #[inline]
    pub fn intersects_aabb(&self, b: &Aabb2) -> bool {
        b.distance_squared(self.center) <= self.radius * self.radius
    }

    /// Bounding box.
    #[inline]
    pub fn aabb(&self) -> Aabb2 {
        Aabb2::from_center_half(self.center, Vec2::splat(self.radius))
    }
}

impl Segment2 {
    /// Creates a segment.
    #[inline]
    pub const fn new(a: Vec2, b: Vec2) -> Self {
        Self { a, b }
    }

    /// Parameter `t ∈ [0, 1]` and point of the segment closest to `p`.
    #[inline]
    pub fn closest_point(&self, p: Vec2) -> (f32, Vec2) {
        let d = self.b - self.a;
        let len2 = d.length_squared();
        // Exactly degenerate only (also catches NaN): a fixed tolerance would make short
        // segments in small-scale worlds behave like points.
        if !positive(len2) {
            return (0.0, self.a);
        }
        let t = ((p - self.a).dot(d) / len2).clamp(0.0, 1.0);
        (t, self.a + d * t)
    }

    /// Squared distance from `p` to the segment.
    #[inline]
    pub fn distance_squared(&self, p: Vec2) -> f32 {
        (self.closest_point(p).1 - p).length_squared()
    }

    /// Intersection with another segment: parameters on `self` and `o` and the point, or
    /// `None` if they do not cross. Parallel, collinear and zero-length segments count as
    /// not crossing; "parallel" is judged relative to the segment lengths, so the answer
    /// does not depend on the scale of the world.
    pub fn intersect(&self, o: &Segment2) -> Option<(f32, f32, Vec2)> {
        let r = self.b - self.a;
        let s = o.b - o.a;
        let denom = r.perp_dot(s);
        if nearly_parallel(denom, r, s) {
            return None;
        }
        let q = o.a - self.a;
        let t = q.perp_dot(s) / denom;
        let u = q.perp_dot(r) / denom;
        ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| (t, u, self.a + r * t))
    }

    /// Bounding box.
    #[inline]
    pub fn aabb(&self) -> Aabb2 {
        Aabb2::from_corners(self.a, self.b)
    }
}

/// `true` if `v > 0`; `false` for zero, negative values and NaN.
#[inline]
fn positive(v: f32) -> bool {
    v > 0.0
}

/// `true` if `cross = a × b` is zero within rounding relative to the lengths of `a` and
/// `b` (angle below ~`f32::EPSILON`), or if either vector is zero or not finite.
#[inline]
fn nearly_parallel(cross: f32, a: Vec2, b: Vec2) -> bool {
    let scale = (a.x.abs() + a.y.abs()) * (b.x.abs() + b.y.abs());
    !positive(cross.abs() - f32::EPSILON * scale)
}

impl Capsule2 {
    /// Creates a capsule.
    #[inline]
    pub const fn new(a: Vec2, b: Vec2, radius: f32) -> Self {
        Self {
            segment: Segment2::new(a, b),
            radius,
        }
    }

    /// `true` if `p` is inside or on the border.
    #[inline]
    pub fn contains(&self, p: Vec2) -> bool {
        self.segment.distance_squared(p) <= self.radius * self.radius
    }

    /// `true` if the capsule overlaps or touches the circle.
    #[inline]
    pub fn intersects_circle(&self, c: &Circle) -> bool {
        let r = self.radius + c.radius;
        self.segment.distance_squared(c.center) <= r * r
    }

    /// Bounding box.
    #[inline]
    pub fn aabb(&self) -> Aabb2 {
        self.segment.aabb().expanded(self.radius)
    }
}

impl Ray2 {
    /// Creates a ray.
    #[inline]
    pub const fn new(origin: Vec2, dir: Vec2) -> Self {
        Self { origin, dir }
    }

    /// Point at parameter `t`.
    #[inline]
    pub fn at(&self, t: f32) -> Vec2 {
        self.origin + self.dir * t
    }

    /// Smallest `t ∈ [0, max_t]` where the ray enters the box (0 if it starts inside).
    /// A ray with zero direction hits only if its origin is inside.
    pub fn cast_aabb(&self, b: &Aabb2, max_t: f32) -> Option<f32> {
        if max_t.is_nan() || max_t < 0.0 {
            return None;
        }
        let mut t0 = 0.0f32;
        let mut t1 = max_t;
        for (o, d, lo, hi) in [
            (self.origin.x, self.dir.x, b.min.x, b.max.x),
            (self.origin.y, self.dir.y, b.min.y, b.max.y),
        ] {
            // Parallel to this slab only if the component is zero or subnormal (then `1 / d`
            // would not be finite). No fixed tolerance: `t` is in units of `dir`, and `dir`
            // may be short.
            if d.abs() < f32::MIN_POSITIVE {
                if o < lo || o > hi {
                    return None;
                }
            } else {
                let inv = 1.0 / d;
                let (mut a, mut c) = ((lo - o) * inv, (hi - o) * inv);
                if a > c {
                    core::mem::swap(&mut a, &mut c);
                }
                // Explicit comparisons instead of `f32::max`/`min`, whose result for
                // `-0.0` against `+0.0` is platform-dependent (ADR 0011).
                if a > t0 {
                    t0 = a;
                }
                if c < t1 {
                    t1 = c;
                }
                if t0 > t1 {
                    return None;
                }
            }
        }
        Some(t0)
    }

    /// Smallest `t ∈ [0, max_t]` where the ray enters the circle (0 if it starts inside).
    /// A ray with zero direction hits only if its origin is inside.
    pub fn cast_circle(&self, c: &Circle, max_t: f32) -> Option<f32> {
        if max_t.is_nan() || max_t < 0.0 {
            return None;
        }
        let m = self.origin - c.center;
        let cc = m.length_squared() - c.radius * c.radius;
        if cc <= 0.0 {
            return Some(0.0);
        }
        let a = self.dir.length_squared();
        let b = m.dot(self.dir);
        if b >= 0.0 || !positive(a) {
            return None;
        }
        let disc = b * b - a * cc;
        if disc < 0.0 {
            return None;
        }
        let t = (-b - disc.sqrt()) / a;
        (t <= max_t).then_some(t)
    }

    /// Smallest `t ∈ [0, max_t]` where the ray crosses the segment. Rays parallel to the
    /// segment (judged relative to the lengths, independent of scale), zero directions and
    /// zero-length segments do not hit.
    pub fn cast_segment(&self, s: &Segment2, max_t: f32) -> Option<f32> {
        let r = self.dir;
        let e = s.b - s.a;
        let denom = r.perp_dot(e);
        if nearly_parallel(denom, r, e) {
            return None;
        }
        let q = s.a - self.origin;
        let t = q.perp_dot(e) / denom;
        let u = q.perp_dot(r) / denom;
        (t >= 0.0 && t <= max_t && (0.0..=1.0).contains(&u)).then_some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y)
    }

    #[test]
    fn aabb_queries() {
        let b = Aabb2::from_corners(v(4.0, 3.0), v(0.0, 0.0));
        assert_eq!((b.min, b.max), (v(0.0, 0.0), v(4.0, 3.0)));
        assert_eq!((b.center(), b.half_extents()), (v(2.0, 1.5), v(2.0, 1.5)));
        assert!(b.contains(v(4.0, 0.0)) && !b.contains(v(4.1, 0.0)));
        assert!(b.intersects(&Aabb2::from_corners(v(4.0, 3.0), v(9.0, 9.0))));
        assert!(!b.intersects(&Aabb2::from_corners(v(4.1, 0.0), v(9.0, 9.0))));
        assert_eq!(b.distance_squared(v(7.0, 7.0)), 9.0 + 16.0);
        assert_eq!(b.distance_squared(v(1.0, 1.0)), 0.0);
        assert_eq!(
            b.union(&Aabb2::from_corners(v(-1.0, 5.0), v(0.0, 6.0))).max,
            v(4.0, 6.0)
        );
    }

    #[test]
    fn circle_and_capsule() {
        let c = Circle::new(v(0.0, 0.0), 1.0);
        assert!(c.contains(v(1.0, 0.0)) && !c.contains(v(1.0, 0.1)));
        assert!(c.intersects_circle(&Circle::new(v(2.0, 0.0), 1.0)));
        assert!(!c.intersects_circle(&Circle::new(v(2.1, 0.0), 1.0)));
        assert!(c.intersects_aabb(&Aabb2::from_corners(v(0.5, 0.5), v(3.0, 3.0))));
        assert!(!c.intersects_aabb(&Aabb2::from_corners(v(0.8, 0.8), v(3.0, 3.0))));
        let cap = Capsule2::new(v(0.0, 0.0), v(10.0, 0.0), 1.0);
        assert!(
            cap.contains(v(5.0, 1.0)) && cap.contains(v(-1.0, 0.0)) && !cap.contains(v(5.0, 1.1))
        );
        assert!(cap.intersects_circle(&Circle::new(v(5.0, 2.5), 1.5)));
        assert_eq!(cap.aabb(), Aabb2::from_corners(v(-1.0, -1.0), v(11.0, 1.0)));
    }

    #[test]
    fn segments() {
        let s = Segment2::new(v(0.0, 0.0), v(10.0, 0.0));
        assert_eq!(s.closest_point(v(3.0, 5.0)), (0.3, v(3.0, 0.0)));
        assert_eq!(s.closest_point(v(-5.0, 1.0)).0, 0.0);
        assert_eq!(s.distance_squared(v(12.0, 0.0)), 4.0);
        let (t, u, p) = s
            .intersect(&Segment2::new(v(5.0, -1.0), v(5.0, 1.0)))
            .unwrap();
        assert_eq!((t, u, p), (0.5, 0.5, v(5.0, 0.0)));
        assert!(
            s.intersect(&Segment2::new(v(11.0, -1.0), v(11.0, 1.0)))
                .is_none()
        );
        assert!(
            s.intersect(&Segment2::new(v(0.0, 1.0), v(10.0, 1.0)))
                .is_none(),
            "parallel"
        );
        assert_eq!(
            Segment2::new(v(1.0, 1.0), v(1.0, 1.0)).closest_point(v(5.0, 5.0)),
            (0.0, v(1.0, 1.0))
        );
    }

    #[test]
    fn ray_casts() {
        let r = Ray2::new(v(0.0, 0.5), v(1.0, 0.0));
        let b = Aabb2::from_corners(v(2.0, 0.0), v(3.0, 1.0));
        assert_eq!(r.cast_aabb(&b, 10.0), Some(2.0));
        assert_eq!(r.cast_aabb(&b, 1.5), None, "beyond max_t");
        assert_eq!(
            Ray2::new(v(2.5, 0.5), v(1.0, 0.0)).cast_aabb(&b, 10.0),
            Some(0.0),
            "starts inside"
        );
        assert_eq!(
            Ray2::new(v(0.0, 2.0), v(1.0, 0.0)).cast_aabb(&b, 10.0),
            None,
            "parallel outside"
        );
        let c = Circle::new(v(5.0, 0.5), 1.0);
        assert_eq!(r.cast_circle(&c, 10.0), Some(4.0));
        assert_eq!(
            Ray2::new(v(0.0, 0.5), v(-1.0, 0.0)).cast_circle(&c, 10.0),
            None,
            "pointing away"
        );
        assert_eq!(
            Ray2::new(v(0.0, 3.0), v(1.0, 0.0)).cast_circle(&c, 10.0),
            None,
            "miss"
        );
        let wall = Segment2::new(v(7.0, -1.0), v(7.0, 1.0));
        assert_eq!(r.cast_segment(&wall, 10.0), Some(7.0));
        assert_eq!(r.cast_segment(&wall, 6.0), None);
        assert_eq!(Ray2::new(v(0.0, 0.5), v(2.0, 0.0)).at(1.5), v(3.0, 0.5));
    }

    #[test]
    fn intersections_do_not_depend_on_scale() {
        // The same cross at world scale 1 and at 1e-4 (e.g. a world measured in km).
        for k in [1.0f32, 1e-4, 1e-8, 1e4] {
            let s = Segment2::new(v(0.0, 0.0), v(10.0 * k, 0.0));
            let o = Segment2::new(v(5.0 * k, -k), v(5.0 * k, k));
            let (t, u, _) = s.intersect(&o).unwrap_or_else(|| panic!("scale {k}"));
            assert!(
                (t - 0.5).abs() < 1e-6 && (u - 0.5).abs() < 1e-6,
                "scale {k}"
            );
            let r = Ray2::new(v(0.0, 0.0), v(k, 0.0));
            assert!(
                (r.cast_segment(&o, 10.0).unwrap() - 5.0).abs() < 1e-5,
                "scale {k}"
            );
            let c = Circle::new(v(5.0 * k, 0.0), k);
            assert!(
                (r.cast_circle(&c, 10.0).unwrap() - 4.0).abs() < 1e-4,
                "scale {k}"
            );
            let b = Aabb2::from_corners(v(4.0 * k, -k), v(6.0 * k, k));
            assert!(
                (r.cast_aabb(&b, 10.0).unwrap() - 4.0).abs() < 1e-5,
                "scale {k}"
            );
        }
    }

    #[test]
    fn degenerate_shapes_give_defined_answers() {
        let zero_ray = Ray2::new(v(0.0, 0.0), v(0.0, 0.0));
        let b = Aabb2::from_corners(v(2.0, 0.0), v(3.0, 1.0));
        let c = Circle::new(v(5.0, 0.0), 1.0);
        let wall = Segment2::new(v(7.0, -1.0), v(7.0, 1.0));
        assert_eq!(zero_ray.cast_aabb(&b, 10.0), None);
        assert_eq!(zero_ray.cast_circle(&c, 10.0), None);
        assert_eq!(zero_ray.cast_segment(&wall, 10.0), None);
        assert_eq!(
            Ray2::new(v(2.5, 0.5), v(0.0, 0.0)).cast_aabb(&b, 10.0),
            Some(0.0),
            "inside"
        );
        // Radius 0: a point. Hit exactly, miss next to it.
        let point = Circle::new(v(5.0, 0.0), 0.0);
        let r = Ray2::new(v(0.0, 0.0), v(1.0, 0.0));
        assert_eq!(r.cast_circle(&point, 10.0), Some(5.0));
        assert_eq!(
            Ray2::new(v(0.0, 0.1), v(1.0, 0.0)).cast_circle(&point, 10.0),
            None
        );
        assert!(point.contains(v(5.0, 0.0)) && !point.contains(v(5.0, 1e-3)));
        // Zero-length segments never cross anything and behave like a point otherwise.
        let dot = Segment2::new(v(7.0, 0.0), v(7.0, 0.0));
        assert_eq!(r.cast_segment(&dot, 10.0), None);
        assert_eq!(dot.intersect(&wall), None);
        assert_eq!(dot.distance_squared(v(7.0, 2.0)), 4.0);
        assert!(Capsule2::new(v(7.0, 0.0), v(7.0, 0.0), 1.0).contains(v(7.5, 0.0)));
        // Negative range: nothing is hit, not even from inside.
        assert_eq!(
            Ray2::new(v(2.5, 0.5), v(0.0, 0.0)).cast_aabb(&b, -1.0),
            None
        );
        assert_eq!(
            Ray2::new(v(2.5, 0.5), v(1.0, 0.0)).cast_aabb(&b, -1.0),
            None
        );
        assert_eq!(
            Ray2::new(v(5.0, 0.0), v(1.0, 0.0)).cast_circle(&c, -1.0),
            None
        );
        // No NaN leaves a cast.
        for ray in [
            Ray2::new(v(2.0, 0.5), v(1e-30, 1.0)),
            Ray2::new(v(2.0, 0.0), v(0.0, 1e-30)),
            Ray2::new(v(2.0, 0.0), v(f32::MIN_POSITIVE, 0.0)),
        ] {
            assert!(
                ray.cast_aabb(&b, f32::MAX).is_none_or(f32::is_finite),
                "{ray:?}"
            );
        }
    }
}
