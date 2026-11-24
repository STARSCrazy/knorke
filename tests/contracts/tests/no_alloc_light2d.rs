//! Contract: the steady-state paths of `knorke-light2d` allocate nothing once their buffers have
//! grown: transient frame lights, snapshot save and restore of the light store, triangulation
//! into a warm buffer, moving and re-describing obstacles with their dirty tracking, snapshot save
//! of the obstacle store, and the pixel and normal-map codecs (ADR 0006, ADR 0010).
//!
//! Own test binary with a single test, so the process-wide allocation counter is not disturbed
//! by other tests running concurrently.

#![allow(unsafe_code)] // counting allocator

use knorke_light2d::codec::{
    Material, MaterialPreset, NormalMapOptions, convert_normal_map, encode_normal, material_pixel,
    surface_pixel,
};
use knorke_light2d::light::{Height, LightDesc, LightType};
use knorke_light2d::obstacle::{ShapeDesc, TileGridDesc};
use knorke_light2d::{LightStore, NoResources, ObstacleStore, Triangulator};
use knorke_math::{At, Vec2, Vec3, World2};
use knorke_types::{Reader, SnapshotBuf, restore};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

struct Counting;
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
// SAFETY: forwards to the system allocator unchanged; only counts calls.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn steady_state_light2d_paths_allocate_nothing() {
    // Persistent lights, a snapshot buffer, frame lights, a polygon and a small normal map.
    let mut store = LightStore::new();
    let mut ids = Vec::new();
    for i in 0..32 {
        let mut d = LightDesc::new(LightType::Spot);
        d.position = At::new(Vec2::new(i as f32, 0.0));
        ids.push(store.create(&d, &NoResources).unwrap());
    }
    let frame: Vec<LightDesc> = (0..64).map(|_| LightDesc::new(LightType::Point)).collect();
    let mut snap = SnapshotBuf::new();
    let poly = [
        Vec2::new(0.0, 0.0),
        Vec2::new(4.0, 0.0),
        Vec2::new(4.0, 1.0),
        Vec2::new(1.0, 1.0),
        Vec2::new(1.0, 4.0),
        Vec2::new(0.0, 4.0),
    ];
    let mut tri = Triangulator::new();
    let mut triangles = Vec::new();
    let normal_map = vec![200u8; 16 * 16 * 4];
    let mut material_map = vec![0u8; 16 * 16 * 4];

    // Obstacles: polygons, a box, a circle and a tile grid; a warm world-triangle buffer.
    let mut obstacles = ObstacleStore::new();
    let mut moving = Vec::new();
    for i in 0u32..8 {
        let mut d = ShapeDesc::polygon(&poly[..]);
        d.common.dynamic = i.is_multiple_of(2);
        moving.push(obstacles.create_shape(&d).unwrap());
    }
    moving.push(obstacles.create_shape(&ShapeDesc::rect(2.0, 1.0)).unwrap());
    moving.push(obstacles.create_shape(&ShapeDesc::circle(1.5)).unwrap());
    let cells = [1u8, 2, 0, 3];
    let grid = obstacles
        .create_tile_grid(&TileGridDesc::new(2, 2, &cells))
        .unwrap();
    let mut obstacle_snap = SnapshotBuf::new();
    let mut world_triangles = Vec::new();

    let mut step = |i: u32| {
        // Obstacles: move, re-describe in place (same sizes), consume the dirty sets, save.
        for (k, &id) in moving.iter().enumerate() {
            let p = At::<World2, Vec2>::new(Vec2::new(k as f32, i as f32));
            obstacles.set_transform(id, p, i as f32).unwrap();
        }
        let mut d = ShapeDesc::polygon(&poly[..]);
        d.common.opacity = if i.is_multiple_of(2) { 0.5 } else { 1.0 };
        obstacles.set_shape(moving[0], &d).unwrap();
        obstacles
            .set_shape(moving[8], &ShapeDesc::rect(2.0, 1.0))
            .unwrap();
        obstacles
            .set_shape(moving[9], &ShapeDesc::circle(1.5))
            .unwrap();
        obstacles
            .set_tile_grid(grid, &TileGridDesc::new(2, 2, &cells))
            .unwrap();
        let _ = (
            obstacles.any_partial(),
            obstacles.any_finite_height(),
            obstacles.max_finite_top(),
            obstacles.dirty_static().count() + obstacles.dirty_dynamic().count(),
        );
        world_triangles.clear();
        obstacles
            .get(moving[0])
            .unwrap()
            .append_world_triangles(&mut world_triangles);
        obstacles.clear_dirty();
        obstacle_snap.clear();
        obstacle_snap.save(&obstacles);

        // Frame lights: validated and derived into the pool.
        store.set_frame_lights(&frame[..], &NoResources).unwrap();
        store.clear_frame_lights();
        // Moving lights and updating one in place.
        for (k, &id) in ids.iter().enumerate() {
            let p = At::new(Vec2::new(k as f32, i as f32));
            store.set_position(id, p, Height::At(1.0)).unwrap();
        }
        // Snapshot round trip into reused buffers.
        snap.clear();
        snap.save(&store);
        restore(&mut Reader::new(snap.bytes()), &mut store).unwrap();
        // Geometry and codecs.
        tri.triangulate(&poly[..], &mut triangles).unwrap();
        convert_normal_map(
            &normal_map,
            16,
            16,
            64,
            &NormalMapOptions::default(),
            &mut material_map,
        )
        .unwrap();
        let n = Some(Vec3::new(0.2, 0.3, 0.9));
        let _ = (
            encode_normal(n),
            surface_pixel(3.0, 0.5, Vec3::new(0.0, 0.0, 1.0)),
            material_pixel(Material::preset(MaterialPreset::Glass), n),
        );
    };
    // Warm-up: the pool, the snapshot buffer, the slots and the triangulator grow.
    for i in 0..4 {
        step(i);
    }
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    for i in 4..40 {
        step(i);
    }
    let allocated = ALLOCATIONS.load(Ordering::Relaxed) - before;
    assert_eq!(allocated, 0, "allocations in 36 steady-state steps");
}
