//! Origin: chiaro @ 9bc5b9b, tests/unit/PatternGeometryTests.cpp (zlib license, see
//! `src/pattern_geometry.rs`): where a ray piece first enters a box, polygon, circle or tile
//! cell, the side crossed, `u` by the projection rules (same `u` from both sides of a thin
//! obstacle, continuous across a tile grid), the ray height there, and the fallback to the
//! nearest point when the piece does not cross.

use core::f32::consts::PI;
use knorke_light2d::pattern_geometry::{
    EntryHit, SIDE_X_HIGH, SIDE_X_LOW, SIDE_Y_HIGH, SIDE_Y_LOW, entry_box, entry_circle,
    entry_polygon, entry_tile_cell,
};
use knorke_math::Vec2;

const EDGE_X_HIGH: u32 = 2;
const EDGE_Y_LOW: u32 = 4;
const EDGE_Y_HIGH: u32 = 8;

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

fn finite(h: &EntryHit) -> bool {
    h.u.is_finite() && h.world_z.is_finite()
}

#[track_caller]
fn approx(a: f32, b: f32) {
    assert!((a - b).abs() <= 1e-5 * (1.0 + b.abs()), "{a} != {b}");
}

#[test]
fn a_thin_fence_shows_the_same_u_from_front_and_back() {
    let front = entry_box(2.0, 0.1, v(1.3, -1.0), v(1.3, 1.0), 0.0, 2.0);
    assert!(front.found);
    approx(front.u, 3.3);
    assert_eq!(front.side, SIDE_Y_LOW);
    approx(front.world_z, 0.9); // enters y = -0.1 at t 0.45
    let back = entry_box(2.0, 0.1, v(1.3, 1.0), v(1.3, -1.0), 0.0, 2.0);
    assert!(back.found);
    approx(back.u, 3.3);
    assert_eq!(back.side, SIDE_Y_HIGH);
    approx(back.world_z, 0.9);
}

#[test]
fn an_oblique_ray_through_the_narrow_side_uses_y_plus_half_d() {
    let h = entry_box(2.0, 0.1, v(3.0, 0.55), v(1.0, -0.45), 1.0, 3.0);
    assert!(h.found);
    assert_eq!(h.side, SIDE_X_HIGH);
    approx(h.u, 0.15);
    approx(h.world_z, 2.0);
    let low = entry_box(2.0, 0.1, v(-3.0, 0.05), v(-1.0, 0.05), 4.0, 0.0);
    assert!(low.found);
    assert_eq!(low.side, SIDE_X_LOW);
    approx(low.u, 0.15);
    approx(low.world_z, 2.0);
}

#[test]
fn a_box_without_crossing_falls_back_to_the_nearest_edge_point_of_b() {
    let inside = entry_box(2.0, 0.1, v(0.5, 0.0), v(1.0, 0.05), 0.0, 5.0);
    assert!(!inside.found && finite(&inside));
    assert_eq!(inside.side, SIDE_Y_HIGH);
    approx(inside.u, 3.0);
    approx(inside.world_z, 5.0);
    let miss = entry_box(2.0, 0.1, v(3.0, 1.0), v(2.5, 0.05), 0.0, 1.0);
    assert!(!miss.found && finite(&miss));
    assert_eq!(miss.side, SIDE_X_HIGH);
    approx(miss.u, 0.15);
    let point = entry_box(2.0, 0.1, v(0.0, 0.0), v(0.0, 0.0), 1.0, 1.0);
    assert!(!point.found && finite(&point));
}

#[test]
fn a_ray_exactly_through_a_box_corner_gives_a_finite_u() {
    let h = entry_box(1.0, 1.0, v(-2.0, -2.0), v(0.0, 0.0), 0.0, 2.0);
    assert!(h.found && finite(&h));
    approx(h.u, 0.0);
    approx(h.world_z, 1.0);
    let graze = entry_box(1.0, 1.0, v(-2.0, 1.0), v(0.0, 1.0), 0.0, 2.0);
    assert!(finite(&graze));
}

#[test]
fn parallel_opposite_polygon_edges_show_the_same_u_from_both_sides() {
    let poly = [v(0.0, 0.0), v(4.0, 2.0), v(4.0, 2.3), v(0.0, 0.3)];
    let n = v(-1.0, 2.0);
    let p = v(2.0, 1.0);
    let q = v(1.88, 1.24);
    let expected = 5.0 / 5.0f32.sqrt();
    let front = entry_polygon(&poly, p - n, p + n * 0.05, 0.0, 1.0);
    assert!(front.found);
    assert_eq!(front.side, 0);
    approx(front.u, expected);
    let back = entry_polygon(&poly, q + n, q - n * 0.05, 0.0, 1.0);
    assert!(back.found);
    assert_eq!(back.side, 2);
    approx(back.u, expected);
    approx(back.world_z, 1.0 / 1.05);
}

#[test]
fn the_winding_does_not_matter_and_the_first_entry_counts() {
    let ccw = [v(0.0, 0.0), v(2.0, 0.0), v(2.0, 1.0), v(0.0, 1.0)];
    let cw = [v(0.0, 0.0), v(0.0, 1.0), v(2.0, 1.0), v(2.0, 0.0)];
    for poly in [&ccw[..], &cw[..]] {
        let h = entry_polygon(poly, v(0.5, -1.0), v(0.5, 2.0), 0.0, 3.0);
        assert!(h.found);
        approx(h.u, 0.5);
        approx(h.world_z, 1.0);
    }
    let top = entry_polygon(&ccw, v(1.5, 2.0), v(1.5, 0.5), 0.0, 0.0);
    assert!(top.found);
    assert_eq!(top.side, 2);
    approx(top.u, 1.5);
    let left = entry_polygon(&ccw, v(-1.0, 0.25), v(0.5, 0.25), 0.0, 0.0);
    assert!(left.found);
    assert_eq!(left.side, 3);
    approx(left.u, 0.25);
}

#[test]
fn near_vertical_edges_whose_x_differs_only_by_rounding_agree() {
    let d = 1.0 / 16_777_216.0;
    let poly = [
        v(-0.0625 + d, -1.0),
        v(0.0625, -1.0),
        v(0.0625 + d, 1.0),
        v(-0.0625, 1.0),
    ];
    let right = entry_polygon(&poly, v(1.0, 0.5), v(0.03, 0.5), 0.0, 0.0);
    assert!(right.found);
    assert_eq!(right.side, 1);
    let left = entry_polygon(&poly, v(-1.0, 0.5), v(-0.03, 0.5), 0.0, 0.0);
    assert!(left.found);
    assert_eq!(left.side, 3);
    approx(right.u, 0.5);
    approx(left.u, 0.5);
}

#[test]
fn polygon_fallback_and_corners_stay_finite() {
    let poly = [v(0.0, 0.0), v(2.0, 0.0), v(2.0, 1.0), v(0.0, 1.0)];
    let inside = entry_polygon(&poly, v(0.5, 0.5), v(1.5, 0.9), 0.0, 2.0);
    assert!(!inside.found && finite(&inside));
    assert_eq!(inside.side, 2);
    approx(inside.u, 1.5);
    approx(inside.world_z, 2.0);
    let corner = entry_polygon(&poly, v(-1.0, -1.0), v(0.5, 0.5), 0.0, 1.0);
    assert!(corner.found && finite(&corner));
    approx(corner.u, 0.0);
}

#[test]
fn circle_u_is_the_arc_length_from_local_x_counter_clockwise() {
    let top = entry_circle(1.0, v(0.0, 3.0), v(0.0, 0.5), 0.0, 2.5);
    assert!(top.found);
    approx(top.u, PI / 2.0);
    approx(top.world_z, 2.0);
    assert_eq!(top.side, 0);
    let bottom = entry_circle(2.0, v(0.0, -3.0), v(0.0, -1.0), 0.0, 0.0);
    assert!(bottom.found);
    approx(bottom.u, 2.0 * 1.5 * PI);
    let inside = entry_circle(1.0, v(0.0, 0.0), v(-0.5, 0.0), 0.0, 1.0);
    assert!(!inside.found);
    approx(inside.u, PI);
    approx(inside.world_z, 1.0);
    let centre = entry_circle(1.0, v(0.0, 0.0), v(0.0, 0.0), 0.0, 1.0);
    assert!(!centre.found && finite(&centre));
    let tangent = entry_circle(1.0, v(-2.0, 1.0), v(2.0, 1.0), 0.0, 1.0);
    assert!(finite(&tangent));
}

#[test]
fn an_edge_strip_u_runs_continuously_across_the_grid() {
    let size = v(2.0, 2.0);
    let h = entry_tile_cell(
        size,
        5,
        2,
        EDGE_Y_LOW,
        0.25,
        v(10.5, 3.0),
        v(10.5, 4.1),
        0.0,
        1.1,
    );
    assert!(h.found);
    approx(h.u, 10.5);
    assert_eq!(h.side, SIDE_Y_LOW);
    approx(h.world_z, 1.0);
    let x = entry_tile_cell(
        size,
        5,
        2,
        EDGE_X_HIGH,
        0.25,
        v(11.9, 7.0),
        v(11.9, 5.5),
        0.0,
        0.0,
    );
    assert!(x.found);
    assert_eq!(x.side, SIDE_Y_HIGH);
    approx(x.u, 6.0);
    let far = entry_tile_cell(
        size,
        5,
        2,
        EDGE_Y_HIGH,
        0.25,
        v(11.0, 3.0),
        v(11.0, 5.9),
        0.0,
        0.0,
    );
    assert!(far.found);
    approx(far.u, 11.0);
}

#[test]
fn a_whole_cell_uses_the_coordinate_along_the_entered_side() {
    let size = v(2.0, 2.0);
    let side = entry_tile_cell(size, 5, 2, 0, 0.0, v(9.0, 5.3), v(10.5, 5.3), 0.0, 0.0);
    assert!(side.found);
    assert_eq!(side.side, SIDE_X_LOW);
    approx(side.u, 5.3);
    let top = entry_tile_cell(size, 5, 2, 0, 0.0, v(11.2, 8.0), v(11.2, 5.0), 0.0, 0.0);
    assert!(top.found);
    assert_eq!(top.side, SIDE_Y_HIGH);
    approx(top.u, 11.2);
}

#[test]
fn a_tile_cell_without_crossing_and_its_corners_stay_finite() {
    let size = v(2.0, 2.0);
    let inside = entry_tile_cell(size, 5, 2, 0, 0.0, v(11.0, 5.0), v(11.8, 5.0), 0.0, 3.0);
    assert!(!inside.found && finite(&inside));
    assert_eq!(inside.side, SIDE_X_HIGH);
    approx(inside.u, 5.0);
    approx(inside.world_z, 3.0);
    let strip = entry_tile_cell(
        size,
        5,
        2,
        EDGE_Y_LOW,
        0.25,
        v(10.5, 4.1),
        v(10.6, 4.2),
        0.0,
        0.0,
    );
    assert!(!strip.found && finite(&strip));
    approx(strip.u, 10.6);
    let corner = entry_tile_cell(size, 5, 2, 0, 0.0, v(9.0, 3.0), v(10.5, 4.5), 0.0, 0.0);
    assert!(corner.found && finite(&corner));
}
