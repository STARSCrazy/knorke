//! Profile tables, shapes and the profile store (chiaro ProfileTests, ported).

// Origin: chiaro @ 9bc5b9b, tests/unit/ProfileTests.cpp (zlib license, see `src/profile.rs`).
//
// Ported test by test with the same inputs, expected values and tolerances. Tolerances follow
// doctest's `Approx`: |a - b| < eps * (1 + max(|a|, |b|)), default eps = 100 * f32::EPSILON.
// Not ported: the C ABI test (`CHI_InitProfileShape`, ADR 0014 5) and the LightStore test, which
// needs the profile wiring of the light store (see `src/profile_store.rs`).

// The reference oracle uses platform `std` maths for the test inputs, as in the C++ original.
#![allow(clippy::disallowed_methods)]

use knorke_light2d::light_store::LightStore;
use knorke_light2d::profile::{
    PROFILE_AZIMUTH, PROFILE_POLAR, PROFILE_VALUES, ProfileError, ProfilePreset, ProfileShape,
    build_shape_profile, profile_factor, profile_reference, sample_profile_table,
};
use knorke_light2d::profile_store::{MAX_PROFILES, ProfileStore};

fn approx_eps(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() < eps * (1.0 + a.abs().max(b.abs()))
}

fn approx(a: f32, b: f32) -> bool {
    approx_eps(a, b, 100.0 * f32::EPSILON)
}

fn build(s: &ProfileShape) -> Vec<f32> {
    let t = build_shape_profile(s).expect("shape must build");
    assert_eq!(t.len(), PROFILE_POLAR * PROFILE_AZIMUTH);
    t
}

fn preset(p: ProfilePreset) -> ProfileShape {
    ProfileShape::preset(p)
}

/// A table whose value is polar + 1000 * azimuth index, to check addressing.
fn ramp() -> Vec<f32> {
    let mut t = vec![0.0f32; PROFILE_POLAR * PROFILE_AZIMUTH];
    for j in 0..PROFILE_AZIMUTH {
        for i in 0..PROFILE_POLAR {
            t[j * PROFILE_POLAR + i] = (i + 1000 * j) as f32;
        }
    }
    t
}

fn flat() -> Vec<f32> {
    vec![1.0; PROFILE_VALUES]
}

#[test]
fn profile_table_sampling_interpolates_polar_and_azimuth_and_wraps_the_azimuth() {
    let t = ramp();
    let s = |g: f32, c: f32| sample_profile_table(&t, g, c);
    assert_eq!(s(0.0, 0.0), 0.0);
    assert!(approx(s(10.0, 0.0), 10.0));
    assert!(approx(s(10.5, 0.0), 10.5));
    assert!(approx(s(180.0, 0.0), 180.0));
    assert!(approx(s(200.0, 0.0), 180.0)); // clamped
    assert!(approx(s(-5.0, 0.0), 0.0));
    assert!(approx(s(0.0, 5.0), 1000.0));
    assert!(approx(s(0.0, 7.5), 1500.0));
    // Between the last azimuth row (355) and the first (0 = 360).
    assert!(approx(s(0.0, 357.5), 71000.0 * 0.5));
    assert!(approx(s(0.0, -2.5), 71000.0 * 0.5));
    assert!(approx(s(0.0, 365.0), 1000.0));
}

#[test]
fn profile_reference_c0_is_plus_x_projected_off_the_axis_rotation_turns_it_towards_c90() {
    let down = [0.0f32, 0.0, -1.0];
    let r = profile_reference(down, 0.0);
    assert!(approx(r[0], 1.0));
    assert!(approx(r[1], 0.0));
    assert!(approx(r[2], 0.0));
    let r = profile_reference(down, 90.0);
    assert!(approx_eps(r[0], 0.0, 1e-5));
    assert!(approx(r[1], 1.0)); // C90 of the unrotated frame is +y
    let along_x = [1.0f32, 0.0, 0.0];
    let r = profile_reference(along_x, 0.0);
    assert!(approx(r[0], 0.0));
    assert!(approx(r[1], 1.0));
    let tilted = [0.6f32, 0.0, -0.8];
    let r = profile_reference(tilted, 0.0);
    assert!(approx_eps(
        r[0] * tilted[0] + r[1] * tilted[1] + r[2] * tilted[2],
        0.0,
        1e-5
    ));
    assert!(approx(
        (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt(),
        1.0
    ));
}

#[test]
fn profile_factor_finds_polar_and_azimuth_of_a_world_direction() {
    let t = ramp();
    let down = [0.0f32, 0.0, -1.0];
    let r = profile_reference(down, 0.0);
    let straight_down = [0.0f32, 0.0, -5.0];
    assert!(approx_eps(
        profile_factor(&t, down, r, straight_down),
        0.0,
        1e-3
    ));
    let horizontal_x = [3.0f32, 0.0, 0.0];
    assert!(approx(profile_factor(&t, down, r, horizontal_x), 90.0));
    let horizontal_y = [0.0f32, 3.0, 0.0]; // C90
    assert!(approx(
        profile_factor(&t, down, r, horizontal_y),
        90.0 + 18000.0
    ));
    let horizontal_minus_y = [0.0f32, -3.0, 0.0]; // C270
    assert!(approx(
        profile_factor(&t, down, r, horizontal_minus_y),
        90.0 + 54000.0
    ));
    let zero = [0.0f32, 0.0, 0.0];
    assert!(approx(profile_factor(&t, down, r, zero), 0.0)); // at the light: along the axis
}

#[test]
fn presets_are_normalized_to_1_and_follow_their_luminaire_downlight() {
    // downlight: brightest along the axis, dark beyond 60 degrees
    let t = build(&preset(ProfilePreset::Downlight));
    let max = t.iter().copied().fold(f32::MIN, f32::max);
    assert!(approx(max, 1.0));
    assert!(approx(sample_profile_table(&t, 0.0, 0.0), 1.0));
    assert!(sample_profile_table(&t, 30.0, 0.0) < sample_profile_table(&t, 10.0, 0.0));
    assert_eq!(sample_profile_table(&t, 61.0, 120.0), 0.0);
}

#[test]
fn presets_hat_lamp_strong_to_the_side_cut_off_just_above_the_horizontal_symmetric() {
    let t = build(&preset(ProfilePreset::HatLamp));
    let max = t.iter().copied().fold(f32::MIN, f32::max);
    assert!(approx(max, 1.0));
    assert!(approx(sample_profile_table(&t, 72.0, 0.0), 1.0));
    assert!(sample_profile_table(&t, 72.0, 0.0) > sample_profile_table(&t, 0.0, 0.0));
    assert!(approx_eps(
        sample_profile_table(&t, 0.0, 0.0),
        0.55,
        0.02 * 1.0
    ));
    assert!(sample_profile_table(&t, 92.0, 0.0) > 0.1); // still light just above the horizontal
    assert_eq!(sample_profile_table(&t, 98.0, 0.0), 0.0);
    for c in [35.0f32, 90.0, 200.0, 290.0] {
        assert!(approx(
            sample_profile_table(&t, 60.0, c),
            sample_profile_table(&t, 60.0, 0.0)
        ));
    }
}

#[test]
fn presets_led_street_lamp_full_cutoff_batwing_along_the_street_little_to_the_house_side() {
    let t = build(&preset(ProfilePreset::LedStreet));
    let max = t.iter().copied().fold(f32::MIN, f32::max);
    assert!(approx(max, 1.0));
    assert!(approx(sample_profile_table(&t, 68.0, 0.0), 1.0));
    assert!(approx(sample_profile_table(&t, 68.0, 180.0), 1.0));
    assert!(sample_profile_table(&t, 68.0, 0.0) > 2.0 * sample_profile_table(&t, 0.0, 0.0));
    let mut c = 0.0f32;
    while c < 360.0 {
        assert_eq!(sample_profile_table(&t, 85.0, c), 0.0);
        assert_eq!(sample_profile_table(&t, 120.0, c), 0.0);
        c += 5.0;
    }
    let mut street = 0.0f32;
    let mut house = 0.0f32;
    let mut g = 0.0f32;
    while g <= 84.0 {
        street = street.max(sample_profile_table(&t, g, 90.0));
        house = house.max(sample_profile_table(&t, g, 270.0));
        g += 1.0;
    }
    assert!(house < 0.25 * street);
}

#[test]
fn shape_gain_and_peak_follow_the_azimuth() {
    let mut s = preset(ProfilePreset::HatLamp);
    s.gain = [1.0, 0.5, 1.0, 0.0];
    let t = build(&s);
    assert!(approx(sample_profile_table(&t, 72.0, 90.0), 0.5));
    assert_eq!(sample_profile_table(&t, 72.0, 270.0), 0.0);
    assert!(approx(sample_profile_table(&t, 72.0, 45.0), 0.75));

    let mut s = preset(ProfilePreset::HatLamp);
    s.peak_angle = 20.0;
    s.peak_angle_c90 = 70.0;
    s.nadir = 0.0;
    s.spread_angle = 5.0;
    let u = build(&s);
    assert!(approx(sample_profile_table(&u, 20.0, 0.0), 1.0));
    assert!(approx(sample_profile_table(&u, 70.0, 90.0), 1.0));
    assert!(sample_profile_table(&u, 70.0, 0.0) < 0.01);
}

#[test]
fn hard_cutoff_ends_exactly_at_the_cutoff_angle() {
    let mut s = preset(ProfilePreset::Downlight);
    s.cutoff_fade_angle = 0.0;
    s.cutoff_angle = 40.0;
    let t = build(&s);
    assert!(sample_profile_table(&t, 40.0, 0.0) > 0.0);
    assert_eq!(sample_profile_table(&t, 41.0, 0.0), 0.0);
}

#[test]
fn invalid_shapes_are_rejected_with_a_message() {
    let reject = |edit: fn(&mut ProfileShape), field: &str| {
        let mut s = preset(ProfilePreset::HatLamp);
        edit(&mut s);
        let err = build_shape_profile(&s).expect_err("shape must be rejected");
        let text = err.to_string();
        assert!(text.contains(field), "{text} does not contain {field}");
    };
    reject(|s| s.peak_angle = -1.0, "peak_angle");
    reject(|s| s.peak_angle_c90 = 181.0, "peak_angle_c90");
    reject(|s| s.spread_angle = 0.0, "spread_angle");
    reject(|s| s.nadir = 1.5, "nadir");
    reject(|s| s.cutoff_angle = 0.0, "cutoff_angle");
    reject(|s| s.cutoff_fade_angle = 200.0, "cutoff_fade_angle");
    reject(|s| s.gain[2] = -1.0, "gain");
    reject(|s| s.gain = [0.0; 4], "gain");
    reject(|s| s.spread_angle = f32::NAN, "spread_angle");
    // A peak beyond the cutoff with no nadir leaves (almost) no light at all.
    reject(
        |s| {
            s.peak_angle = 170.0;
            s.peak_angle_c90 = 170.0;
            s.spread_angle = 1.0;
            s.nadir = 0.0;
            s.cutoff_angle = 30.0;
        },
        "no light",
    );
}

#[test]
fn profile_store_blocks_are_slot_indices_the_version_changes_at_most_128_alive() {
    let mut store = ProfileStore::new();
    let lights = LightStore::new();
    assert_eq!(store.block_count(), 0);
    let v0 = store.version();
    let a = store.create(flat()).expect("first profile");
    let b = store.create(flat()).expect("second profile");
    assert_ne!(store.version(), v0);
    assert_eq!(store.block_of(a), Some(0));
    assert_eq!(store.block_of(b), Some(1));
    assert_eq!(store.block_count(), 2);
    assert!(store.find(a).is_some());
    store.destroy(a, &lights).expect("destroy a");
    assert!(store.find(a).is_none());
    assert_eq!(store.block_of(a), None);
    assert_eq!(store.block_count(), 2);
    let err = store.destroy(a, &lights).expect_err("second destroy");
    assert!(err.to_string().contains("invalid or destroyed profile"));
    let c = store.create(flat()).expect("reuses block 0"); // new generation
    assert_eq!(store.block_of(c), Some(0));
    assert_ne!(c, a);
    assert!(store.find(a).is_none());
    while store.len() < MAX_PROFILES {
        store.create(flat()).expect("below the limit");
    }
    let err = store.create(flat()).expect_err("limit reached");
    assert!(err.to_string().contains("too many light profiles"));
    assert_eq!(store.block_count(), MAX_PROFILES as u32);
    assert!(store.create(vec![0.0; 3]).is_err());
}

#[test]
fn profile_store_rejects_a_table_of_the_wrong_size() {
    let mut store = ProfileStore::new();
    let err = store
        .create(vec![0.0; 3])
        .expect_err("wrong size must be rejected");
    assert!(matches!(err, ProfileError::WrongSize { .. }));
    assert_eq!(store.len(), 0);
}

/// From chiaro's ProfileTests ("light desc defaults have no profile"): the light description
/// starts without a profile and without rotation.
#[test]
fn light_desc_defaults_have_no_profile() {
    use knorke_light2d::light::{LightDesc, LightType};
    let d = LightDesc::new(LightType::Point);
    assert!(d.profile.is_none());
    assert_eq!(d.profile_rotation, 0.0);
}

// ---- P9b: wiring to the light store, resources and culling ----

#[test]
fn a_profile_in_use_by_a_light_cannot_be_destroyed_and_the_resources_see_it() {
    use knorke_light2d::StoreResources;
    use knorke_light2d::animation::AnimationStore;
    use knorke_light2d::light::{LightDesc, LightResources, LightType};
    use knorke_light2d::mask_store::MaskStore;

    let animations = AnimationStore::new();
    let masks = MaskStore::new();
    let mut profiles = ProfileStore::new();
    let mut lights = LightStore::new();
    let p = profiles.create(flat()).expect("profile");

    let mut d = LightDesc::new(LightType::Spot);
    d.profile = Some(p);
    // Without the profile store no profile exists.
    let bare = StoreResources::new(&animations, &masks);
    assert!(!bare.has_profile(p));
    assert!(lights.create(&d, &bare).is_err());

    let res = StoreResources::new(&animations, &masks).with_profiles(&profiles);
    assert!(res.has_profile(p));
    let id = lights.create(&d, &res).expect("light with profile");
    assert_eq!(lights.profile_users(p), 1);

    let err = profiles.destroy(p, &lights).expect_err("in use");
    assert!(matches!(err, ProfileError::InUse { users: 1 }));
    assert_eq!(err.code().as_str(), "knorke.light2d.profile.in_use");
    assert!(profiles.contains(p), "refused destroy keeps the profile");

    d.profile = None;
    lights
        .update(id, &d, &StoreResources::new(&animations, &masks))
        .expect("clear profile");
    profiles.destroy(p, &lights).expect("unused now");
    assert!(!profiles.contains(p));
}

#[test]
fn the_profile_store_answers_the_atlas_rows_for_culling() {
    use knorke_light2d::light_culling::ProfileAtlas;

    let mut store = ProfileStore::new();
    let lights = LightStore::new();
    assert_eq!(store.inv_atlas_height(), 0.0);
    let a = store.create(flat()).unwrap();
    let b = store.create(flat()).unwrap();
    assert_eq!(store.atlas_row(a), Some(0.0));
    assert_eq!(store.atlas_row(b), Some(PROFILE_AZIMUTH as f32));
    let h = store.inv_atlas_height();
    assert!(h > 0.0 && h <= 1.0 / (2 * PROFILE_AZIMUTH) as f32);
    store.destroy(a, &lights).unwrap();
    assert_eq!(store.atlas_row(a), None);
    assert!(store.atlas_row(b).is_some());
}
