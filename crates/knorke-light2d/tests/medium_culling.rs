//! Origin: chiaro @ 9bc5b9b, tests/unit/MediumCullingTests.cpp (zlib license, see
//! `src/medium_culling.rs`), plus tests for what that file's header promises and its suite does
//! not check: animation and flicker, shaft entries, masked lights and the directional lights.
//! Lights are built with the public defaults (shadows on), so the ported tests that need the C++
//! internal defaults switch shadows off.

// Tests compare against platform `std` maths as an oracle; only library code must be platform
// independent (ADR 0011).
#![allow(clippy::disallowed_methods)]

use knorke_light2d::light::{Height, LightDesc, LightId, LightType, MaskMode, ShadowFilter};
use knorke_light2d::light_culling::{Casters, LightList, LightModes, collect_lights};
use knorke_light2d::mask::{MaskPreset, MaskShape};
use knorke_light2d::medium_culling::{
    MediumLightList, MediumSun, Shafts, collect_medium_lights, collect_medium_suns,
};
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{
    AnimationStore, LightStore, MaskStore, NoResources, Projection, StoreResources,
};
use knorke_math::{At, Vec2, Vec3};

fn at(x: f32, y: f32) -> At<knorke_math::World2, Vec2> {
    At::new(Vec2::new(x, y))
}

fn point(x: f32, y: f32, z: f32, radius: f32, haze: f32) -> LightDesc {
    let mut d = LightDesc::new(LightType::Point);
    d.position = at(x, y);
    d.height = Height::At(z);
    d.radius = radius;
    d.color = Vec3::ONE;
    d.intensity = 1.0;
    d.cast_shadows = false;
    d.haze = haze;
    d
}

fn add_point(store: &mut LightStore, x: f32, y: f32, z: f32, radius: f32, haze: f32) -> LightId {
    store
        .create(&point(x, y, z, radius, haze), &NoResources)
        .unwrap()
}

fn collect(
    store: &LightStore,
    projection: &Projection,
    medium_top: f32,
    modes: &LightModes<'_>,
) -> MediumLightList {
    let mut out = MediumLightList::default();
    collect_medium_lights(store, projection, 128, 64, medium_top, 0.0, modes, &mut out);
    out
}

#[test]
fn a_light_off_screen_whose_sphere_reaches_into_the_iso_view_is_collected() {
    let mut store = LightStore::new();
    let iso = Projection::iso(at(0.0, 0.0), 1.0, 1.0, 128, 64);
    // Ground point at scene (64, 102), below the 64 px high scene; its ground disc (radius
    // sqrt(30^2 - 20^2)) ends at scene y 79.6, but the sphere up to height 50 is raised by up to 50
    // px and reaches scene y 22.
    add_point(&mut store, 70.0, 70.0, 20.0, 30.0, 1.0);
    let mut ground = LightList::default();
    collect_lights(
        &store,
        &LightSettings::default(),
        &iso,
        0.0,
        128,
        64,
        &LightModes::default(),
        &mut ground,
    );
    assert!(ground.ordered.is_empty());

    let mut out = MediumLightList {
        skipped_haze: 7,
        ..Default::default()
    };
    collect_medium_lights(
        &store,
        &iso,
        128,
        64,
        100.0,
        0.0,
        &LightModes::default(),
        &mut out,
    );
    assert_eq!(out.lights.len(), 1);
    assert_eq!(out.skipped_haze, 0);
    // Reach heights [max(20 - 30, 0), min(20 + 30, 100)], horizontal half extent the full radius
    // (the light's height lies inside).
    assert_eq!(out.lights[0].area_mask[0], 0.0);
    assert_eq!(out.lights[0].area_mask[1], 50.0);
    assert_eq!(out.lights[0].spot_cos[2], 30.0);

    // With the media only up to height 5 the sphere stays below the scene.
    collect_medium_lights(
        &store,
        &iso,
        128,
        64,
        5.0,
        0.0,
        &LightModes::default(),
        &mut out,
    );
    assert!(out.lights.is_empty());
}

#[test]
fn haze_0_is_skipped_the_haze_factor_is_in_the_colour() {
    let mut store = LightStore::new();
    add_point(&mut store, 64.0, 32.0, 4.0, 20.0, 0.0);
    add_point(&mut store, 32.0, 32.0, 4.0, 20.0, 2.5);
    let out = collect(&store, &Projection::IDENTITY, 10.0, &LightModes::default());
    assert_eq!(out.skipped_haze, 1);
    assert_eq!(out.lights.len(), 1);
    let g = &out.lights[0];
    assert_eq!(g.position_radius[0], 32.0);
    assert!((g.color_falloff[0] - 2.5).abs() < 1e-6);
    assert!((g.color_falloff[1] - 2.5).abs() < 1e-6);
    assert!((g.color_falloff[2] - 2.5).abs() < 1e-6);
    assert_eq!(g.area_mask[0], 0.0);
    assert_eq!(g.area_mask[1], 10.0);
    assert_eq!(g.shadow[1], 0.0); // no highlights in the medium
}

#[test]
fn a_light_more_than_its_radius_above_the_media_is_not_collected() {
    let mut store = LightStore::new();
    add_point(&mut store, 64.0, 32.0, 140.0, 30.0, 1.0); // reach from 110: above medium_top 100
    add_point(&mut store, 64.0, 32.0, 120.0, 30.0, 1.0); // reach from 90
    let out = collect(&store, &Projection::IDENTITY, 100.0, &LightModes::default());
    assert_eq!(out.lights.len(), 1);
    let g = &out.lights[0];
    assert_eq!(g.position_radius[2], 120.0);
    assert_eq!(g.area_mask[0], 90.0);
    assert_eq!(g.area_mask[1], 100.0);
    // The nearest reachable height 100 is 20 below the light: horizontal half extent
    // sqrt(30^2 - 20^2).
    assert!((g.spot_cos[2] - 500.0f32.sqrt()).abs() < 1e-4);
}

#[test]
fn disabled_and_directional_lights_are_not_collected() {
    let mut store = LightStore::new();
    let mut sun = LightDesc::new(LightType::Directional);
    sun.intensity = 1.0;
    store.create(&sun, &NoResources).unwrap();
    let mut off = point(64.0, 32.0, 4.0, 20.0, 1.0);
    off.enabled = false;
    store.create(&off, &NoResources).unwrap();
    let out = collect(&store, &Projection::IDENTITY, 100.0, &LightModes::default());
    assert!(out.lights.is_empty());
    assert_eq!(out.skipped_haze, 0);
}

#[test]
fn shaft_entries_follow_the_lights_and_the_list_keeps_its_capacity() {
    let mut store = LightStore::new();
    store
        .create(&point(20.0, 20.0, 4.0, 20.0, 1.0), &NoResources)
        .unwrap(); // no shadows: no shafts
    let mut shadowed = point(40.0, 20.0, 4.0, 20.0, 1.0);
    shadowed.cast_shadows = true;
    shadowed.shadow_filter = ShadowFilter::StaticOnly;
    shadowed.inside_obstacle = true;
    store.create(&shadowed, &NoResources).unwrap();
    let mut line = LightDesc::new(LightType::Line);
    line.position = at(60.0, 20.0);
    line.height = Height::At(4.0);
    line.axis = Vec3::new(5.0, 0.0, 0.0);
    line.radius = 20.0;
    line.cast_shadows = true;
    store.create(&line, &NoResources).unwrap(); // line lights get no shafts (E33)

    let mut out = MediumLightList::default();
    let modes = LightModes::default();
    collect_medium_lights(
        &store,
        &Projection::IDENTITY,
        128,
        64,
        10.0,
        0.0,
        &modes,
        &mut out,
    );
    assert_eq!(out.lights.len(), 3);
    assert_eq!(
        out.shafts,
        [
            None,
            Some(Shafts {
                casters: Casters::Static,
                inside: true,
                key: 4, // the second light's row key
            }),
            None
        ]
    );
    // The line's reach is widened by the emitter's half box.
    assert_eq!(out.lights[2].area[0], 5.0);
    assert!(out.lights[2].spot_cos[2] >= 20.0 + 5.0 - 1e-4);

    let cap = out.lights.capacity();
    collect_medium_lights(
        &store,
        &Projection::IDENTITY,
        128,
        64,
        10.0,
        0.0,
        &modes,
        &mut out,
    );
    assert_eq!(out.lights.capacity(), cap);
}

#[test]
fn masked_lights_go_to_their_own_list_and_windows_get_no_shafts() {
    let animations = AnimationStore::new();
    let mut masks = MaskStore::new();
    let mut texels = Vec::new();
    MaskShape::preset(MaskPreset::WindowCross).build(&mut texels);
    let mask = masks.create(texels).unwrap();
    let res = StoreResources::new(&animations, &masks);

    let mut store = LightStore::new();
    let mut projector = point(40.0, 32.0, 6.0, 30.0, 2.0);
    projector.kind = LightType::Spot;
    projector.direction = Vec3::new(0.0, 0.0, -1.0);
    projector.outer_angle = 40.0;
    projector.cast_shadows = true;
    projector.mask = Some(mask);
    store.create(&projector, &res).unwrap();
    let mut window = projector.clone();
    window.position = at(80.0, 32.0);
    window.mask_mode = MaskMode::Window;
    window.mask_center = Vec3::new(80.0, 40.0, 3.0);
    window.mask_u = Vec3::new(4.0, 0.0, 0.0);
    window.mask_v = Vec3::new(0.0, 0.0, 3.0);
    store.create(&window, &res).unwrap();
    // A window in the light's own plane lights nothing.
    let mut flat = window.clone();
    flat.mask_center = Vec3::new(80.0, 32.0, 6.0);
    flat.mask_u = Vec3::new(1.0, 0.0, 0.0);
    flat.mask_v = Vec3::new(0.0, 1.0, 0.0);
    store.create(&flat, &res).unwrap();
    store
        .create(&point(20.0, 32.0, 6.0, 30.0, 1.0), &res)
        .unwrap();

    let modes = LightModes {
        masks: Some(&masks),
        ..LightModes::default()
    };
    let out = collect(&store, &Projection::IDENTITY, 20.0, &modes);
    assert_eq!(out.lights.len(), 1);
    assert_eq!(out.masked.len(), 2);
    assert_eq!(out.masked.masks.len(), 2);
    assert_eq!(out.masked.lights[0].position_radius[0], 40.0);
    assert!((out.masked.lights[0].color_falloff[0] - 2.0).abs() < 1e-6); // haze factor
    assert_eq!(out.masked.masks[0].n[3], 0.0); // projector
    assert_eq!(out.masked.masks[1].n[3], 1.0); // window
    assert_eq!(
        out.masked_shafts,
        [
            Some(Shafts {
                casters: Casters::All,
                inside: false,
                key: 0,
            }),
            None
        ]
    );

    // Without the mask store masked lights are dropped, never drawn unmasked.
    let out = collect(&store, &Projection::IDENTITY, 20.0, &LightModes::default());
    assert_eq!(out.lights.len(), 1);
    assert!(out.masked.is_empty());
    assert!(out.masked_shafts.is_empty());
}

#[test]
fn flicker_and_animation_darkening_apply_as_for_the_ground() {
    let mut store = LightStore::new();
    let mut d = point(30.0, 30.0, 4.0, 20.0, 1.0);
    d.flicker = 0.5;
    d.flicker_speed = 8.0;
    store.create(&d, &NoResources).unwrap();
    let modes = LightModes::default();
    let mut ground = LightList::default();
    collect_lights(
        &store,
        &LightSettings::default(),
        &Projection::IDENTITY,
        0.37,
        128,
        64,
        &modes,
        &mut ground,
    );
    let mut out = MediumLightList::default();
    collect_medium_lights(
        &store,
        &Projection::IDENTITY,
        128,
        64,
        10.0,
        0.37,
        &modes,
        &mut out,
    );
    assert_eq!(
        out.lights[0].color_falloff[0],
        ground.ordered[0].color_falloff[0]
    );
    assert!(out.lights[0].color_falloff[0] < 1.0);
}

#[test]
fn suns_are_every_enabled_directional_light_with_haze() {
    let mut store = LightStore::new();
    let mut sun = LightDesc::new(LightType::Directional);
    sun.direction = Vec3::new(0.0, 0.0, -2.0);
    sun.color = Vec3::new(1.0, 0.5, 0.25);
    sun.intensity = 2.0;
    sun.haze = 3.0;
    sun.cast_shadows = true;
    store.create(&sun, &NoResources).unwrap();
    let mut hazeless = sun.clone();
    hazeless.haze = 0.0;
    store.create(&hazeless, &NoResources).unwrap();
    let mut off = sun.clone();
    off.enabled = false;
    store.create(&off, &NoResources).unwrap();
    let mut filtered = sun.clone();
    filtered.shadow_filter = ShadowFilter::DynamicOnly;
    store.create(&filtered, &NoResources).unwrap();
    store
        .create(&point(30.0, 30.0, 4.0, 20.0, 1.0), &NoResources)
        .unwrap();

    let mut out: Vec<MediumSun> = Vec::new();
    collect_medium_suns(&store, 0.0, &LightModes::default(), &mut out);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].color, Vec3::new(6.0, 3.0, 1.5)); // colour * intensity * haze
    assert_eq!(out[0].to_light, Vec3::new(0.0, 0.0, 1.0)); // unit vector toward the light
    assert!(!out[0].shadows); // shadows need the height mode
    assert_eq!(out[0].casters, Casters::All);

    let modes = LightModes {
        height_mode: true,
        ..LightModes::default()
    };
    collect_medium_suns(&store, 0.0, &modes, &mut out);
    assert!(out[0].shadows);
    assert_eq!(out[0].casters, Casters::All);
    assert_eq!(out[1].casters, Casters::Dynamic);
}
