//! Origin: chiaro @ 9bc5b9b, tests/unit/LightCullingTests.cpp, the `CollectLights` tests of
//! tests/unit/AnimationTests.cpp and the culling test of tests/unit/MaskTests.cpp (zlib license,
//! see `src/light_culling.rs`), plus tests for what the redesign adds: lights are built with the
//! public defaults (shadows on, height automatic), so tests that need the C++ internal defaults
//! (no shadows) set them.
//!
//! The beacon test of the C++ suite turns a profile's `C0` as well; profiles are P9, so a fake
//! [`ProfileAtlas`] stands in.

// Tests compare against platform `std` maths as an oracle; only library code must be platform
// independent (ADR 0011).
#![allow(clippy::disallowed_methods)]

use knorke_light2d::animation::{AnimationDesc, AnimationId, AnimationType};
use knorke_light2d::gpu_light::GpuLight;
use knorke_light2d::light::{Height, LightDesc, LightResources, LightType, MaskMode, ProfileId};
use knorke_light2d::light_culling::*;
use knorke_light2d::light_math::NEAR_FIELD_SHARE;
use knorke_light2d::mask::{MaskPreset, MaskShape};
use knorke_light2d::mask_store::MaskId;
use knorke_light2d::{
    AnimationStore, LightStore, MaskStore, NoResources, Projection, StoreResources,
};
use knorke_math::{At, Vec2, Vec3};
use knorke_types::Handle;

const W: u32 = 128;
const H: u32 = 64;

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-5 * b.abs().max(1.0)
}

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

fn at(x: f32, y: f32) -> At<knorke_math::World2, Vec2> {
    At::new(Vec2::new(x, y))
}

fn add_point(
    store: &mut LightStore,
    x: f32,
    y: f32,
    cast_shadows: bool,
    source_radius: f32,
) -> knorke_light2d::light::LightId {
    let mut d = LightDesc::new(LightType::Point);
    d.position = at(x, y);
    d.radius = 50.0;
    d.cast_shadows = cast_shadows;
    d.source_radius = source_radius;
    store.create(&d, &NoResources).unwrap()
}

fn collect(
    store: &LightStore,
    settings: &CullSettings,
    projection: &Projection,
    time: f64,
    modes: &LightModes<'_>,
) -> LightList {
    let mut list = LightList::default();
    collect_lights(store, settings, projection, time, W, H, modes, &mut list);
    list
}

fn plain(store: &LightStore) -> LightList {
    collect(
        store,
        &CullSettings::default(),
        &Projection::IDENTITY,
        0.0,
        &LightModes::default(),
    )
}

#[test]
fn inside_obstacle_turns_the_source_radius_in_shadow_x_into_minus_one_minus_r() {
    let mut store = LightStore::new();
    let mut d = LightDesc::new(LightType::Point);
    d.position = at(16.0, 32.0);
    d.radius = 50.0;
    d.cast_shadows = true;
    d.inside_obstacle = true;
    d.source_radius = 6.0;
    store.create(&d, &NoResources).unwrap();
    d.source_radius = 0.0;
    store.create(&d, &NoResources).unwrap();
    add_point(&mut store, 64.0, 32.0, true, 6.0); // unmarked: unchanged

    let list = plain(&store);
    assert_eq!(list.shadowed.len(), 3);
    assert!(near(list.shadowed[0].shadow[0], -7.0));
    assert_eq!(list.shadowed[1].shadow[0], -1.0);
    assert!(near(list.shadowed[2].shadow[0], 6.0));
    assert!(near(list.shadowed[0].shadow[2], 48.0)); // the rest as before
}

#[test]
fn shadowed_lights_carry_their_mask_space_shadow_parameters() {
    let mut store = LightStore::new();
    add_point(&mut store, 16.0, 32.0, true, 6.0);
    add_point(&mut store, 64.0, 32.0, false, 3.0);
    add_point(&mut store, 100.0, 10.0, true, 0.0);

    let list = plain(&store);
    assert_eq!(list.shadowed.len(), 2);
    assert_eq!(list.unshadowed.len(), 1);
    // Margin 32 at scale 1: mask = scene + 32. x = R, y = specular factor (E47), zw = ground
    // position in mask texels.
    let a = &list.shadowed[0];
    assert!(near(a.shadow[0], 6.0));
    assert_eq!(a.shadow[1], 1.0);
    assert!(near(a.shadow[2], 48.0));
    assert!(near(a.shadow[3], 64.0));
    let b = &list.shadowed[1];
    assert_eq!(b.shadow[0], 0.0);
    assert_eq!(b.shadow[1], 1.0);
    assert!(near(b.shadow[2], 132.0));
    assert!(near(b.shadow[3], 42.0));
    // Every light carries its specular factor, only shadow casters the rest.
    assert_eq!(list.unshadowed[0].shadow, [0.0, 1.0, 0.0, 0.0]);
}

#[test]
fn directional_lights_go_into_the_ambient_sum_unless_they_cast_shadows_in_height_mode() {
    let mut store = LightStore::new();
    let mut d = LightDesc::new(LightType::Directional);
    d.color = v(0.5, 1.0, 1.0);
    d.direction = v(1.0, 0.0, -1.0); // elevation 45 degrees, light from -x
    d.cast_shadows = true;
    store.create(&d, &NoResources).unwrap();
    let (s, p) = (CullSettings::default(), Projection::IDENTITY);

    // No heights: phase 3 behaviour.
    let list = collect(&store, &s, &p, 0.0, &LightModes::default());
    assert!(near(list.ambient.x, 1.5));
    assert!(list.directional.is_empty());

    let height = LightModes {
        height_mode: true,
        ..LightModes::default()
    };
    let list = collect(&store, &s, &p, 0.0, &height);
    assert!(near(list.ambient.x, 1.0));
    assert_eq!(list.directional.len(), 1);
    let g = &list.directional[0];
    assert!(near(g.dir_slope[0], -1.0)); // toward the light in the mask
    assert!(g.dir_slope[1].abs() < 1e-6);
    assert!(near(g.dir_slope[2], 1.0)); // tan(45 deg) world units of height per mask texel
    assert_eq!(g.dir_slope[3], 0.0); // no angular size
    assert!(near(g.color[0], 0.5));
    assert_eq!(g.color[3], 1.0); // shadows

    d.cast_shadows = false;
    let mut unshadowed = LightStore::new();
    unshadowed.create(&d, &NoResources).unwrap();
    let list = collect(&unshadowed, &s, &p, 0.0, &height);
    assert!(list.directional.is_empty());
    assert!(near(list.ambient.x, 1.5));
}

#[test]
fn shadow_parameters_follow_zoom_camera_and_shadow_scale() {
    let mut store = LightStore::new();
    let settings = CullSettings {
        shadow_resolution: 0.5,
        ..CullSettings::default()
    };
    // Two scene pixels per world unit.
    let projection = Projection::ortho(at(10.0, 20.0), 2.0, W, H);
    add_point(&mut store, 12.0, 21.0, true, 6.0);

    let list = collect(&store, &settings, &projection, 0.0, &LightModes::default());
    assert_eq!(list.shadowed.len(), 1);
    let g = &list.shadowed[0];
    // R = source_radius * axis scale * s = 6 * 2 * 0.5.
    assert!(near(g.shadow[0], 6.0));
    // Scene position: camera centre (64, 32) + 2 * (2, 1) = (68, 34); mask = (scene + 32) * 0.5.
    assert!(near(g.shadow[2], 50.0));
    assert!(near(g.shadow[3], 33.0));
}

#[test]
fn normal_mode_draws_every_directional_light_per_pixel() {
    let mut store = LightStore::new();
    let mut d = LightDesc::new(LightType::Directional);
    d.direction = v(1.0, 0.0, -1.0);
    d.cast_shadows = false;
    store.create(&d, &NoResources).unwrap();
    d.cast_shadows = true;
    store.create(&d, &NoResources).unwrap();
    let (s, p) = (CullSettings::default(), Projection::IDENTITY);

    let normal = LightModes {
        normal_mode: true,
        ..LightModes::default()
    };
    let list = collect(&store, &s, &p, 0.0, &normal);
    assert_eq!(list.directional.len(), 2);
    assert_eq!(list.directional_shadows, 0); // no height mode: nothing can shadow them
    assert!(near(list.ambient.x, 1.0)); // settings ambient only
    let list = collect(&store, &s, &p, 0.0, &LightModes::default());
    assert!(list.directional.is_empty());
}

#[test]
fn line_and_rect_lights_type_axis_quad_extent_and_mask_space_axes() {
    let mut store = LightStore::new();
    let mut d = LightDesc::new(LightType::Line);
    d.set_line_endpoints(v(10.0, 20.0, 0.0), v(50.0, 30.0, 0.0));
    d.radius = 8.0;
    d.cast_shadows = true;
    store.create(&d, &NoResources).unwrap();
    let mut r = LightDesc::new(LightType::Rect);
    r.position = at(80.0, 32.0);
    r.height = Height::At(5.0);
    r.direction = v(0.0, 1.0, 0.0); // faces +y
    r.axis = v(6.0, 0.0, 0.0);
    r.half_width = 2.0; // across: z (n x u = y x x = -z, so v = (0, 0, -2))
    r.radius = 13.0;
    r.two_sided = true;
    r.cast_shadows = false;
    store.create(&r, &NoResources).unwrap();

    let list = plain(&store);
    assert_eq!(list.ordered.len(), 2);
    // A shadowed line or rect goes to its own list.
    assert!(list.shadowed.is_empty());
    assert_eq!(list.shadowed_area.len(), 1);
    let line = &list.shadowed_area[0];
    assert_eq!(line.direction_type[3], 2.0);
    assert!(near(line.position_radius[0], 30.0));
    assert!(near(line.area[0], 20.0));
    assert!(near(line.area[1], 5.0));
    assert_eq!(line.area[3], 0.0);
    assert!(near(line.spot_cos[2], 28.0)); // ground radius 8 + larger half extent 20
    assert_eq!(line.profile[0], -1.0);
    // Identity projection, scale 1: mask texels = scene pixels.
    assert!(near(line.area_mask[0], 20.0));
    assert!(near(line.area_mask[1], 5.0));
    assert_eq!((line.area_mask[2], line.area_mask[3]), (0.0, 0.0));
    let rect = &list.unshadowed[0];
    assert_eq!(rect.direction_type[3], 4.0);
    assert_eq!(rect.area[3], 2.0);
    // Lowest point at z = 3: ground radius sqrt(13^2 - 3^2), plus the half extent 6.
    assert!(near(rect.spot_cos[2], 160.0f32.sqrt() + 6.0));
}

#[test]
fn a_line_light_reaching_the_ground_only_near_one_low_end_is_still_drawn() {
    let mut store = LightStore::new();
    let mut d = LightDesc::new(LightType::Line);
    // Centre at height 21, radius 10.
    d.set_line_endpoints(v(40.0, 30.0, 2.0), v(60.0, 30.0, 40.0));
    d.radius = 10.0;
    store.create(&d, &NoResources).unwrap();
    assert_eq!(plain(&store).ordered.len(), 1);
}

#[test]
fn soft_near_field_rides_on_the_type_as_a_share_of_the_source_radius() {
    let mut store = LightStore::new();
    let mut d = LightDesc::new(LightType::Spot);
    d.position = at(20.0, 0.0);
    d.radius = 50.0;
    d.source_radius = 5.0;
    store.create(&d, &NoResources).unwrap(); // off: the plain type
    d.soft_near_field = true;
    store.create(&d, &NoResources).unwrap();
    d.source_radius = 0.0; // the flag without a source radius is an error (E70)
    assert!(store.create(&d, &NoResources).is_err());
    d.source_radius = 80.0; // clamped to the reach
    d.kind = LightType::Point;
    store.create(&d, &NoResources).unwrap();

    let list = plain(&store);
    assert_eq!(list.ordered.len(), 3);
    assert_eq!(list.ordered[0].direction_type[3], 1.0);
    assert!(near(
        list.ordered[1].direction_type[3],
        1.0 + NEAR_FIELD_SHARE * 0.1
    ));
    assert!(near(list.ordered[2].direction_type[3], NEAR_FIELD_SHARE));
    assert!(list.ordered[2].direction_type[3] < 0.5); // still a point light for the shaders
}

// ---- animation ----

fn point(x: f32, y: f32) -> LightDesc {
    let mut d = LightDesc::new(LightType::Point);
    d.position = at(x, y);
    d.radius = 30.0;
    d.color = v(1.0, 0.5, 0.25);
    d.intensity = 2.0;
    d
}

#[test]
fn lights_without_animation_are_bit_identical_animated_ones_get_factors() {
    let masks = MaskStore::new();
    let mut animations = AnimationStore::new();
    let mut pulse = AnimationDesc::new(AnimationType::Pulse);
    pulse.period = 2.0;
    pulse.minimum = 0.0;
    pulse.low_color.z = 0.0;
    let a = animations.create(&pulse).unwrap();
    let res = StoreResources::new(&animations, &masks);
    let mut lights = LightStore::new();
    lights.create(&point(20.0, 20.0), &res).unwrap();
    let mut animated = point(80.0, 20.0);
    animated.animation = Some(a);
    animated.animation_offset = 0.5; // at time 0 the phase is 0.25: w = 0.5
    let al = lights.create(&animated, &res).unwrap();
    let modes = LightModes {
        animations: Some(&animations),
        ..LightModes::default()
    };
    let (s, p) = (CullSettings::default(), Projection::IDENTITY);

    let list = collect(&lights, &s, &p, 0.0, &modes);
    assert_eq!(list.ordered.len(), 2);
    // The plain light equals a store without any animation.
    let mut plain_store = LightStore::new();
    plain_store.create(&point(20.0, 20.0), &res).unwrap();
    let only = collect(&plain_store, &s, &p, 0.0, &LightModes::default());
    assert_eq!(only.ordered.len(), 1);
    assert_eq!(only.ordered[0], list.ordered[0]);

    let g = &list.ordered[1];
    assert!(near(g.color_falloff[0], 1.0 * 2.0 * 0.5));
    assert!(near(g.color_falloff[1], 0.5 * 2.0 * 0.5));
    assert!(near(g.color_falloff[2], 0.25 * 2.0 * 0.5 * 0.5)); // low_color blue 0 at w = 0.5
    let sample = animations.sample(Some(a), 0.0, 0.5, al.index());
    assert!(near(sample.intensity, 0.5));
    // At the trough the animated light is black and not drawn at all.
    let trough = collect(&lights, &s, &p, -0.5, &modes);
    assert_eq!(trough.ordered.len(), 1);
    assert_eq!(only.ordered[0], trough.ordered[0]);
}

/// Stands in for the profile store of P9: every profile is row 0 of a one-block atlas.
struct FakeProfiles;

impl ProfileAtlas for FakeProfiles {
    fn atlas_row(&self, _: ProfileId) -> Option<f32> {
        Some(0.0)
    }
    fn inv_atlas_height(&self) -> f32 {
        1.0 / 72.0
    }
}

struct AnyProfile<'a>(StoreResources<'a>);

impl LightResources for AnyProfile<'_> {
    fn has_profile(&self, _: ProfileId) -> bool {
        true
    }
    fn has_animation(&self, id: AnimationId) -> bool {
        self.0.has_animation(id)
    }
    fn has_mask(&self, id: MaskId) -> bool {
        self.0.has_mask(id)
    }
}

#[test]
fn a_beacon_turns_the_spot_direction_and_the_profile_c0_about_z() {
    let masks = MaskStore::new();
    let mut animations = AnimationStore::new();
    let mut beacon = AnimationDesc::new(AnimationType::Beacon);
    beacon.period = 4.0;
    let a = animations.create(&beacon).unwrap();
    let res = AnyProfile(StoreResources::new(&animations, &masks));
    let mut lights = LightStore::new();
    let mut spot = LightDesc::new(LightType::Spot);
    spot.position = at(40.0, 30.0);
    spot.height = Height::At(3.0);
    spot.direction = v(1.0, 0.0, -0.5);
    spot.animation = Some(a);
    lights.create(&spot, &res).unwrap();
    let mut profiled = point(80.0, 30.0);
    profiled.direction = v(0.0, 0.0, -1.0);
    profiled.profile = Some(Handle::new(0, std::num::NonZeroU32::MIN));
    profiled.animation = Some(a);
    lights.create(&profiled, &res).unwrap();

    let modes = LightModes {
        animations: Some(&animations),
        profiles: Some(&FakeProfiles),
        ..LightModes::default()
    };
    let list = collect(
        &lights,
        &CullSettings::default(),
        &Projection::IDENTITY,
        1.0,
        &modes,
    ); // a quarter turn
    assert_eq!(list.ordered.len(), 2);
    let len = 1.25f32.sqrt();
    let s = &list.ordered[0];
    assert!(s.direction_type[0].abs() < 1e-6);
    assert!(near(s.direction_type[1], 1.0 / len));
    assert!(near(s.direction_type[2], -0.5 / len));
    assert!(near(s.color_falloff[0], 1.0)); // a beacon keeps the intensity
    let q = &list.ordered[1];
    // C0 was +x; after a quarter turn it is +y, as with profile_rotation = 90.
    assert!(q.profile[1].abs() < 1e-6);
    assert!(near(q.profile[2], 1.0));
    assert!(q.profile[3].abs() < 1e-6);
    assert!(near(q.direction_type[2], -1.0));
    assert_eq!(list.profiled_lights, 1);
    assert!(near(q.spot_cos[3], 1.0 / 72.0));
    assert_eq!(q.profile[0], 0.0);
}

#[test]
fn animated_directional_lights_in_the_ambient_sum_are_tinted_and_scaled() {
    let masks = MaskStore::new();
    let mut animations = AnimationStore::new();
    let mut env = AnimationDesc::new(AnimationType::Envelope);
    env.start = 1.0;
    let a = animations.create(&env).unwrap();
    let res = StoreResources::new(&animations, &masks);
    let mut lights = LightStore::new();
    let mut sun = LightDesc::new(LightType::Directional);
    sun.intensity = 1.0;
    sun.animation = Some(a);
    lights.create(&sun, &res).unwrap();
    let modes = LightModes {
        animations: Some(&animations),
        ..LightModes::default()
    };
    let (s, p) = (CullSettings::default(), Projection::IDENTITY);
    // Before the trigger: no light from the sun.
    let base = collect(&lights, &s, &p, 0.0, &modes).ambient;
    let at_peak = collect(
        &lights,
        &s,
        &p,
        f64::from(1.0 + env.attack + 0.5 * env.hold),
        &modes,
    );
    assert!(near(at_peak.ambient.x, base.x + 1.0));
    assert!(near(at_peak.ambient.y, base.y + 1.0));
}

#[test]
fn a_beacon_turns_a_rect_lights_normal_and_length_axis() {
    let masks = MaskStore::new();
    let mut animations = AnimationStore::new();
    let mut beacon = AnimationDesc::new(AnimationType::Beacon);
    beacon.period = 4.0;
    let a = animations.create(&beacon).unwrap();
    let res = StoreResources::new(&animations, &masks);
    let mut lights = LightStore::new();
    let mut rect = LightDesc::new(LightType::Rect);
    rect.position = at(40.0, 30.0);
    rect.height = Height::At(4.0);
    rect.direction = v(1.0, 0.0, 0.0); // faces +x
    rect.axis = v(0.0, 3.0, 0.0);
    rect.animation = Some(a);
    lights.create(&rect, &res).unwrap();
    let modes = LightModes {
        animations: Some(&animations),
        ..LightModes::default()
    };
    let list = collect(
        &lights,
        &CullSettings::default(),
        &Projection::IDENTITY,
        1.0,
        &modes,
    );
    assert_eq!(list.ordered.len(), 1);
    let g = &list.ordered[0];
    assert!(g.direction_type[0].abs() < 1e-6);
    assert!(near(g.direction_type[1], 1.0));
    assert!(near(g.area[0], -3.0));
    assert!(g.area[1].abs() < 1e-6);
}

// ---- masks ----

fn cross_mask(masks: &mut MaskStore) -> MaskId {
    let shape = MaskShape::preset(MaskPreset::WindowCross);
    let mut texels = Vec::new();
    shape.build(&mut texels);
    masks.create(texels).unwrap()
}

fn spot(x: f32, y: f32, z: f32) -> LightDesc {
    let mut d = LightDesc::new(LightType::Spot);
    d.position = at(x, y);
    d.height = Height::At(z);
    d.direction = v(0.0, 0.0, -1.0);
    d.outer_angle = 40.0;
    d.radius = 100.0;
    d
}

#[test]
fn masked_lights_leave_the_plain_lists_for_the_masked_ones() {
    let animations = AnimationStore::new();
    let mut masks = MaskStore::new();
    let mask = cross_mask(&mut masks);
    let res = StoreResources::new(&animations, &masks);
    let mut store = LightStore::new();
    let plain_light = spot(30.0, 30.0, 5.0);
    store.create(&plain_light, &res).unwrap();
    let mut masked = plain_light.clone();
    masked.position = at(60.0, 30.0);
    masked.mask = Some(mask);
    store.create(&masked, &res).unwrap();
    // A window in the light's own plane: lights nothing, not drawn.
    let mut flat = masked.clone();
    flat.mask_mode = MaskMode::Window;
    flat.mask_center = v(60.0, 30.0, 5.0);
    flat.mask_u = v(1.0, 0.0, 0.0);
    flat.mask_v = v(0.0, 1.0, 0.0);
    store.create(&flat, &res).unwrap();

    let modes = LightModes {
        masks: Some(&masks),
        ..LightModes::default()
    };
    let (s, p) = (CullSettings::default(), Projection::IDENTITY);
    let list = collect(&store, &s, &p, 0.0, &modes);
    assert_eq!(list.ordered.len(), 1);
    assert_eq!(list.masked_ordered.len(), 1);
    assert_eq!(list.masked_lights, 1);
    assert_eq!(list.mask_blocks, 1);
    assert_eq!(list.masked_shadowed.len() + list.masked_unshadowed.len(), 1);
    assert_eq!(list.masked_ordered.masks.len(), 1);
    assert!(near(list.masked_ordered.masks[0].n[2], -1.0)); // projector: the direction
    assert_eq!(list.masked_ordered.lights[0].position_radius[0], 60.0);

    // Culling counters treat a masked light like an unmasked one: one too high to reach the
    // ground is unreachable even when its window would also leave it dark (placement runs only
    // for lights that reach the scene).
    let mut high = flat.clone();
    high.height = Height::At(200.0);
    high.mask_center = v(60.0, 30.0, 200.0);
    store.create(&high, &res).unwrap();
    let mut high_plain = plain_light.clone();
    high_plain.height = Height::At(200.0);
    store.create(&high_plain, &res).unwrap();
    let list = collect(&store, &s, &p, 0.0, &modes);
    assert_eq!(list.unreachable_lights, 2);
    assert_eq!(list.masked_lights, 1);

    // Without the mask store masked lights are dropped, never drawn unmasked.
    let list = collect(&store, &s, &p, 0.0, &LightModes::default());
    assert_eq!(list.ordered.len(), 1);
    assert_eq!(list.masked_lights, 0);
}

#[test]
fn caster_filters_split_the_shadowed_lights_and_set_the_class_bits() {
    use knorke_light2d::light::ShadowFilter;
    let mut store = LightStore::new();
    let mut d = LightDesc::new(LightType::Point);
    d.position = at(20.0, 20.0);
    d.radius = 30.0;
    d.shadow_filter = ShadowFilter::StaticOnly;
    store.create(&d, &NoResources).unwrap();
    d.shadow_filter = ShadowFilter::DynamicOnly;
    store.create(&d, &NoResources).unwrap();
    let mut line = LightDesc::new(LightType::Line);
    line.set_line_endpoints(v(40.0, 20.0, 1.0), v(60.0, 20.0, 1.0));
    line.radius = 10.0;
    line.shadow_filter = ShadowFilter::DynamicOnly;
    store.create(&line, &NoResources).unwrap();

    let list = plain(&store);
    assert_eq!(list.ordered.len(), 3);
    assert!(list.shadowed.is_empty() && list.shadowed_area.is_empty());
    assert_eq!(
        list.filtered[Casters::Static.filter_index().unwrap()][0].len(),
        1
    );
    assert_eq!(
        list.filtered[Casters::Dynamic.filter_index().unwrap()][0].len(),
        1
    );
    assert_eq!(
        list.filtered[Casters::Dynamic.filter_index().unwrap()][1].len(),
        1
    );
    assert_eq!(list.caster_classes, 0b11);
}

#[test]
fn lights_outside_the_scene_are_culled_and_disabled_ones_are_skipped() {
    let mut store = LightStore::new();
    add_point(&mut store, 20.0, 20.0, false, 0.0);
    add_point(&mut store, 1000.0, 20.0, false, 0.0); // far outside
    let mut off = LightDesc::new(LightType::Point);
    off.position = at(30.0, 30.0);
    off.enabled = false;
    store.create(&off, &NoResources).unwrap();
    let mut high = LightDesc::new(LightType::Point);
    high.position = at(30.0, 30.0);
    high.height = Height::At(500.0);
    store.create(&high, &NoResources).unwrap();
    let list = plain(&store);
    assert_eq!(list.ordered.len(), 1);
    assert_eq!(list.unreachable_lights, 1);
    assert_eq!(list.auto_height_lights, 1);
}

#[test]
fn a_one_sided_rect_facing_away_from_everything_is_counted_and_not_drawn() {
    let mut store = LightStore::new();
    // A panel on the ground facing down: everything it could reach lies behind it.
    let mut r = LightDesc::new(LightType::Rect);
    r.position = at(40.0, 30.0);
    r.height = Height::At(10.0);
    r.direction = v(0.0, 0.0, 1.0); // faces up, the whole reach above ground is below it
    r.axis = v(2.0, 0.0, 0.0);
    r.radius = 8.0;
    store.create(&r, &NoResources).unwrap();
    let list = plain(&store);
    assert_eq!(list.facing_away_lights, 1);
    assert!(list.ordered.is_empty());
}

#[test]
fn the_gpu_light_is_128_bytes() {
    assert_eq!(size_of::<GpuLight>(), 128);
}
