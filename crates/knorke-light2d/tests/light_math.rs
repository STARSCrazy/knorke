//! Origin: chiaro @ 9bc5b9b, tests/unit/LightMathTests.cpp (zlib license, see `src/light_math.rs`).

// Tests compare against platform `std` maths as an oracle and use `f32::max` in tolerances;
// only library code must be platform independent (ADR 0011).
#![allow(clippy::disallowed_methods)]

use knorke_light2d::Projection;
use knorke_light2d::light_math::*;
use knorke_math::{At, Vec2, Vec3, World2};

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-5 * b.abs().max(1.0)
}

fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

fn world(x: f32, y: f32) -> At<World2, Vec2> {
    At::new(Vec2::new(x, y))
}

#[test]
fn attenuation_is_a_smooth_window_that_reaches_zero_at_the_radius() {
    assert!(approx(attenuation(0.0, 10.0, 2.0), 1.0));
    assert!(approx(attenuation(5.0, 10.0, 2.0), 0.5625));
    assert_eq!(attenuation(10.0, 10.0, 2.0), 0.0);
    assert_eq!(attenuation(20.0, 10.0, 2.0), 0.0);
    assert!(approx(attenuation(5.0, 10.0, 1.0), 0.75));
    assert_eq!(attenuation(1.0, 0.0, 1.0), 0.0);
}

#[test]
fn spot_factor_fades_between_inner_and_outer_cone() {
    let dir = v3(1., 0., 0.);
    let deg = std::f32::consts::PI / 180.0;
    let (cos_inner, cos_outer) = ((20.0 * deg).cos(), (30.0 * deg).cos());
    let a25 = 25.0 * deg;
    let between = v3(a25.cos(), a25.sin(), 0.0);
    assert!(approx(
        spot_factor(v3(5., 0., 0.), dir, cos_inner, cos_outer),
        1.0
    ));
    assert_eq!(spot_factor(v3(0., 5., 0.), dir, cos_inner, cos_outer), 0.0);
    assert_eq!(spot_factor(v3(-5., 0., 0.), dir, cos_inner, cos_outer), 0.0);
    let mid = spot_factor(between, dir, cos_inner, cos_outer);
    assert!(mid > 0.0 && mid < 1.0);
}

#[test]
fn flicker_noise_is_deterministic_and_bounded() {
    assert_eq!(flicker_noise(1.25, 8.0, 3), flicker_noise(1.25, 8.0, 3));
    assert_ne!(flicker_noise(1.25, 8.0, 3), flicker_noise(1.25, 8.0, 4));
    let mut sum = 0.0;
    for i in 0..1000 {
        let n = flicker_noise(f64::from(i) * 0.0137, 8.0, 11);
        assert!((0.0..=1.0).contains(&n));
        sum += f64::from(n);
    }
    assert!(sum / 1000.0 > 0.3 && sum / 1000.0 < 0.7);
}

#[test]
fn flicker_noise_hash_is_pinned() {
    // The splitmix64-based hash is part of the look of a scene: pin a few values (bits) so a
    // refactor cannot change it silently. Values computed independently (Python, float32 emulation of the C++ formula).
    let bits: Vec<u32> = [(0.0, 1.0, 0), (3.5, 2.0, 7), (-1.25, 8.0, 99)]
        .iter()
        .map(|&(t, s, seed)| flicker_noise(t, s, seed).to_bits())
        .collect();
    assert_eq!(bits, PINNED_FLICKER);
}
const PINNED_FLICKER: [u32; 3] = [0, 1_064_655_281, 1_062_201_759];

#[test]
fn scaled_buffer_sizes_round_up_without_float_noise() {
    assert_eq!(scaled_buffer_size(64, 0.5), 32);
    assert_eq!(scaled_buffer_size(33, 0.5), 17);
    assert_eq!(scaled_buffer_size(100, 0.3), 30); // 0.3f is slightly above 0.3
    assert_eq!(scaled_buffer_size(10, 0.1), 1);
    assert_eq!(scaled_buffer_size(320, 1.0), 320);
    assert_eq!(scaled_buffer_size(3, 0.25), 1);
}

#[test]
fn ground_radius_of_a_raised_light() {
    assert_eq!(ground_radius(0.0, 10.0), 10.0);
    assert!(approx(ground_radius(6.0, 10.0), 8.0));
    assert!(approx(ground_radius(-6.0, 10.0), 8.0));
    assert_eq!(ground_radius(10.0, 10.0), 0.0);
    assert_eq!(ground_radius(12.0, 10.0), 0.0);
}

#[test]
fn light_quads_are_culled_against_the_scene() {
    let p = Projection::IDENTITY;
    let hits = |x, y, r| quad_intersects_scene(&light_quad(&p, world(x, y), r), 100, 100);
    assert!(hits(-5.0, 50.0, 10.0)); // half outside
    assert!(!hits(-20.0, 50.0, 10.0));
    assert!(hits(150.0, 150.0, 60.0)); // corner overlap
    assert!(hits(50.0, 50.0, 500.0)); // covers everything
    let q = light_quad(&p, world(10.0, 20.0), 5.0);
    assert_eq!((q.corner[0].raw().x, q.corner[0].raw().y), (5.0, 15.0));
    assert_eq!((q.corner[3].raw().x, q.corner[3].raw().y), (15.0, 25.0));
}

#[test]
fn near_field_lambert_levels_off_near_a_wall_and_matches_a_point_far_away() {
    let n = v3(0., 0., 1.);
    // Far away the radius hardly matters.
    assert!((near_field_lambert(n, v3(0., 0., 100.), 1.0) - 1.0).abs() < 1e-3);
    // A light 0.1 above the surface: a point gives 1 right under it, the sphere of radius 0.3 only 0.1/sqrt(0.1).
    let close = v3(0., 0., 0.1);
    assert!(approx(near_field_lambert(n, close, 0.0), 1.0));
    assert!(approx(
        near_field_lambert(n, close, 0.3),
        0.1 / 0.1_f32.sqrt()
    ));
    // Monotonic away from the foot point: no ring.
    let mut last = 2.0;
    let mut x = 0.0;
    while x < 2.0 {
        let l = near_field_lambert(n, v3(x, 0., 0.1), 0.3);
        assert!(l <= last);
        last = l;
        x += 0.05;
    }
    assert_eq!(near_field_lambert(n, v3(0., 0., -1.), 0.3), 0.0);
}
