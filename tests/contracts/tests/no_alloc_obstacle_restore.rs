//! Contract (P13b): restoring the obstacle store from a snapshot allocates nothing once its
//! buffers have grown, although the obstacles of the snapshot differ in shape and size from
//! those in the store, and the arenas are compacted along the way (ADR 0006, ADR 0010: restore
//! without allocation).
//!
//! The cell channels of tile grids are still owned per obstacle (P14), so the scene has shapes
//! only. Own test binary with a single test, so the process-wide allocation counter is not
//! disturbed by other tests.

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_light2d::ObstacleStore;
use knorke_light2d::obstacle::ShapeDesc;
use knorke_math::Vec2;
use knorke_types::{Reader, SnapshotBuf, restore};

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn restoring_the_obstacle_store_allocates_nothing_in_steady_state() {
    let l_shape = [
        Vec2::new(0.0, 0.0),
        Vec2::new(4.0, 0.0),
        Vec2::new(4.0, 1.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(1.0, 4.0),
        Vec2::new(0.0, 4.0),
    ];
    let mut store = ObstacleStore::new();
    let mut ids = Vec::new();
    for i in 0..24 {
        let d = match i % 3 {
            0 => ShapeDesc::polygon(&l_shape[..]),
            1 => ShapeDesc::rect(2.0, 1.0),
            _ => ShapeDesc::circle(1.5),
        };
        ids.push(store.create_shape(&d).unwrap());
    }
    // The store the snapshots are restored into: same handles, other geometry.
    let mut target = store.clone();
    let mut snap = SnapshotBuf::new();

    let mut step = |i: usize| {
        // Churn in `store`: sizes change, so the arenas fill with garbage and compact.
        for (k, &id) in ids.iter().enumerate() {
            let d = match (i + k) % 3 {
                0 => ShapeDesc::polygon(&l_shape[..]),
                1 => ShapeDesc::rect(2.0, 1.0),
                _ => ShapeDesc::circle(1.5),
            };
            store.set_shape(id, &d).unwrap();
        }
        snap.clear();
        snap.save(&store);
        restore(&mut Reader::new(snap.bytes()), &mut target).unwrap();
    };
    // Warm-up: arenas, spare buffers, scratch and the snapshot buffer grow (several compactions).
    for i in 0..60 {
        step(i);
    }
    let before = allocations();
    for i in 60..260 {
        step(i);
    }
    let allocated = allocations() - before;
    assert_eq!(allocated, 0, "allocations in 200 steady-state restores");

    // The restored store is the saved one.
    for &id in &ids {
        assert_eq!(target.get(id), store.get(id));
        assert_eq!(target.triangles(id), store.triangles(id));
        assert_eq!(target.outline(id), store.outline(id));
    }
}
