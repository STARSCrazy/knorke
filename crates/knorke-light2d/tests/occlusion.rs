//! P23 without a device: the uniform block of the rectangle shaders and the shift copy of a
//! camera pan (chiaro @ 9bc5b9b, src/rendering/OcclusionMask.cpp, zlib license).

use knorke_light2d::occlusion::{RectParams, ShiftCopy};
use knorke_light2d::shadow::{DirtyRect, DirtyRectList};

fn f32_at(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn i32_at(bytes: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
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
