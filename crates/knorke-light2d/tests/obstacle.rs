//! Origin: chiaro @ 9bc5b9b, tests/unit/ObstacleStoreTests.cpp (zlib license, see
//! `src/obstacle/`): the cases of the shape and tile-grid creation part, plus tests for what the
//! redesign adds: handles from `Slots`, typed errors with stable codes, batches, dirty tracking
//! and snapshots. The cases for cell editing, compared cells and patterns belong to P14/P15.

// Tests compare against platform `std` maths as an oracle and use `f32::min`/`max` for bounds;
// only library code must be platform independent (ADR 0011).
#![allow(clippy::disallowed_methods)]

use knorke_light2d::obstacle::{
    ObstacleCommon, ObstacleError, ObstacleKind, ObstaclePose, ShapeDesc, TileGridDesc,
};
use knorke_light2d::{ObstacleId, ObstacleStore};
use knorke_math::{At, Vec2, World2};
use knorke_types::{Reader, Snapshot, SnapshotBuf, SnapshotError, restore};

const NAN: f32 = f32::NAN;
const INF: f32 = f32::INFINITY;

const SQUARE: [Vec2; 4] = [
    Vec2::new(0.0, 0.0),
    Vec2::new(1.0, 0.0),
    Vec2::new(1.0, 1.0),
    Vec2::new(0.0, 1.0),
];
const BOWTIE: [Vec2; 4] = [
    Vec2::new(0.0, 0.0),
    Vec2::new(4.0, 4.0),
    Vec2::new(4.0, 0.0),
    Vec2::new(0.0, 4.0),
];

fn square() -> ShapeDesc<'static> {
    ShapeDesc::polygon(&SQUARE)
}

fn at(x: f32, y: f32) -> At<World2, Vec2> {
    At::new(Vec2::new(x, y))
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-5 * b.abs().max(1.0)
}

/// Axis-aligned bounds (min x, min y, max x, max y) of world triangles.
fn bounds(t: &[At<World2, Vec2>]) -> (f32, f32, f32, f32) {
    let mut b = (1e9f32, 1e9f32, -1e9f32, -1e9f32);
    for p in t {
        let p = p.raw();
        b = (b.0.min(p.x), b.1.min(p.y), b.2.max(p.x), b.3.max(p.y));
    }
    b
}

fn world(store: &ObstacleStore, id: ObstacleId) -> Vec<At<World2, Vec2>> {
    let mut out = Vec::new();
    store.append_world_triangles(id, &mut out);
    out
}

fn tiles(solid: &[u8]) -> TileGridDesc<'_> {
    TileGridDesc::new(2, 2, solid)
}

fn rejected(d: &ShapeDesc<'_>) -> ObstacleError {
    ObstacleStore::new()
        .create_shape(d)
        .expect_err("must be rejected")
}

// ---- descriptions ----

#[test]
fn obstacle_desc_defaults() {
    let c = ObstacleCommon::default();
    assert!(c.enabled && !c.dynamic && !c.shadow_footprint);
    assert_eq!((c.height, c.base_height, c.opacity), (0.0, 0.0, 1.0));
    let t = tiles(&[1, 0, 0, 1]);
    assert_eq!(t.cell_size, Vec2::new(1.0, 1.0));
    assert_eq!((t.level_height, t.edge_width), (0.0, 0.0));
    assert!(t.base_levels.is_none() && t.opacity.is_none() && t.edges.is_none());
}

#[test]
fn obstacle_desc_validation() {
    let cells = [1u8, 0, 0, 1];
    assert_eq!(square().validate(), Ok(()));
    assert_eq!(tiles(&cells).validate(), Ok(()));

    // Polygon corner counts.
    assert!(matches!(
        ShapeDesc::polygon(&SQUARE[..2]).validate(),
        Err(ObstacleError::VertexCount { count: 2 })
    ));
    let many = vec![Vec2::ZERO; 4097];
    assert!(matches!(
        ShapeDesc::polygon(&many).validate(),
        Err(ObstacleError::VertexCount { count: 4097 })
    ));
    let max = vec![Vec2::ZERO; 4096];
    assert_eq!(ShapeDesc::polygon(&max).validate(), Ok(()));

    // Circles.
    for bad in [0.0, NAN, -1.0, INF] {
        assert!(ShapeDesc::circle(bad).validate().is_err(), "radius {bad}");
    }
    assert_eq!(ShapeDesc::circle(2.0).validate(), Ok(()));

    // Grid dimensions and cell size.
    let mut t = tiles(&cells);
    t.columns = 0;
    assert!(matches!(t.validate(), Err(ObstacleError::GridSize { .. })));
    t.columns = 4097;
    assert!(matches!(t.validate(), Err(ObstacleError::GridSize { .. })));
    let mut t = tiles(&cells);
    t.rows = 0;
    assert!(matches!(t.validate(), Err(ObstacleError::GridSize { .. })));
    for bad in [0.0, NAN] {
        let mut t = tiles(&cells);
        t.cell_size = Vec2::new(bad, 1.0);
        assert!(t.validate().is_err());
        t.cell_size = Vec2::new(1.0, bad);
        assert!(t.validate().is_err());
    }

    // Poses.
    let mut d = square();
    d.common.position = at(0.0, NAN);
    assert!(matches!(d.validate(), Err(ObstacleError::NotFinite { .. })));
    let mut d = square();
    d.common.rotation = NAN;
    assert!(matches!(d.validate(), Err(ObstacleError::NotFinite { .. })));
    let mut t = tiles(&cells);
    t.common.position = at(0.0, NAN);
    assert!(t.validate().is_err());
    let mut t = tiles(&cells);
    t.common.rotation = NAN;
    assert!(t.validate().is_err());
}

#[test]
fn channel_lengths_are_checked() {
    let mut t = TileGridDesc::new(2, 2, &[1, 0, 0]);
    assert!(matches!(
        t.validate(),
        Err(ObstacleError::ChannelLength {
            field: "solid",
            expected: 4,
            got: 3
        })
    ));
    t.solid = &[1, 0, 0, 1];
    t.opacity = Some(&[255; 5]);
    assert!(matches!(
        t.validate(),
        Err(ObstacleError::ChannelLength {
            field: "opacity",
            ..
        })
    ));
}

// ---- the store ----

#[test]
fn store_copies_the_callers_arrays() {
    let mut store = ObstacleStore::new();
    let mut verts = SQUARE;
    let mut cells = [1u8, 0, 0, 1];
    let poly = store.create_shape(&ShapeDesc::polygon(&verts)).unwrap();
    let grid = store.create_tile_grid(&tiles(&cells)).unwrap();
    verts.fill(Vec2::new(99.0, 99.0));
    cells.fill(0);

    let w = world(&store, poly);
    assert_eq!(w.len(), 6);
    let b = bounds(&w);
    assert!(near(b.2, 1.0) && near(b.3, 1.0));

    let t = store.get(grid).unwrap();
    assert_eq!(t.kind, ObstacleKind::TileGrid);
    assert_eq!(t.grid.solid, [1, 0, 0, 1]);
    assert_eq!(store.len(), 2);
}

#[test]
fn degenerate_polygons_are_rejected_without_a_state_change() {
    let mut store = ObstacleStore::new();
    let e = store
        .create_shape(&ShapeDesc::polygon(&BOWTIE))
        .unwrap_err();
    assert!(e.code().as_str().starts_with("knorke.light2d.polygon."));
    assert!(store.is_empty());
    assert_eq!(store.static_version().0, 0);
    assert_eq!(store.revision().0, 0);
}

#[test]
fn transform_rotates_counter_clockwise_then_translates() {
    let mut store = ObstacleStore::new();
    let id = store.create_shape(&square()).unwrap();
    store.set_transform(id, at(10.0, 0.0), 90.0).unwrap();
    let b = bounds(&world(&store, id));
    assert!(near(b.0, 9.0) && near(b.2, 10.0));
    assert!(b.1.abs() < 1e-5 && near(b.3, 1.0));
    assert!(matches!(
        store.set_transform(id, at(INF, 0.0), 0.0),
        Err(ObstacleError::NotFinite { .. })
    ));
    // The failed call changed nothing.
    assert_eq!(store.get(id).unwrap().common.position, at(10.0, 0.0));
}

#[test]
fn circle_tessellation() {
    let mut store = ObstacleStore::new();
    let mut d = ShapeDesc::circle(2.0);
    d.common.position = at(5.0, 0.0);
    let id = store.create_shape(&d).unwrap();
    let w = world(&store, id);
    assert_eq!(w.len(), 48 * 3);
    let b = bounds(&w);
    assert!(near(b.0, 3.0) && near(b.2, 7.0));
    assert_eq!(store.get(id).unwrap().kind, ObstacleKind::Circle);
    assert_eq!(store.get(id).unwrap().radius, 2.0);
}

#[test]
fn static_version_tracks_changes_to_static_geometry_only() {
    let mut store = ObstacleStore::new();
    assert_eq!(store.static_version().0, 0);
    let s = store.create_shape(&square()).unwrap();
    assert_eq!(store.static_version().0, 1);
    store.set_transform(s, at(1.0, 1.0), 0.0).unwrap();
    assert_eq!(store.static_version().0, 2);

    let mut dyn_desc = square();
    dyn_desc.common.dynamic = true;
    let h = store.create_shape(&dyn_desc).unwrap();
    assert_eq!(store.static_version().0, 2);
    assert!(store.any_dynamic_enabled());
    store.set_transform(h, at(3.0, 3.0), 45.0).unwrap();
    assert_eq!(store.static_version().0, 2);

    let mut off = dyn_desc;
    off.common.enabled = false;
    store.set_shape(h, &off).unwrap();
    assert!(!store.any_dynamic_enabled());
    assert_eq!(store.static_version().0, 2);

    store.set_shape(h, &square()).unwrap(); // dynamic -> static
    assert_eq!(store.static_version().0, 3);
    store.set_shape(h, &dyn_desc).unwrap(); // static -> dynamic
    assert_eq!(store.static_version().0, 4);

    store.destroy(s).unwrap();
    assert_eq!(store.static_version().0, 5);
    store.destroy(h).unwrap(); // dynamic
    assert_eq!(store.static_version().0, 5);
}

#[test]
fn update_grows_the_revision_and_errors_change_nothing() {
    let mut store = ObstacleStore::new();
    let id = store.create_shape(&square()).unwrap();
    let rev = store.get(id).unwrap().revision;
    let version = store.static_version();

    let big = [
        Vec2::new(0.0, 0.0),
        Vec2::new(2.0, 0.0),
        Vec2::new(2.0, 2.0),
        Vec2::new(0.0, 2.0),
    ];
    store.set_shape(id, &ShapeDesc::polygon(&big)).unwrap();
    assert_eq!(store.get(id).unwrap().revision, rev + 1);
    assert_eq!(store.static_version().0, version.0 + 1);

    // A move does not touch the revision.
    store.set_transform(id, at(1.0, 0.0), 0.0).unwrap();
    assert_eq!(store.get(id).unwrap().revision, rev + 1);

    let before = store.get(id).unwrap().clone();
    let (v2, r2) = (store.static_version(), store.revision());
    assert!(store.set_shape(id, &ShapeDesc::polygon(&BOWTIE)).is_err());
    assert!(
        store
            .set_shape(id, &ShapeDesc::polygon(&SQUARE[..2]))
            .is_err()
    );
    assert_eq!(store.get(id).unwrap(), &before);
    assert_eq!((store.static_version(), store.revision()), (v2, r2));

    store.destroy(id).unwrap();
    assert!(matches!(
        store.set_shape(id, &square()),
        Err(ObstacleError::InvalidHandle(_))
    ));
    assert!(store.set_transform(id, at(0.0, 0.0), 0.0).is_err());
    assert!(store.destroy(id).is_err());
    assert!(store.get(id).is_none());
}

#[test]
fn a_reused_slot_does_not_reach_the_new_obstacle() {
    let mut store = ObstacleStore::new();
    let old = store.create_shape(&square()).unwrap();
    store.destroy(old).unwrap();
    let new = store.create_shape(&ShapeDesc::circle(1.0)).unwrap();
    assert_eq!(old.index(), new.index());
    assert!(store.get(old).is_none());
    assert!(store.set_transform(old, at(1.0, 1.0), 0.0).is_err());
    assert_eq!(store.get(new).unwrap().common.position, at(0.0, 0.0));
}

#[test]
fn flags_map_onto_the_stored_obstacle() {
    let mut d = square();
    assert!(!d.common.dynamic && d.common.enabled && !d.common.shadow_footprint);
    d.common.dynamic = true;
    d.common.enabled = false;
    d.common.shadow_footprint = true;
    let mut store = ObstacleStore::new();
    let id = store.create_shape(&d).unwrap();
    let c = &store.get(id).unwrap().common;
    assert!(c.dynamic && !c.enabled && c.shadow_footprint);

    store.set_shape(id, &square()).unwrap();
    let c = &store.get(id).unwrap().common;
    assert!(!c.dynamic && c.enabled && !c.shadow_footprint);
}

#[test]
fn a_box_is_stored_as_the_four_vertex_polygon_around_its_centre() {
    let mut a = ShapeDesc::rect(4.0, 2.0);
    a.common.position = at(5.0, 6.0);
    a.common.rotation = 30.0;
    a.common.height = 3.0;
    let v = [
        Vec2::new(-2.0, -1.0),
        Vec2::new(2.0, -1.0),
        Vec2::new(2.0, 1.0),
        Vec2::new(-2.0, 1.0),
    ];
    let mut b = ShapeDesc::polygon(&v);
    b.common = a.common;

    let mut store = ObstacleStore::new();
    let (ia, ib) = (
        store.create_shape(&a).unwrap(),
        store.create_shape(&b).unwrap(),
    );
    let (sa, sb) = (store.get(ia).unwrap(), store.get(ib).unwrap());
    assert_eq!(store.triangles(ia), store.triangles(ib));
    assert_eq!(sa.common.height, sb.common.height);
    assert!(sa.box_shape && !sb.box_shape);
    assert_eq!(store.outline(ia), &v[..]);

    for (w, d) in [(0.0, 1.0), (NAN, 1.0), (1.0, 0.0), (1.0, -2.0)] {
        assert!(
            rejected(&ShapeDesc::rect(w, d))
                .code()
                .as_str()
                .starts_with("knorke.light2d.obstacle.")
        );
    }
}

#[test]
fn heights_default_to_infinitely_tall() {
    let d = square();
    assert_eq!((d.common.height, d.common.base_height), (0.0, 0.0));
    let mut store = ObstacleStore::new();
    let id = store.create_shape(&d).unwrap();
    assert!(!store.get(id).unwrap().has_finite_height());
    assert!(!store.any_finite_height());
    assert!(store.any_infinite_enabled());
}

#[test]
fn invalid_heights_are_rejected() {
    let mut store = ObstacleStore::new();
    let mut d = square();
    d.common.height = 2.0;
    d.common.base_height = 3.0;
    assert!(store.create_shape(&d).is_err());
    d.common.base_height = 2.0; // empty interval
    assert!(store.create_shape(&d).is_err());
    d.common.base_height = -1.0;
    assert!(store.create_shape(&d).is_err());
    d.common.base_height = 0.0;
    for bad in [NAN, INF] {
        d.common.height = bad;
        assert!(store.create_shape(&d).is_err());
    }
    d.common.height = 0.0;
    d.common.base_height = 4.0; // a base without a top
    assert!(store.create_shape(&d).is_err());
    d.common.base_height = 0.0;

    let cells = [1u8, 0, 0, 1];
    let mut t = tiles(&cells);
    t.level_height = -1.0;
    assert!(store.create_tile_grid(&t).is_err());
    t.level_height = 0.0;
    t.common.base_height = 4.0;
    assert!(store.create_tile_grid(&t).is_err());
    t.common.height = 2.0;
    t.common.base_height = 3.0;
    assert!(store.create_tile_grid(&t).is_err());
    t.common.height = 0.0;
    t.common.base_height = 0.0;

    assert!(store.is_empty());
    assert!(store.create_shape(&d).is_ok());
    assert!(store.create_tile_grid(&t).is_ok());
}

#[test]
fn tile_bottoms_per_cell_are_validated_and_copied() {
    let mut store = ObstacleStore::new();
    let cells = [0u8, 1, 3, 2];
    let mut t = tiles(&cells);
    t.base_levels = Some(&[7, 0, 2, 1]);
    assert!(matches!(
        store.create_tile_grid(&t),
        Err(ObstacleError::Unsupported(_))
    )); // needs level_height
    t.level_height = 2.0;
    t.base_levels = Some(&[7, 0, 2, 2]); // bottom == top of a solid cell
    assert!(matches!(
        store.create_tile_grid(&t),
        Err(ObstacleError::CellValue {
            field: "base_levels",
            cell: 3,
            ..
        })
    ));
    // The store holds copies: the channel is gone once the call returned.
    let id = {
        let base = vec![7u8, 0, 2, 1];
        t.base_levels = Some(&base);
        store.create_tile_grid(&t).unwrap()
    };
    assert_eq!(store.get(id).unwrap().grid.base_levels, [7, 0, 2, 1]);
}

#[test]
fn finite_height_and_highest_top() {
    let mut store = ObstacleStore::new();
    assert!(!store.any_finite_height());
    assert_eq!(store.max_finite_top(), 0.0);
    store.create_shape(&square()).unwrap();
    assert!(!store.any_finite_height());

    let mut b = square();
    b.common.height = 3.0;
    let a = store.create_shape(&b).unwrap();
    b.common.height = 7.0;
    b.common.base_height = 2.0;
    let bid = store.create_shape(&b).unwrap();
    assert!(store.any_finite_height());
    assert_eq!(store.max_finite_top(), 7.0);

    b.common.enabled = false;
    store.set_shape(bid, &b).unwrap();
    assert_eq!(store.max_finite_top(), 3.0);
    store.destroy(a).unwrap();
    assert!(!store.any_finite_height());
    assert_eq!(store.max_finite_top(), 0.0);

    let cells = [0u8, 1, 3, 0];
    let mut t = tiles(&cells);
    t.level_height = 2.0;
    t.common.height = 100.0; // level_height overrides height
    let levels = store.create_tile_grid(&t).unwrap();
    assert!(store.get(levels).unwrap().has_finite_height());
    assert!(store.any_finite_height());
    assert_eq!(store.max_finite_top(), 6.0);
}

#[test]
fn opacity_defaults_to_one_and_bad_values_are_rejected() {
    let mut store = ObstacleStore::new();
    let id = store.create_shape(&square()).unwrap();
    assert_eq!(store.get(id).unwrap().common.opacity, 1.0);
    assert!(!store.any_partial());

    let mut d = square();
    for bad in [NAN, -0.1, 1.1, INF] {
        d.common.opacity = bad;
        assert!(store.create_shape(&d).is_err(), "opacity {bad}");
    }
    assert_eq!(store.len(), 1);
    for good in [0.0, 0.3, 1.0] {
        d.common.opacity = good;
        assert!(store.create_shape(&d).is_ok());
    }
}

#[test]
fn cell_opacities_are_copied() {
    let mut store = ObstacleStore::new();
    let cells = [1u8, 1, 1, 0];
    let id = {
        let opacity = vec![255u8, 77, 0, 9];
        let mut t = tiles(&cells);
        t.opacity = Some(&opacity);
        store.create_tile_grid(&t).unwrap()
    };
    assert_eq!(store.get(id).unwrap().grid.opacity, [255, 77, 0, 9]);
    // Solid cells below 255: 77 and 0 (the 9 is in an empty cell).
    assert_eq!(store.get(id).unwrap().grid.partial_cells, 2);

    let plain = store.create_tile_grid(&tiles(&[1, 0, 0, 1])).unwrap();
    assert!(store.get(plain).unwrap().grid.opacity.is_empty());
}

#[test]
fn any_partial_follows_set_disable_and_destroy() {
    let mut store = ObstacleStore::new();
    assert!(!store.any_partial());
    let a = store.create_shape(&square()).unwrap();
    assert!(!store.any_partial());

    let mut fence = square();
    fence.common.opacity = 0.5;
    let b = store.create_shape(&fence).unwrap();
    assert!(store.any_partial());
    fence.common.enabled = false;
    store.set_shape(b, &fence).unwrap();
    assert!(!store.any_partial());
    fence.common.enabled = true;
    store.set_shape(b, &fence).unwrap();
    assert!(store.any_partial());
    store.set_transform(b, at(3.0, 4.0), 10.0).unwrap();
    assert!(store.any_partial());
    store.destroy(b).unwrap();
    assert!(!store.any_partial());

    // Opacity 0 is no obstacle at all: not partial, not active for heights.
    let mut invisible = square();
    invisible.common.opacity = 0.0;
    invisible.common.height = 5.0;
    store.create_shape(&invisible).unwrap();
    assert!(!store.any_partial());
    assert!(!store.any_finite_height());
    assert_eq!(store.max_finite_top(), 0.0);
    assert_eq!(store.len(), 2);

    let cells = [1u8, 1, 1, 1];
    let full = [255u8; 4];
    let lighter = [255u8, 255, 254, 255];
    let mut t = tiles(&cells);
    t.opacity = Some(&full);
    let g = store.create_tile_grid(&t).unwrap();
    assert!(!store.any_partial());
    t.opacity = Some(&lighter);
    store.set_tile_grid(g, &t).unwrap();
    assert!(store.any_partial());
    t.opacity = None;
    store.set_tile_grid(g, &t).unwrap();
    assert!(!store.any_partial());
    store.set_shape(a, &fence).unwrap(); // the opaque square becomes the 0.5 fence
    assert!(store.any_partial());
}

#[test]
fn cell_edge_strips_default_to_none_and_are_validated_and_copied() {
    let mut store = ObstacleStore::new();
    let cells = [1u8, 1, 1, 0];
    let id = store.create_tile_grid(&tiles(&cells)).unwrap();
    let g = &store.get(id).unwrap().grid;
    assert_eq!(g.edge_width, 0.0);
    assert!(g.edges.is_empty());

    let mut t = tiles(&cells);
    t.edges = Some(&[0, 3, 15, 16]);
    t.edge_width = 0.5;
    assert!(matches!(
        store.create_tile_grid(&t),
        Err(ObstacleError::CellValue {
            field: "edges",
            cell: 3,
            ..
        })
    )); // 16 is no edge bit
    t.edges = Some(&[0, 3, 15, 9]);
    t.edge_width = 0.0;
    assert!(store.create_tile_grid(&t).is_err()); // needs a width
    for bad in [NAN, -1.0, INF] {
        t.edge_width = bad;
        assert!(store.create_tile_grid(&t).is_err());
    }
    t.edge_width = 0.5;
    let h = {
        let edges = vec![0u8, 3, 15, 9];
        t.edges = Some(&edges);
        store.create_tile_grid(&t).unwrap()
    };
    assert_eq!(store.get(h).unwrap().grid.edges, [0, 3, 15, 9]);
    // Edge cells count only solid cells with bits: cells 1 and 2 (cell 3 is empty).
    assert_eq!(store.get(h).unwrap().grid.edge_cells, 2);

    let mut width_only = tiles(&cells);
    width_only.edge_width = 3.0; // allowed without edges, has no effect
    assert!(store.create_tile_grid(&width_only).is_ok());
}

#[test]
fn footprint_shadow_is_kept_for_shapes_and_rejected_for_tile_grids() {
    let cells = [1u8; 4];
    let mut store = ObstacleStore::new();
    assert!(store.create_tile_grid(&tiles(&cells)).is_ok());
    let mut t = tiles(&cells);
    t.common.shadow_footprint = true;
    let e = store.create_tile_grid(&t).unwrap_err();
    assert!(e.to_string().contains("shadow_footprint") && e.to_string().contains("shapes"));
    assert_eq!(e.code().as_str(), "knorke.light2d.obstacle.unsupported");
}

#[test]
fn an_obstacle_can_change_its_kind() {
    let mut store = ObstacleStore::new();
    let id = store.create_shape(&square()).unwrap();
    let rev = store.get(id).unwrap().revision;
    store.set_tile_grid(id, &tiles(&[1, 1, 0, 0])).unwrap();
    let s = store.get(id).unwrap();
    assert_eq!(s.kind, ObstacleKind::TileGrid);
    assert!(store.triangles(id).is_empty() && store.outline(id).is_empty());
    assert_eq!(s.revision, rev + 1);
    store.set_shape(id, &ShapeDesc::circle(1.0)).unwrap();
    let s = store.get(id).unwrap();
    assert_eq!(s.kind, ObstacleKind::Circle);
    assert!(s.grid.solid.is_empty() && s.grid.columns == 0);
}

// ---- batches ----

#[test]
fn batches_are_all_or_nothing_and_name_the_bad_index() {
    let mut store = ObstacleStore::new();
    let descs = [
        square(),
        ShapeDesc::circle(1.0),
        ShapeDesc::polygon(&BOWTIE),
        square(),
    ];
    let mut out = Vec::new();
    let e = store.create_shapes(&descs[..], &mut out).unwrap_err();
    assert!(matches!(
        e,
        ObstacleError::Batch {
            field: "descs",
            index: 2,
            ..
        }
    ));
    assert_eq!(e.to_diagnostic().path.as_deref(), Some("descs[2]"));
    assert!(e.code().as_str().starts_with("knorke.light2d.polygon."));
    assert!(store.is_empty() && out.is_empty());

    store.create_shapes(&descs[..2], &mut out).unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(store.len(), 2);

    // set_shapes: a stale handle and a bad desc, nothing changes.
    let stale = out[0];
    store.destroy(stale).unwrap();
    let before = store.revision();
    let e = store
        .set_shapes(&[out[1], stale][..], &[square(), square()][..])
        .unwrap_err();
    assert!(matches!(
        e,
        ObstacleError::Batch {
            field: "ids",
            index: 1,
            ..
        }
    ));
    let e = store
        .set_shapes(&[out[1]][..], &[ShapeDesc::polygon(&BOWTIE)][..])
        .unwrap_err();
    assert!(matches!(
        e,
        ObstacleError::Batch {
            field: "descs",
            index: 0,
            ..
        }
    ));
    assert!(matches!(
        store.set_shapes(&[out[1]][..], &[square(), square()][..]),
        Err(ObstacleError::LengthMismatch { .. })
    ));
    assert_eq!(store.revision(), before);
    store.set_shapes(&[out[1]][..], &[square()][..]).unwrap();
    assert_eq!(store.get(out[1]).unwrap().kind, ObstacleKind::Polygon);
    assert_eq!(store.get(out[1]).unwrap().revision, 2);
}

#[test]
fn tile_grid_batches_and_transform_batches() {
    let mut store = ObstacleStore::new();
    let cells = [1u8, 0, 0, 1];
    let mut bad = tiles(&cells);
    bad.common.opacity = 2.0;
    let mut out = Vec::new();
    assert!(
        store
            .create_tile_grids(&[tiles(&cells), bad][..], &mut out)
            .is_err()
    );
    assert!(store.is_empty());
    store
        .create_tile_grids(&[tiles(&cells), tiles(&cells)][..], &mut out)
        .unwrap();
    assert_eq!(out.len(), 2);

    let mut with_levels = tiles(&cells);
    with_levels.level_height = 2.0;
    store
        .set_tile_grids(&out[..], &[with_levels, with_levels][..])
        .unwrap();
    assert_eq!(store.get(out[1]).unwrap().grid.level_height, 2.0);
    assert!(
        store
            .set_tile_grids(&out[..], &[with_levels, bad][..])
            .is_err()
    );

    let poses = [
        ObstaclePose {
            position: at(1.0, 2.0),
            rotation: 3.0,
        },
        ObstaclePose {
            position: at(NAN, 2.0),
            rotation: 3.0,
        },
    ];
    let e = store.set_transforms(&out[..], &poses[..]).unwrap_err();
    assert!(matches!(
        e,
        ObstacleError::Batch {
            field: "poses",
            index: 1,
            ..
        }
    ));
    assert_eq!(store.get(out[0]).unwrap().common.position, at(0.0, 0.0));
    store
        .set_transforms(&out[..], &poses[..1].repeat(2)[..])
        .unwrap();
    assert_eq!(store.get(out[1]).unwrap().common.position, at(1.0, 2.0));
}

#[test]
fn destroy_many_rejects_stale_and_repeated_handles() {
    let mut store = ObstacleStore::new();
    let mut out = Vec::new();
    store
        .create_shapes(&[square(), square(), square()][..], &mut out)
        .unwrap();
    let e = store
        .destroy_many(&[out[0], out[1], out[0]][..])
        .unwrap_err();
    assert!(matches!(
        e,
        ObstacleError::Batch {
            field: "ids",
            index: 2,
            ..
        }
    ));
    assert_eq!(store.len(), 3);
    store.destroy(out[2]).unwrap();
    let e = store.destroy_many(&[out[0], out[2]][..]).unwrap_err();
    assert!(matches!(e, ObstacleError::Batch { index: 1, .. }));
    assert_eq!(store.len(), 2);
    store.destroy_many(&out[..2]).unwrap();
    assert!(store.is_empty());
}

// ---- dirty tracking ----

#[test]
fn dirty_sets_separate_static_and_dynamic_changes() {
    let mut store = ObstacleStore::new();
    let s = store.create_shape(&square()).unwrap();
    let mut dyn_desc = square();
    dyn_desc.common.dynamic = true;
    let d = store.create_shape(&dyn_desc).unwrap();
    assert!(store.is_dirty());
    assert!(store.dirty_static().contains(s.index() as usize));
    assert!(!store.dirty_static().contains(d.index() as usize));
    assert!(store.dirty_dynamic().contains(d.index() as usize));

    store.clear_dirty();
    assert!(!store.is_dirty());
    assert!(store.dirty_static().is_empty() && store.dirty_dynamic().is_empty());
    let version = store.static_version();
    let revision = store.revision();

    // A moving dynamic obstacle is a dynamic change only.
    store.set_transform(d, at(1.0, 1.0), 5.0).unwrap();
    assert!(store.dirty_dynamic().contains(d.index() as usize));
    assert!(store.dirty_static().is_empty());
    assert_eq!(store.static_version(), version);
    assert_eq!(store.revision().0, revision.0 + 1);

    // Switching `dynamic` is a static change; so is every change of a static obstacle.
    store.set_shape(d, &square()).unwrap();
    assert!(store.dirty_static().contains(d.index() as usize));
    store.clear_dirty();
    store.set_transform(s, at(2.0, 2.0), 0.0).unwrap();
    assert_eq!(
        store.dirty_static().iter().collect::<Vec<_>>(),
        [s.index() as usize]
    );

    // Clearing keeps the versions.
    let (v, r) = (store.static_version(), store.revision());
    store.clear_dirty();
    assert_eq!((store.static_version(), store.revision()), (v, r));
}

#[test]
fn destroying_marks_the_slot_dirty_and_a_failed_call_marks_nothing() {
    let mut store = ObstacleStore::new();
    let a = store.create_shape(&square()).unwrap();
    store.clear_dirty();
    assert!(store.set_shape(a, &ShapeDesc::polygon(&BOWTIE)).is_err());
    assert!(store.set_transform(a, at(NAN, 0.0), 0.0).is_err());
    assert!(!store.is_dirty());
    store.destroy(a).unwrap();
    assert!(store.dirty_static().contains(a.index() as usize));
    assert!(store.get(a).is_none()); // "removed here"
    let revision = store.revision();
    assert!(store.destroy(a).is_err());
    assert_eq!(store.revision(), revision);
}

// ---- snapshots ----

fn filled() -> (ObstacleStore, Vec<ObstacleId>) {
    let mut store = ObstacleStore::new();
    let mut ids = Vec::new();
    let mut d = ShapeDesc::rect(4.0, 2.0);
    d.common.dynamic = true;
    d.common.opacity = 0.5;
    d.common.height = 3.0;
    ids.push(store.create_shape(&d).unwrap());
    ids.push(store.create_shape(&square()).unwrap());
    ids.push(store.create_shape(&ShapeDesc::circle(2.0)).unwrap());
    let cells = [1u8, 2, 0, 3];
    let (base, opacity, edges) = ([0u8, 1, 0, 2], [255u8, 100, 0, 255], [0u8, 5, 0, 15]);
    let mut t = tiles(&cells);
    t.level_height = 1.5;
    t.edge_width = 0.25;
    t.base_levels = Some(&base);
    t.opacity = Some(&opacity);
    t.edges = Some(&edges);
    ids.push(store.create_tile_grid(&t).unwrap());
    store.destroy(ids.remove(1)).unwrap(); // a hole in the slots
    store.set_transform(ids[0], at(3.0, 4.0), 33.0).unwrap();
    store.set_shape(ids[1], &ShapeDesc::circle(3.0)).unwrap();
    (store, ids)
}

#[test]
fn snapshot_round_trip_keeps_handles_geometry_and_tracking() {
    let (store, ids) = filled();
    let mut buf = SnapshotBuf::new();
    buf.save(&store);

    let mut other = ObstacleStore::new();
    other.create_shape(&square()).unwrap(); // something to be replaced
    restore(&mut Reader::new(buf.bytes()), &mut other).unwrap();

    assert_eq!(other.len(), store.len());
    for &id in &ids {
        assert_eq!(other.get(id), store.get(id), "obstacle {id:?}");
    }
    assert_eq!(other.revision(), store.revision());
    assert_eq!(other.static_version(), store.static_version());
    assert_eq!(other.dirty_static(), store.dirty_static());
    assert_eq!(other.dirty_dynamic(), store.dirty_dynamic());
    assert_eq!(other.any_partial(), store.any_partial());
    assert!(other.any_partial());
    assert_eq!(other.max_finite_top(), store.max_finite_top());

    // The restored store keeps working: new handles are the same as in the original.
    let mut a = store.clone();
    let id_a = a.create_shape(&square()).unwrap();
    let id_b = other.create_shape(&square()).unwrap();
    assert_eq!(id_a, id_b);

    // Saving the restored store gives the same bytes.
    let mut again = SnapshotBuf::new();
    again.save(&store);
    assert_eq!(again.bytes(), buf.bytes());
}

fn tamper(store: &ObstacleStore, edit: impl FnOnce(&mut Vec<u8>)) -> Result<(), SnapshotError> {
    let mut buf = SnapshotBuf::new();
    buf.save(store);
    let mut bytes = buf.bytes().to_vec();
    edit(&mut bytes);
    restore(&mut Reader::new(&bytes), &mut ObstacleStore::new())
}

#[test]
fn restore_rejects_invalid_obstacles() {
    let mut store = ObstacleStore::new();
    let mut d = ShapeDesc::circle(2.0);
    d.common.opacity = 0.25;
    store.create_shape(&d).unwrap();
    assert!(tamper(&store, |_| {}).is_ok());

    // The opacity 0.25 (f32 little endian 0x3e800000) becomes 5.0, the radius 2.0 becomes NaN.
    let find = |b: &[u8], v: f32| {
        let n = v.to_le_bytes();
        b.windows(4)
            .position(|w| w == n)
            .expect("value in snapshot")
    };
    for (from, to) in [(0.25f32, 5.0f32), (2.0, f32::NAN), (2.0, -1.0)] {
        let r = tamper(&store, |b| {
            let at = find(b, from);
            b[at..at + 4].copy_from_slice(&to.to_le_bytes());
        });
        assert!(
            matches!(r, Err(SnapshotError::Invalid(_))),
            "{from} -> {to}: {r:?}"
        );
    }

    // A polygon that turned self-intersecting.
    let mut p = ObstacleStore::new();
    p.create_shape(&square()).unwrap();
    let r = tamper(&p, |b| {
        // The corners (0,0) (1,0) (1,1) (0,1) become (0,0) (1,1) (1,0) (0,1): a bow tie.
        let mut tail = Vec::new();
        for v in [0.0f32, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0] {
            tail.extend_from_slice(&v.to_le_bytes());
        }
        let start = b
            .windows(tail.len())
            .position(|w| {
                w == SQUARE
                    .iter()
                    .flat_map(|v| [v.x.to_le_bytes(), v.y.to_le_bytes()].concat())
                    .collect::<Vec<u8>>()
            })
            .expect("outline");
        b[start..start + tail.len()].copy_from_slice(&tail);
    });
    assert!(matches!(r, Err(SnapshotError::Invalid(_))), "{r:?}");
}

#[test]
fn restore_rejects_a_wrong_version() {
    let mut store = ObstacleStore::new();
    let mut r = Reader::new(&[]);
    assert!(matches!(
        store.restore(9, &mut r),
        Err(SnapshotError::UnsupportedVersion { version: 9, .. })
    ));
}

// ---- P13b: geometry lives in store arenas ----

/// Replacing obstacles by shapes of changing size leaves garbage in the arenas and compacts it;
/// every obstacle keeps reading its own geometry, and a snapshot of the churned store restores
/// to the same bytes.
#[test]
fn arenas_survive_churn_and_compaction() {
    let mut store = ObstacleStore::new();
    let poly = [
        Vec2::new(0.0, 0.0),
        Vec2::new(4.0, 0.0),
        Vec2::new(4.0, 1.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(1.0, 4.0),
        Vec2::new(0.0, 4.0),
    ];
    let shape = |k: usize| match k % 3 {
        0 => ShapeDesc::polygon(&poly[..]),
        1 => ShapeDesc::rect(2.0, 1.0),
        _ => ShapeDesc::circle(1.5),
    };
    let ids: Vec<ObstacleId> = (0..10)
        .map(|k| store.create_shape(&shape(k)).unwrap())
        .collect();
    // Reference: a fresh store holding the same final shapes.
    for round in 1..40 {
        for (k, &id) in ids.iter().enumerate() {
            store.set_shape(id, &shape(k + round)).unwrap();
        }
    }
    let mut fresh = ObstacleStore::new();
    let fresh_ids: Vec<ObstacleId> = (0..10)
        .map(|k| fresh.create_shape(&shape(k + 39)).unwrap())
        .collect();
    for (&id, &f) in ids.iter().zip(&fresh_ids) {
        assert_eq!(store.triangles(id), fresh.triangles(f));
        assert_eq!(store.outline(id), fresh.outline(f));
    }
    // A batch replacement and destroying obstacles also keep the rest readable.
    let descs: Vec<ShapeDesc<'_>> = (0..5).map(|k| shape(k + 1)).collect();
    store.set_shapes(&ids[..5], &descs[..]).unwrap();
    store.destroy_many(&ids[5..8]).unwrap();
    for (k, &id) in ids[..5].iter().enumerate() {
        let mut f = ObstacleStore::new();
        let fid = f.create_shape(&shape(k + 1)).unwrap();
        assert_eq!(store.triangles(id), f.triangles(fid), "obstacle {k}");
    }
    assert!(store.triangles(ids[6]).is_empty(), "stale handle");

    let mut buf = SnapshotBuf::new();
    buf.save(&store);
    let mut other = ObstacleStore::new();
    restore(&mut Reader::new(buf.bytes()), &mut other).unwrap();
    let mut again = SnapshotBuf::new();
    again.save(&other);
    assert_eq!(again.bytes(), buf.bytes());
}

/// A failing batch leaves the arenas as they were.
#[test]
fn a_failed_batch_leaves_no_geometry_behind() {
    let mut store = ObstacleStore::new();
    let id = store.create_shape(&square()).unwrap();
    let before = store.triangles(id).to_vec();
    let mut buf = SnapshotBuf::new();
    buf.save(&store);
    let bow = [
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(0.0, 1.0),
    ];
    let descs = [ShapeDesc::circle(1.0), ShapeDesc::polygon(&bow[..])];
    let mut out = Vec::new();
    assert!(store.create_shapes(&descs[..], &mut out).is_err());
    assert!(out.is_empty());
    assert_eq!(store.triangles(id), &before[..]);
    let mut after = SnapshotBuf::new();
    after.save(&store);
    assert_eq!(after.bytes(), buf.bytes());
}

/// Triangles that do not belong to the described shape, or a span outside the arena, are
/// refused on restore.
#[test]
fn restore_rejects_foreign_triangles_and_stray_spans() {
    let mut store = ObstacleStore::new();
    store.create_shape(&ShapeDesc::circle(2.0)).unwrap();
    let mut buf = SnapshotBuf::new();
    buf.save(&store);
    // The last vertex of the triangle arena is the tail of the stream before the empty outline
    // arena (an 8 byte length prefix with 0): nudge its last float.
    let mut bytes = buf.bytes().to_vec();
    let n = bytes.len();
    let at = n - 8 /* outline arena length */ - 4;
    bytes[at] ^= 0x01;
    let r = restore(&mut Reader::new(&bytes), &mut ObstacleStore::new());
    assert!(matches!(r, Err(SnapshotError::Invalid(_))), "{r:?}");

    // Triangle span longer than the arena.
    let mut bytes = buf.bytes().to_vec();
    let key = 2.0f32.to_le_bytes(); // radius, directly followed by the span (start, len)
    let radius = bytes.windows(4).position(|w| w == key).expect("radius");
    let len_at = radius + 4 + 4;
    bytes[len_at] = bytes[len_at].wrapping_add(3);
    let r = restore(&mut Reader::new(&bytes), &mut ObstacleStore::new());
    assert!(matches!(r, Err(SnapshotError::Invalid(_))), "{r:?}");
}
