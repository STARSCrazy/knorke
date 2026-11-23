//! Origin: chiaro @ 9bc5b9b, tests/unit/ProjectionTests.cpp (zlib license, see `src/projection.rs`).
//! Ported to the Rust API: typed coordinate spaces, errors instead of SDL_GetError.

// Tests compare against platform `std` maths as an oracle and use `f32::max` in tolerances;
// only library code must be platform independent (ADR 0011).
#![allow(clippy::disallowed_methods)]

use knorke_light2d::{Projection, ProjectionError};
use knorke_math::{At, Vec2, World2};

fn world(x: f32, y: f32) -> At<World2, Vec2> {
    At::new(Vec2::new(x, y))
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-5 * b.abs().max(1.0)
}

#[test]
fn identity_projection_maps_world_to_scene_pixels_and_ignores_height() {
    let p = Projection::IDENTITY;
    assert_eq!(p.validate(), Ok(()));
    let s = p.project(world(3.0, 4.0), 9.0).raw();
    assert_eq!((s.x, s.y), (3.0, 4.0));
    assert_eq!(Projection::default(), Projection::IDENTITY);
}

#[test]
fn ortho_projection_centers_the_camera_and_applies_zoom() {
    let p = Projection::ortho(world(100.0, 50.0), 2.0, 320, 180);
    assert_eq!(p.validate(), Ok(()));
    let center = p.project(world(100.0, 50.0), 0.0).raw();
    assert!(approx(center.x, 160.0) && approx(center.y, 90.0));
    let right = p.project(world(101.0, 50.0), 0.0).raw();
    assert!(approx(right.x, 162.0) && approx(right.y, 90.0));
}

#[test]
fn singular_or_non_finite_projections_are_rejected() {
    let mut p = Projection::IDENTITY;
    p.x_axis = Vec2::new(1.0, 1.0);
    p.y_axis = Vec2::new(2.0, 2.0); // collinear axes
    assert_eq!(p.validate(), Err(ProjectionError::CollinearAxes));

    let mut p = Projection::IDENTITY;
    p.origin = At::new(Vec2::new(f32::NAN, 0.0));
    assert_eq!(p.validate(), Err(ProjectionError::NotFinite));
    let code = p.validate().unwrap_err().to_diagnostic().code;
    assert!(code.as_str().starts_with("knorke.light2d.projection."));
}

#[test]
fn iso_projection_is_a_2_to_1_dimetric_view_centred_on_the_camera() {
    let p = Projection::iso(world(0.0, 0.0), 2.0, 1.0, 200, 100);
    assert_eq!(p.validate(), Ok(()));
    let at = |x, y, z| p.project(world(x, y), z).raw();
    let o = at(0.0, 0.0, 0.0);
    assert!(approx(o.x, 100.0) && approx(o.y, 50.0));
    let x = at(1.0, 0.0, 0.0);
    assert!(approx(x.x, 102.0) && approx(x.y, 51.0));
    let y = at(0.0, 1.0, 0.0);
    assert!(approx(y.x, 98.0) && approx(y.y, 51.0));
    let z = at(0.0, 0.0, 1.0);
    assert!(approx(z.x, 100.0) && approx(z.y, 48.0));

    let p = Projection::iso(world(10.0, -4.0), 3.0, 1.5, 320, 180);
    let cam = p.project(world(10.0, -4.0), 0.0).raw();
    assert!(approx(cam.x, 160.0) && approx(cam.y, 90.0));
    assert!(approx(p.z_axis.y, -4.5));
}

#[test]
fn ground_transform_agrees_with_project_at_height_zero() {
    let p = Projection::iso(world(1.0, 2.0), 2.0, 1.0, 64, 64);
    let t = p.ground_transform();
    for (x, y) in [(0.0, 0.0), (3.0, -2.0), (-7.5, 4.25)] {
        let a = t.point(world(x, y)).raw();
        let b = p.project(world(x, y), 0.0).raw();
        assert!(approx(a.x, b.x) && approx(a.y, b.y));
    }
}
