//! Origin: chiaro @ 9bc5b9b, tests/unit/ObstacleStoreTests.cpp (zlib license, see
//! `src/obstacle/`): the "pattern assignment" cases of P14. The pattern store is P15; the tests
//! use a small `Patterns` table with the same two questions (live? mean opacity?), plus tests for
//! the snapshot and the replay of assignments.

use knorke_light2d::ObstacleStore;
use knorke_light2d::obstacle::{CellRect, ObstacleError, PatternSource, ShapeDesc, TileGridDesc};
use knorke_types::{Reader, SnapshotBuf, SnapshotError, restore};
use std::cell::RefCell;

/// A stand-in for the pattern store: ids from 1, `None` marks a destroyed pattern.
#[derive(Default)]
struct Patterns(RefCell<Vec<Option<f32>>>);

impl Patterns {
    /// A pattern whose mean is `opaque_texels / 4`.
    fn make(&self, opaque_texels: u32) -> u32 {
        let mut v = self.0.borrow_mut();
        v.push(Some(opaque_texels as f32 / 4.0));
        v.len() as u32
    }

    fn destroy(&self, id: u32) {
        self.0.borrow_mut()[id as usize - 1] = None;
    }
}

impl PatternSource for Patterns {
    fn mean_opacity(&self, id: u32) -> Option<f32> {
        self.0.borrow().get(id as usize - 1).copied().flatten()
    }
}

fn square() -> ShapeDesc<'static> {
    ShapeDesc::rect(1.0, 1.0)
}

fn ngon(n: usize) -> Vec<knorke_math::Vec2> {
    (0..n)
        .map(|i| {
            let a = core::f32::consts::TAU * i as f32 / n as f32;
            knorke_math::Vec2::new(
                knorke_math::det::cos(a) * 5.0,
                knorke_math::det::sin(a) * 5.0,
            )
        })
        .collect()
}

/// A 4 x 3 grid, every cell solid at level 2 and opacity 200 (or opaque).
fn cells(opacity: Option<&[u8]>) -> TileGridDesc<'_> {
    static SOLID: [u8; 12] = [2; 12];
    let mut d = TileGridDesc::new(4, 3, &SOLID);
    d.opacity = opacity;
    d
}

#[test]
fn obstacles_start_without_a_pattern() {
    let mut store = ObstacleStore::new();
    let id = store.create_shape(&square()).unwrap();
    let p = &store.get(id).unwrap().patterns;
    assert_eq!((p.pattern, p.refs), (0, 0));
    assert!(p.table.is_empty() && p.cells.is_empty());
    assert!(!store.any_pattern() && !store.any_partial());
}

#[test]
fn a_grid_of_another_size_drops_the_cell_indices_but_keeps_table_and_pattern() {
    let pats = Patterns::default();
    let pat = pats.make(2);
    let mut store = ObstacleStore::new();
    let six = [1u8; 6];
    let three = TileGridDesc::new(3, 2, &six);
    let grid = store.create_tile_grid(&three).unwrap();
    store.set_pattern(grid, pat, &pats).unwrap();
    store.set_tile_pattern_table(grid, &[pat], &pats).unwrap();
    store
        .set_tile_pattern_cells(grid, CellRect::new(0, 0, 2, 1), &[1, 1])
        .unwrap();
    assert!(!store.get(grid).unwrap().patterns.cells.is_empty());
    store.set_tile_grid(grid, &three).unwrap(); // same size: the indices stay
    assert!(!store.get(grid).unwrap().patterns.cells.is_empty());
    let four = [1u8; 4];
    store
        .set_tile_grid(grid, &TileGridDesc::new(2, 2, &four))
        .unwrap(); // another size: the indices go
    let t = store.tile(grid).unwrap();
    assert!(t.pattern_cells.is_empty());
    assert_eq!((t.pattern, t.pattern_table.len()), (pat, 1));
}

#[test]
fn set_pattern_on_box_circle_and_polygon_with_64_corners_but_not_65() {
    let pats = Patterns::default();
    let pat = pats.make(2);
    let mut store = ObstacleStore::new();
    let boxed = store.create_shape(&square()).unwrap();
    let round = store.create_shape(&ShapeDesc::circle(2.0)).unwrap();
    store.set_pattern(boxed, pat, &pats).unwrap();
    store.set_pattern(round, pat, &pats).unwrap();
    assert_eq!(store.get(boxed).unwrap().patterns.pattern, pat);
    assert_eq!(store.get(round).unwrap().patterns.pattern, pat);

    let (v64, v65) = (ngon(64), ngon(65));
    let ok = store.create_shape(&ShapeDesc::polygon(&v64)).unwrap();
    store.set_pattern(ok, pat, &pats).unwrap();
    let big = store.create_shape(&ShapeDesc::polygon(&v65)).unwrap();
    let revision = store.get(big).unwrap().revision;
    let e = store.set_pattern(big, pat, &pats).unwrap_err();
    assert_eq!(e, ObstacleError::PatternVertices { corners: 65 });
    assert_eq!(
        e.code().as_str(),
        "knorke.light2d.obstacle.pattern_vertices"
    );
    assert_eq!(store.get(big).unwrap().patterns.pattern, 0);
    assert_eq!(store.get(big).unwrap().revision, revision);
    store.set_pattern(big, 0, &pats).unwrap(); // no pattern is always fine

    // A polygon that carries a pattern cannot be replaced by one with more than 64 corners.
    assert!(matches!(
        store.set_shape(boxed, &ShapeDesc::polygon(&v65)),
        Err(ObstacleError::PatternVertices { corners: 65 })
    ));
    assert_eq!(store.outline(boxed).len(), 4);
    // The same check in the batch, which then changes nothing.
    let e = store
        .set_shapes(&[boxed][..], &[ShapeDesc::polygon(&v65)][..])
        .unwrap_err();
    assert!(matches!(
        e,
        ObstacleError::Batch {
            field: "descs",
            index: 0,
            ..
        }
    ));
    assert_eq!(store.outline(boxed).len(), 4);
}

#[test]
fn bad_handles_and_patterns_change_nothing() {
    let pats = Patterns::default();
    let pat = pats.make(2);
    let mut store = ObstacleStore::new();
    let boxed = store.create_shape(&square()).unwrap();
    let stale = store.create_shape(&square()).unwrap();
    store.destroy(stale).unwrap();
    assert!(matches!(
        store.set_pattern(stale, pat, &pats),
        Err(ObstacleError::InvalidHandle(_))
    ));
    let gone = pats.make(1);
    pats.destroy(gone);
    let e = store.set_pattern(boxed, gone, &pats).unwrap_err();
    assert_eq!(e, ObstacleError::PatternNotLive { pattern: gone });
    assert_eq!(
        e.code().as_str(),
        "knorke.light2d.obstacle.pattern_not_live"
    );
    assert_eq!(store.get(boxed).unwrap().patterns.pattern, 0);
    assert!(!store.any_pattern());
}

#[test]
fn the_batch_checks_everything_first() {
    let pats = Patterns::default();
    let pat = pats.make(2);
    let mut store = ObstacleStore::new();
    let a = store.create_shape(&square()).unwrap();
    let b = store.create_shape(&square()).unwrap();
    let e = store
        .set_patterns(&[a, b][..], &[pat, 999][..], &|id| {
            if id == pat { Some(0.5) } else { None }
        })
        .unwrap_err();
    match &e {
        ObstacleError::Batch {
            field,
            index,
            source,
        } => {
            assert_eq!((*field, *index), ("patterns", 1));
            assert_eq!(**source, ObstacleError::PatternNotLive { pattern: 999 });
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(store.get(a).unwrap().patterns.pattern, 0);
    let twice = store.set_patterns(&[a, a][..], &[pat, pat][..], &pats);
    assert!(matches!(
        twice,
        Err(ObstacleError::Batch {
            field: "ids",
            index: 1,
            ..
        })
    ));
    assert_eq!(store.get(a).unwrap().patterns.pattern, 0);
    assert!(matches!(
        store.set_patterns(&[a][..], &[pat, pat][..], &pats),
        Err(ObstacleError::LengthMismatch { .. })
    ));
    store
        .set_patterns(&[a, b][..], &[pat, pat][..], &pats)
        .unwrap();
    assert_eq!(store.get(a).unwrap().patterns.pattern, pat);
    assert_eq!(store.get(b).unwrap().patterns.pattern, pat);
}

#[test]
fn table_limit_cell_indices_and_what_an_error_changes() {
    let pats = Patterns::default();
    let (p1, p2) = (pats.make(1), pats.make(3));
    let mut store = ObstacleStore::new();
    let grid = store.create_tile_grid(&cells(None)).unwrap();

    store
        .set_tile_pattern_table(grid, &[p1; 255], &pats)
        .unwrap();
    let e = store
        .set_tile_pattern_table(grid, &[p1; 256], &pats)
        .unwrap_err();
    assert_eq!(e, ObstacleError::PatternTable { count: 256 });
    assert_eq!(store.tile(grid).unwrap().pattern_table.len(), 255);

    store
        .set_tile_pattern_table(grid, &[p1, p2], &pats)
        .unwrap();
    store
        .set_tile_pattern_cells(grid, CellRect::new(1, 1, 2, 1), &[1, 2])
        .unwrap();
    let c = store.tile(grid).unwrap().pattern_cells;
    assert_eq!((c[4 + 1], c[4 + 2]), (1, 2));
    let version = store.cell_version();
    let revision = store.get(grid).unwrap().revision;
    // 3 > table size 2: nothing is written, also not the first.
    let e = store
        .set_tile_pattern_cells(grid, CellRect::new(0, 0, 3, 1), &[1, 3, 1])
        .unwrap_err();
    assert_eq!(
        e,
        ObstacleError::PatternIndex {
            cell: 1,
            index: 3,
            table: 2
        }
    );
    assert_eq!(store.tile(grid).unwrap().pattern_cells[0], 0);
    assert!(matches!(
        store.set_tile_pattern_cells(grid, CellRect::new(3, 0, 2, 1), &[1, 2]),
        Err(ObstacleError::CellRect { .. })
    ));
    assert!(matches!(
        store.set_tile_pattern_cells(grid, CellRect::new(0, 0, 2, 1), &[1]),
        Err(ObstacleError::ChannelLength {
            field: "indices",
            ..
        })
    ));
    assert_eq!(store.cell_version(), version);
    assert_eq!(store.get(grid).unwrap().revision, revision);
    // Unchanged indices log nothing.
    store
        .set_tile_pattern_cells(grid, CellRect::new(1, 1, 2, 1), &[1, 2])
        .unwrap();
    assert_eq!(store.cell_version(), version);

    // A table entry that is no live pattern changes nothing.
    assert_eq!(
        store.set_tile_pattern_table(grid, &[p1, 77], &pats),
        Err(ObstacleError::PatternNotLive { pattern: 77 })
    );
    assert_eq!(store.tile(grid).unwrap().pattern_table[1], p2);

    // A shape is no grid.
    let boxed = store.create_shape(&square()).unwrap();
    assert!(matches!(
        store.set_tile_pattern_table(boxed, &[p1], &pats),
        Err(ObstacleError::NotTileGrid(_))
    ));
    assert!(matches!(
        store.set_tile_pattern_cells(boxed, CellRect::new(0, 0, 1, 1), &[1]),
        Err(ObstacleError::NotTileGrid(_))
    ));
}

#[test]
fn any_pattern_and_any_partial_follow_set_remove_disable_and_destroy() {
    let pats = Patterns::default();
    let pat = pats.make(4); // opaque everywhere: partial only through the pattern
    let mut store = ObstacleStore::new();
    let boxed = store.create_shape(&square()).unwrap();
    assert!(!store.any_pattern() && !store.any_partial());
    store.set_pattern(boxed, pat, &pats).unwrap();
    assert!(store.any_pattern() && store.any_partial());
    store.set_pattern(boxed, 0, &pats).unwrap(); // remove
    assert!(!store.any_pattern() && !store.any_partial());

    store.set_pattern(boxed, pat, &pats).unwrap();
    let mut off = square();
    off.common.enabled = false;
    store.set_shape(boxed, &off).unwrap(); // the pattern stays assigned, the obstacle is inactive
    assert_eq!(store.get(boxed).unwrap().patterns.pattern, pat);
    assert!(!store.any_pattern() && !store.any_partial());
    store.set_shape(boxed, &square()).unwrap();
    assert!(store.any_pattern() && store.any_partial());

    store.destroy(boxed).unwrap();
    assert!(!store.any_pattern() && !store.any_partial());

    let other = store.create_shape(&square()).unwrap();
    store.set_pattern(other, pat, &pats).unwrap();
    assert!(store.any_pattern());
    pats.destroy(pat);
    assert_eq!(store.count_destroyed_pattern_users(&pats, false), 1);
    store.on_patterns_destroyed(&[pat]);
    // The assignment stays, it acts like none.
    assert_eq!(store.get(other).unwrap().patterns.pattern, pat);
    assert!(!store.any_pattern() && !store.any_partial());
}

#[test]
fn a_grid_counts_through_its_table() {
    let pats = Patterns::default();
    let (p1, p2) = (pats.make(4), pats.make(2));
    let mut store = ObstacleStore::new();
    let grid = store.create_tile_grid(&cells(None)).unwrap(); // opaque cells
    assert!(!store.any_partial());
    store
        .set_tile_pattern_table(grid, &[p1, p2, p1], &pats)
        .unwrap();
    assert_eq!(store.get(grid).unwrap().patterns.refs, 2); // distinct
    assert!(store.any_pattern() && store.any_partial());
    pats.destroy(p1);
    store.on_patterns_destroyed(&[p1]);
    assert!(store.any_pattern()); // p2 is still live
    pats.destroy(p2);
    store.on_patterns_destroyed(&[p2]);
    assert!(!store.any_pattern() && !store.any_partial());
}

#[test]
fn mask_opacity_is_the_scalar_or_the_clamped_mean_times_the_opacity() {
    let pats = Patterns::default();
    let (half, full, empty) = (pats.make(2), pats.make(4), pats.make(0));
    let mut store = ObstacleStore::new();
    let mut d = square();
    d.common.opacity = 0.8;
    let boxed = store.create_shape(&d).unwrap();
    let mask = |store: &ObstacleStore| store.mask_opacity(boxed, 0, &pats).unwrap();

    assert_eq!(mask(&store), 0.8); // no pattern: the old value
    store.set_pattern(boxed, half, &pats).unwrap();
    assert!((mask(&store) - 0.4).abs() < 1e-6);
    store.set_pattern(boxed, full, &pats).unwrap();
    assert_eq!(mask(&store), 0.8);
    store.set_shape(boxed, &square()).unwrap(); // opacity 1, pattern kept
    assert_eq!(mask(&store), 254.0 / 255.0);
    store.set_pattern(boxed, empty, &pats).unwrap();
    assert_eq!(mask(&store), 1.0 / 255.0);
    pats.destroy(empty);
    assert_eq!(mask(&store), 1.0); // destroyed: scalar again
}

#[test]
fn opacity_zero_stays_zero_with_a_live_pattern() {
    let pats = Patterns::default();
    let half = pats.make(2);
    let mut store = ObstacleStore::new();
    let mut d = square();
    d.common.opacity = 0.0;
    let boxed = store.create_shape(&d).unwrap();
    store.set_pattern(boxed, half, &pats).unwrap();
    assert_eq!(store.mask_opacity(boxed, 0, &pats).unwrap(), 0.0);

    let zero = [0u8; 12];
    let grid = store.create_tile_grid(&cells(Some(&zero))).unwrap();
    store.set_pattern(grid, half, &pats).unwrap();
    assert_eq!(store.mask_opacity(grid, 3, &pats).unwrap(), 0.0);
}

#[test]
fn a_big_polygon_replaces_a_grid_with_a_pattern_because_the_grid_drops_it() {
    let pats = Patterns::default();
    let pat = pats.make(2);
    let mut store = ObstacleStore::new();
    let grid = store.create_tile_grid(&cells(None)).unwrap();
    store.set_pattern(grid, pat, &pats).unwrap();
    let big = ngon(65);
    store.set_shape(grid, &ShapeDesc::polygon(&big)).unwrap();
    assert_eq!(store.get(grid).unwrap().patterns.pattern, 0);
    assert!(!store.any_pattern());
}

#[test]
fn a_grid_cell_reads_its_table_entry_the_grid_pattern_or_the_scalar() {
    let pats = Patterns::default();
    let (half, quarter) = (pats.make(2), pats.make(1));
    let mut store = ObstacleStore::new();
    let opacity = [200u8; 12];
    let grid = store.create_tile_grid(&cells(Some(&opacity))).unwrap();
    let scalar = 200.0 / 255.0;
    let at = |store: &ObstacleStore, cell| store.mask_opacity(grid, cell, &pats).unwrap();
    let near = |a: f32, b: f32| (a - b).abs() < 1e-6;
    assert_eq!(at(&store, 5), scalar);

    store.set_pattern(grid, half, &pats).unwrap();
    store
        .set_tile_pattern_table(grid, &[quarter, 0], &pats)
        .unwrap();
    store
        .set_tile_pattern_cells(grid, CellRect::new(0, 0, 3, 1), &[1, 2, 0])
        .unwrap();
    assert!(near(at(&store, 0), 0.25 * scalar)); // table[0]
    assert_eq!(at(&store, 1), scalar); // table[1] is "none"
    assert!(near(at(&store, 2), 0.5 * scalar)); // the grid pattern
    assert!(near(at(&store, 5), 0.5 * scalar)); // never set
    pats.destroy(quarter);
    store.on_patterns_destroyed(&[quarter]);
    assert_eq!(at(&store, 0), scalar);
    assert!(near(at(&store, 2), 0.5 * scalar));
}

#[test]
fn destroying_table_patterns_logs_only_the_cells_that_point_at_them() {
    let pats = Patterns::default();
    let (p1, p2) = (pats.make(1), pats.make(2));
    let mut store = ObstacleStore::new();
    let grid = store.create_tile_grid(&cells(None)).unwrap();
    store
        .set_tile_pattern_table(grid, &[p1, p2], &pats)
        .unwrap();
    store
        .set_tile_pattern_cells(grid, CellRect::new(0, 0, 4, 1), &[0, 1, 2, 1])
        .unwrap();
    let (version, revision) = (store.cell_version(), store.get(grid).unwrap().revision);
    pats.destroy(p1);
    store.on_patterns_destroyed(&[p1]);
    // Cells 1 and 3 pointed at p1: one rectangle around them; the grid itself did not change.
    let mut log = Vec::new();
    store.cell_log().for_each(version, |c| log.push(c.cells));
    assert_eq!(
        log,
        vec![CellRect {
            x0: 1,
            y0: 0,
            x1: 4,
            y1: 1
        }]
    );
    assert_eq!(store.get(grid).unwrap().revision, revision);
    assert_eq!(store.get(grid).unwrap().patterns.refs, 1);

    // The grid's own pattern is a change of the whole obstacle.
    store.set_pattern(grid, p2, &pats).unwrap();
    let revision = store.get(grid).unwrap().revision;
    pats.destroy(p2);
    store.on_patterns_destroyed(&[p2]);
    assert_eq!(store.get(grid).unwrap().revision, revision + 1);
}

#[test]
fn shape_to_shape_keeps_the_assignment_a_change_of_kind_drops_it() {
    let pats = Patterns::default();
    let pat = pats.make(2);
    let mut store = ObstacleStore::new();
    let boxed = store.create_shape(&square()).unwrap();
    store.set_pattern(boxed, pat, &pats).unwrap();
    store.set_shape(boxed, &square()).unwrap();
    assert_eq!(store.get(boxed).unwrap().patterns.pattern, pat);
    assert!(store.any_pattern());
    // The batch keeps it as well.
    store
        .set_shapes(&[boxed][..], &[ShapeDesc::circle(1.0)][..])
        .unwrap();
    assert_eq!(store.get(boxed).unwrap().patterns.pattern, pat);
    store.set_tile_grid(boxed, &cells(None)).unwrap(); // shape to grid: the assignment goes
    assert_eq!(store.get(boxed).unwrap().patterns.pattern, 0);
    assert!(!store.any_pattern() && !store.any_partial());
}

/// A store with assignments of every kind, as a replay would build it.
fn filled(pats: &Patterns) -> (ObstacleStore, Vec<knorke_light2d::ObstacleId>) {
    let (p1, p2) = (pats.make(1), pats.make(3));
    let mut store = ObstacleStore::new();
    let boxed = store.create_shape(&square()).unwrap();
    let grid = store.create_tile_grid(&cells(None)).unwrap();
    store.set_pattern(boxed, p1, pats).unwrap();
    store.set_pattern(grid, p2, pats).unwrap();
    store.set_tile_pattern_table(grid, &[p1, p2], pats).unwrap();
    store
        .set_tile_pattern_cells(grid, CellRect::new(1, 1, 2, 2), &[1, 2, 2, 0])
        .unwrap();
    store
        .set_tile_cells(grid, CellRect::new(0, 0, 1, 1), Some(&[5]), None, None)
        .unwrap();
    (store, vec![boxed, grid])
}

#[test]
fn snapshot_round_trip_keeps_assignments_and_the_cell_log() {
    let pats = Patterns::default();
    let (store, ids) = filled(&pats);
    let mut buf = SnapshotBuf::new();
    buf.save(&store);
    let mut other = ObstacleStore::new();
    restore(&mut Reader::new(buf.bytes()), &mut other).unwrap();
    for &id in &ids {
        assert_eq!(other.get(id), store.get(id));
    }
    assert_eq!(other.cell_version(), store.cell_version());
    assert_eq!(other.any_partial(), store.any_partial());
    assert!(other.any_pattern());
    let mut again = SnapshotBuf::new();
    again.save(&other);
    assert_eq!(again.bytes(), buf.bytes());
}

/// Replay: the same commands give the same bytes; a snapshot in the middle continues identically.
#[test]
fn replaying_the_commands_gives_the_same_state_and_a_restored_store_continues_identically() {
    let run = |upto: usize| {
        let pats = Patterns::default();
        let (mut store, ids) = filled(&pats);
        let grid = ids[1];
        for t in 0..upto {
            let i = (t % 4) as u32;
            store
                .set_tile_pattern_cells(grid, CellRect::new(i, 2, 1, 1), &[(t % 3) as u8])
                .unwrap();
            store
                .set_tile_cells(
                    grid,
                    CellRect::new(i, 0, 1, 1),
                    Some(&[(t % 4) as u8 + 1]),
                    None,
                    None,
                )
                .unwrap();
        }
        let mut buf = SnapshotBuf::new();
        buf.save(&store);
        buf.bytes().to_vec()
    };
    let full = run(40);
    assert_eq!(full, run(40));
    // Snapshot after 10 commands, then the same 30 more: only the tail of the loop is repeated,
    // so compare by running the whole loop on the restored middle state.
    let mid = run(10);
    let mut resumed = ObstacleStore::new();
    restore(&mut Reader::new(&mid), &mut resumed).unwrap();
    let pats = Patterns::default();
    let (_, ids) = filled(&pats);
    for t in 10..40 {
        let i = (t % 4) as u32;
        resumed
            .set_tile_pattern_cells(ids[1], CellRect::new(i, 2, 1, 1), &[(t % 3) as u8])
            .unwrap();
        resumed
            .set_tile_cells(
                ids[1],
                CellRect::new(i, 0, 1, 1),
                Some(&[(t % 4) as u8 + 1]),
                None,
                None,
            )
            .unwrap();
    }
    let mut buf = SnapshotBuf::new();
    buf.save(&resumed);
    assert_eq!(buf.bytes(), &full[..]);
}

#[test]
fn restore_rejects_broken_pattern_assignments() {
    let pats = Patterns::default();
    let (store, _) = filled(&pats);
    let mut buf = SnapshotBuf::new();
    buf.save(&store);
    // The table of the grid has two entries; the cell index bytes of the grid are [0, 1, 2, 2, 0, ...]:
    // find the 12-byte cell channel and raise one index above the table.
    let mut bytes = buf.bytes().to_vec();
    let at = bytes
        .windows(12)
        .position(|w| w == [0, 0, 0, 0, 0, 1, 2, 0, 0, 2, 0, 0])
        .expect("cell indices in the snapshot");
    bytes[at + 5] = 3;
    let r = restore(&mut Reader::new(&bytes), &mut ObstacleStore::new());
    assert!(matches!(r, Err(SnapshotError::Invalid(_))), "{r:?}");
}
