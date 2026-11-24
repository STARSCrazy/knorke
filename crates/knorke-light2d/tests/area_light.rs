//! Origin: chiaro @ 9bc5b9b, tests/unit/AreaLightTests.cpp (zlib license, see `src/area_light.rs`),
//! plus a test for the brightness normalization (E51), which the C++ unit tests only cover through
//! the GPU tests.

// Tests compare against platform `std` maths as an oracle; only library code must be platform
// independent (ADR 0011).
#![allow(clippy::disallowed_methods)]

use knorke_light2d::area_light::*;
use knorke_light2d::light_math::attenuation;
use knorke_math::Vec3;

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-5 * b.abs().max(1.0)
}

fn near_eps(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps * b.abs().max(1.0)
}

fn line(c: Vec3, u: Vec3) -> AreaEmitter {
    AreaEmitter {
        kind: AreaKind::Line,
        center: c,
        u,
        v: Vec3::ZERO,
        normal: Vec3::ZERO,
    }
}

fn rect(c: Vec3, n: Vec3, u: Vec3, half_width: f32, two_sided: bool) -> AreaEmitter {
    AreaEmitter {
        kind: if two_sided {
            AreaKind::RectTwoSided
        } else {
            AreaKind::Rect
        },
        center: c,
        u,
        v: rect_across(n, u, half_width),
        normal: n,
    }
}

fn nearest(e: &AreaEmitter, w: Vec3) -> Vec3 {
    nearest_emitter_point(e, w).point
}

#[test]
fn the_nearest_point_clamps_to_the_segment_and_the_rectangle() {
    let l = line(v(0.0, 0.0, 5.0), v(4.0, 0.0, 0.0));
    let mut p = nearest_emitter_point(&l, v(1.0, 3.0, 0.0));
    assert!(near(p.point.x, 1.0));
    assert!(near(p.point.z, 5.0));
    assert!(near(p.t, 0.25));
    p = nearest_emitter_point(&l, v(10.0, 0.0, 0.0));
    assert!(near(p.point.x, 4.0));
    assert!(near(p.t, 1.0));

    // A window in the plane x = 0 facing +x, 2 wide along y and 1 high along z.
    let r = rect(
        v(0.0, 0.0, 3.0),
        v(1.0, 0.0, 0.0),
        v(0.0, 1.0, 0.0),
        0.5,
        false,
    );
    assert!(near(r.v.z, 0.5)); // n x u = x x y = z
    p = nearest_emitter_point(&r, v(5.0, 0.5, 10.0));
    assert!(near(p.point.x, 0.0));
    assert!(near(p.point.y, 0.5));
    assert!(near(p.point.z, 3.5));
    assert!(near(p.s, 1.0));
}

#[test]
fn a_zero_size_emitter_is_a_point_light() {
    let c = v(2.0, 3.0, 4.0);
    let w = v(7.0, -1.0, 0.0);
    let d = (25.0f32 + 16.0 + 16.0).sqrt();
    // A line of length (almost) zero.
    let l = line(c, v(1e-6, 0.0, 0.0));
    let reach = area_reach(&l, 20.0, 2.0, w);
    assert!(near(reach.reach, attenuation(d, 20.0, 2.0)));
    assert!(near(reach.to_light.x, -5.0 / d));
    let n = v(0.0, 0.0, 1.0);
    assert!(near(area_lambert(&l, n, w, c), 4.0 / d));
}

#[test]
fn reach_is_measured_from_the_nearest_point_of_a_line() {
    let l = line(v(0.0, 0.0, 0.0), v(10.0, 0.0, 0.0));
    let a = area_reach(&l, 8.0, 2.0, v(-7.0, 4.0, 0.0));
    assert!(near(a.reach, attenuation(4.0, 8.0, 2.0)));
    assert!(near(a.to_light.y, -1.0));
    assert!(near(
        area_reach(&l, 8.0, 2.0, v(13.0, 4.0, 0.0)).reach,
        attenuation(5.0, 8.0, 2.0)
    ));
    assert_eq!(area_reach(&l, 8.0, 2.0, v(0.0, 8.0, 0.0)).reach, 0.0);
    // On the emitter: full reach, no direction.
    let on = area_reach(&l, 8.0, 2.0, v(3.0, 0.0, 0.0));
    assert!(near(on.reach, 1.0));
    assert_eq!((on.to_light.x, on.to_light.y), (0.0, 0.0));
}

#[test]
fn a_one_sided_rectangle_is_dark_behind_and_in_its_plane_a_two_sided_one_is_not() {
    let front = rect(
        v(0.0, 0.0, 3.0),
        v(1.0, 0.0, 0.0),
        v(0.0, 2.0, 0.0),
        1.0,
        false,
    );
    let both = rect(
        v(0.0, 0.0, 3.0),
        v(1.0, 0.0, 0.0),
        v(0.0, 2.0, 0.0),
        1.0,
        true,
    );
    let at = attenuation(4.0, 10.0, 2.0);
    assert!(near(
        area_reach(&front, 10.0, 2.0, v(4.0, 1.0, 3.0)).reach,
        at
    ));
    assert_eq!(area_reach(&front, 10.0, 2.0, v(-4.0, 1.0, 3.0)).reach, 0.0);
    assert!(near(
        area_reach(&both, 10.0, 2.0, v(-4.0, 1.0, 3.0)).reach,
        at
    ));
    assert!(area_reach(&front, 10.0, 2.0, v(0.0, 5.0, 3.0)).reach.abs() < 1e-6);
    // At 45 degrees beside the edge: the emitter cosine.
    let r = area_reach(&front, 10.0, 2.0, v(3.0, 5.0, 3.0)).reach;
    assert!(near(
        r,
        attenuation(18.0f32.sqrt(), 10.0, 2.0) * 0.5f32.sqrt()
    ));
}

#[test]
fn the_receiver_lambert_term_weights_the_nearest_point_and_the_ends_or_corners_by_one_over_d_squared()
 {
    // A floor point below the left end of a tube at height 2 (ends at x = 0 and x = 8): the
    // nearest point and the left end are straight above (cos 1, d^2 4), the far end at (8, 0, 2)
    // has cos 2 / sqrt(68) at d^2 68.
    let l = line(v(4.0, 0.0, 2.0), v(4.0, 0.0, 0.0));
    let up = v(0.0, 0.0, 1.0);
    let w = Vec3::ZERO;
    let far = 1.0 / 68.0;
    assert!(near(
        area_lambert(&l, up, w, nearest(&l, w)),
        (0.5 + far * 2.0 / 68.0f32.sqrt()) / (0.5 + far)
    ));
    // A wall facing +x at x = 0 sees only the half of the tube in front of it.
    let east = v(1.0, 0.0, 0.0);
    assert!(near(
        area_lambert(&l, east, w, nearest(&l, w)),
        far * 8.0 / 68.0f32.sqrt() / (0.5 + far)
    ));
    // A rectangle straight above a floor point: the nearest point at d^2 1, four corners at
    // d^2 3 with cos 1/sqrt(3).
    let r = rect(
        v(0.0, 0.0, 1.0),
        v(0.0, 0.0, -1.0),
        v(1.0, 0.0, 0.0),
        1.0,
        false,
    );
    let corner = 1.0 / 3.0f32.sqrt();
    assert!(near(
        area_lambert(&r, up, w, nearest(&r, w)),
        (1.0 + 4.0 / 3.0 * corner) / (1.0 + 4.0 / 3.0)
    ));
}

#[test]
fn near_a_large_emitter_the_receiver_lambert_term_is_that_of_the_nearest_point() {
    // A 10 m x 6 m billboard facing -y and a wall 5 cm in front of it facing it: the corners lie
    // almost in the wall's plane, but the emitter fills the wall's view, so the term is close to
    // the nearest point's 1.
    let r = rect(
        v(0.0, 0.0, 3.0),
        v(0.0, -1.0, 0.0),
        v(5.0, 0.0, 0.0),
        3.0,
        false,
    );
    let toward = v(0.0, 1.0, 0.0);
    for w in [v(0.0, -0.05, 3.0), v(4.0, -0.05, 1.0), v(-4.5, -0.05, 5.5)] {
        assert!(area_lambert(&r, toward, w, nearest(&r, w)) > 0.99);
    }
    // Facing along the board instead: the nearest point lies in its plane, so (almost) nothing.
    let along = v(1.0, 0.0, 0.0);
    let w = v(0.0, -0.05, 3.0);
    assert!(area_lambert(&r, along, w, nearest(&r, w)) < 0.01);
    // The same for a tube: a floor point 5 cm below the middle of a 20 m tube.
    let l = line(v(0.0, 0.0, 0.05), v(10.0, 0.0, 0.0));
    assert!(area_lambert(&l, v(0.0, 0.0, 1.0), Vec3::ZERO, v(0.0, 0.0, 0.05)) > 0.99);
}

#[test]
fn far_from_the_emitter_the_receiver_lambert_term_is_the_point_lights() {
    // A 2 m emitter 1 km away: every sample has nearly the direction of the centre.
    let n = v(0.5f32.sqrt(), 0.0, 0.5f32.sqrt());
    let w = Vec3::ZERO;
    let point = 0.5f32.sqrt(); // n . (0, 0, 1)
    let r = rect(
        v(0.0, 0.0, 1000.0),
        v(0.0, 0.0, -1.0),
        v(1.0, 0.0, 0.0),
        1.0,
        false,
    );
    assert!(near_eps(
        area_lambert(&r, n, w, nearest(&r, w)),
        point,
        1e-3
    ));
    let l = line(v(0.0, 0.0, 1000.0), v(1.0, 0.0, 0.0));
    assert!(near_eps(
        area_lambert(&l, n, w, nearest(&l, w)),
        point,
        1e-3
    ));
}

#[test]
fn the_specular_point_follows_the_reflected_ray() {
    // A ceiling tube along x at height 4; the viewer straight above a floor point at x = 0. The
    // reflected ray goes straight up and meets the tube above the pixel.
    let l = line(v(0.0, 0.0, 4.0), v(10.0, 0.0, 0.0));
    let up = v(0.0, 0.0, 1.0);
    let mut q = area_specular_point(&l, v(3.0, 2.0, 0.0), up, up, v(3.0, 0.0, 4.0));
    assert!(near(q.x, 3.0));
    assert!(q.y.abs() < 1e-5);
    assert!(near(q.z, 4.0));
    // A tilted view (towards +x) moves the point to the far side, the end clamps it.
    let view = v(-0.6, 0.0, 0.8);
    q = area_specular_point(&l, Vec3::ZERO, up, view, v(0.0, 0.0, 4.0));
    assert!(near(q.x, 3.0)); // reflected ray (0.6, 0, 0.8) reaches z = 4 at x = 3
    q = area_specular_point(&l, v(9.0, 0.0, 0.0), up, view, v(9.0, 0.0, 4.0));
    assert!(near(q.x, 10.0));

    // A rectangle on the ceiling facing down: the hit of the reflected ray, clamped into it.
    let r = rect(
        v(0.0, 0.0, 4.0),
        v(0.0, 0.0, -1.0),
        v(2.0, 0.0, 0.0),
        2.0,
        false,
    );
    q = area_specular_point(&r, Vec3::ZERO, up, view, v(0.0, 0.0, 4.0));
    assert!(near(q.x, 2.0));
    assert!(near(q.z, 4.0));
    // The ray going away from the plane: the nearest point.
    let down = v(0.0, 0.0, -1.0);
    q = area_specular_point(&r, Vec3::ZERO, down, down, v(0.0, 0.0, 4.0));
    assert!(near(q.z, 4.0));
    assert!(q.x.abs() < 1e-6);
}

#[test]
fn the_normalization_makes_an_area_light_as_bright_as_a_point_light_in_front_of_it() {
    // A rect facing +x seen from the middle of its reach (radius / 2) in front: 1 / lambert,
    // between 1 and 4.
    let r = rect(
        v(0.0, 0.0, 3.0),
        v(1.0, 0.0, 0.0),
        v(0.0, 2.0, 0.0),
        1.0,
        false,
    );
    let n = area_normalization(&r, 20.0);
    let front = v(10.0, 0.0, 3.0);
    let lambert = area_lambert(&r, v(-1.0, 0.0, 0.0), front, r.center);
    assert!(near(n, 1.0 / lambert));
    assert!((1.0..=4.0).contains(&n));
    // A tiny emitter is a point light: factor 1.
    let tiny = line(v(0.0, 0.0, 3.0), v(1e-6, 0.0, 0.0));
    assert!(near(area_normalization(&tiny, 20.0), 1.0));
    // A huge one in front of the receiver is nearly a plane: the factor stays in [1, 4].
    let huge = rect(
        v(0.0, 0.0, 3.0),
        v(1.0, 0.0, 0.0),
        v(0.0, 500.0, 0.0),
        500.0,
        false,
    );
    assert!((1.0..=4.0).contains(&area_normalization(&huge, 20.0)));
    // A line seen from above (the receiver straight below its middle) has a Lambert term below
    // 1, so the factor lifts it to a point light's brightness.
    let tube = line(v(0.0, 0.0, 3.0), v(8.0, 0.0, 0.0));
    let n = area_normalization(&tube, 8.0);
    assert!(n > 1.0 && n <= 4.0, "{n}");
}
