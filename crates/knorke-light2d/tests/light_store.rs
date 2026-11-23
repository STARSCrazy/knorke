//! Origin: chiaro @ 9bc5b9b, tests/unit/LightStoreTests.cpp (zlib license, see `src/light.rs` and
//! `src/light_store.rs`), plus tests for what the redesign adds: handles from `Slots`, typed
//! errors with stable codes, resource checks through a trait, batches, frame lights and snapshots.

// Tests compare against platform `std` maths as an oracle and use `f32::max` in tolerances;
// only library code must be platform independent (ADR 0011).
#![allow(clippy::disallowed_methods)]

use knorke_light2d::light::*;
use knorke_light2d::{LightStore, NoResources};
use knorke_math::{At, Vec2, Vec3};
use knorke_types::{
    Handle, Reader, Slots, Snapshot, SnapshotBuf, SnapshotError, SnapshotValue, Writer, restore,
};

const NAN: f32 = f32::NAN;
const INF: f32 = f32::INFINITY;

fn light(kind: LightType) -> LightDesc {
    LightDesc::new(kind)
}

fn rejected(kind: LightType, mutate: impl FnOnce(&mut LightDesc)) -> LightError {
    let mut d = light(kind);
    mutate(&mut d);
    d.validate().expect_err("must be rejected")
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-5 * b.abs().max(1.0)
}

#[test]
fn light_desc_defaults() {
    let d = light(LightType::Spot);
    assert_eq!(d.kind, LightType::Spot);
    assert_eq!(d.color, Vec3::ONE);
    assert_eq!(
        (d.intensity, d.specular, d.radius, d.falloff),
        (1.0, 1.0, 100.0, 2.0)
    );
    assert_eq!((d.inner_angle, d.outer_angle), (20.0, 30.0));
    assert_eq!((d.flicker, d.flicker_speed), (0.0, 8.0));
    assert_eq!(d.direction.x, 1.0);
    assert!(d.enabled);
    // Public defaults of CHI_InitLightDesc: shadows on, height resolved per frame.
    assert!(d.cast_shadows);
    assert_eq!(d.height, Height::Auto);
    assert_eq!(d.validate(), Ok(()));
    assert_eq!(light(LightType::Point).axis.x, 1.0);
}

#[test]
fn invalid_light_descs_are_rejected() {
    let cases: [fn(&mut LightDesc); 9] = [
        |d| d.radius = 0.0,
        |d| d.position = At::new(Vec2::new(0.0, NAN)),
        |d| d.outer_angle = 10.0, // < inner
        |d| d.outer_angle = 90.0,
        |d| d.intensity = -1.0,
        |d| d.color.z = -0.5,
        |d| d.flicker = 1.5,
        |d| d.falloff = 0.0,
        |d| d.direction.x = 0.0,
    ];
    for (i, mutate) in cases.iter().enumerate() {
        let mut d = light(LightType::Spot);
        mutate(&mut d);
        let e = d.validate().unwrap_err();
        assert!(!e.to_string().is_empty(), "case {i}");
    }
}

#[test]
fn derived_light_values_are_computed_once_when_a_light_is_stored() {
    let mut d = light(LightType::Spot);
    d.direction = Vec3::new(3.0, 4.0, 0.0);
    let mut store = LightStore::new();
    store.create(&d, &NoResources).unwrap();
    let mut visited = 0;
    for (_, l) in store.iter() {
        visited += 1;
        let deg = std::f32::consts::PI / 180.0;
        assert!(near(l.cos_inner, (20.0 * deg).cos()));
        assert!(near(l.cos_outer, (30.0 * deg).cos()));
        assert!(near(l.direction.x, 0.6) && near(l.direction.y, 0.8));
        assert_eq!(l.desc.direction.x, 3.0); // the desc itself is kept as given
    }
    assert_eq!(visited, 1);
}

#[test]
fn directional_lights_need_no_radius() {
    let mut d = light(LightType::Directional);
    d.radius = 0.0;
    assert_eq!(d.validate(), Ok(()));
}

#[test]
fn light_store_lifecycle() {
    let mut store = LightStore::new();
    let mut d = light(LightType::Point);
    d.position = At::new(Vec2::new(12.0, 0.0));
    let id = store.create(&d, &NoResources).unwrap();
    assert_eq!(store.get(id).unwrap().position.raw().x, 12.0);

    let mut bad = d.clone();
    bad.radius = -1.0;
    assert!(store.update(id, &bad, &NoResources).is_err());
    assert_eq!(store.get(id).unwrap().radius, d.radius);

    let p = At::new(Vec2::new(1.0, 2.0));
    store.set_position(id, p, Height::At(3.0)).unwrap();
    assert_eq!(store.get(id).unwrap().height, Height::At(3.0));
    assert_eq!(store.get(id).unwrap().position, p);
    assert!(
        store
            .set_position(id, At::new(Vec2::new(INF, 0.0)), Height::At(0.0))
            .is_err()
    );
    assert!(store.set_position(id, p, Height::At(-2.0)).is_err());
    store.set_position(id, p, Height::Auto).unwrap();
    assert_eq!(store.get(id).unwrap().height, Height::Auto);

    store.destroy(id).unwrap();
    assert_eq!(store.get(id), Err(LightError::InvalidHandle(id.to_bits())));
    assert!(store.destroy(id).is_err());
    assert!(store.create(&bad, &NoResources).is_err());
    assert_eq!(store.len(), 0);
}

#[test]
fn a_stale_handle_never_reaches_a_reused_slot() {
    let mut store = LightStore::new();
    let a = store
        .create(&light(LightType::Point), &NoResources)
        .unwrap();
    store.destroy(a).unwrap();
    let b = store
        .create(&light(LightType::Point), &NoResources)
        .unwrap();
    assert_eq!(a.index(), b.index());
    assert!(store.get(a).is_err());
    assert!(store.get(b).is_ok());
}

#[test]
fn source_radius_defaults_to_0_must_be_finite_and_non_negative() {
    let mut d = light(LightType::Point);
    assert_eq!(d.source_radius, 0.0);
    let mut store = LightStore::new();
    d.source_radius = 3.5;
    let id = store.create(&d, &NoResources).unwrap();
    assert_eq!(store.get(id).unwrap().source_radius, 3.5);
    for bad in [-1.0, NAN, INF] {
        d.source_radius = bad;
        assert!(store.create(&d, &NoResources).is_err());
        assert!(store.update(id, &d, &NoResources).is_err());
    }
    assert_eq!(store.get(id).unwrap().source_radius, 3.5);
}

#[test]
fn a_directional_lights_source_radius_is_an_angle_of_at_most_10_degrees() {
    let mut store = LightStore::new();
    let mut d = light(LightType::Directional);
    d.source_radius = 10.0;
    assert!(store.create(&d, &NoResources).is_ok());
    d.source_radius = 11.0;
    assert!(store.create(&d, &NoResources).is_err());
    let mut d = light(LightType::Point);
    d.source_radius = 11.0; // world units for point lights, no upper limit
    assert!(store.create(&d, &NoResources).is_ok());
}

#[test]
fn light_errors_name_the_field_and_the_received_value() {
    let s = |kind, f: fn(&mut LightDesc)| rejected(kind, f).to_string();
    let spot = LightType::Spot;
    assert!(s(spot, |d| d.radius = -4.0).contains("desc.radius must be > 0 (got -4)"));
    assert!(s(spot, |d| d.falloff = 0.0).contains("desc.falloff"));
    assert!(s(spot, |d| d.height = Height::At(INF)).contains("desc.x/y/height must be finite"));
    assert!(s(spot, |d| d.color.x = INF).contains("desc.color must be finite"));
    assert!(s(spot, |d| d.flicker_speed = INF).contains("desc.flicker_speed must be finite"));
    assert!(s(spot, |d| d.intensity = -1.0).contains("desc.intensity must be >= 0 (got -1)"));
    assert!(s(spot, |d| d.outer_angle = 95.0).contains("(got 20, 95)"));
    assert!(s(spot, |d| d.specular = -0.5).contains("desc.specular must be in [0, 4] (got -0.5)"));
    assert!(s(spot, |d| d.specular = 4.5).contains("desc.specular must be in [0, 4]"));
    assert!(s(spot, |d| d.specular = INF).contains("desc.specular must be finite"));
    assert_eq!(
        rejected(spot, |d| d.radius = -4.0)
            .to_diagnostic()
            .code
            .as_str(),
        "knorke.light2d.light.out_of_range"
    );
}

#[test]
fn line_and_rect_lights_defaults_validation_and_the_stored_axis() {
    let d = light(LightType::Line);
    assert_eq!((d.axis.x, d.half_width), (1.0, 0.5));
    assert!(!d.two_sided);
    assert_eq!(d.validate(), Ok(()));
    assert_eq!(light(LightType::Rect).direction.z, -1.0); // a panel facing down
    assert_eq!(light(LightType::Rect).validate(), Ok(()));

    let s = |kind, f: fn(&mut LightDesc)| rejected(kind, f).to_string();
    assert!(s(LightType::Line, |l| l.axis.x = 0.0).contains("axis"));
    assert!(s(LightType::Rect, |l| l.half_width = 0.0).contains("half_width"));
    assert!(
        s(LightType::Rect, |l| {
            l.axis = Vec3::new(0.0, 0.0, 2.0); // along direction (0, 0, -1)
        })
        .contains("parallel")
    );
    assert!(s(LightType::Rect, |l| l.direction.z = 0.0).contains("direction"));
    assert!(s(LightType::Line, |l| l.axis.y = INF).contains("axis"));
    // A line light needs no direction.
    let mut d = light(LightType::Line);
    d.direction.x = 0.0;
    assert_eq!(d.validate(), Ok(()));

    // A rect's axis is made perpendicular to its direction, its length kept.
    let mut d = light(LightType::Rect);
    d.axis = Vec3::new(3.0, 0.0, 4.0); // 5 long, partly along the normal (0, 0, -1)
    let mut store = LightStore::new();
    store.create(&d, &NoResources).unwrap();
    for (_, l) in store.iter() {
        assert!(near(l.axis.x, 5.0) && near(l.axis.z, 0.0));
        assert_eq!(l.desc.axis.z, 4.0); // the desc itself is kept as given
    }
}

#[test]
fn line_and_rect_lights_reject_a_profile_and_line_endpoints_give_centre_and_half_axis() {
    let mut d = light(LightType::Line);
    d.profile = Some(Handle::new(1, std::num::NonZeroU32::MIN)); // checked before the handle
    let mut store = LightStore::new();
    let e = store.create(&d, &NoResources).unwrap_err();
    assert!(e.to_string().contains("line or rect"));

    let mut d = light(LightType::Line);
    d.set_line_endpoints(Vec3::new(1.0, 2.0, 3.0), Vec3::new(5.0, 0.0, 3.0));
    assert_eq!(d.position.raw(), Vec2::new(3.0, 1.0));
    assert_eq!(d.height, Height::At(3.0));
    assert_eq!(d.axis, Vec3::new(2.0, -1.0, 0.0));
}

#[test]
fn soft_near_field_defaults_off_and_needs_a_source_size() {
    let d = light(LightType::Point);
    assert!(!d.soft_near_field);
    let mut d = light(LightType::Point);
    d.soft_near_field = true;
    assert!(
        d.validate()
            .unwrap_err()
            .to_string()
            .contains("source_radius")
    );
    d.source_radius = 0.5;
    assert_eq!(d.validate(), Ok(()));
    let mut line = light(LightType::Line);
    line.soft_near_field = true;
    line.source_radius = 0.5;
    assert!(line.validate().is_err());
}

#[test]
fn inside_obstacle_defaults_off_needs_shadows_and_a_local_light() {
    assert!(!light(LightType::Point).inside_obstacle);
    for kind in [
        LightType::Point,
        LightType::Spot,
        LightType::Line,
        LightType::Rect,
    ] {
        let mut d = light(kind);
        d.inside_obstacle = true; // shadows on
        assert_eq!(d.validate(), Ok(()), "{kind:?}");
    }
    let mut d = light(LightType::Point);
    d.inside_obstacle = true;
    d.cast_shadows = false;
    assert!(
        d.validate()
            .unwrap_err()
            .to_string()
            .contains("cast_shadows")
    );
    let mut d = light(LightType::Directional);
    d.inside_obstacle = true;
    assert!(
        d.validate()
            .unwrap_err()
            .to_string()
            .contains("directional")
    );
}

#[test]
fn shadow_filter_needs_shadows_and_two_sided_needs_a_rect() {
    let mut d = light(LightType::Point);
    d.shadow_filter = ShadowFilter::StaticOnly;
    assert_eq!(d.validate(), Ok(()));
    d.cast_shadows = false;
    assert!(d.validate().is_err());
    let mut d = light(LightType::Point);
    d.two_sided = true;
    assert!(d.validate().is_err());
    let mut d = light(LightType::Rect);
    d.two_sided = true;
    assert_eq!(d.validate(), Ok(()));
}

// ---- resources: profiles, animations and masks live in other stores ----

#[derive(Default)]
struct Known {
    profile: Option<ProfileId>,
    animation: Option<AnimationId>,
    mask: Option<MaskId>,
}

impl LightResources for Known {
    fn has_profile(&self, id: ProfileId) -> bool {
        self.profile == Some(id)
    }
    fn has_animation(&self, id: AnimationId) -> bool {
        self.animation == Some(id)
    }
    fn has_mask(&self, id: MaskId) -> bool {
        self.mask == Some(id)
    }
}

fn handle<T>(index: u32) -> Handle<T> {
    Handle::new(index, std::num::NonZeroU32::MIN)
}

#[test]
fn profiles_animations_and_masks_must_exist_and_fit_the_light() {
    let res = Known {
        profile: Some(handle(0)),
        animation: Some(handle(0)),
        mask: Some(handle(0)),
    };
    let mut store = LightStore::new();

    let mut d = light(LightType::Spot);
    d.profile = Some(handle(0));
    d.animation = Some(handle(0));
    d.mask = Some(handle(0));
    let id = store.create(&d, &res).unwrap();
    assert_eq!(store.profile_users(handle(0)), 1);
    assert_eq!(store.animation_users(handle(0)), 1);
    assert_eq!(store.mask_users(handle(0)), 1);
    assert_eq!(store.mask_users(handle(1)), 0);

    // Unknown handles name the resource.
    let mut bad = d.clone();
    bad.profile = Some(handle(5));
    assert!(
        store
            .create(&bad, &res)
            .unwrap_err()
            .to_string()
            .contains("profile")
    );
    bad = d.clone();
    bad.animation = Some(handle(5));
    assert!(
        store
            .update(id, &bad, &res)
            .unwrap_err()
            .to_string()
            .contains("animation")
    );
    bad = d.clone();
    bad.mask = Some(handle(5));
    assert!(
        store
            .create(&bad, &res)
            .unwrap_err()
            .to_string()
            .contains("mask")
    );
    // A profile needs an axis, a directional light takes no profile or mask.
    bad = d.clone();
    bad.direction = Vec3::ZERO;
    assert!(store.create(&bad, &res).is_err());
    let mut dir = light(LightType::Directional);
    dir.mask = Some(handle(0));
    assert!(
        store
            .create(&dir, &res)
            .unwrap_err()
            .to_string()
            .contains("directional light cannot have a mask")
    );
    // Without a resource store every non-zero handle is rejected.
    assert!(store.create(&d, &NoResources).is_err());
}

#[test]
fn window_masks_need_spanning_axes() {
    let res = Known {
        mask: Some(handle(0)),
        ..Known::default()
    };
    let mut d = light(LightType::Point);
    d.mask = Some(handle(0));
    d.mask_mode = MaskMode::Window;
    d.mask_u = Vec3::new(0.5, 0.0, 0.0);
    d.mask_v = Vec3::new(1.0, 0.0, 0.0); // parallel to u
    let mut store = LightStore::new();
    assert!(
        store
            .create(&d, &res)
            .unwrap_err()
            .to_string()
            .contains("parallel")
    );
    d.mask_v = Vec3::new(0.0, 0.0, 0.5);
    assert!(store.create(&d, &res).is_ok());
    d.mask_u = Vec3::ZERO;
    assert!(store.create(&d, &res).is_err());
    d.mask_u = Vec3::new(0.5, 0.0, 0.0);
    d.mask_angle = Some(120.0);
    assert!(store.create(&d, &res).is_err());
}

// ---- batches ----

#[test]
fn batches_are_all_or_nothing_and_name_the_index() {
    let mut store = LightStore::new();
    let mut descs = vec![light(LightType::Point); 4];
    descs[2].radius = -1.0;
    let mut ids = Vec::new();
    let e = store
        .create_many(&descs[..], &NoResources, &mut ids)
        .unwrap_err();
    assert!(e.to_string().contains("descs[2]"), "{e}");
    assert_eq!(store.len(), 0);
    assert!(ids.is_empty());

    descs[2].radius = 5.0;
    store
        .create_many(&descs[..], &NoResources, &mut ids)
        .unwrap();
    assert_eq!(ids.len(), 4);
    assert_eq!(store.len(), 4);

    // update_many: one bad desc changes nothing.
    let mut updated = descs.clone();
    for u in &mut updated {
        u.intensity = 9.0;
    }
    updated[3].falloff = 0.0;
    assert!(
        store
            .update_many(&ids[..], &updated[..], &NoResources)
            .is_err()
    );
    assert!(ids.iter().all(|&i| store.get(i).unwrap().intensity == 1.0));
    updated[3].falloff = 2.0;
    store
        .update_many(&ids[..], &updated[..], &NoResources)
        .unwrap();
    assert!(ids.iter().all(|&i| store.get(i).unwrap().intensity == 9.0));

    // destroy_many: stale and duplicate ids are rejected before anything is removed.
    let dup = [ids[0], ids[1], ids[0]];
    assert!(
        store
            .destroy_many(&dup[..])
            .unwrap_err()
            .to_string()
            .contains("ids[2]")
    );
    assert_eq!(store.len(), 4);
    store.destroy_many(&ids[..2]).unwrap();
    assert_eq!(store.len(), 2);
    assert!(store.destroy_many(&ids[..2]).is_err());

    // set_positions
    let pos = [
        LightPosition::new(At::new(Vec2::new(1.0, 1.0)), Height::At(2.0)),
        LightPosition::new(At::new(Vec2::new(3.0, 3.0)), Height::Auto),
    ];
    store.set_positions(&ids[2..], &pos[..]).unwrap();
    assert_eq!(
        store.get(ids[3]).unwrap().position.raw(),
        Vec2::new(3.0, 3.0)
    );
    let bad = [
        pos[0],
        LightPosition::new(At::new(Vec2::new(NAN, 0.0)), Height::Auto),
    ];
    assert!(store.set_positions(&ids[2..], &bad[..]).is_err());
    assert_eq!(
        store.get(ids[2]).unwrap().position.raw(),
        Vec2::new(1.0, 1.0)
    );
    assert!(store.set_positions(&ids[2..], &pos[..1]).is_err()); // length mismatch
}

// ---- frame lights ----

#[test]
fn frame_lights_are_transient_follow_the_persistent_ones_and_keep_their_pool() {
    let mut store = LightStore::new();
    let a = store
        .create(&light(LightType::Point), &NoResources)
        .unwrap();
    let frame = vec![light(LightType::Spot), light(LightType::Directional)];
    store.set_frame_lights(&frame[..], &NoResources).unwrap();
    assert_eq!(store.frame_light_count(), 2);
    let all: Vec<_> = store.iter().map(|(r, l)| (r, l.desc.kind)).collect();
    assert_eq!(
        all,
        vec![
            (LightRef::Persistent(a), LightType::Point),
            (LightRef::Transient(0), LightType::Spot),
            (LightRef::Transient(1), LightType::Directional),
        ]
    );
    assert_eq!(all[0].0.seed_index(), a.index());
    assert_eq!(all[2].0.seed_index(), 1);

    // all or nothing: a bad desc keeps the previous frame lights
    let mut bad = frame.clone();
    bad[1].radius = -1.0;
    bad[1].kind = LightType::Point;
    assert!(
        store
            .set_frame_lights(&bad[..], &NoResources)
            .unwrap_err()
            .to_string()
            .contains("descs[1]")
    );
    assert_eq!(store.frame_light_count(), 2);

    store.clear_frame_lights();
    assert_eq!(store.frame_light_count(), 0);
    assert_eq!(store.iter().count(), 1);

    let many = vec![light(LightType::Point); MAX_FRAME_LIGHTS + 1];
    assert!(store.set_frame_lights(&many[..], &NoResources).is_err());
}

// ---- snapshots (ADR 0010) ----

fn world_with_lights() -> (LightStore, Vec<LightId>) {
    let mut store = LightStore::new();
    let mut ids = Vec::new();
    for i in 0..5 {
        let mut d = light(LightType::Spot);
        d.position = At::new(Vec2::new(i as f32, 2.0 * i as f32));
        d.intensity = 1.0 + i as f32;
        ids.push(store.create(&d, &NoResources).unwrap());
    }
    store.destroy(ids[1]).unwrap(); // a hole in the slots must survive the round trip
    store.destroy(ids[3]).unwrap();
    (store, ids)
}

#[test]
fn snapshot_round_trip_keeps_handles_and_derived_values() {
    let (store, ids) = world_with_lights();
    let mut buf = SnapshotBuf::new();
    buf.save(&store);
    let checksum = buf.checksum();

    let mut other = LightStore::new();
    other
        .create(&light(LightType::Point), &NoResources)
        .unwrap();
    let mut r = Reader::new(buf.bytes());
    restore(&mut r, &mut other).unwrap();
    assert_eq!(r.remaining(), 0);
    assert_eq!(other.len(), 3);
    for &id in &[ids[0], ids[2], ids[4]] {
        assert_eq!(other.get(id), store.get(id));
        assert_eq!(other.stored(id), store.stored(id));
    }
    assert!(other.get(ids[1]).is_err());
    let mut again = SnapshotBuf::new();
    again.save(&other);
    assert_eq!(again.checksum(), checksum, "equal state gives equal bytes");

    // Restored slots keep their free list: the next id is the one the original would issue.
    let mut a = store.clone();
    let mut b = other;
    let d = light(LightType::Point);
    assert_eq!(
        a.create(&d, &NoResources).unwrap(),
        b.create(&d, &NoResources).unwrap()
    );
}

#[test]
fn restore_drops_frame_lights_and_rejects_invalid_data() {
    let (mut store, _) = world_with_lights();
    store
        .set_frame_lights(&[light(LightType::Point)][..], &NoResources)
        .unwrap();
    let mut buf = SnapshotBuf::new();
    buf.save(&store);
    restore(&mut Reader::new(buf.bytes()), &mut store).unwrap();
    assert_eq!(
        store.frame_light_count(),
        0,
        "transient lights are not part of the state"
    );

    // A light that validation would reject is `SnapshotError::Invalid`, never a later panic.
    let mut bad = light(LightType::Point);
    bad.radius = -3.0;
    let mut slots: Slots<LightDesc> = Slots::new();
    slots.insert(bad);
    let mut forged = SnapshotBuf::new();
    forged.write_section(<LightStore as Snapshot>::ID, 1, |w: &mut Writer<'_>| {
        w.put(&slots)
    });
    let e = restore(&mut Reader::new(forged.bytes()), &mut store).unwrap_err();
    assert!(matches!(e, SnapshotError::Invalid(_)), "{e:?}");
    // Unsupported version.
    let mut v2 = SnapshotBuf::new();
    v2.write_section(<LightStore as Snapshot>::ID, 9, |_| {});
    assert!(matches!(
        restore(&mut Reader::new(v2.bytes()), &mut store),
        Err(SnapshotError::UnsupportedVersion { .. })
    ));
}

#[test]
fn light_desc_snapshot_value_round_trips_every_field() {
    let mut d = light(LightType::Rect);
    d.position = At::new(Vec2::new(1.5, -2.5));
    d.height = Height::At(4.0);
    d.profile = Some(handle(3));
    d.animation = Some(handle(4));
    d.mask = Some(handle(5));
    d.mask_mode = MaskMode::Window;
    d.mask_angle = Some(33.0);
    d.shadow_filter = ShadowFilter::DynamicOnly;
    d.two_sided = true;
    d.inside_obstacle = true;
    d.mask_center = Vec3::new(1.0, 2.0, 3.0);
    let mut bytes = Vec::new();
    d.put(&mut Writer::new(&mut bytes));
    let back = LightDesc::get(&mut Reader::new(&bytes)).unwrap();
    assert_eq!(back, d);
    // An unknown enum tag is invalid, not a panic.
    bytes[0] = 99;
    assert!(LightDesc::get(&mut Reader::new(&bytes)).is_err());
}
