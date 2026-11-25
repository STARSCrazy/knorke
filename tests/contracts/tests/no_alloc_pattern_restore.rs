//! Contract (P15): restoring the pattern store from a snapshot allocates nothing once its
//! buffers have grown, although the snapshots differ in which patterns live and where the
//! packer put them (ADR 0006, ADR 0010: restore without allocation). Sampling allocates nothing
//! either. Own test binary with a single test, so the process-wide allocation counter is not
//! disturbed by other tests.

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_light2d::PatternStore;
use knorke_light2d::pattern_store::{PatternDesc, Sheet, SheetFormat};
use knorke_types::{Reader, SnapshotBuf, restore};

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn restoring_the_pattern_store_allocates_nothing_in_steady_state() {
    let px: Vec<u8> = (0..64 * 64 * 4).map(|i| (i * 37 % 251) as u8).collect();
    let sheet = Sheet {
        width: 64,
        height: 64,
        pitch: 256,
        format: SheetFormat::Rgba8,
        pixels: &px,
    };
    let descs: Vec<_> = (0..16u32)
        .map(|i| PatternDesc::new(i, i, 8 + i * 3, 4 + i * 2))
        .collect();
    // Snapshots of different states: churn between them.
    let mut store = PatternStore::new();
    let mut ids = Vec::new();
    let mut snaps = Vec::new();
    for round in 0..4usize {
        store
            .create(&sheet, &descs[round..round + 8], &mut ids)
            .unwrap();
        store.destroy(&[ids.remove(round)]).unwrap();
        let mut snap = SnapshotBuf::new();
        snap.save(&store);
        snaps.push(snap);
    }
    let mut target = PatternStore::new();
    // Warm-up: the arena, the slots and the shelves grow to the largest state.
    for snap in &snaps {
        restore(&mut Reader::new(snap.bytes()), &mut target).unwrap();
    }
    let before = allocations();
    let mut sum = 0.0;
    for k in 0..200 {
        restore(&mut Reader::new(snaps[k % 4].bytes()), &mut target).unwrap();
        for (_, p) in target.iter() {
            sum += p.sample(k as f32 * 0.37, 0.5, true);
        }
    }
    let allocated = allocations() - before;
    assert_eq!(allocated, 0, "allocations in 200 steady-state restores");
    assert!(sum > 0.0);
}
