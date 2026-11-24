//! Contract: the GPU-free half of the occlusion stage (P23) allocates nothing per frame: the
//! jump flooding schedule, the field and rectangle uniform blocks (written into a kept block)
//! and the shift copy of a pan (ADR 0006). The GPU stage records with exactly these values.
//!
//! Own test binary with a single test, so the process-wide allocation counter is not disturbed
//! by other tests running concurrently.

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_light2d::occlusion::{JfaParams, RectParams, ShiftCopy, jfa_steps};
use knorke_light2d::shadow::{DirtyRect, DirtyRectList};

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn occlusion_frame_values_allocate_nothing() {
    let mut rects = DirtyRectList::new();
    for i in 0..DirtyRectList::MAX_DIRTY_RECTS as i32 {
        rects.add(DirtyRect::new(i * 3, 0, i * 3 + 2, 2));
    }
    let mut block = RectParams::new();
    let before = allocations();
    let mut sum = 0u64;
    for frame in 0..100u32 {
        sum += u64::from(block.write(&rects, 192, 128));
        let params = JfaParams::new(192, 128);
        for k in jfa_steps() {
            sum += u64::from(params.with_step(k as f32).to_bytes()[16]);
        }
        sum += u64::from(params.distance().to_bytes()[20]);
        let shift = [frame as i32 % 7 - 3, 2];
        if let Some(c) = ShiftCopy::new(shift, 192, 128) {
            sum += u64::from(c.size[0]);
        }
        sum += u64::from(block.bytes()[1024]);
    }
    let after = allocations();
    assert!(sum > 0);
    assert_eq!(after - before, 0, "the occlusion frame values allocated");
}
