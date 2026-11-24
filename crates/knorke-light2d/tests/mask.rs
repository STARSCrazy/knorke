//! Origin: chiaro @ 9bc5b9b, tests/unit/MaskTests.cpp (zlib license, see `src/mask.rs`,
//! `src/mask_store.rs` and `src/light_mask.rs`), without the culling test (it lives in
//! `light_culling.rs`), plus tests for what the redesign adds: typed errors with stable codes,
//! the in-use check of `destroy` and snapshots.

// Tests compare against platform `std` maths as an oracle; only library code must be platform
// independent (ADR 0011).
#![allow(clippy::disallowed_methods)]

use knorke_light2d::light::{Height, LightDesc, LightType, MaskMode, StoredLight};
use knorke_light2d::light_mask::*;
use knorke_light2d::mask::*;
use knorke_light2d::mask_store::*;
use knorke_light2d::{AnimationStore, LightStore, StoreResources};
use knorke_math::{At, Vec2, Vec3};
use knorke_types::{Reader, SnapshotBuf, SnapshotError, restore};

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-5 * b.abs().max(1.0)
}

fn near_eps(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps * b.abs().max(1.0)
}

fn shape_texels(shape: &MaskShape) -> Vec<u8> {
    shape.validate().unwrap();
    let mut texels = Vec::new();
    shape.build(&mut texels);
    assert_eq!(texels.len(), MASK_BYTES);
    texels
}

fn cross() -> Vec<u8> {
    shape_texels(&MaskShape::preset(MaskPreset::WindowCross))
}

fn sample_r(texels: &[u8], u: f32, v: f32) -> f32 {
    sample_mask_texels(texels, u, v).x
}

fn spot(x: f32, y: f32, z: f32) -> LightDesc {
    let mut d = LightDesc::new(LightType::Spot);
    d.position = At::new(Vec2::new(x, y));
    d.height = Height::At(z);
    d.direction = v(0.0, 0.0, -1.0);
    d.outer_angle = 40.0;
    d.radius = 100.0;
    d
}

fn rejected(base: &MaskShape, mutate: impl FnOnce(&mut MaskShape)) -> String {
    let mut s = *base;
    mutate(&mut s);
    s.validate().expect_err("must be rejected").to_string()
}

#[test]
fn presets_fill_every_field_and_pass_their_own_check() {
    for preset in [
        MaskPreset::WindowCross,
        MaskPreset::WindowHall,
        MaskPreset::Blinds,
    ] {
        let s = MaskShape::preset(preset);
        assert!(s.columns >= 1 && s.rows >= 1);
        assert_eq!(s.glass.x, 1.0);
        s.validate().unwrap();
    }
    let hall = MaskShape::preset(MaskPreset::WindowHall);
    assert_eq!((hall.columns, hall.rows), (4, 3));
    let cross = MaskShape::preset(MaskPreset::WindowCross);
    assert_eq!((cross.columns, cross.rows), (2, 2));
}

#[test]
fn shape_check_names_the_bad_field() {
    let base = MaskShape::preset(MaskPreset::WindowCross);
    assert!(rejected(&base, |s| s.columns = 0).contains("shape.columns"));
    assert!(rejected(&base, |s| s.bar_width = -0.1).contains("bar_width"));
    assert!(rejected(&base, |s| s.softness = f32::NAN).contains("softness"));
    assert!(rejected(&base, |s| s.glass.y = 2.0).contains("glass"));
    // With a bar of 0.06 nothing is left for the panes.
    assert!(rejected(&base, |s| s.frame_width = 0.48).contains("no glass"));
    let e = MaskShape { columns: 0, ..base }.validate().unwrap_err();
    assert_eq!(e.code().as_str(), "knorke.light2d.mask.out_of_range");
}

#[test]
fn window_cross_has_dark_bars_through_the_middle_and_the_frame_and_clear_panes() {
    let c = cross();
    assert!(sample_r(&c, 0.0, 0.5) < 0.01); // vertical bar
    assert!(sample_r(&c, 0.5, 0.0) < 0.01); // horizontal bar
    assert!(near(sample_r(&c, 0.5, 0.5), 1.0)); // a pane
    assert!(near(sample_r(&c, -0.5, -0.5), 1.0));
    assert!(sample_r(&c, 0.99, 0.5) < 0.01); // the frame
    assert_eq!(sample_r(&c, 1.5, 0.5), 0.0); // outside the mask
}

#[test]
fn coloured_glass_tints_per_channel() {
    let mut shape = MaskShape::preset(MaskPreset::WindowCross);
    shape.glass = v(1.0, 0.5, 0.0);
    let texels = shape_texels(&shape);
    let c = sample_mask_texels(&texels, 0.5, 0.5);
    assert!(near(c.x, 1.0));
    assert!(near_eps(c.y, 0.5, 0.01)); // sRGB 8 bit round trip
    assert_eq!(c.z, 0.0);
}

#[test]
fn pixel_masks_are_resampled_in_linear_light_first_row_on_top_errors_named() {
    let mut texels = Vec::new();
    build_pixel_mask(1, 1, &[255, 0, 0, 7], 4, &mut texels).unwrap();
    let c = sample_mask_texels(&texels, 0.3, -0.7);
    assert!(near(c.x, 1.0));
    assert_eq!(c.y, 0.0);
    // 2 x 2: top row white, bottom row black.
    let rows = [
        255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255,
    ];
    build_pixel_mask(2, 2, &rows, 8, &mut texels).unwrap();
    assert!(near(sample_r(&texels, 0.0, 0.9), 1.0));
    assert!(near(sample_r(&texels, 0.0, -0.9), 0.0));
    // Linear mid, not sRGB mid.
    assert!(near_eps(sample_r(&texels, 0.0, 0.0), 0.5, 0.02));
    // A 512 x 512 checker of 0 and 255 averages to linear 0.5 when shrunk 2:1.
    let mut checker = vec![0u8; 512 * 512 * 4];
    for y in 0..512usize {
        for x in 0..512usize {
            let value = if (x + y) & 1 == 1 { 255 } else { 0 };
            let p = (y * 512 + x) * 4;
            checker[p..p + 4].copy_from_slice(&[value, value, value, 255]);
        }
    }
    build_pixel_mask(512, 512, &checker, 512 * 4, &mut texels).unwrap();
    assert!(near_eps(sample_r(&texels, 0.1, 0.2), 0.5, 0.02));

    let red = [255u8, 0, 0, 7];
    let e = build_pixel_mask(0, 1, &red, 4, &mut texels).unwrap_err();
    assert!(e.to_string().contains("width and height"));
    let big = MAX_MASK_PIXELS + 1;
    let e = build_pixel_mask(big, 1, &red, 4 * big as usize, &mut texels).unwrap_err();
    assert!(e.to_string().contains("width and height"));
    let e = build_pixel_mask(2, 1, &red, 4, &mut texels).unwrap_err();
    assert!(e.to_string().contains("pitch"));
    // The slice replaces the NULL check: too short for width x height.
    let e = build_pixel_mask(2, 2, &rows[..8], 8, &mut texels).unwrap_err();
    assert!(e.to_string().contains("pixels"));
    assert_eq!(e.code().as_str(), "knorke.light2d.mask.pixels");
}

#[test]
fn atlas_blocks_are_a_power_of_two() {
    assert_eq!(mask_atlas_blocks(0), 0);
    assert_eq!(mask_atlas_blocks(1), 1);
    assert_eq!(mask_atlas_blocks(2), 2);
    assert_eq!(mask_atlas_blocks(3), 4);
    assert_eq!(mask_atlas_blocks(33), 64);
    assert_eq!(mask_atlas_blocks(64), 64);
}

#[test]
fn store_blocks_are_the_lowest_free_and_dead_handles_are_refused() {
    let lights = LightStore::new();
    let mut store = MaskStore::new();
    let a = store.create(cross()).unwrap();
    let b = store.create(cross()).unwrap();
    assert_eq!(store.block_of(a), Some(0));
    assert_eq!(store.block_of(b), Some(1));
    assert_eq!(store.block_count(), 2);
    assert!(store.contains(a));

    let version = store.version();
    store.destroy(a, &lights).unwrap();
    assert_ne!(store.version(), version);
    assert!(store.find(a).is_none());
    assert_eq!(store.block_of(a), None);
    assert_eq!(store.block_count(), 2); // block 1 is still alive

    let e = store.destroy(a, &lights).unwrap_err();
    assert!(e.to_string().contains("invalid or destroyed mask"));
    assert_eq!(e.code().as_str(), "knorke.light2d.mask.invalid_handle");

    // A new mask reuses the free block.
    let c = store.create(cross()).unwrap();
    assert_eq!(store.block_of(c), Some(0));
}

#[test]
fn atlas_blocks_stay_below_64_under_create_destroy_churn() {
    // Blocks bound the atlas height (mask_atlas_blocks <= 64), whatever slots the churn uses.
    let lights = LightStore::new();
    let mut store = MaskStore::new();
    let keep = store.create(cross()).unwrap();
    let texels = cross();
    for _ in 0..300 {
        let id = store.create(texels.clone()).unwrap();
        assert_eq!(store.block_of(id), Some(1));
        store.destroy(id, &lights).unwrap();
    }
    assert_eq!(store.block_of(keep), Some(0));
    assert_eq!(store.block_count(), 1);
    let last = store.create(texels.clone()).unwrap();
    assert_eq!(store.block_of(last), Some(1));
    assert_eq!(mask_atlas_blocks(store.block_count()), 2);
    // Full again after the churn: 64 masks in blocks 0..63, the 65th refused.
    for _ in 2..MAX_MASKS {
        store.create(texels.clone()).unwrap();
    }
    assert_eq!(store.block_count(), MAX_MASKS);
    assert_eq!(mask_atlas_blocks(store.block_count()), MAX_MASKS);
    let e = store.create(texels).unwrap_err();
    assert!(e.to_string().contains("64"));
    assert_eq!(e.code().as_str(), "knorke.light2d.mask.too_many");
    let mut blocks = 0;
    for (block, t) in store.iter() {
        assert!(block < MAX_MASKS);
        assert_eq!(t.len(), MASK_BYTES);
        blocks += 1;
    }
    assert_eq!(blocks, MAX_MASKS);
}

#[test]
fn store_refuses_texels_of_the_wrong_size() {
    let mut store = MaskStore::new();
    let e = store.create(vec![0; 10]).unwrap_err();
    assert_eq!(e.code().as_str(), "knorke.light2d.mask.texel_size");
    assert!(store.is_empty());
}

#[test]
fn a_mask_in_use_cannot_be_destroyed() {
    let animations = AnimationStore::new();
    let mut masks = MaskStore::new();
    let mask = masks.create(cross()).unwrap();
    let mut lights = LightStore::new();
    let mut d = spot(0.0, 0.0, 5.0);
    d.mask = Some(mask);
    let id = lights
        .create(&d, &StoreResources::new(&animations, &masks))
        .unwrap();
    let e = masks.destroy(mask, &lights).unwrap_err();
    assert_eq!(e.code().as_str(), "knorke.light2d.mask.in_use");
    assert!(masks.contains(mask));
    lights.destroy(id).unwrap();
    masks.destroy(mask, &lights).unwrap();
}

#[test]
fn window_axes_the_lights_check_and_the_placement_use_one_criterion() {
    let animations = AnimationStore::new();
    let mut masks = MaskStore::new();
    let mask = masks.create(cross()).unwrap();
    let res = StoreResources::new(&animations, &masks);
    let mut store = LightStore::new();
    let mut d = spot(0.0, 0.0, 5.0);
    d.mask = Some(mask);
    d.mask_mode = MaskMode::Window;
    d.mask_center = v(0.0, 0.0, 2.0);
    for sine in [0.002f32, 0.0005] {
        let u = v(1.0, 0.0, 0.0);
        let w = v((1.0 - sine * sine).sqrt(), sine, 0.0);
        d.mask_u = u;
        d.mask_v = w;
        let spans = window_axes_span_plane(u, w);
        assert_eq!(spans, sine > 0.001, "sine {sine}");
        let created = store.create(&d, &res);
        assert_eq!(created.is_ok(), spans, "sine {sine}");
        if !spans {
            assert!(created.unwrap_err().to_string().contains("parallel"));
        }
        assert_eq!(
            make_mask_placement(&StoredLight::derive(&d), 5.0).is_some(),
            spans
        );
    }
}

#[test]
fn projector_placement_u_along_c0_v_along_c90_the_half_angle_at_the_edge() {
    let mut d = spot(10.0, 20.0, 8.0);
    d.mask_mode = MaskMode::Projector;
    d.mask_angle = Some(45.0);
    let m = make_mask_placement(&StoredLight::derive(&d), 8.0).unwrap();
    assert!(!m.window);
    let c = mask_coordinates(&m, v(4.0, 0.0, -8.0)).unwrap(); // tan = 0.5 toward +x
    assert!(near(c.u, 0.5));
    assert!(c.v.abs() < 1e-6);
    assert_eq!(c.t, 0.0);
    let top = mask_coordinates(&m, v(0.0, 8.0, -8.0)).unwrap(); // tan = 1 toward +y
    assert!(near(top.v, 1.0));
    assert!(mask_coordinates(&m, v(0.0, 0.0, 3.0)).is_none()); // behind
    assert!(mask_coordinates(&m, v(9.0, 0.0, -8.0)).is_none()); // outside
    // Auto: a spot's outer angle, a point light's 45 degrees.
    d.mask_angle = None;
    assert_eq!(mask_angle_of(&d), 40.0);
    d.kind = LightType::Point;
    assert_eq!(mask_angle_of(&d), 45.0);
}

#[test]
fn window_placement_the_rays_crossing_gives_u_v_and_t_both_ways_round() {
    // Window in the plane z = 5 above the origin, 2 x 1 (half extents 1 and 0.5), light at
    // height 10 above it.
    let mut d = spot(0.0, 0.0, 10.0);
    d.mask_mode = MaskMode::Window;
    d.mask_center = v(0.0, 0.0, 5.0);
    d.mask_u = v(1.0, 0.0, 0.0);
    d.mask_v = v(0.0, 0.5, 0.0);
    let place = |d: &LightDesc, z: f32| make_mask_placement(&StoredLight::derive(d), z);
    let m = place(&d, 10.0).unwrap();
    assert!(m.window);
    // Ground point (1, 0.5, 0): the ray crosses z = 5 halfway, at (0.5, 0.25).
    let c = mask_coordinates(&m, v(1.0, 0.5, -10.0)).unwrap();
    assert!(near(c.u, 0.5));
    assert!(near(c.v, 0.5));
    assert!(near(c.t, 0.5));
    // A pixel on the light's side of the window gets nothing.
    assert!(mask_coordinates(&m, v(0.1, 0.0, -2.0)).is_none());

    // The other way round (light outside below, pixel above): the same window from the far side.
    let m = place(&d, 0.0).unwrap();
    let c = mask_coordinates(&m, v(2.0, 0.0, 10.0)).unwrap();
    assert!(near(c.u, 1.0));
    assert!(near(c.t, 0.5));

    // A wall window (vertical plane x = 3), light inside at x = 0: the cross lands sheared on
    // the ground outside.
    d.mask_center = v(3.0, 0.0, 2.0);
    d.mask_u = v(0.0, 1.0, 0.0);
    d.mask_v = v(0.0, 0.0, 1.0);
    let m = place(&d, 4.0).unwrap();
    let c = mask_coordinates(&m, v(6.0, 1.0, -4.0)).unwrap(); // reaches x = 3 at t = 0.5: (y 0.5, z 2)
    assert!(near(c.u, 0.5));
    assert!(c.v.abs() < 1e-6);
    assert!(near(c.t, 0.5));

    // A light in the window's plane lights nothing.
    d.position = At::new(Vec2::new(3.0, 0.0));
    assert!(place(&d, 4.0).is_none());
}

#[test]
fn mask_factor_samples_the_texels_where_the_ray_passes() {
    let mut d = spot(0.0, 0.0, 8.0);
    d.mask_angle = Some(45.0);
    let m = make_mask_placement(&StoredLight::derive(&d), 8.0).unwrap();
    let texels = cross();
    // A pane (u = v = 0.5 maps to a pane of the cross) and outside the square.
    let lit = mask_factor(&m, &texels, v(4.0, 4.0, -8.0));
    assert!(near(lit.x, 1.0));
    let dark = mask_factor(&m, &texels, v(20.0, 0.0, -8.0));
    assert_eq!(dark, Vec3::ZERO);
}

#[test]
fn gpu_mask_carries_the_mode_and_the_atlas_block() {
    let m = MaskPlacement {
        window: true,
        ..MaskPlacement::default()
    };
    let g = make_gpu_mask(&m, 3, 4);
    assert_eq!(g.n[3], 1.0);
    assert!(near(g.p[3], 0.75));
    assert!(near(g.q[3], 0.25));
    let none = make_gpu_mask(&MaskPlacement::default(), 0, 0);
    assert_eq!((none.n[3], none.p[3], none.q[3]), (0.0, 0.0, 0.0));
}

#[test]
fn light_store_mask_checks_name_the_field_and_the_way_out() {
    let animations = AnimationStore::new();
    let mut masks = MaskStore::new();
    let mask = masks.create(cross()).unwrap();
    let gone = masks.create(cross()).unwrap();
    masks.destroy(gone, &LightStore::new()).unwrap();
    let res = StoreResources::new(&animations, &masks);
    let mut store = LightStore::new();

    let mut d = spot(0.0, 0.0, 5.0);
    d.mask = Some(mask);
    let lit = store.create(&d, &res).unwrap();
    assert_eq!(store.mask_users(mask), 1);

    let err =
        |store: &mut LightStore, d: &LightDesc| store.create(d, &res).unwrap_err().to_string();

    let mut dir = LightDesc::new(LightType::Directional);
    dir.direction = v(0.0, 0.0, -1.0);
    dir.mask = Some(mask);
    assert!(err(&mut store, &dir).contains("directional light cannot have a mask"));

    let mut line = LightDesc::new(LightType::Line);
    line.axis = v(1.0, 0.0, 0.0);
    line.mask = Some(mask);
    assert!(err(&mut store, &line).contains("line or rect light cannot have a mask"));

    let mut bad = d.clone();
    bad.mask_angle = Some(90.0);
    assert!(err(&mut store, &bad).contains("mask_angle"));
    bad = d.clone();
    bad.mask_mode = MaskMode::Window;
    bad.mask_u = Vec3::ZERO;
    assert!(err(&mut store, &bad).contains("mask_u"));
    bad = d.clone();
    bad.mask_mode = MaskMode::Window;
    bad.mask_v = bad.mask_u * 2.0;
    assert!(err(&mut store, &bad).contains("parallel"));
    bad = d.clone();
    bad.mask_center.y = f32::NAN;
    assert!(err(&mut store, &bad).contains("mask_center"));

    // A destroyed handle.
    bad = d.clone();
    bad.mask = Some(gone);
    assert!(err(&mut store, &bad).contains("invalid or destroyed mask handle"));
    assert_eq!(store.len(), 1, "nothing was added on error");

    // The description round-trips the mask fields.
    let out = store.get(lit).unwrap();
    assert_eq!(out.mask, Some(mask));
    assert_eq!(out.mask_mode, MaskMode::Projector);
    assert_eq!(out.mask_angle, None);
    assert_eq!(out.mask_v.z, 0.5);
}

#[test]
fn snapshot_round_trips_handles_blocks_and_texels() {
    let lights = LightStore::new();
    let mut store = MaskStore::new();
    let a = store.create(cross()).unwrap();
    let b = store
        .create(shape_texels(&MaskShape::preset(MaskPreset::Blinds)))
        .unwrap();
    store.destroy(a, &lights).unwrap();
    let c = store
        .create(shape_texels(&MaskShape::preset(MaskPreset::WindowHall)))
        .unwrap(); // block 0 again
    let mut buf = SnapshotBuf::new();
    buf.save(&store);

    let mut other = MaskStore::new();
    other.create(cross()).unwrap();
    let before = other.version();
    restore(&mut Reader::new(buf.bytes()), &mut other).unwrap();
    assert_ne!(other.version(), before, "the atlas needs a re-upload");
    assert_eq!(other.len(), 2);
    assert_eq!(other.block_of(c), Some(0));
    assert_eq!(other.block_of(b), Some(1));
    assert_eq!(other.find(b), store.find(b));
    assert_eq!(other.find(c), store.find(c));
    assert!(other.find(a).is_none());
    // Blocks are free or used as before.
    let d = other.create(cross()).unwrap();
    assert_eq!(other.block_of(d), Some(2));
    let mut again = SnapshotBuf::new();
    again.save(&store);
    assert_eq!(again.bytes(), buf.bytes());
}

#[test]
fn restore_rejects_a_block_that_is_taken_twice() {
    let mut store = MaskStore::new();
    store.create(cross()).unwrap();
    store.create(cross()).unwrap();
    let mut buf = SnapshotBuf::new();
    buf.save(&store);
    // Layout of the payload: slot count (u32), then per slot a tag (u8), the generation (u32)
    // and the block (u8). Set the block of the second slot to 0.
    let mut bytes = buf.bytes().to_vec();
    let payload = buf.sections()[0].payload.start;
    let second_block = payload + 4 + 6 + 5;
    assert_eq!(bytes[second_block], 1);
    bytes[second_block] = 0;
    let mut target = MaskStore::new();
    let err = restore(&mut Reader::new(&bytes), &mut target).unwrap_err();
    assert!(matches!(err, SnapshotError::Invalid(_)), "{err:?}");
}
