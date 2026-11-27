//! P22, the GPU-free half of the obstacle raster: what goes into the triangle buffers and the
//! cell textures, which cells are pending, the uniform blocks. Origin of the expectations:
//! chiaro @ 9bc5b9b, src/rendering/OccluderGeometry.cpp and TileOccluders.cpp, and the cases of
//! tests/gpu/OcclusionTests.cpp that do not need the device (zlib license, see
//! `src/occluder_raster/`). The pixels are checked in `gpu_occluder_raster.rs`.

use std::rc::Rc;

use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc, TileGridDesc};
use knorke_light2d::occluder_raster::{
    CELL_BYTES, DomainParams, INFINITE_TOP, NoPatterns, TileSet, TileUniforms, TriangleSet,
    interval_channels, thin_circle_radius, triangle_buffer_floats, write_cell_texels,
    write_triangle_buffer,
};
use knorke_light2d::shadow::{CellRect, MaskAnchor, OcclusionDomain};
use knorke_light2d::{ObstacleId, ObstacleStore, Projection};
use knorke_math::{At, Vec2, det};

fn at(x: f32, y: f32) -> At<knorke_math::World2, Vec2> {
    At::new(Vec2::new(x, y))
}

fn shape(store: &mut ObstacleStore, shape: Shape<'_>, common: ObstacleCommon) -> ObstacleId {
    store.create_shape(&ShapeDesc { common, shape }).unwrap()
}

fn rect(w: f32, d: f32) -> Shape<'static> {
    Shape::Box { width: w, depth: d }
}

fn grid(store: &mut ObstacleStore, columns: u32, rows: u32) -> ObstacleId {
    let solid = vec![1u8; (columns * rows) as usize];
    store
        .create_tile_grid(&TileGridDesc::new(columns, rows, &solid))
        .unwrap()
}

fn collect(store: &ObstacleStore, dynamic: bool, min_radius: f32) -> TriangleSet {
    let mut set = TriangleSet::new();
    set.collect(store, &NoPatterns, dynamic, min_radius);
    set
}

// ---- domain parameters ----

#[test]
fn domain_params_follow_projection_domain_and_anchor() {
    let mut p = Projection::IDENTITY;
    p.origin = At::new(Vec2::new(10.5, 3.25));
    p.x_axis = Vec2::new(2.0, 0.5);
    p.y_axis = Vec2::new(-0.5, 1.5);
    let d = OcclusionDomain::new(128, 64, 0.5);
    let a = MaskAnchor::compute(&p, &d);
    let params = DomainParams::new(&p, &d, &a);
    assert_eq!(params.origin_x_axis, [10.5, 3.25, 2.0, 0.5]);
    assert_eq!(params.y_axis_margin, [-0.5, 1.5, 32.0, 32.0]);
    assert_eq!(
        params.texel_scale_domain_size,
        [
            d.texels_per_scene[0],
            d.texels_per_scene[1],
            d.width as f32,
            d.height as f32
        ]
    );
    assert_eq!(
        params.remainder_pad,
        [a.remainder[0], a.remainder[1], 0.0, 0.0]
    );

    // The start of the opacities travels as the bits of a u32 in the pad.
    let params = params.with_opacity_start(7);
    assert_eq!(params.remainder_pad[2].to_bits(), 7);
    let bytes = params.to_bytes();
    assert_eq!(bytes.len(), 64);
    assert_eq!(bytes[..4], 10.5f32.to_le_bytes());
    assert_eq!(bytes[56..60], 7u32.to_le_bytes());
}

#[test]
fn thin_circle_radius_is_half_a_texel_along_the_shorter_axis() {
    let identity = Projection::IDENTITY;
    let full = OcclusionDomain::new(128, 64, 1.0);
    assert_eq!(thin_circle_radius(&identity, &full), 0.5);
    // Half resolution: a texel is two scene pixels wide.
    let half = OcclusionDomain::new(128, 64, 0.5);
    assert_eq!(thin_circle_radius(&identity, &half), 1.0);
    // A stretched x axis: the shorter axis (y) decides.
    let mut wide = Projection::IDENTITY;
    wide.x_axis = Vec2::new(4.0, 0.0);
    assert_eq!(thin_circle_radius(&wide, &full), 0.5);
}

// ---- triangles ----

#[test]
fn static_and_dynamic_triangles_are_collected_apart_and_inactive_ones_are_skipped() {
    let mut store = ObstacleStore::new();
    let fixed = ObstacleCommon::default();
    let moving = ObstacleCommon {
        dynamic: true,
        ..fixed
    };
    shape(&mut store, rect(8.0, 4.0), fixed); // 2 triangles
    shape(&mut store, rect(2.0, 2.0), moving); // 2 triangles
    shape(
        &mut store,
        Shape::Polygon(&[
            Vec2::new(0.0, 0.0),
            Vec2::new(4.0, 0.0),
            Vec2::new(0.0, 4.0),
        ]),
        moving,
    ); // 1 triangle
    grid(&mut store, 4, 3); // not triangles
    shape(
        &mut store,
        rect(5.0, 5.0),
        ObstacleCommon {
            enabled: false,
            ..fixed
        },
    );
    shape(
        &mut store,
        rect(5.0, 5.0),
        ObstacleCommon {
            opacity: 0.0,
            ..fixed
        },
    );

    let fixed_set = collect(&store, false, 0.5);
    assert_eq!(fixed_set.vertices().len(), 6);
    assert_eq!(fixed_set.opacities(), &[1.0, 1.0]);
    let moving_set = collect(&store, true, 0.5);
    assert_eq!(moving_set.vertices().len(), 9);
    assert_eq!(moving_set.opacities().len(), 3);
}

#[test]
fn triangles_are_in_world_space() {
    let mut store = ObstacleStore::new();
    let common = ObstacleCommon {
        position: at(10.0, 20.0),
        rotation: 90.0,
        ..Default::default()
    };
    shape(&mut store, rect(8.0, 4.0), common);
    let set = collect(&store, false, 0.5);
    // Local x in [-4, 4], y in [-2, 2]; a quarter turn counter-clockwise swaps the extents.
    let (mut lo, mut hi) = (Vec2::new(1e9, 1e9), Vec2::new(-1e9, -1e9));
    for v in set.vertices() {
        let v = v.raw();
        lo = Vec2::new(det::min(lo.x, v.x), det::min(lo.y, v.y));
        hi = Vec2::new(det::max(hi.x, v.x), det::max(hi.y, v.y));
    }
    for (got, want) in [(lo.x, 8.0), (lo.y, 16.0), (hi.x, 12.0), (hi.y, 24.0)] {
        assert!((got - want).abs() < 1e-4, "{got} vs {want}");
    }
}

#[test]
fn every_triangle_carries_the_mask_opacity_of_its_obstacle() {
    let mut store = ObstacleStore::new();
    let base = ObstacleCommon::default();
    shape(
        &mut store,
        rect(4.0, 4.0),
        ObstacleCommon {
            opacity: 0.5,
            ..base
        },
    );
    shape(
        &mut store,
        rect(4.0, 4.0),
        ObstacleCommon {
            opacity: 0.25,
            shadow_footprint: true,
            ..base
        },
    );
    let set = collect(&store, false, 0.5);
    // A footprint obstacle passes its opacity negated (the mask's G channel).
    assert_eq!(set.opacities(), &[0.5, 0.5, -0.25, -0.25]);
}

#[test]
fn a_pattern_lowers_the_opacity_to_its_mean() {
    let mut store = ObstacleStore::new();
    let id = shape(&mut store, rect(4.0, 4.0), ObstacleCommon::default());
    let patterns = |pattern: u32| (pattern == 1).then_some(0.5);
    store.set_pattern(id, 1, &patterns).unwrap();
    let mut set = TriangleSet::new();
    set.collect(&store, &patterns, false, 0.5);
    assert_eq!(set.opacities(), &[0.5, 0.5]);
    // Without a pattern store the pattern is gone: the scalar opacity.
    set.collect(&store, &NoPatterns, false, 0.5);
    assert_eq!(set.opacities(), &[1.0, 1.0]);
}

#[test]
fn circles_thinner_than_a_texel_are_widened_about_their_centre() {
    let mut store = ObstacleStore::new();
    let centre = ObstacleCommon {
        position: at(5.0, 5.0),
        ..Default::default()
    };
    shape(&mut store, Shape::Circle { radius: 0.25 }, centre);
    shape(&mut store, Shape::Circle { radius: 2.0 }, centre);
    let set = collect(&store, false, 0.5);
    assert_eq!(set.thin(), 1);
    assert_eq!(set.smallest_circle(), 0.25);
    let per = set.vertices().len() / 2;
    let reach = |vs: &[At<knorke_math::World2, Vec2>]| {
        vs.iter()
            .map(|v| {
                let v = v.raw();
                ((v.x - 5.0) * (v.x - 5.0) + (v.y - 5.0) * (v.y - 5.0)).sqrt()
            })
            .fold(0.0f32, det::max)
    };
    assert!((reach(&set.vertices()[..per]) - 0.5).abs() < 1e-4);
    assert!((reach(&set.vertices()[per..]) - 2.0).abs() < 1e-4);

    // Nothing thin at a smaller limit; no circle: nothing to report.
    let none = collect(&store, false, 0.1);
    assert_eq!(none.thin(), 0);
    let empty = collect(&ObstacleStore::new(), false, 0.5);
    assert!(empty.is_empty());
    assert_eq!(empty.smallest_circle(), 1e30);
}

#[test]
fn height_intervals_hold_the_obstacle_interval_and_its_finite_copy() {
    let mut store = ObstacleStore::new();
    let infinite = shape(&mut store, rect(2.0, 2.0), ObstacleCommon::default());
    let finite = shape(
        &mut store,
        rect(2.0, 2.0),
        ObstacleCommon {
            height: 3.0,
            base_height: 1.0,
            ..Default::default()
        },
    );
    assert_eq!(
        interval_channels(store.get(infinite).unwrap()),
        [0.0, INFINITE_TOP, -INFINITE_TOP, -INFINITE_TOP]
    );
    assert_eq!(
        interval_channels(store.get(finite).unwrap()),
        [-1.0, 3.0, -1.0, 3.0]
    );
    let set = collect(&store, false, 0.5);
    assert_eq!(set.intervals().len(), 4);
    assert_eq!(
        set.intervals()[0],
        [0.0, INFINITE_TOP, -INFINITE_TOP, -INFINITE_TOP]
    );
    assert_eq!(set.intervals()[3], [-1.0, 3.0, -1.0, 3.0]);
}

#[test]
fn the_triangle_buffer_holds_vertices_then_opacities_padded_to_whole_pairs() {
    assert_eq!(triangle_buffer_floats(3, 1), 8);
    assert_eq!(triangle_buffer_floats(6, 2), 14);
    assert_eq!(triangle_buffer_floats(0, 0), 0);

    let vertices = [at(1.0, 2.0), at(3.0, 4.0), at(5.0, 6.0)];
    let mut out = vec![0xAAu8; 8 * 4 + 4];
    write_triangle_buffer(&vertices, &[0.75], &mut out);
    let floats: Vec<f32> = out[..32]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    assert_eq!(floats, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.75, 0.0]);
    assert_eq!(out[32], 0xAA, "bytes past the layout are not touched");
}

// ---- tile uniforms ----

#[test]
fn tile_uniforms_place_the_grid_and_describe_its_heights() {
    let mut store = ObstacleStore::new();
    let solid = vec![1u8; 6];
    let edges = vec![1u8; 6];
    let mut d = TileGridDesc::new(3, 2, &solid);
    d.common.position = at(3.0, 4.0);
    d.common.rotation = 90.0;
    d.cell_size = Vec2::new(8.0, 6.0);
    d.level_height = 2.0;
    d.edge_width = 2.0;
    d.edges = Some(&edges);
    let id = store.create_tile_grid(&d).unwrap();
    let u = TileUniforms::of(store.get(id).unwrap());
    assert_eq!(u.placement[0], 3.0);
    assert_eq!(u.placement[1], 4.0);
    assert!((u.placement[2] - core::f32::consts::FRAC_PI_2).abs() < 1e-6);
    assert_eq!(u.grid_size, [8.0, 6.0, 3.0, 2.0]);
    // Cell height 2, the obstacle itself infinitely tall, but finite through its levels.
    assert_eq!(u.heights[..4], [2.0, INFINITE_TOP, 0.0, 1.0]);
    // Edge strips: width as a share of the cell per axis, and the grid size.
    assert_eq!(u.heights[4..], [0.25, 2.0 / 6.0, 3.0, 2.0]);

    let domain = DomainParams::default();
    let params = u.params(&domain);
    assert_eq!(params.placement, u.placement);
    assert_eq!(params.to_bytes().len(), 96);
    assert_eq!(u.heights_bytes().len(), 32);

    // Whole cells: no strips.
    let whole = grid(&mut store, 2, 2);
    let u = TileUniforms::of(store.get(whole).unwrap());
    assert_eq!(u.heights[4..], [0.0; 4]);
    assert_eq!(u.heights[3], 0.0);
}

// ---- which grids have a texture and which cells are pending ----

type Made = Rc<()>;

fn sync(set: &mut TileSet<Made>, store: &ObstacleStore, made: &Made, log: &mut Vec<(u32, u32)>) {
    set.sync(store, |c, r| {
        log.push((c, r));
        Ok::<_, ()>(Rc::clone(made))
    })
    .unwrap();
}

fn clear_pending(set: &mut TileSet<Made>) {
    for s in set.slots_mut() {
        s.pending = None;
    }
}

fn all(columns: u32, rows: u32) -> Option<CellRect> {
    Some(CellRect {
        x0: 0,
        y0: 0,
        x1: columns,
        y1: rows,
    })
}

#[test]
fn a_new_grid_gets_a_texture_with_every_cell_pending() {
    let mut store = ObstacleStore::new();
    let id = grid(&mut store, 4, 3);
    grid(&mut store, 2, 2);
    let (made, mut log, mut set) = (Rc::new(()), Vec::new(), TileSet::new());
    sync(&mut set, &store, &made, &mut log);
    assert_eq!(log, [(4, 3), (2, 2)]);
    assert_eq!(set.len(), 2);
    assert_eq!(set.find(id).unwrap().pending, all(4, 3));
    assert!(set.any_pending());

    // Uploaded and unchanged: nothing is created or pending again.
    clear_pending(&mut set);
    sync(&mut set, &store, &made, &mut log);
    assert_eq!(log.len(), 2);
    assert!(!set.any_pending());
}

#[test]
fn edited_cells_are_pending_as_one_bounding_box() {
    let mut store = ObstacleStore::new();
    let id = grid(&mut store, 8, 6);
    let (made, mut log, mut set) = (Rc::new(()), Vec::new(), TileSet::new());
    sync(&mut set, &store, &made, &mut log);
    clear_pending(&mut set);

    store
        .set_tile_cells(id, CellRect::new(1, 1, 1, 1), Some(&[0]), None, None)
        .unwrap();
    store
        .set_tile_cells(id, CellRect::new(4, 3, 2, 1), Some(&[0, 0]), None, None)
        .unwrap();
    sync(&mut set, &store, &made, &mut log);
    assert_eq!(
        set.find(id).unwrap().pending,
        Some(CellRect {
            x0: 1,
            y0: 1,
            x1: 6,
            y1: 4
        })
    );
    assert_eq!(log.len(), 1, "no new texture for an edit");

    // The edits are consumed: the same version does not mark them again.
    clear_pending(&mut set);
    sync(&mut set, &store, &made, &mut log);
    assert!(!set.any_pending());
}

#[test]
fn a_cell_log_that_lost_changes_uploads_everything() {
    let mut store = ObstacleStore::new();
    let id = grid(&mut store, 8, 6);
    let (made, mut log, mut set) = (Rc::new(()), Vec::new(), TileSet::new());
    sync(&mut set, &store, &made, &mut log);
    clear_pending(&mut set);
    for _ in 0..70 {
        store
            .set_tile_cells(id, CellRect::new(0, 0, 1, 1), Some(&[1]), None, None)
            .unwrap();
    }
    sync(&mut set, &store, &made, &mut log);
    assert_eq!(set.find(id).unwrap().pending, all(8, 6));
}

#[test]
fn edits_of_another_grid_do_not_touch_this_one() {
    let mut store = ObstacleStore::new();
    let a = grid(&mut store, 4, 4);
    let b = grid(&mut store, 4, 4);
    let (made, mut log, mut set) = (Rc::new(()), Vec::new(), TileSet::new());
    sync(&mut set, &store, &made, &mut log);
    clear_pending(&mut set);
    store
        .set_tile_cells(b, CellRect::new(2, 2, 1, 1), Some(&[0]), None, None)
        .unwrap();
    sync(&mut set, &store, &made, &mut log);
    assert_eq!(set.find(a).unwrap().pending, None);
    assert_eq!(
        set.find(b).unwrap().pending,
        Some(CellRect {
            x0: 2,
            y0: 2,
            x1: 3,
            y1: 3
        })
    );
}

#[test]
fn a_replaced_grid_uploads_everything_and_a_resized_one_gets_a_new_texture() {
    let mut store = ObstacleStore::new();
    let id = grid(&mut store, 4, 3);
    let (made, mut log, mut set) = (Rc::new(()), Vec::new(), TileSet::new());
    sync(&mut set, &store, &made, &mut log);
    clear_pending(&mut set);

    let same = vec![1u8; 12];
    store
        .set_tile_grid(id, &TileGridDesc::new(4, 3, &same))
        .unwrap();
    sync(&mut set, &store, &made, &mut log);
    assert_eq!(log.len(), 1, "same size keeps the texture");
    assert_eq!(set.find(id).unwrap().pending, all(4, 3));
    clear_pending(&mut set);

    let bigger = vec![1u8; 20];
    store
        .set_tile_grid(id, &TileGridDesc::new(5, 4, &bigger))
        .unwrap();
    sync(&mut set, &store, &made, &mut log);
    assert_eq!(log, [(4, 3), (5, 4)]);
    assert_eq!(set.find(id).unwrap().pending, all(5, 4));
    assert_eq!(Rc::strong_count(&made), 2, "the old texture is released");
}

#[test]
fn destroyed_disabled_or_retyped_grids_release_their_texture() {
    let mut store = ObstacleStore::new();
    let a = grid(&mut store, 2, 2);
    let b = grid(&mut store, 2, 2);
    let c = grid(&mut store, 2, 2);
    let (made, mut log, mut set) = (Rc::new(()), Vec::new(), TileSet::new());
    sync(&mut set, &store, &made, &mut log);
    assert_eq!(Rc::strong_count(&made), 4);

    store.destroy(a).unwrap();
    let solid = vec![1u8; 4];
    let mut off = TileGridDesc::new(2, 2, &solid);
    off.common.enabled = false;
    store.set_tile_grid(b, &off).unwrap();
    store
        .set_shape(
            c,
            &ShapeDesc {
                common: ObstacleCommon::default(),
                shape: rect(1.0, 1.0),
            },
        )
        .unwrap();
    sync(&mut set, &store, &made, &mut log);
    assert!(set.is_empty());
    assert_eq!(Rc::strong_count(&made), 1);

    // Enabled again: a new texture, everything pending.
    let on = TileGridDesc::new(2, 2, &solid);
    store.set_tile_grid(b, &on).unwrap();
    sync(&mut set, &store, &made, &mut log);
    assert_eq!(set.len(), 1);
    assert_eq!(set.find(b).unwrap().pending, all(2, 2));
}

#[test]
fn a_failing_texture_keeps_the_other_grids_and_is_retried() {
    let mut store = ObstacleStore::new();
    let a = grid(&mut store, 2, 2);
    let b = grid(&mut store, 3, 3);
    let mut set: TileSet<u32> = TileSet::new();
    let result = set.sync(&store, |c, _| if c == 3 { Err("no memory") } else { Ok(c) });
    assert_eq!(result, Err("no memory"));
    assert_eq!(set.find(a).unwrap().texture, Some(2));
    assert_eq!(set.find(b).unwrap().texture, None);

    set.sync(&store, |c, _| Ok::<_, &str>(c)).unwrap();
    assert_eq!(set.find(b).unwrap().texture, Some(3));
    assert_eq!(set.find(b).unwrap().pending, all(3, 3));
}

// ---- cell texels ----

fn texels(
    store: &ObstacleStore,
    id: ObstacleId,
    r: CellRect,
    patterns: &impl knorke_light2d::obstacle::PatternSource,
) -> Vec<u8> {
    let mut out = vec![0u8; r.area() * CELL_BYTES];
    write_cell_texels(store, id, &r, patterns, &mut out);
    out
}

#[test]
fn cell_texels_hold_solid_bottom_opacity_and_edges() {
    let mut store = ObstacleStore::new();
    let mut solid = vec![2u8; 12];
    solid[0] = 0;
    let base = vec![1u8; 12];
    let mut opacity = vec![200u8; 12];
    opacity[1] = 255;
    let mut edges = vec![0u8; 12];
    edges[2] = 5;
    let mut d = TileGridDesc::new(4, 3, &solid);
    d.level_height = 10.0;
    d.base_levels = Some(&base);
    d.opacity = Some(&opacity);
    d.edge_width = 0.25;
    d.edges = Some(&edges);
    d.common.opacity = 0.5;
    let id = store.create_tile_grid(&d).unwrap();

    // Cells (0..3, 0..2): row 0 is cells 0, 1, 2; row 1 cells 4, 5, 6.
    let got = texels(&store, id, CellRect::new(0, 0, 3, 2), &NoPatterns);
    #[rustfmt::skip]
    let want = [
        0, 1, 100, 255,   2, 1, 128, 255,   2, 1, 100, 5,
        2, 1, 100, 255,   2, 1, 100, 255,   2, 1, 100, 255,
    ];
    assert_eq!(got, want);

    // A partial rectangle reads the cells it names.
    let got = texels(&store, id, CellRect::new(1, 1, 2, 1), &NoPatterns);
    assert_eq!(got, [2, 1, 100, 255, 2, 1, 100, 255]);
    let got = texels(&store, id, CellRect::new(2, 0, 1, 1), &NoPatterns);
    assert_eq!(got, [2, 1, 100, 5]);
}

#[test]
fn without_an_opacity_channel_the_grids_opacity_fills_every_cell() {
    let mut store = ObstacleStore::new();
    let solid = vec![1u8; 4];
    let mut d = TileGridDesc::new(2, 2, &solid);
    d.common.opacity = 0.5;
    let id = store.create_tile_grid(&d).unwrap();
    let got = texels(&store, id, CellRect::new(0, 0, 2, 2), &NoPatterns);
    assert_eq!(got[2], 128);
    assert_eq!(got[6], 128);
    assert_eq!(got[3], 255);
}

#[test]
fn a_patterned_grid_writes_the_mask_opacity_per_cell() {
    let mut store = ObstacleStore::new();
    let solid = vec![1u8; 4];
    let opacity = [255u8, 100, 255, 255];
    let mut d = TileGridDesc::new(2, 2, &solid);
    d.opacity = Some(&opacity);
    let id = store.create_tile_grid(&d).unwrap();
    let patterns = |pattern: u32| (pattern == 3).then_some(0.5);
    store.set_pattern(id, 3, &patterns).unwrap();
    let got = texels(&store, id, CellRect::new(0, 0, 2, 1), &patterns);
    // mean 0.5 times the cell's opacity: cell 0 0.5, cell 1 0.5 * 100 / 255.
    assert_eq!(got[2], 128);
    assert_eq!(got[6], (255.0f32 * 0.5 * 100.0 / 255.0).round() as u8);
}

#[test]
fn cell_texels_ignore_stale_handles_and_rectangles_outside_the_grid() {
    let mut store = ObstacleStore::new();
    let id = grid(&mut store, 2, 2);
    let mut out = vec![9u8; 16];
    write_cell_texels(
        &store,
        id,
        &CellRect::new(1, 1, 2, 2),
        &NoPatterns,
        &mut out,
    );
    assert_eq!(out, vec![9u8; 16]);
    store.destroy(id).unwrap();
    write_cell_texels(
        &store,
        id,
        &CellRect::new(0, 0, 2, 2),
        &NoPatterns,
        &mut out,
    );
    assert_eq!(out, vec![9u8; 16]);
}
