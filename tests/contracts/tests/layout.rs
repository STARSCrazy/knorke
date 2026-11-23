//! Contract: types crossing module, GPU and FFI boundaries keep their documented layout.
//! A change here is a breaking change of the data contracts (ADR 0006).

use knorke_math::{
    Affine2, At, IVec2, IVec3, LinearRgba, Mat4, Screen, Srgba8, Tile, Vec2, Vec3, Vec4, World2,
};
use knorke_types::{Handle, Tick};
use std::mem::{align_of, offset_of, size_of};

#[test]
fn math_layouts() {
    assert_eq!((size_of::<Vec2>(), align_of::<Vec2>()), (8, 4));
    assert_eq!((size_of::<Vec3>(), align_of::<Vec3>()), (12, 4));
    assert_eq!((size_of::<Vec4>(), align_of::<Vec4>()), (16, 4));
    assert_eq!((size_of::<IVec2>(), size_of::<IVec3>()), (8, 12));
    assert_eq!(
        (
            offset_of!(Vec3, x),
            offset_of!(Vec3, y),
            offset_of!(Vec3, z)
        ),
        (0, 4, 8)
    );
    assert_eq!(size_of::<Affine2>(), 24);
    assert_eq!(offset_of!(Affine2, translation), 16);
    assert_eq!(size_of::<Mat4>(), 64);
    assert_eq!((size_of::<LinearRgba>(), size_of::<Srgba8>()), (16, 4));
    assert_eq!(offset_of!(Srgba8, a), 3);
}

#[test]
fn space_tags_are_free() {
    assert_eq!(size_of::<At<World2, Vec2>>(), size_of::<Vec2>());
    assert_eq!(size_of::<At<Screen, Vec2>>(), size_of::<Vec2>());
    assert_eq!(size_of::<At<Tile, IVec2>>(), size_of::<IVec2>());
    assert_eq!(align_of::<At<World2, Vec3>>(), align_of::<Vec3>());
}

#[test]
fn handles_and_ticks() {
    struct X;
    assert_eq!(size_of::<Handle<X>>(), 8);
    assert_eq!(size_of::<Option<Handle<X>>>(), 8);
    assert_eq!(size_of::<Tick>(), 8);
}

#[test]
fn geometry_layouts() {
    use knorke_math::{Aabb2, Capsule2, Circle, Ray2, Segment2};
    assert_eq!(size_of::<Aabb2>(), 16);
    assert_eq!(offset_of!(Aabb2, max), 8);
    assert_eq!(size_of::<Circle>(), 12);
    assert_eq!(offset_of!(Circle, radius), 8);
    assert_eq!(size_of::<Segment2>(), 16);
    assert_eq!(
        (size_of::<Capsule2>(), offset_of!(Capsule2, radius)),
        (20, 16)
    );
    assert_eq!(size_of::<Ray2>(), 16);
    let boxes = [Aabb2::from_corners(Vec2::ZERO, Vec2::ONE)];
    assert_eq!(
        knorke_math::bytemuck::cast_slice::<_, f32>(&boxes),
        &[0.0, 0.0, 1.0, 1.0]
    );
}

#[test]
fn switched_off_features_are_zero_sized() {
    // "Off" variants carry no data: after inlining nothing of them remains (ADR 0011).
    assert_eq!(size_of::<knorke_observe::NoProbe>(), 0);
    assert_eq!(size_of::<knorke_observe::NoopSink>(), 0);
    assert_eq!(size_of::<knorke_replay::NoRecord>(), 0);
    assert_eq!(size_of::<knorke_types::Serial>(), 0);
}
