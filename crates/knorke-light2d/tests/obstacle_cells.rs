//! Origin: chiaro @ 9bc5b9b, tests/unit/ObstacleStoreTests.cpp (zlib license, see
//! `src/obstacle/`): the cases "tile cells", "tile cell levels" and "compare cells" of P14, plus
//! tests for what the redesign adds: typed errors with stable codes, the cell log in snapshots.

use knorke_light2d::obstacle::{
    CellChange, CellLog, CellRect, ObstacleError, ShapeDesc, TileGridDesc,
};
use knorke_light2d::{ObstacleId, ObstacleStore};
use knorke_types::{Reader, Snapshot, SnapshotBuf, SnapshotError, restore};

/// A 4 x 3 grid with every cell solid at level 2, base level 1 and opacity 200.
struct CellGrid {
    solid: Vec<u8>,
    base: Vec<u8>,
    opacity: Vec<u8>,
}

impl CellGrid {
    fn new() -> Self {
        Self {
            solid: vec![2; 12],
            base: vec![1; 12],
            opacity: vec![200; 12],
        }
    }

    fn desc(&self) -> TileGridDesc<'_> {
        let mut d = TileGridDesc::new(4, 3, &self.solid);
        d.level_height = 10.0;
        d.base_levels = Some(&self.base);
        d.opacity = Some(&self.opacity);
        d.edge_width = 0.25;
        d
    }
}

fn changes(store: &ObstacleStore, since: u64) -> Vec<CellChange> {
    let mut out = Vec::new();
    assert!(store.cell_log().for_each(since, |c| out.push(*c)));
    out
}

fn grid(store: &ObstacleStore, id: ObstacleId) -> &knorke_light2d::obstacle::TileCells {
    &store.get(id).unwrap().grid
}

#[test]
fn a_rectangle_of_cells_changes_only_the_given_channels() {
    let mut store = ObstacleStore::new();
    let g = CellGrid::new();
    let t = store.create_tile_grid(&g.desc()).unwrap();
    let static_version = store.static_version();
    let revision = store.get(t).unwrap().revision;
    assert_eq!(store.cell_version(), 0);

    // Cells (1, 1) and (2, 1).
    store
        .set_tile_cells(t, CellRect::new(1, 1, 2, 1), Some(&[0, 3]), None, None)
        .unwrap();
    let c = grid(&store, t);
    assert_eq!(c.solid[4 + 1], 0);
    assert_eq!(c.solid[4 + 2], 3);
    assert_eq!(c.solid[0], 2);
    assert_eq!(c.opacity[4 + 1], 200); // unchanged channel
    assert_eq!(c.max_level, 3);
    assert_eq!(c.partial_cells, 11); // the empty cell no longer counts
    assert_eq!(store.max_finite_top(), 30.0);
    // No full replacement: the static version and revision stay, the cell log grows.
    assert_eq!(store.static_version(), static_version);
    assert_eq!(store.get(t).unwrap().revision, revision);
    assert_eq!(store.cell_version(), 1);
    assert_eq!(
        changes(&store, 0),
        vec![CellChange {
            grid: t,
            cells: CellRect {
                x0: 1,
                y0: 1,
                x1: 3,
                y1: 2
            }
        }]
    );

    // Lowering the highest cell recounts the top.
    store
        .set_tile_cells(t, CellRect::new(2, 1, 1, 1), Some(&[2]), None, None)
        .unwrap();
    assert_eq!(grid(&store, t).max_level, 2);

    // Opacity and edges alone; the edges channel is created with 0 = whole cell elsewhere.
    store
        .set_tile_cells(
            t,
            CellRect::new(0, 2, 3, 1),
            None,
            Some(&[255, 255, 255]),
            Some(&[1, 0, 15]),
        )
        .unwrap();
    let c = grid(&store, t);
    assert_eq!(c.partial_cells, 8);
    assert_eq!(c.edge_cells, 2);
    assert_eq!(c.edges.len(), 12);
    assert_eq!(c.edges[2 * 4 + 2], 15);
    assert_eq!(c.edges[0], 0);
}

#[test]
fn the_result_equals_a_full_replacement_with_the_same_data() {
    let g = CellGrid::new();
    let mut plain = TileGridDesc::new(4, 3, &g.solid);
    plain.level_height = 10.0;
    plain.base_levels = Some(&g.base);
    plain.edge_width = 0.25;
    let (mut a, mut b) = (ObstacleStore::new(), ObstacleStore::new());
    let ta = a.create_tile_grid(&plain).unwrap();
    let tb = b.create_tile_grid(&plain).unwrap();
    let solid = [0u8, 5, 0, 3];
    let opacity = [10u8, 20, 30, 40];
    a.set_tile_cells(
        ta,
        CellRect::new(2, 0, 2, 2),
        Some(&solid),
        Some(&opacity),
        None,
    )
    .unwrap();

    let mut full_solid = g.solid.clone();
    let mut full_opacity = vec![255u8; 12];
    for y in 0..2 {
        for x in 0..2 {
            full_solid[y * 4 + 2 + x] = solid[y * 2 + x];
            full_opacity[y * 4 + 2 + x] = opacity[y * 2 + x];
        }
    }
    let mut next = plain;
    next.solid = &full_solid;
    next.opacity = Some(&full_opacity);
    b.set_tile_grid(tb, &next).unwrap();
    let (ca, cb) = (grid(&a, ta), grid(&b, tb));
    assert_eq!(ca, cb);
    assert_eq!(a.any_partial(), b.any_partial());
}

#[test]
fn invalid_rectangles_handles_and_values_change_nothing() {
    let mut store = ObstacleStore::new();
    let g = CellGrid::new();
    let t = store.create_tile_grid(&g.desc()).unwrap();
    let zeros = [0u8; 16];
    let rect = |x, y, w, h| CellRect::new(x, y, w, h);
    for r in [
        rect(0, 0, 0, 1),
        rect(0, 0, 1, 0),
        rect(3, 0, 2, 1),
        rect(0, 2, 1, 2),
        rect(u32::MAX, 0, 5, 1),
    ] {
        let e = store.set_tile_cells(t, r, Some(&zeros[..1]), None, None);
        assert!(
            matches!(e, Err(ObstacleError::CellRect { .. })),
            "{r:?}: {e:?}"
        );
    }
    // A stale handle.
    let stale = store.create_shape(&ShapeDesc::circle(1.0)).unwrap();
    store.destroy(stale).unwrap();
    assert!(matches!(
        store.set_tile_cells(stale, rect(0, 0, 1, 1), Some(&zeros[..1]), None, None),
        Err(ObstacleError::InvalidHandle(_))
    ));
    // Edge bits above 15; a top at the stored base level; a wrong channel length.
    assert!(matches!(
        store.set_tile_cells(t, rect(0, 0, 2, 1), None, None, Some(&[0, 16])),
        Err(ObstacleError::CellValue {
            field: "edges",
            cell: 1,
            ..
        })
    ));
    assert!(matches!(
        store.set_tile_cells(t, rect(0, 0, 2, 1), Some(&[0, 1]), None, None),
        Err(ObstacleError::CellValue {
            field: "solid",
            cell: 1,
            ..
        })
    ));
    assert!(matches!(
        store.set_tile_cells(t, rect(0, 0, 2, 1), Some(&[0, 1, 1]), None, None),
        Err(ObstacleError::ChannelLength {
            field: "solid",
            expected: 2,
            got: 3
        })
    ));
    // Not a tile grid.
    let shape = store.create_shape(&ShapeDesc::circle(1.0)).unwrap();
    let e = store
        .set_tile_cells(shape, rect(0, 0, 1, 1), Some(&zeros[..1]), None, None)
        .unwrap_err();
    assert!(matches!(e, ObstacleError::NotTileGrid(_)));
    assert_eq!(e.code().as_str(), "knorke.light2d.obstacle.not_tile_grid");
    // Edges need an edge width.
    let mut flat = g.desc();
    flat.edge_width = 0.0;
    let no_width = store.create_tile_grid(&flat).unwrap();
    assert!(matches!(
        store.set_tile_cells(no_width, rect(0, 0, 1, 1), None, None, Some(&[1])),
        Err(ObstacleError::Unsupported(_))
    ));

    assert_eq!(grid(&store, t).solid, g.solid);
    assert!(grid(&store, t).edges.is_empty());
    assert_eq!(store.cell_version(), 0);
    // No channel at all is a valid no-op.
    store
        .set_tile_cells(t, rect(0, 0, 1, 1), None, None, None)
        .unwrap();
    assert_eq!(store.cell_version(), 0);
}

#[test]
fn the_change_log_reports_what_it_lost() {
    let mut store = ObstacleStore::new();
    let g = CellGrid::new();
    let t = store.create_tile_grid(&g.desc()).unwrap();
    for _ in 0..=CellLog::SIZE {
        store
            .set_tile_cells(t, CellRect::new(0, 0, 1, 1), Some(&[2]), None, None)
            .unwrap();
    }
    let mut count = 0u64;
    assert!(!store.cell_log().for_each(0, |_| count += 1));
    assert!(store.cell_log().for_each(1, |_| count += 1));
    assert_eq!(count, CellLog::SIZE);
    count = 0;
    assert!(
        store
            .cell_log()
            .for_each(store.cell_version(), |_| count += 1)
    );
    assert_eq!(count, 0);
}

#[test]
fn levels_change_top_and_base_together_through_the_cell_log() {
    let mut store = ObstacleStore::new();
    let g = CellGrid::new(); // top 2, base 1 everywhere
    let t = store.create_tile_grid(&g.desc()).unwrap();
    let static_version = store.static_version();
    let revision = store.get(t).unwrap().revision;

    // A run falls (top 1, base 0) and a new one rises (top 5, base 3): neither is possible with
    // set_tile_cells, which checks the new top against the old base.
    let r = CellRect::new(1, 0, 2, 1);
    assert!(
        store
            .set_tile_cells(t, r, Some(&[1, 5]), None, None)
            .is_err()
    );
    store
        .set_tile_cell_levels(t, r, Some(&[1, 5]), Some(&[0, 3]))
        .unwrap();
    let c = grid(&store, t);
    assert_eq!((c.solid[1], c.base_levels[1]), (1, 0));
    assert_eq!((c.solid[2], c.base_levels[2]), (5, 3));
    assert_eq!((c.solid[0], c.base_levels[0]), (2, 1));
    assert_eq!(c.max_level, 5);
    assert_eq!(store.max_finite_top(), 50.0);
    assert_eq!(store.static_version(), static_version);
    assert_eq!(store.get(t).unwrap().revision, revision);
    assert_eq!(store.cell_version(), 1);

    // One channel alone is checked against the other's current values; an empty cell takes any base.
    store
        .set_tile_cell_levels(t, CellRect::new(3, 2, 1, 1), None, Some(&[0]))
        .unwrap();
    assert_eq!(grid(&store, t).base_levels[2 * 4 + 3], 0);
    assert!(
        store
            .set_tile_cell_levels(t, CellRect::new(0, 2, 1, 1), None, Some(&[2]))
            .is_err()
    );
    store
        .set_tile_cell_levels(t, CellRect::new(0, 1, 2, 1), Some(&[0, 0]), Some(&[200, 9]))
        .unwrap();
    assert_eq!(store.cell_version(), 3);
    // Lowering the highest cell recounts the top; its base 3 would be above the new top 2.
    let top = CellRect::new(2, 0, 1, 1);
    assert!(
        store
            .set_tile_cell_levels(t, top, Some(&[2]), None)
            .is_err()
    );
    assert_eq!(grid(&store, t).max_level, 5);
    store
        .set_tile_cell_levels(t, top, Some(&[2]), Some(&[0]))
        .unwrap();
    assert_eq!(grid(&store, t).max_level, 2);
    assert_eq!(store.max_finite_top(), 20.0);
    // Nothing to change is a valid no-op.
    store
        .set_tile_cell_levels(t, CellRect::new(0, 0, 1, 1), None, None)
        .unwrap();
    assert_eq!(store.cell_version(), 4);
}

#[test]
fn a_grid_without_base_levels_gets_the_channel_and_invalid_level_calls_change_nothing() {
    let mut store = ObstacleStore::new();
    let g = CellGrid::new();
    let mut d = g.desc();
    d.base_levels = None;
    let t = store.create_tile_grid(&d).unwrap();
    store
        .set_tile_cell_levels(t, CellRect::new(2, 1, 1, 1), None, Some(&[1]))
        .unwrap();
    let c = grid(&store, t);
    assert_eq!(c.base_levels.len(), 12);
    assert_eq!((c.base_levels[4 + 2], c.base_levels[0]), (1, 0));

    // The second cell: base 4 at its top 4.
    let e = store
        .set_tile_cell_levels(t, CellRect::new(0, 0, 2, 1), Some(&[3, 4]), Some(&[0, 4]))
        .unwrap_err();
    assert!(matches!(
        e,
        ObstacleError::CellValue {
            field: "base_levels",
            cell: 1,
            value: 4,
            ..
        }
    ));
    assert_eq!(e.code().as_str(), "knorke.light2d.obstacle.cell_value");

    let mut flat = g.desc();
    flat.level_height = 0.0;
    flat.base_levels = None;
    let flat_grid = store.create_tile_grid(&flat).unwrap();
    assert!(matches!(
        store.set_tile_cell_levels(flat_grid, CellRect::new(0, 0, 1, 1), None, Some(&[1])),
        Err(ObstacleError::Unsupported(_))
    ));
    assert!(grid(&store, flat_grid).base_levels.is_empty());
    assert_eq!(grid(&store, t).solid, g.solid);
    assert_eq!(store.cell_version(), 1);
}

/// A 96 x 48 grid with levels, base levels, opacity and edges, for compared replacement.
struct Compared {
    solid: Vec<u8>,
    base: Vec<u8>,
    opacity: Vec<u8>,
    edges: Vec<u8>,
}

const COLS: usize = 96;
const ROWS: usize = 48;

impl Compared {
    fn new() -> Self {
        let n = COLS * ROWS;
        Self {
            solid: (0..n).map(|i| if i % 7 == 0 { 4 } else { 0 }).collect(),
            base: (0..n).map(|i| if i % 14 == 0 { 2 } else { 0 }).collect(),
            opacity: (0..n).map(|i| if i % 5 == 0 { 128 } else { 255 }).collect(),
            edges: (0..n).map(|i| u8::from(i % 11 == 0)).collect(),
        }
    }

    fn desc(&self) -> TileGridDesc<'_> {
        let mut d = TileGridDesc::new(COLS as u32, ROWS as u32, &self.solid);
        d.level_height = 16.0;
        d.edge_width = 2.0;
        d.base_levels = Some(&self.base);
        d.opacity = Some(&self.opacity);
        d.edges = Some(&self.edges);
        d
    }
}

#[test]
fn compare_logs_only_the_changed_cells() {
    let mut store = ObstacleStore::new();
    let mut g = Compared::new();
    let t = store.create_tile_grid(&g.desc()).unwrap();
    let static_version = store.static_version();
    let revision = store.get(t).unwrap().revision;

    // The same cells again: nothing changes, nothing is logged.
    store.set_tile_grid_compare(t, &g.desc()).unwrap();
    assert_eq!(store.cell_version(), 0);
    assert_eq!(store.static_version(), static_version);
    assert_eq!(store.get(t).unwrap().revision, revision);

    // One cell (all four channels) and, far away, another one: two rectangles.
    g.solid[5 * COLS + 9] = 6;
    g.base[5 * COLS + 9] = 3;
    g.opacity[5 * COLS + 9] = 10;
    g.edges[5 * COLS + 9] = 4;
    g.opacity[33 * COLS + 50] = 0;
    store.set_tile_grid_compare(t, &g.desc()).unwrap();
    assert_eq!(store.static_version(), static_version);
    assert_eq!(store.get(t).unwrap().revision, revision);
    let log = changes(&store, 0);
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].grid, t);
    assert_eq!(log[0].cells, CellRect::new(9, 5, 1, 1));
    assert_eq!(log[1].cells, CellRect::new(50, 33, 1, 1));
    let c = grid(&store, t);
    assert_eq!((&c.solid, &c.base_levels), (&g.solid, &g.base));
    assert_eq!((&c.opacity, &c.edges), (&g.opacity, &g.edges));
    assert_eq!(c.max_level, 6);

    // The stored grid equals one built from the same desc (derived counts included).
    let mut fresh = ObstacleStore::new();
    let f = fresh.create_tile_grid(&g.desc()).unwrap();
    assert_eq!(grid(&store, t), grid(&fresh, f));
    assert_eq!(store.any_partial(), fresh.any_partial());

    // One cell in each of 12 of the 6 x 3 blocks: one rectangle each.
    let before = store.cell_version();
    for by in 0..3 {
        for bx in 0..4 {
            g.solid[(by * 16 + 1) * COLS + bx * 16 + 2] ^= 1;
        }
    }
    store.set_tile_grid_compare(t, &g.desc()).unwrap();
    let log = changes(&store, before);
    assert_eq!(log.len(), 12);
    assert_eq!((log[0].cells.x0, log[0].cells.y0), (2, 1));
    assert_eq!((log[11].cells.x0, log[11].cells.y0), (50, 33));
    // All 18 blocks: more than 16, so one rectangle around all changes.
    let many = store.cell_version();
    for by in 0..3 {
        for bx in 0..6 {
            g.opacity[(by * 16 + 2 + bx) * COLS + bx * 16 + 3] ^= 1;
        }
    }
    store.set_tile_grid_compare(t, &g.desc()).unwrap();
    let log = changes(&store, many);
    assert_eq!(log.len(), 1);
    assert_eq!(
        log[0].cells,
        CellRect {
            x0: 3,
            y0: 2,
            x1: 84,
            y1: 40
        }
    );
    assert_eq!(grid(&store, t).solid, g.solid);
    assert_eq!(store.get(t).unwrap().revision, revision);
}

#[test]
fn compare_replaces_the_grid_as_before_for_anything_but_cells() {
    let g = Compared::new();
    let replaced = |edit: &dyn Fn(&mut TileGridDesc<'_>)| {
        let mut store = ObstacleStore::new();
        let t = store.create_tile_grid(&g.desc()).unwrap();
        let revision = store.get(t).unwrap().revision;
        let static_version = store.static_version();
        let mut next = g.desc();
        edit(&mut next);
        store.set_tile_grid_compare(t, &next).unwrap();
        assert_eq!(store.cell_version(), 0);
        store.get(t).unwrap().revision == revision + 1 && store.static_version() != static_version
    };
    use knorke_math::{At, Vec2};
    assert!(replaced(
        &|d| d.common.position = At::new(Vec2::new(3.0, 0.0))
    ));
    assert!(replaced(&|d| d.common.rotation = 10.0));
    assert!(replaced(&|d| d.common.opacity = 0.5));
    assert!(replaced(&|d| d.level_height = 8.0));
    assert!(replaced(&|d| d.edge_width = 3.0));
    assert!(replaced(&|d| d.cell_size = Vec2::new(1.0, 2.0)));
    assert!(replaced(&|d| d.common.enabled = false));
    assert!(replaced(&|d| d.opacity = None)); // a channel goes away

    // Another size.
    static BIGGER: [u8; 97 * 48] = [0; 97 * 48];
    assert!(replaced(&|d| {
        d.columns = 97;
        d.solid = &BIGGER;
        d.base_levels = None;
        d.opacity = None;
        d.edges = None;
    }));

    // An invalid desc changes nothing; a shape is replaced as by set_tile_grid.
    let mut store = ObstacleStore::new();
    let t = store.create_tile_grid(&g.desc()).unwrap();
    let mut wrong = g.base.clone();
    wrong[0] = 200; // above its top
    let mut bad = g.desc();
    bad.base_levels = Some(&wrong);
    assert!(store.set_tile_grid_compare(t, &bad).is_err());
    assert_eq!(store.cell_version(), 0);
    assert_eq!(grid(&store, t).base_levels, g.base);
    let shape = store.create_shape(&ShapeDesc::circle(1.0)).unwrap();
    store.set_tile_grid_compare(shape, &g.desc()).unwrap();
    assert!(store.get(shape).unwrap().is_tile_grid());
    assert_eq!(store.cell_version(), 0);
}

#[test]
fn the_cell_log_is_part_of_the_snapshot() {
    let mut store = ObstacleStore::new();
    let g = CellGrid::new();
    let t = store.create_tile_grid(&g.desc()).unwrap();
    for i in 0..5 {
        store
            .set_tile_cells(t, CellRect::new(i % 4, 0, 1, 1), Some(&[3]), None, None)
            .unwrap();
    }
    let mut buf = SnapshotBuf::new();
    buf.save(&store);

    let mut other = ObstacleStore::new();
    restore(&mut Reader::new(buf.bytes()), &mut other).unwrap();
    assert_eq!(other.cell_version(), 5);
    assert_eq!(changes(&other, 0), changes(&store, 0));
    assert_eq!(other.get(t), store.get(t));
    // The same bytes again, and the restored store goes on logging at the same version.
    let mut again = SnapshotBuf::new();
    again.save(&other);
    assert_eq!(again.bytes(), buf.bytes());
    other
        .set_tile_cells(t, CellRect::new(0, 1, 1, 1), Some(&[3]), None, None)
        .unwrap();
    assert_eq!(other.cell_version(), 6);

    // Version 1 of the section (before cell logs) is refused, not misread.
    let r = Snapshot::restore(&mut other, 1, &mut Reader::new(&[]));
    assert!(matches!(r, Err(SnapshotError::UnsupportedVersion { .. })));
}
