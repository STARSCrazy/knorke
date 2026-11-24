//! P23 without a device: the jump flooding schedule, the uniform blocks of the field and rectangle
//! shaders and the shift copy of a camera pan (chiaro @ 9bc5b9b, src/rendering/SdfShadows.* and
//! OcclusionMask.cpp, zlib license).

use knorke_light2d::occlusion::{
    FIELD_BOUND, JfaParams, RectParams, ShiftCopy, UPDATE_LIMITS, jfa_span, jfa_steps,
};
use knorke_light2d::shadow::{DirtyRect, DirtyRectList, FULL_REBUILD_RATIO};

fn f32_at(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn i32_at(bytes: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

#[test]
fn jump_flooding_halves_from_the_span_and_repeats_the_last_step() {
    assert_eq!(FIELD_BOUND, 32);
    assert_eq!(jfa_span(), 32);
    let steps: Vec<u32> = jfa_steps().collect();
    assert_eq!(steps, [16, 8, 4, 2, 1, 1]);
    // A texel reads seeds at most the sum of the steps away: within the field bound.
    assert_eq!(steps.iter().sum::<u32>(), FIELD_BOUND);
}

#[test]
fn the_update_limits_follow_the_field_bound() {
    assert_eq!(UPDATE_LIMITS.field_grow, 32);
    assert_eq!(UPDATE_LIMITS.seed_grow, 34);
    assert_eq!(UPDATE_LIMITS.full_rebuild_ratio, FULL_REBUILD_RATIO);
}

#[test]
fn jfa_params_match_the_cbuffer_layout() {
    let p = JfaParams::new(192, 128);
    let b = p.with_step(8.0).to_bytes();
    assert_eq!(b.len(), 32);
    assert_eq!(
        [f32_at(&b, 0), f32_at(&b, 4), f32_at(&b, 8), f32_at(&b, 12)],
        [192.0, 128.0, 1.0 / 192.0, 1.0 / 128.0]
    );
    assert_eq!([f32_at(&b, 16), f32_at(&b, 20)], [8.0, 0.0]);
    let d = p.distance().to_bytes();
    assert_eq!([f32_at(&d, 16), f32_at(&d, 20)], [0.0, 32.0]);
}

#[test]
fn rect_params_hold_the_rectangles_and_the_target_size() {
    let mut list = DirtyRectList::new();
    assert!(list.add(DirtyRect::new(1, 2, 3, 4)));
    assert!(list.add(DirtyRect::new(50, 60, 70, 80)));
    let mut p = RectParams::new();
    let n = p.write(&list, 192, 128);
    assert_eq!(n, 2);
    let b = p.bytes();
    assert_eq!(b.len(), 64 * 16 + 16);
    let rect = |i: usize| [0, 1, 2, 3].map(|c| i32_at(b, i * 16 + c * 4));
    assert_eq!(rect(0), [1, 2, 3, 4]);
    assert_eq!(rect(1), [50, 60, 70, 80]);
    assert_eq!([f32_at(b, 1024), f32_at(b, 1028)], [192.0, 128.0]);
}

#[test]
fn a_pan_copies_the_overlap_of_old_and_new_mask() {
    // new[x] = old[x - shift]: a positive shift moves the contents right/down.
    let c = ShiftCopy::new([3, -2], 192, 128).unwrap();
    assert_eq!((c.src, c.dst, c.size), ([0, 2], [3, 0], [189, 126]));
    let c = ShiftCopy::new([-5, 7], 192, 128).unwrap();
    assert_eq!((c.src, c.dst, c.size), ([5, 0], [0, 7], [187, 121]));
    assert!(ShiftCopy::new([0, 0], 192, 128).is_none());
    // Nothing overlaps: no copy (the plan rebuilds fully long before).
    assert!(ShiftCopy::new([192, 0], 192, 128).is_none());
}
