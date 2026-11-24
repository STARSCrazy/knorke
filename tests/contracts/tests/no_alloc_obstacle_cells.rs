//! Contract: editing the cells of a tile grid and the pattern assignments of obstacles allocate
//! nothing once the grid has its channels and the working buffers have grown: cell edits, level
//! edits, compared grid replacement, pattern changes, the destroy notification, the mask value,
//! reading the cell log, saving and restoring the store (P14, ADR 0006, ADR 0010).
//!
//! Own test binary with a single test, so the process-wide allocation counter is not disturbed
//! by other tests running concurrently.

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_light2d::ObstacleStore;
use knorke_light2d::obstacle::{CellRect, TileGridDesc};
use knorke_types::{Reader, SnapshotBuf, restore};

#[global_allocator]
static GLOBAL: Counting = Counting;

const COLS: usize = 40;
const ROWS: usize = 20;

#[test]
fn steady_state_cell_and_pattern_edits_allocate_nothing() {
    let n = COLS * ROWS;
    let solid = vec![2u8; n];
    let base = vec![1u8; n];
    let opacity = vec![200u8; n];
    let edges = vec![0u8; n];
    let mut next_solid = solid.clone();
    let mut desc = TileGridDesc::new(COLS as u32, ROWS as u32, &solid);
    desc.level_height = 10.0;
    desc.edge_width = 0.5;
    desc.base_levels = Some(&base);
    desc.opacity = Some(&opacity);
    desc.edges = Some(&edges);

    let live = |id: u32| if id != 0 { Some(0.5f32) } else { None };
    let mut store = ObstacleStore::new();
    let grid = store.create_tile_grid(&desc).unwrap();
    let other = store.create_tile_grid(&desc).unwrap();
    let table = [1u32, 2, 3];
    let mut snap = SnapshotBuf::new();

    // Warm-up: creates the pattern cell channel, grows the scratch buffers and the snapshot.
    let patch = [3u8; 6];
    let edge_patch = [1u8; 6];
    store.set_tile_pattern_table(grid, &table, &live).unwrap();
    store
        .set_tile_pattern_cells(grid, CellRect::new(0, 0, 1, 1), &[1])
        .unwrap();
    next_solid[7] = 5;
    let mut changed = desc;
    changed.solid = &next_solid;
    store.set_tile_grid_compare(grid, &changed).unwrap();
    store.set_tile_grid_compare(grid, &desc).unwrap();
    store
        .set_patterns(&[grid, other][..], &[1u32, 1][..], &live)
        .unwrap();
    store.on_patterns_destroyed(&[9]);
    snap.clear();
    snap.save(&store);
    let mut target = ObstacleStore::new();
    restore(&mut Reader::new(snap.bytes()), &mut target).unwrap();

    let before = allocations();
    let mut sum = 0.0f32;
    for t in 0..200u32 {
        let x = t % 30;
        store
            .set_tile_cells(
                grid,
                CellRect::new(x, 3, 3, 2),
                Some(&patch),
                None,
                Some(&edge_patch),
            )
            .unwrap();
        store
            .set_tile_cell_levels(
                grid,
                CellRect::new(x, 8, 3, 2),
                Some(&patch),
                Some(&[0u8; 6]),
            )
            .unwrap();
        // A compared replacement of a mostly unchanged grid, then back.
        store.set_tile_grid_compare(grid, &changed).unwrap();
        store.set_tile_grid_compare(grid, &desc).unwrap();
        store.set_pattern(grid, 1 + t % 3, &live).unwrap();
        store.set_tile_pattern_table(grid, &table, &live).unwrap();
        store
            .set_tile_pattern_cells(grid, CellRect::new(x, 10, 3, 2), &[1, 2, 3, 0, 1, 2])
            .unwrap();
        store
            .set_patterns(&[grid, other][..], &[1 + t % 3, 2][..], &live)
            .unwrap();
        store.on_patterns_destroyed(&[9]); // names nothing: the scan only
        for cell in [0usize, 7, 45, 600] {
            sum += store.mask_opacity(grid, cell, &live).unwrap();
        }
        sum += f32::from(u8::from(store.any_pattern()));
        sum += store.count_destroyed_pattern_users(&live, true) as f32;
        let mut seen = 0u64;
        store
            .cell_log()
            .for_each(store.cell_version() - 3, |_| seen += 1);
        assert_eq!(seen, 3);
        snap.clear();
        snap.save(&store);
        // Restoring a store with grids, patterns and a cell log allocates nothing either.
        restore(&mut Reader::new(snap.bytes()), &mut target).unwrap();
    }
    let allocated = allocations() - before;
    assert!(sum > 0.0);
    assert_eq!(
        allocated, 0,
        "steady-state cell and pattern edits allocated"
    );
}
