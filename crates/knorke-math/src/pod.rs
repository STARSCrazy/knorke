//! `bytemuck` marker impls, the only `unsafe` in this crate.
//!
//! Written by hand instead of `#[derive(Pod)]` to avoid the proc-macro dependency tree
//! (syn, quote, proc-macro2) in every build. Everything a `Pod` impl relies on is checked
//! at compile time from the field list given here, not from numbers kept in sync by hand:
//!
//! - the list names **every** field (exhaustive destructuring – adding a field to a struct
//!   without listing it here fails the build),
//! - every field has the stated type and that type is itself `Pod`,
//! - the struct is exactly as large as its fields together, i.e. has **no padding**, and
//! - size and alignment equal the documented layout (also asserted in
//!   `tests/contracts/tests/layout.rs`).
//!
//! `#[repr(C)]` itself cannot be checked by a macro; it is covered by the `offset_of!`
//! assertions below, which pin every field to the offset `repr(C)` gives it.

#![allow(unsafe_code)]

use crate::{
    Aabb2, Affine2, At, Capsule2, Circle, IVec2, IVec3, LinearRgba, Mat4, Ray2, Segment2, Space,
    Srgba8, Transform2, Vec2, Vec3, Vec4,
};
use bytemuck::{Pod, Zeroable};
use core::mem::{align_of, offset_of, size_of};

const fn assert_pod<T: Pod>() {}

macro_rules! pod {
    ($($t:ident { $($f:ident: $ft:ty),+ $(,)? } => $size:expr, $align:expr);+ $(;)?) => {$(
        const _: () = {
            // Documented layout.
            assert!(size_of::<$t>() == $size && align_of::<$t>() == $align);
            // No padding: the fields fill the struct completely ...
            assert!(size_of::<$t>() == 0 $(+ size_of::<$ft>())+);
            // ... in declaration order without gaps (what `repr(C)` guarantees).
            let mut expected = 0;
            $(
                assert!(offset_of!($t, $f) == expected);
                expected += size_of::<$ft>();
                assert_pod::<$ft>();
            )+
            assert!(expected == size_of::<$t>());
        };
        // Fails to compile if a field is missing from the list or has another type.
        const _: fn($t) = |v| {
            let $t { $($f),+ } = v;
            $(let _: $ft = $f;)+
        };
        // SAFETY: `#[repr(C)]`; the assertions above prove that the listed fields are all
        // fields, that each is `Pod` (so any bit pattern including all-zero is valid, and
        // it is `Copy + 'static`), and that there are no padding bytes between or after
        // them.
        unsafe impl Zeroable for $t {}
        // SAFETY: see above.
        unsafe impl Pod for $t {}
    )+};
}

pod! {
    Vec2 { x: f32, y: f32 } => 8, 4;
    Vec3 { x: f32, y: f32, z: f32 } => 12, 4;
    Vec4 { x: f32, y: f32, z: f32, w: f32 } => 16, 4;
    IVec2 { x: i32, y: i32 } => 8, 4;
    IVec3 { x: i32, y: i32, z: i32 } => 12, 4;
    Affine2 { x_axis: Vec2, y_axis: Vec2, translation: Vec2 } => 24, 4;
    Mat4 { cols: [Vec4; 4] } => 64, 4;
    LinearRgba { r: f32, g: f32, b: f32, a: f32 } => 16, 4;
    Srgba8 { r: u8, g: u8, b: u8, a: u8 } => 4, 1;
    Aabb2 { min: Vec2, max: Vec2 } => 16, 4;
    Circle { center: Vec2, radius: f32 } => 12, 4;
    Segment2 { a: Vec2, b: Vec2 } => 16, 4;
    Capsule2 { segment: Segment2, radius: f32 } => 20, 4;
    Ray2 { origin: Vec2, dir: Vec2 } => 16, 4;
}

// The generic wrappers cannot be checked per instantiation by the macro; these assertions
// pin the layout facts the impls below rely on for representative instantiations.
const _: () = {
    assert!(size_of::<At<crate::World2, Vec2>>() == size_of::<Vec2>());
    assert!(align_of::<At<crate::World2, Vec2>>() == align_of::<Vec2>());
    assert!(size_of::<At<crate::Tile, IVec2>>() == size_of::<IVec2>());
    assert!(size_of::<At<crate::Screen, u8>>() == 1);
    assert!(size_of::<Transform2<crate::World2, crate::Screen>>() == size_of::<Affine2>());
    assert!(align_of::<Transform2<crate::World2, crate::Screen>>() == align_of::<Affine2>());
};

// SAFETY: `At` is `#[repr(transparent)]` over its only non-zero-sized field `value: V`; the
// other field is `PhantomData<S>`, a 1-aligned zero-sized type for every `S`. Layout and
// valid bit patterns are therefore exactly `V`'s, and `V: Zeroable` carries over. `At` has
// no invariant of its own (`value` is public).
unsafe impl<S: Space, V: Zeroable> Zeroable for At<S, V> {}
// SAFETY: as above with `V: Pod`. `Pod` additionally needs `Copy + 'static`: `At<S, V>` is
// `Copy` for `V: Copy` (manual impl in `space.rs`), `V: Pod` implies `V: Copy + 'static`,
// and `S: Space` implies `S: 'static`.
unsafe impl<S: Space, V: Pod> Pod for At<S, V> {}

// SAFETY: `#[repr(transparent)]` over `affine: Affine2` (`Pod`, see above) plus
// `PhantomData<(A, B)>`, zero-sized and 1-aligned for all `A`, `B`.
unsafe impl<A: Space, B: Space> Zeroable for Transform2<A, B> {}
// SAFETY: as above; `Transform2` is `Copy` for all `A`, `B` (manual impl in `space.rs`) and
// `'static` because `Space: 'static`.
unsafe impl<A: Space, B: Space> Pod for Transform2<A, B> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Screen, World2};

    #[test]
    fn bulk_views_without_copy() {
        let pts = [Vec2::new(1.0, 2.0), Vec2::new(3.0, 4.0)];
        let floats: &[f32] = bytemuck::cast_slice(&pts);
        assert_eq!(floats, &[1.0, 2.0, 3.0, 4.0]);
        assert!(core::ptr::eq(floats.as_ptr(), &pts[0].x));
        let bytes: &[u8] = bytemuck::cast_slice(&pts);
        assert_eq!(bytes.len(), 16);
        let arrays: &[[f32; 2]] = bytemuck::cast_slice(&pts);
        assert_eq!(arrays[1], [3.0, 4.0]);
    }

    #[test]
    fn srgba8_is_four_bytes_in_rgba_order() {
        // `Srgba8` has alignment 1: viewing a slice of it as `&[u32]` is only valid at
        // 4-aligned addresses (`cast_slice` panics otherwise). Read unaligned or view the
        // other way round.
        let colors = [Srgba8::new(1, 2, 3, 4), Srgba8::new(5, 6, 7, 8)];
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&colors),
            &[1, 2, 3, 4, 5, 6, 7, 8]
        );
        let packed: u32 = bytemuck::pod_read_unaligned(bytemuck::bytes_of(&colors[1]));
        assert_eq!(packed.to_le_bytes(), [5, 6, 7, 8]);
        let words = [u32::from_le_bytes([9, 10, 11, 12])];
        assert_eq!(
            bytemuck::cast_slice::<u32, Srgba8>(&words),
            &[Srgba8::new(9, 10, 11, 12)]
        );
        // At an odd address the cast is refused instead of producing a misaligned slice.
        // Of two neighbouring addresses at most one is 4-aligned.
        let bytes = [0u8; 9];
        assert!(
            bytemuck::try_cast_slice::<Srgba8, u32>(bytemuck::cast_slice(&bytes[1..5])).is_err()
                || bytemuck::try_cast_slice::<Srgba8, u32>(bytemuck::cast_slice(&bytes[2..6]))
                    .is_err()
        );
    }

    #[test]
    fn generic_wrappers_cast_like_their_content() {
        let typed = [At::<World2, Vec2>::new(Vec2::new(1.0, 2.0))];
        assert_eq!(bytemuck::cast_slice::<_, f32>(&typed), &[1.0, 2.0]);
        let t: Transform2<World2, Screen> =
            Transform2::new(Affine2::from_translation(Vec2::new(5.0, 6.0)));
        assert_eq!(
            bytemuck::cast::<_, [f32; 6]>(t),
            [1.0, 0.0, 0.0, 1.0, 5.0, 6.0]
        );
        let zero: At<World2, IVec2> = Zeroable::zeroed();
        assert_eq!(zero.raw(), IVec2::ZERO);
    }
}
