//! Origin: chiaro @ 9bc5b9b, tests/unit/AnimationTests.cpp (zlib license, see `src/animation.rs`),
//! plus tests for what the redesign adds: typed errors with stable codes, handles from `Slots`,
//! the in-use check of `destroy` and snapshots.

// Tests compare against platform `std` maths as an oracle; only library code must be platform
// independent (ADR 0011).
#![allow(clippy::disallowed_methods)]

use knorke_light2d::animation::*;
use knorke_light2d::light::{LightDesc, LightRef, LightType};
use knorke_light2d::{LightStore, MaskStore, StoreResources};
use knorke_math::Vec3;
use knorke_types::{Reader, Slots, Snapshot, SnapshotBuf, SnapshotError, Writer, restore};

const NAN: f32 = f32::NAN;

fn make(kind: AnimationType) -> AnimationDesc {
    AnimationDesc::new(kind)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-5 * b.abs().max(1.0)
}

fn w(d: &AnimationDesc, u: f64, seed: u32) -> f32 {
    d.curve(u, seed).w
}

fn rejected(kind: AnimationType, mutate: impl FnOnce(&mut AnimationDesc)) -> String {
    let mut d = make(kind);
    mutate(&mut d);
    d.validate().expect_err("must be rejected").to_string()
}

#[test]
fn every_types_defaults_are_valid() {
    for kind in AnimationType::ALL {
        let d = make(kind);
        assert_eq!(d.kind, kind);
        d.validate().unwrap();
    }
}

#[test]
fn pulse_is_a_smooth_cosine_periodic_and_defined_for_negative_times() {
    let mut d = make(AnimationType::Pulse);
    d.period = 2.0;
    assert!(w(&d, 0.0, 0).abs() < 1e-6);
    assert!(near(w(&d, 1.0, 0), 1.0));
    assert!(near(w(&d, 0.5, 0), 0.5));
    assert!(near(w(&d, 7.0, 0), 1.0));
    assert!(near(w(&d, -1.5, 0), w(&d, 0.5, 0)));
    d.start = 10.0;
    assert!(near(d.sample(11.0, 0.0, 0).intensity, 1.0));
    assert!(near(d.sample(10.0, 0.0, 0).intensity, d.minimum));
    // The offset moves the light to the peak.
    assert!(near(d.sample(10.0, 1.0, 0).intensity, 1.0));
}

#[test]
fn flash_has_count_flashes_of_duty_times_period_spaced_twice_that() {
    let mut d = make(AnimationType::Flash);
    d.period = 1.0;
    d.duty = 0.1;
    d.count = 2;
    assert_eq!(w(&d, 0.0, 0), 1.0);
    assert_eq!(w(&d, 0.05, 0), 1.0);
    assert_eq!(w(&d, 0.15, 0), 0.0);
    assert_eq!(w(&d, 0.25, 0), 1.0);
    assert_eq!(w(&d, 0.35, 0), 0.0);
    assert_eq!(w(&d, 0.45, 0), 0.0); // a third flash would start at 0.4
    assert_eq!(w(&d, 0.9, 0), 0.0);
    assert_eq!(w(&d, 1.02, 0), 1.0);
    assert_eq!(d.sample(0.5, 0.0, 0).intensity, 0.0);
}

#[test]
fn beacon_keeps_full_intensity_and_turns_once_per_period() {
    let mut d = make(AnimationType::Beacon);
    d.period = 2.0;
    let s = d.sample(0.5, 0.0, 0);
    assert_eq!(s.intensity, 1.0);
    assert_eq!(s.color.x, 1.0);
    assert!(near(s.rotation, 90.0));
    assert!(near(d.sample(3.0, 0.0, 0).rotation, 180.0));
    assert!(near(d.sample(-0.5, 0.0, 0).rotation, 270.0));
}

#[test]
fn fire_is_bounded_deterministic_differs_between_seeds_and_redder_when_dark() {
    let d = make(AnimationType::Fire);
    let (mut lo, mut hi, mut differ) = (1.0f32, 0.0f32, 0);
    for i in 0..400 {
        let u = f64::from(i) * 0.013;
        let v = w(&d, u, 3);
        assert!((0.0..=1.0).contains(&v));
        assert_eq!(v, w(&d, u, 3));
        lo = lo.min(v);
        hi = hi.max(v);
        differ += i32::from((v - w(&d, u, 4)).abs() > 0.05);
    }
    assert!(hi - lo > 0.4);
    assert!(differ > 100);
    // The sample with the lowest w has the smallest green/red ratio.
    let (mut dark, mut bright) = (AnimationSample::IDENTITY, AnimationSample::IDENTITY);
    let (mut w_dark, mut w_bright) = (2.0f32, -1.0f32);
    for i in 0..400 {
        let u = f64::from(i) * 0.013;
        let v = w(&d, u, 3);
        if v < w_dark {
            w_dark = v;
            dark = d.sample(u, 0.0, 3);
        }
        if v > w_bright {
            w_bright = v;
            bright = d.sample(u, 0.0, 3);
        }
    }
    assert!(dark.intensity < bright.intensity);
    assert!(dark.color.y / dark.color.x < bright.color.y / bright.color.x);
    assert!(dark.intensity >= d.minimum);
}

#[test]
fn envelope_has_attack_hold_quadratic_decay_once_or_repeated() {
    let mut d = make(AnimationType::Envelope);
    d.attack = 0.1;
    d.hold = 0.2;
    d.decay = 0.5;
    d.period = 0.0;
    d.start = 5.0;
    let i = |d: &AnimationDesc, t: f64| d.sample(t, 0.0, 0).intensity;
    assert_eq!(i(&d, 4.0), 0.0);
    assert!(near(i(&d, 5.05), 0.5));
    assert!(near(i(&d, 5.2), 1.0));
    assert!(near(i(&d, 5.55), 0.25)); // halfway: (1 - 0.5)^2
    assert_eq!(i(&d, 5.9), 0.0);
    assert_eq!(i(&d, 500.0), 0.0);
    d.period = 2.0;
    assert!(near(i(&d, 7.2), 1.0));
    assert!(near(i(&d, 3.2), 1.0)); // repeats before start too
    d.attack = 0.0;
    assert_eq!(i(&d, 5.0), 1.0);
}

#[test]
fn colour_factor_follows_low_color_at_zero_and_is_white_at_one() {
    let mut d = make(AnimationType::Pulse);
    d.minimum = 0.0;
    d.low_color = Vec3::new(1.0, 0.5, 0.0);
    let low = d.sample(0.0, 0.0, 0);
    assert!(near(low.color.y, 0.5));
    assert!(low.color.z.abs() < 1e-6);
    let high = d.sample(0.5, 0.0, 0);
    assert!(near(high.color.y, 1.0));
    assert!(near(high.color.z, 1.0));
}

#[test]
fn validation_rejects_unusable_descs_with_a_message_naming_the_field() {
    use AnimationType::*;
    assert!(rejected(Pulse, |d| d.period = 0.0).contains("period"));
    assert!(rejected(Pulse, |d| d.minimum = 1.5).contains("minimum"));
    assert!(rejected(Pulse, |d| d.low_color.y = -0.1).contains("low_color"));
    let mut d = make(Flash);
    d.duty = 0.3;
    d.count = 2;
    d.validate().unwrap();
    d.count = 3;
    assert!(d.validate().unwrap_err().to_string().contains("count"));
    assert!(rejected(Flash, |d| d.count = 0).contains("count"));
    assert!(rejected(Flash, |d| d.duty = 0.0).contains("duty"));
    assert!(
        rejected(Envelope, |d| {
            d.attack = 0.0;
            d.hold = 0.0;
            d.decay = 0.0;
        })
        .contains("attack")
    );
    // Shorter than attack + hold + decay.
    assert!(rejected(Envelope, |d| d.period = 0.5).contains("period"));
    assert!(rejected(Envelope, |d| d.decay = -1.0).contains("decay"));
    assert!(rejected(Pulse, |d| d.start = f64::NAN).contains("start"));
    assert!(rejected(Pulse, |d| d.period = NAN).contains("period"));
}

#[test]
fn rotate_about_z_turns_counter_clockwise_and_keeps_z() {
    let v = rotate_about_z(Vec3::new(1.0, 0.0, -0.5), 90.0);
    assert!(v.x.abs() < 1e-6);
    assert!(near(v.y, 1.0));
    assert_eq!(v.z, -0.5);
}

#[test]
fn no_animation_is_the_identity() {
    let s = AnimationSample::IDENTITY;
    assert_eq!((s.intensity, s.color, s.rotation), (1.0, Vec3::ONE, 0.0));
}

#[test]
fn store_creates_updates_gets_and_destroys_and_rejects_invalid_descs() {
    let lights = LightStore::new();
    let mut store = AnimationStore::new();
    let a = store.create(&make(AnimationType::Flash)).unwrap();
    let mut bad = make(AnimationType::Pulse);
    bad.period = -1.0;
    let e = store.create(&bad).unwrap_err();
    assert!(e.to_string().contains("period"));
    assert_eq!(e.code().as_str(), "knorke.light2d.animation.out_of_range");
    assert!(store.update(a, &bad).is_err());
    assert_eq!(store.get(a).unwrap().kind, AnimationType::Flash);
    store.update(a, &make(AnimationType::Beacon)).unwrap();
    assert_eq!(store.get(a).unwrap().kind, AnimationType::Beacon);
    assert!(store.find(a).is_some());
    assert_eq!(store.len(), 1);
    store.destroy(a, &lights).unwrap();
    let e = store.destroy(a, &lights).unwrap_err();
    assert!(e.to_string().contains("invalid or destroyed animation"));
    assert_eq!(e.code().as_str(), "knorke.light2d.animation.invalid_handle");
    assert!(store.get(a).is_err());
    assert!(
        store
            .update(a, &make(AnimationType::Pulse))
            .unwrap_err()
            .to_string()
            .contains("invalid or destroyed")
    );
    assert!(store.is_empty());
}

#[test]
fn store_samples_the_identity_without_an_animation_and_seeds_fire_by_the_light_slot() {
    let mut store = AnimationStore::new();
    let fire = store.create(&make(AnimationType::Fire)).unwrap();
    let none = store.sample(None, 1.0, 0.0, 7);
    assert_eq!((none.intensity, none.rotation), (1.0, 0.0));
    let a = store.sample(Some(fire), 1.234, 0.0, 1);
    let direct = make(AnimationType::Fire).sample(1.234, 0.0, 1);
    assert_eq!(a, direct);
    // The seed of a light is the slot index of its handle, not the whole handle.
    let masks = MaskStore::new();
    let mut lights = LightStore::new();
    let mut d = LightDesc::new(LightType::Point);
    d.animation = Some(fire);
    let id = lights
        .create(&d, &StoreResources::new(&store, &masks))
        .unwrap();
    assert_eq!(LightRef::Persistent(id).seed_index(), id.index());
}

#[test]
fn lights_accept_only_live_animations_of_their_store_and_a_finite_offset() {
    let masks = MaskStore::new();
    let mut animations = AnimationStore::new();
    let a = animations.create(&make(AnimationType::Pulse)).unwrap();
    let mut lights = LightStore::new();
    let animated = |kind: LightType, id| {
        let mut d = LightDesc::new(kind);
        d.animation = Some(id);
        d
    };
    // Without the store the handle is unknown.
    let empty = AnimationStore::new();
    assert!(
        lights
            .create(
                &animated(LightType::Point, a),
                &StoreResources::new(&empty, &masks)
            )
            .is_err()
    );
    let res = StoreResources::new(&animations, &masks);
    let point = lights.create(&animated(LightType::Point, a), &res).unwrap();
    lights
        .create(&animated(LightType::Directional, a), &res)
        .unwrap();
    assert_eq!(lights.animation_users(a), 2);
    // A destroyed animation is stale.
    let mut other = AnimationStore::new();
    let gone = other.create(&make(AnimationType::Pulse)).unwrap();
    other.destroy(gone, &LightStore::new()).unwrap();
    let e = lights
        .create(
            &animated(LightType::Spot, gone),
            &StoreResources::new(&other, &masks),
        )
        .unwrap_err();
    assert!(
        e.to_string()
            .contains("invalid or destroyed animation handle")
    );
    let mut offset = animated(LightType::Point, a);
    offset.animation_offset = NAN;
    let e = lights.create(&offset, &res).unwrap_err();
    assert!(e.to_string().contains("animation_offset"));
    let res_other = StoreResources::new(&other, &masks);
    assert!(
        lights
            .update(point, &animated(LightType::Point, a), &res_other)
            .is_err()
    );
    assert_eq!(lights.get(point).unwrap().animation, Some(a));
    let defaults = LightDesc::new(LightType::Spot);
    assert_eq!((defaults.animation, defaults.animation_offset), (None, 0.0));
}

#[test]
fn an_animation_in_use_cannot_be_destroyed() {
    let masks = MaskStore::new();
    let mut animations = AnimationStore::new();
    let a = animations.create(&make(AnimationType::Beacon)).unwrap();
    let mut lights = LightStore::new();
    let mut d = LightDesc::new(LightType::Spot);
    d.animation = Some(a);
    let id = lights
        .create(&d, &StoreResources::new(&animations, &masks))
        .unwrap();
    let e = animations.destroy(a, &lights).unwrap_err();
    assert!(e.to_string().contains("1 light"));
    assert_eq!(e.code().as_str(), "knorke.light2d.animation.in_use");
    assert!(animations.find(a).is_some());
    lights.destroy(id).unwrap();
    animations.destroy(a, &lights).unwrap();
}

#[test]
fn snapshot_round_trips_handles_and_descriptions() {
    let lights = LightStore::new();
    let mut store = AnimationStore::new();
    let a = store.create(&make(AnimationType::Fire)).unwrap();
    let b = store.create(&make(AnimationType::Envelope)).unwrap();
    store.destroy(a, &lights).unwrap();
    let c = store.create(&make(AnimationType::Flash)).unwrap(); // reuses a's slot, new generation
    let mut buf = SnapshotBuf::new();
    buf.save(&store);

    let mut other = AnimationStore::new();
    other.create(&make(AnimationType::Pulse)).unwrap();
    restore(&mut Reader::new(buf.bytes()), &mut other).unwrap();
    assert_eq!(other.len(), 2);
    assert_eq!(other.get(c).unwrap().kind, AnimationType::Flash);
    assert_eq!(other.get(b).unwrap().kind, AnimationType::Envelope);
    assert!(other.get(a).is_err(), "the old generation stays dead");
    let mut again = SnapshotBuf::new();
    again.save(&other);
    assert_eq!(again.bytes(), buf.bytes());
}

/// Writes a section of the store's id with arbitrary descriptions, to test `restore` against
/// state the store itself would never save.
struct Raw(Slots<AnimationDesc>);

impl Snapshot for Raw {
    const ID: &'static str = AnimationStore::ID;
    const VERSION: u16 = AnimationStore::VERSION;
    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.0);
    }
    fn restore(&mut self, _: u16, _: &mut Reader<'_>) -> Result<(), SnapshotError> {
        unreachable!("write only")
    }
}

#[test]
fn restore_rejects_a_description_that_validation_would_reject() {
    let mut slots = Slots::new();
    let mut bad = make(AnimationType::Pulse);
    bad.period = 0.0;
    slots.insert(bad);
    let mut buf = SnapshotBuf::new();
    buf.save(&Raw(slots));
    let mut target = AnimationStore::new();
    let err = restore(&mut Reader::new(buf.bytes()), &mut target).unwrap_err();
    assert!(matches!(err, SnapshotError::Invalid(_)), "{err:?}");

    // The valid twin restores.
    let mut slots = Slots::new();
    slots.insert(make(AnimationType::Pulse));
    let mut buf = SnapshotBuf::new();
    buf.save(&Raw(slots));
    restore(&mut Reader::new(buf.bytes()), &mut target).unwrap();
    assert_eq!(target.len(), 1);
}
