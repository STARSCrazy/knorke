//! Ambient zone maths, validation, scene rectangle and culling (chiaro AmbientZoneTests, ported).

// Origin: chiaro @ 9bc5b9b, tests/unit/AmbientZoneTests.cpp (zlib license, see `src/ambient_zone.rs`).
//
// Ported test by test with the same inputs, expected values and tolerances (doctest `Approx`
// semantics: |a - b| < eps * (1 + max(|a|, |b|)), default eps = 100 * f32::EPSILON).
// Not ported as tests: the C enum and flag-bit cases (`type` out of range, unknown `flags` bits,
// a NULL vertex pointer) because the Rust types cannot express them; the C function names in the
// error texts are gone. The culling test at the end is new (chiaro has no unit test for
// AmbientZoneCulling.cpp).

// Test oracle maths (`std` trig and `f32::max`), as in the C++ original.
#![allow(clippy::disallowed_methods)]

use knorke_light2d::ambient_zone::{
    AmbientZoneParams, GPU_ZONE_BYTES, GpuZone, StoredZone, ZoneShape, fold_ambient,
    make_stored_zone, validate_zone, zone_edge_weight, zone_height_contains, zone_scene_rect,
    zone_signed_distance, zone_weight,
};
use knorke_light2d::ambient_zone_culling::{AmbientZoneList, CullScene, collect_ambient_zones};
use knorke_light2d::ambient_zone_store::AmbientZoneStore;
use knorke_light2d::projection::Projection;
use knorke_math::{At, Vec2, Vec3, World2};

fn approx_eps(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() < eps * (1.0 + a.abs().max(b.abs()))
}

fn approx(a: f32, b: f32) -> bool {
    approx_eps(a, b, 100.0 * f32::EPSILON)
}

fn pts(flat: &[f32]) -> Vec<Vec2> {
    flat.chunks(2).map(|c| Vec2::new(c[0], c[1])).collect()
}

fn bx(x: f32, y: f32, w: f32, d: f32) -> AmbientZoneParams {
    AmbientZoneParams {
        shape: ZoneShape::Box,
        x,
        y,
        width: w,
        depth: d,
        ..AmbientZoneParams::default()
    }
}

fn bx_col(x: f32, y: f32, w: f32, d: f32, r: f32, g: f32, b: f32) -> AmbientZoneParams {
    AmbientZoneParams {
        ambient: [r, g, b],
        ..bx(x, y, w, d)
    }
}

fn circle(x: f32, y: f32, radius: f32) -> AmbientZoneParams {
    AmbientZoneParams {
        shape: ZoneShape::Circle,
        x,
        y,
        radius,
        ..AmbientZoneParams::default()
    }
}

fn polygon(flat: &[f32]) -> AmbientZoneParams {
    AmbientZoneParams {
        shape: ZoneShape::Polygon,
        vertices: pts(flat),
        ..AmbientZoneParams::default()
    }
}

fn stored(p: &AmbientZoneParams) -> StoredZone {
    make_stored_zone(p).unwrap_or_else(|e| panic!("{e}"))
}

/// The L: (0,0)-(4,0)-(4,1)-(1,1)-(1,4)-(0,4); its notch (1..4 x 1..4) is outside.
const L: [f32; 12] = [0.0, 0.0, 4.0, 0.0, 4.0, 1.0, 1.0, 1.0, 1.0, 4.0, 0.0, 4.0];

const WHITE: [f32; 3] = [1.0, 1.0, 1.0];

fn ortho() -> Projection {
    Projection {
        origin: At::new(Vec2::ZERO),
        x_axis: Vec2::new(1.0, 0.0),
        y_axis: Vec2::new(0.0, 1.0),
        z_axis: Vec2::ZERO,
    }
}

fn iso(zoom: f32, height_scale: f32, w: u32, h: u32) -> Projection {
    Projection::iso(
        At::<World2, Vec2>::new(Vec2::ZERO),
        zoom,
        height_scale,
        w,
        h,
    )
}

#[test]
fn ambient_zone_signed_distance_of_a_circle() {
    let z = stored(&circle(0.0, 0.0, 2.0));
    assert!(approx(zone_signed_distance(&z, 0.0, 0.0), 2.0));
    assert!(approx(zone_signed_distance(&z, 2.0, 0.0), 0.0));
    assert!(approx(zone_signed_distance(&z, 3.0, 0.0), -1.0));
}

#[test]
fn ambient_zone_signed_distance_of_a_rotated_box() {
    let mut p = bx(5.0, -3.0, 4.0, 2.0);
    p.rotation = 30.0;
    let z = stored(&p);
    let rad = 30.0f32 * core::f32::consts::PI / 180.0;
    // On the long axis 1.9 from the centre: 0.1 inside the short end.
    assert!(approx_eps(
        zone_signed_distance(&z, 5.0 + 1.9 * rad.cos(), -3.0 + 1.9 * rad.sin()),
        0.1,
        1e-4
    ));
    // On the short axis 0.5 from the centre: 0.5 inside the long side.
    assert!(approx_eps(
        zone_signed_distance(&z, 5.0 - 0.5 * rad.sin(), -3.0 + 0.5 * rad.cos()),
        0.5,
        1e-4
    ));
    // Beyond a corner: the distance to the corner, negative.
    let cx = 5.0 + rad.cos() * 3.0 - rad.sin() * 2.0;
    let cy = -3.0 + rad.sin() * 3.0 + rad.cos() * 2.0;
    assert!(approx_eps(
        zone_signed_distance(&z, cx, cy),
        -(2.0f32).sqrt(),
        1e-4
    ));
    assert!(approx(zone_signed_distance(&z, 5.0, -3.0), 1.0));
}

#[test]
fn ambient_zone_signed_distance_of_convex_and_concave_polygons() {
    let square = [0.0f32, 0.0, 0.0, 2.0, 2.0, 2.0, 2.0, 0.0]; // clockwise
    let sq = stored(&polygon(&square));
    assert!(approx(zone_signed_distance(&sq, 1.0, 1.0), 1.0));
    assert!(approx(zone_signed_distance(&sq, 3.0, 1.0), -1.0));

    let l = stored(&polygon(&L));
    assert!(zone_signed_distance(&l, 0.5, 0.5) > 0.0);
    assert!(zone_signed_distance(&l, 2.0, 2.0) < 0.0); // the notch is outside
    assert!(approx(zone_signed_distance(&l, 2.0, 2.0), -1.0));
    assert!(approx(zone_signed_distance(&l, 0.5, 2.0), 0.5));

    // Moved and turned: the vertices are local units about (x, y).
    let mut p = polygon(&square);
    p.x = 10.0;
    p.rotation = 90.0;
    let turned = stored(&p);
    assert!(approx_eps(
        zone_signed_distance(&turned, 9.0, 1.0),
        1.0,
        1e-4
    ));
    assert!(zone_signed_distance(&turned, 11.0, 1.0) < 0.0);
}

#[test]
fn ambient_zone_soft_edge_inside_the_outline() {
    assert_eq!(zone_edge_weight(0.0, 1.0), 0.0);
    assert!(approx(zone_edge_weight(0.5, 1.0), 0.5));
    assert_eq!(zone_edge_weight(1.0, 1.0), 1.0);
    assert_eq!(zone_edge_weight(3.0, 1.0), 1.0);
    assert_eq!(zone_edge_weight(-0.1, 1.0), 0.0);
    assert!(approx(zone_edge_weight(0.25, 1.0), 0.15625)); // smoothstep
    // soft_edge 0 is a step; the outline itself counts as inside.
    assert_eq!(zone_edge_weight(0.0, 0.0), 1.0);
    assert_eq!(zone_edge_weight(1e-6, 0.0), 1.0);
    assert_eq!(zone_edge_weight(-1e-6, 0.0), 0.0);
}

#[test]
fn ambient_zone_the_height_span_is_half_open() {
    let mut p = bx(0.0, 0.0, 4.0, 4.0);
    p.height = Some(128.0);
    let z = stored(&p);
    assert!(zone_height_contains(&z, 0.0));
    assert!(zone_height_contains(&z, 120.0));
    assert!(!zone_height_contains(&z, 128.0));
    assert_eq!(zone_weight(&z, Vec3::new(0.0, 0.0, 120.0), true), 1.0);
    assert_eq!(zone_weight(&z, Vec3::new(0.0, 0.0, 128.0), true), 0.0);
    // without a surface buffer: no span
    assert_eq!(zone_weight(&z, Vec3::new(0.0, 0.0, 128.0), false), 1.0);

    p.height = None;
    p.base_height = 128.0;
    let upper = stored(&p);
    assert!(!zone_height_contains(&upper, 120.0));
    assert!(zone_height_contains(&upper, 128.0));
    assert!(zone_height_contains(&upper, 1.0e9));
    assert_eq!(zone_weight(&upper, Vec3::new(0.0, 0.0, 0.0), true), 0.0);
    assert_eq!(zone_weight(&upper, Vec3::new(0.0, 0.0, 0.0), false), 1.0);
}

#[test]
fn ambient_zone_fold_order_relative_and_disabled_zones() {
    let global = [0.2f32, 0.4, 0.6];
    let at = Vec3::new(0.0, 0.0, 0.0);

    // higher priority wins whatever the creation order
    {
        let mut store = AmbientZoneStore::new();
        let mut high = bx_col(0.0, 0.0, 4.0, 4.0, 0.9, 0.9, 0.9);
        high.priority = 1;
        let low = bx_col(0.0, 0.0, 4.0, 4.0, 0.1, 0.1, 0.1);
        store.create(stored(&high)).unwrap();
        store.create(stored(&low)).unwrap();
        assert!(approx(fold_ambient(&store, global, at, true).x, 0.9));
    }
    // equal priority: the later created wins, Set keeps the order, recreating moves to the back
    {
        let mut store = AmbientZoneStore::new();
        let first = store
            .create(stored(&bx_col(0.0, 0.0, 4.0, 4.0, 0.1, 0.1, 0.1)))
            .unwrap();
        let second = store
            .create(stored(&bx_col(0.0, 0.0, 4.0, 4.0, 0.3, 0.3, 0.3)))
            .unwrap();
        assert!(approx(fold_ambient(&store, global, at, true).x, 0.3));
        store
            .set(first, stored(&bx_col(0.0, 0.0, 4.0, 4.0, 0.5, 0.5, 0.5)))
            .unwrap();
        assert!(approx(fold_ambient(&store, global, at, true).x, 0.3)); // Set does not move it up
        store.destroy(first).unwrap();
        store
            .create(stored(&bx_col(0.0, 0.0, 4.0, 4.0, 0.7, 0.7, 0.7)))
            .unwrap();
        assert!(approx(fold_ambient(&store, global, at, true).x, 0.7)); // new creation number: last
        let _ = second;
    }
    // relative zones multiply the global ambient
    {
        let mut store = AmbientZoneStore::new();
        let mut p = bx_col(0.0, 0.0, 4.0, 4.0, 0.5, 0.5, 0.5);
        p.relative = true;
        store.create(stored(&p)).unwrap();
        let a = fold_ambient(&store, global, at, true);
        assert!(approx(a.x, 0.1));
        assert!(approx(a.y, 0.2));
        assert!(approx(a.z, 0.3));
    }
    // disabled zones and points outside keep the global ambient
    {
        let mut store = AmbientZoneStore::new();
        let mut p = bx_col(0.0, 0.0, 4.0, 4.0, 0.0, 0.0, 0.0);
        p.disabled = true;
        let disabled = stored(&p);
        assert_eq!(zone_weight(&disabled, at, true), 0.0);
        store.create(disabled).unwrap();
        assert!(approx(fold_ambient(&store, global, at, true).y, 0.4));
        store
            .create(stored(&bx_col(0.0, 0.0, 4.0, 4.0, 0.0, 0.0, 0.0)))
            .unwrap();
        assert!(approx(
            fold_ambient(&store, global, Vec3::new(3.0, 0.0, 0.0), true).y,
            0.4
        ));
        assert_eq!(fold_ambient(&store, global, at, true).y, 0.0);
    }
    // the soft edge blends linearly between the colours
    {
        let mut store = AmbientZoneStore::new();
        let mut p = bx_col(0.0, 0.0, 8.0, 8.0, 1.0, 1.0, 1.0);
        p.soft_edge = 2.0;
        store.create(stored(&p)).unwrap();
        // 1 inside the edge of width 2: weight 0.5.
        let a = fold_ambient(&store, global, Vec3::new(3.0, 0.0, 0.0), true);
        assert!(approx(a.x, 0.2 + 0.5 * (1.0 - 0.2)));
        assert!(approx(a.z, 0.6 + 0.5 * (1.0 - 0.6)));
    }
}

#[test]
fn ambient_zone_scene_rectangle() {
    let o = ortho();
    let mut r = [0.0f32; 4];

    // ortho: the hull plus one pixel, clipped to the scene
    let rect = zone_scene_rect(&stored(&bx(10.0, 10.0, 4.0, 2.0)), &o, 0.0, false, 64, 64)
        .expect("visible");
    assert!(approx(rect[0], 7.0));
    assert!(approx(rect[1], 8.0));
    assert!(approx(rect[2], 13.0));
    assert!(approx(rect[3], 12.0));
    let rect =
        zone_scene_rect(&stored(&circle(0.0, 0.0, 5.0)), &o, 0.0, false, 64, 64).expect("visible");
    r.copy_from_slice(&rect);
    assert_eq!(r[0], 0.0);
    assert!(approx(r[2], 6.0));
    assert!(zone_scene_rect(&stored(&circle(-20.0, 10.0, 5.0)), &o, 0.0, false, 64, 64).is_none());
    assert!(zone_scene_rect(&stored(&circle(30.0, 90.0, 5.0)), &o, 0.0, false, 64, 64).is_none());

    // iso: the walls up to the top are covered, without a surface buffer only the ground
    let isop = iso(4.0, 1.0, 256, 256);
    let mut p = bx(0.0, 0.0, 8.0, 8.0);
    p.height = Some(8.0);
    let z = stored(&p);
    let rect = zone_scene_rect(&z, &isop, 100.0, true, 256, 256).expect("visible");
    r.copy_from_slice(&rect);
    // The roof corners (z = 8) project 32 pixels up (z axis (0, -4)).
    for k in 0..4 {
        let wx = if k & 1 != 0 { 4.0f32 } else { -4.0 };
        let wy = if k & 2 != 0 { 4.0f32 } else { -4.0 };
        let o_ = isop.origin.raw();
        let sx = o_.x + wx * isop.x_axis.x + wy * isop.y_axis.x + 8.0 * isop.z_axis.x;
        let sy = o_.y + wx * isop.x_axis.y + wy * isop.y_axis.y + 8.0 * isop.z_axis.y;
        assert!(sx >= r[0]);
        assert!(sx <= r[2]);
        assert!(sy >= r[1]);
        assert!(sy <= r[3]);
    }
    let top_with_roof = r[1];
    let rect = zone_scene_rect(&z, &isop, 100.0, false, 256, 256).expect("visible");
    r.copy_from_slice(&rect);
    assert!(approx(r[1], top_with_roof + 32.0)); // ground only

    // A zone above every pixel (base_height > maxHeight) is not drawn with a surface buffer.
    p.height = None;
    p.base_height = 128.0;
    let high = stored(&p);
    assert!(zone_scene_rect(&high, &isop, 100.0, true, 256, 256).is_none());
    assert!(zone_scene_rect(&high, &isop, 100.0, false, 256, 256).is_some());

    // Its ground outline off screen, its walls on screen: still drawn.
    let mut tall = bx(0.0, 0.0, 8.0, 8.0);
    tall.height = Some(100.0);
    let tall = stored(&tall);
    let mut below = iso(4.0, 1.0, 256, 256);
    // the ground outline lies below the scene's bottom edge, the walls rise into it
    let o_ = below.origin.raw();
    below.origin = At::new(Vec2::new(o_.x, o_.y + 160.0));
    assert!(zone_scene_rect(&tall, &below, 100.0, false, 256, 256).is_none());
    assert!(zone_scene_rect(&tall, &below, 100.0, true, 256, 256).is_some());
}

#[test]
fn ambient_zone_validation_names_field_and_value() {
    let mut p = bx(0.0, 0.0, 1.0, 1.0);
    assert!(validate_zone(&p).is_ok());

    p = bx(0.0, 0.0, 1.0, 1.0);
    p.ambient = [1.0, -0.5, 1.0];
    let msg = validate_zone(&p).unwrap_err().to_string();
    assert!(
        msg.contains("ambient must be finite and >= 0 in every channel (got 1, -0.5, 1)"),
        "{msg}"
    );
    p = bx_col(0.0, 0.0, 1.0, 1.0, f32::NAN, 1.0, 1.0);
    let msg = validate_zone(&p).unwrap_err().to_string();
    assert!(msg.contains("ambient must be finite"), "{msg}");

    p = bx(0.0, 0.0, 1.0, 1.0);
    p.soft_edge = -1.0;
    let msg = validate_zone(&p).unwrap_err().to_string();
    assert!(
        msg.contains("soft_edge must be finite and >= 0 (got -1)"),
        "{msg}"
    );

    p = bx(f32::INFINITY, 0.0, 1.0, 1.0);
    let msg = validate_zone(&p).unwrap_err().to_string();
    assert!(msg.contains("x, y and rotation must be finite"), "{msg}");
    p = bx(0.0, 0.0, 1.0, 1.0);
    p.rotation = f32::NAN;
    let msg = validate_zone(&p).unwrap_err().to_string();
    assert!(msg.contains("x, y and rotation must be finite"), "{msg}");

    p = bx(0.0, 0.0, 1.0, 1.0);
    p.base_height = -2.0;
    let msg = validate_zone(&p).unwrap_err().to_string();
    assert!(
        msg.contains("base_height must be finite and >= 0 (got -2)"),
        "{msg}"
    );

    p = bx(0.0, 0.0, 1.0, 1.0);
    p.base_height = 4.0;
    p.height = Some(4.0);
    let msg = validate_zone(&p).unwrap_err().to_string();
    assert!(
        msg.contains("height must be finite and above base_height 4 (got 4)"),
        "{msg}"
    );
    p.height = Some(-1.0);
    p.base_height = 0.0;
    let msg = validate_zone(&p).unwrap_err().to_string();
    assert!(msg.contains("(got -1)"), "{msg}");
    p.height = Some(f32::INFINITY);
    assert!(validate_zone(&p).is_err());
    p.height = None;
    p.base_height = 4.0;
    assert!(validate_zone(&p).is_ok());

    p = bx(0.0, 0.0, 0.0, 1.0);
    let msg = validate_zone(&p).unwrap_err().to_string();
    assert!(
        msg.contains("width and depth must be finite and > 0 (got 0, 1)"),
        "{msg}"
    );
    p = circle(0.0, 0.0, 0.0);
    let msg = validate_zone(&p).unwrap_err().to_string();
    assert!(
        msg.contains("radius must be finite and > 0 (got 0)"),
        "{msg}"
    );
    p.width = 0.0; // fields of other types are not checked
    p.radius = 1.0;
    assert!(validate_zone(&p).is_ok());

    p = polygon(&L);
    p.vertices.truncate(2);
    let msg = validate_zone(&p).unwrap_err().to_string();
    assert!(
        msg.contains("vertex_count must be in 3..64 (got 2)"),
        "{msg}"
    );
    let many = vec![Vec2::ZERO; 65];
    p = polygon(&[]);
    p.vertices = many;
    let msg = validate_zone(&p).unwrap_err().to_string();
    assert!(msg.contains("(got 65)"), "{msg}");
    let mut nan = L;
    nan[5] = f32::NAN;
    let msg = validate_zone(&polygon(&nan)).unwrap_err().to_string();
    assert!(msg.contains("vertices must be finite (vertex 2"), "{msg}");
    let bowtie = [0.0f32, 0.0, 2.0, 2.0, 2.0, 0.0, 0.0, 2.0];
    let msg = validate_zone(&polygon(&bowtie)).unwrap_err().to_string();
    assert!(msg.contains("vertices must form a simple polygon"), "{msg}");
}

#[test]
fn ambient_zone_fold_with_the_white_default_leaves_everything_at_the_global_ambient() {
    let store = AmbientZoneStore::new();
    assert_eq!(
        fold_ambient(&store, WHITE, Vec3::new(1.0, 2.0, 3.0), true).x,
        1.0
    );
    let p = AmbientZoneParams::default(); // the defaults: box 1 x 1, white
    let mut store = AmbientZoneStore::new();
    store.create(stored(&p)).unwrap();
    assert_eq!(
        fold_ambient(&store, [0.25, 0.25, 0.25], Vec3::new(0.0, 0.0, 0.0), true).x,
        1.0
    );
}

/// Not in chiaro's unit tests: culling keeps visible, enabled zones in fold order and writes the
/// GPU record with the directional sum added. Checks the 80-byte layout of `GpuZone` as well.
#[test]
fn ambient_zone_culling_keeps_visible_enabled_zones_in_fold_order() {
    assert_eq!(GPU_ZONE_BYTES, 80);
    assert_eq!(std::mem::size_of::<GpuZone>(), 80);

    let mut store = AmbientZoneStore::new();
    let visible = bx_col(0.0, 0.0, 4.0, 4.0, 0.5, 0.5, 0.5);
    let mut disabled = bx_col(0.0, 0.0, 4.0, 4.0, 0.0, 0.0, 0.0);
    disabled.disabled = true;
    let off = circle(-500.0, 0.0, 5.0);
    store.create(stored(&visible)).unwrap();
    store.create(stored(&disabled)).unwrap();
    store.create(stored(&off)).unwrap();
    let mut out = AmbientZoneList::default();
    let scene = CullScene {
        projection: ortho(),
        max_height: 0.0,
        has_surface: false,
        width: 64,
        height: 64,
    };
    collect_ambient_zones(&store, [0.2, 0.4, 0.6], &scene, [0.1, 0.0, 0.0], &mut out);
    assert_eq!(out.zones.len(), 1);
    assert!(approx(out.zones[0].color[0], 0.5 + 0.1));
    assert!(approx(out.zones[0].color[1], 0.5));
    assert_eq!(out.zones[0].color[3], 2.0); // box = CHI_SHAPE_BOX (2)
    assert!(!out.any_polygon);
    assert_eq!(out.height_spans, 0);
}
