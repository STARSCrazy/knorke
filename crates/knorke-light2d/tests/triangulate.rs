//! Origin: chiaro @ 9bc5b9b, tests/unit/TriangulateTests.cpp (zlib license, see `src/triangulate.rs`).

// Tests compare against platform `std` maths as an oracle and use `f32::max` in tolerances;
// only library code must be platform independent (ADR 0011).
#![allow(clippy::disallowed_methods)]

use knorke_light2d::{TriangulateError, Triangulator, circle_triangles};
use knorke_math::Vec2;

fn pts(xy: &[f32]) -> Vec<Vec2> {
    xy.chunks(2).map(|c| Vec2::new(c[0], c[1])).collect()
}

fn area(t: &[Vec2], i: usize) -> f64 {
    let (a, b, c) = (t[i * 3], t[i * 3 + 1], t[i * 3 + 2]);
    let (ax, ay) = (f64::from(a.x), f64::from(a.y));
    let (bx, by) = (f64::from(b.x), f64::from(b.y));
    let (cx, cy) = (f64::from(c.x), f64::from(c.y));
    0.5 * ((bx - ax) * (cy - ay) - (cx - ax) * (by - ay))
}

fn total_area(t: &[Vec2]) -> f64 {
    (0..t.len() / 3).map(|i| area(t, i)).sum()
}

fn run(xy: &[f32]) -> Result<Vec<Vec2>, TriangulateError> {
    let mut out = Vec::new();
    Triangulator::new().triangulate(&pts(xy)[..], &mut out)?;
    Ok(out)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-5 * b.abs().max(1.0)
}

#[test]
fn square() {
    let t = run(&[0., 0., 4., 0., 4., 4., 0., 4.]).unwrap();
    assert_eq!(t.len(), 6); // two triangles
    assert!(near(total_area(&t), 16.0));
    let cw = run(&[0., 0., 0., 4., 4., 4., 4., 0.]).unwrap();
    assert!(near(total_area(&cw), 16.0));
}

fn check_l_shape(t: &[Vec2]) {
    assert!(near(total_area(t), 7.0));
    for i in 0..t.len() / 3 {
        let cx = (t[i * 3].x + t[i * 3 + 1].x + t[i * 3 + 2].x) / 3.0;
        let cy = (t[i * 3].y + t[i * 3 + 1].y + t[i * 3 + 2].y) / 3.0;
        assert!(!(cx > 1.0 && cy > 1.0));
        assert!(area(t, i) > 0.0);
    }
}

#[test]
fn concave_l_shape() {
    check_l_shape(&run(&[0., 0., 4., 0., 4., 1., 1., 1., 1., 4., 0., 4.]).unwrap());
}

#[test]
fn clockwise_l_shape() {
    check_l_shape(&run(&[0., 0., 0., 4., 1., 4., 1., 1., 4., 1., 4., 0.]).unwrap());
}

#[test]
fn duplicate_and_collinear_points_are_removed() {
    let t = run(&[0., 0., 0., 0., 2., 0., 4., 0., 4., 4., 0., 4.]).unwrap();
    assert!(near(total_area(&t), 16.0));
    assert_eq!(t.len(), 6);
}

#[test]
fn degenerate_input_fails_and_leaves_the_output_alone() {
    let mut tri = Triangulator::new();
    let mut out = pts(&[1., 2.]);
    let before = out.clone();
    let mut fails = |xy: &[f32]| {
        let e = tri.triangulate(&pts(xy)[..], &mut out).unwrap_err();
        assert_eq!(out, before);
        e
    };
    assert_eq!(
        fails(&[0., 0., 4., 4., 4., 0., 0., 4.]),
        TriangulateError::NoArea
    );
    assert_eq!(
        fails(&[0., 0., 1., 1., 2., 2.]),
        TriangulateError::TooFewVertices
    );
    assert_eq!(
        fails(&[
            0., 0., 4., 0., 1., 3., 3., 3., 4., 4., 0., 4., 3., 1., 1., 1.
        ]),
        TriangulateError::SelfIntersecting
    );
    assert_eq!(fails(&[0., 0., 1., 1.]), TriangulateError::TooFewVertices);
    assert_eq!(
        fails(&[0., 0., 0.001, 0., 0., 0.001]),
        TriangulateError::NoArea
    );
    assert_eq!(
        fails(&[0., 0., f32::NAN, 1., 2., 2.]),
        TriangulateError::NotFinite
    );
}

#[test]
fn zero_area_figure_eight_hits_the_area_check() {
    // Neighbours are not collinear, so cleaning keeps all 4 points; the two lobes cancel to area 0.
    assert_eq!(
        run(&[0., 0., 4., 4., 4., 0., 0., 4.]),
        Err(TriangulateError::NoArea)
    );
}

#[test]
fn plain_x_crossing_with_distinct_vertices() {
    assert_eq!(
        run(&[0., 0., 4., 4., 4., 0., 0., 2.]),
        Err(TriangulateError::SelfIntersecting)
    );
}

#[test]
fn vertex_touching_a_non_adjacent_edge() {
    assert_eq!(
        run(&[0., 0., 4., 0., 4., 4., 2., 0., 0., 4.]),
        Err(TriangulateError::SelfIntersecting)
    );
}

#[test]
fn pinched_polygon_with_coinciding_non_adjacent_vertices() {
    assert_eq!(
        run(&[0., 0., 2., 0., 1., 1., 2., 2., 0., 2., 1., 1.]),
        Err(TriangulateError::SelfIntersecting)
    );
}

#[test]
fn error_messages_name_the_cause_and_have_stable_codes() {
    assert!(TriangulateError::NoArea.to_string().contains("no area"));
    assert!(
        TriangulateError::SelfIntersecting
            .to_string()
            .contains("self-intersecting")
    );
    assert_eq!(
        TriangulateError::SelfIntersecting
            .to_diagnostic()
            .code
            .as_str(),
        "knorke.light2d.polygon.self_intersecting"
    );
}

#[test]
fn circle_triangles_form_a_fan_of_48() {
    let mut t = Vec::new();
    circle_triangles(10.0, &mut t);
    assert_eq!(t.len(), 48 * 3);
    let full = std::f64::consts::PI * 100.0;
    assert!(total_area(&t) >= 0.98 * full);
    assert!(total_area(&t) <= full);
}

#[test]
fn a_warm_triangulator_reuses_its_buffers() {
    let poly = pts(&[0., 0., 4., 0., 4., 1., 1., 1., 1., 4., 0., 4.]);
    let mut tri = Triangulator::new();
    let mut out = Vec::new();
    tri.triangulate(&poly[..], &mut out).unwrap();
    let cap = out.capacity();
    let first = out.clone();
    for _ in 0..3 {
        tri.triangulate(&poly[..], &mut out).unwrap();
    }
    assert_eq!(out.capacity(), cap);
    assert_eq!(out, first);
}
