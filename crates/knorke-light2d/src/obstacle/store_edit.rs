// Origin: chiaro @ 9bc5b9b, src/shadows/ObstacleStore.cpp (SetTileCells, SetTileCellLevels,
// ApplyTileGrid with compareCells, SetPattern(s), SetTilePatternTable, SetTilePatternCells,
// OnPatternsDestroyed, CountDestroyedPatternUsers) and the matching parts of ObstacleStore.hpp.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Cell and pattern operations of the [`ObstacleStore`] (P14).
//!
//! - Cell edits (`set_tile_cells`, `set_tile_cell_levels`, a compared `set_tile_grid_compare`)
//!   change bytes in place and go into the [`CellChangeLog`], not into the revision or the dirty sets
//!   (see the cell module docs).
//! - A pattern assignment (`set_pattern`, `set_tile_pattern_table`) is a change of the obstacle
//!   like `set_shape`: revision and dirty tracking. `set_tile_pattern_cells` only logs the cells.
//! - The pattern store of P15 is reached through [`PatternSource`]; after it destroyed patterns
//!   the host calls [`on_patterns_destroyed`](ObstacleStore::on_patterns_destroyed).
//!
//! The channels, the pattern table and the pattern cell indices live in the byte and word arenas
//! of the store; [`ObstacleStore::tile`] hands them out as slices. Steady state: all of this
//! allocates nothing once a grid has the channels it is edited with (the first edit of a channel
//! it does not have yet appends it to the arena).

use super::cells::{
    CellChangeLog, CellChannels, CellRect, MAX_COMPARE_RECTS, changed_cell_rects,
    same_grid_but_cells,
};
use super::desc::TileGridDesc;
use super::error::ObstacleError;
use super::patterns::{
    PatternSource, cells_of_destroyed, check_assignment, check_table, mask_opacity,
};
use super::store::{Arena, ObstacleId, ObstacleStore, batch, invalid, partial_active, same_length};
use super::stored::{StoredObstacle, TileCells};
use knorke_types::{Column, Slots};

/// The cell log and the working memory of cell and pattern operations.
#[derive(Clone, Debug, Default)]
pub(super) struct EditState {
    pub log: CellChangeLog,
    /// Compared grid: the blocks of one block row.
    pub row: Vec<CellRect>,
    /// `on_patterns_destroyed`: the obstacles that name a destroyed pattern.
    affected: Vec<ObstacleId>,
}

/// A tile grid with its channels and pattern data as slices; empty slices are channels the grid
/// does not have.
#[derive(Clone, Copy, Debug)]
pub struct TileView<'a> {
    /// Size, counts and the rest of the grid.
    pub grid: &'a TileCells,
    /// Top level per cell.
    pub solid: &'a [u8],
    /// Bottom level per cell, or empty.
    pub base_levels: &'a [u8],
    /// Opacity per cell (255 opaque), or empty.
    pub opacity: &'a [u8],
    /// Edge bits per cell, or empty.
    pub edges: &'a [u8],
    /// The grid's own pattern (`0` none).
    pub pattern: u32,
    /// The pattern table, cell index `k` is entry `k - 1`.
    pub pattern_table: &'a [u32],
    /// Table index per cell, or empty (all `0`).
    pub pattern_cells: &'a [u8],
}

/// Views are equal when the cells, counts and patterns they show are (not where the store keeps
/// them).
impl PartialEq for TileView<'_> {
    fn eq(&self, o: &Self) -> bool {
        (self.grid.columns, self.grid.rows) == (o.grid.columns, o.grid.rows)
            && (
                self.grid.max_level,
                self.grid.partial_cells,
                self.grid.edge_cells,
            ) == (o.grid.max_level, o.grid.partial_cells, o.grid.edge_cells)
            && self.solid == o.solid
            && self.base_levels == o.base_levels
            && self.opacity == o.opacity
            && self.edges == o.edges
            && self.pattern == o.pattern
            && self.pattern_table == o.pattern_table
            && self.pattern_cells == o.pattern_cells
    }
}

/// The tile grid behind a handle: an error for a stale handle or another kind.
fn grid_of(
    obstacles: &mut Slots<StoredObstacle>,
    id: ObstacleId,
) -> Result<&mut StoredObstacle, ObstacleError> {
    let s = obstacles.get_mut(id).ok_or_else(|| invalid(id))?;
    if s.is_tile_grid() {
        Ok(s)
    } else {
        Err(ObstacleError::NotTileGrid(id.to_bits()))
    }
}

impl crate::shadow::CellLog for ObstacleStore {
    fn cell_version(&self) -> u64 {
        self.edit.log.version()
    }

    fn for_each_change_since(&self, version: u64, f: &mut dyn FnMut(ObstacleId, CellRect)) -> bool {
        self.edit.log.for_each(version, |c| f(c.grid, c.cells))
    }
}

impl ObstacleStore {
    /// The log of cell changes, for consumers that follow single grids (the shadow cache).
    pub fn cell_log(&self) -> &CellChangeLog {
        &self.edit.log
    }

    /// Number of cell changes so far; shortcut for `cell_log().version()`.
    pub fn cell_version(&self) -> u64 {
        self.edit.log.version()
    }

    /// The channels and patterns of a tile grid; `None` for a stale handle or another kind.
    pub fn tile(&self, id: ObstacleId) -> Option<TileView<'_>> {
        let s = self.obstacles.get(id).filter(|s| s.is_tile_grid())?;
        Some(TileView {
            grid: &s.grid,
            solid: self.bytes.slice(s.grid.solid),
            base_levels: self.bytes.slice(s.grid.base_levels),
            opacity: self.bytes.slice(s.grid.opacity),
            edges: self.bytes.slice(s.grid.edges),
            pattern: s.patterns.pattern,
            pattern_table: self.words.slice(s.patterns.table),
            pattern_cells: self.bytes.slice(s.patterns.cells),
        })
    }

    /// Runs `change` on an obstacle that exists and keeps the partial counter up to date.
    fn retally<R>(
        &mut self,
        id: ObstacleId,
        change: impl FnOnce(&mut StoredObstacle, &Arena<u8>, &Arena<u32>) -> R,
    ) -> R {
        let s = self
            .obstacles
            .get_mut(id)
            .expect("callers checked the handle");
        let before = partial_active(s);
        let out = change(s, &self.bytes, &self.words);
        self.partial_active = self.partial_active - before + partial_active(s);
        out
    }

    /// A change of the obstacle like `set_shape`: its revision grows and the dirty tracking sees
    /// it.
    fn mark_changed(&mut self, id: ObstacleId) {
        let s = self
            .obstacles
            .get_mut(id)
            .expect("callers checked the handle");
        s.revision = s.revision.wrapping_add(1);
        let dynamic = s.common.dynamic;
        self.note_change(id.index(), dynamic);
    }

    // ---- cells ----

    /// Writes new bytes into the cells of `cells` (row-major, `width * height` bytes per
    /// channel; `None` leaves a channel). Edges need an `edge_width`; a new top must stay above
    /// the stored base level. Goes into the cell log. Unchanged on error.
    ///
    /// # Errors
    /// Stale handle, not a tile grid, a rectangle that is empty or outside the grid, a channel of
    /// the wrong length, or a value the grid cannot use.
    pub fn set_tile_cells(
        &mut self,
        id: ObstacleId,
        cells: CellRect,
        solid: Option<&[u8]>,
        opacity: Option<&[u8]>,
        edges: Option<&[u8]>,
    ) -> Result<(), ObstacleError> {
        let grid = grid_of(&mut self.obstacles, id)?.grid;
        grid.check_rect(&cells)?;
        if solid.is_none() && opacity.is_none() && edges.is_none() {
            return Ok(());
        }
        grid.check_cell_edit(&self.bytes, &cells, solid, opacity, edges)?;
        let input = CellChannels {
            solid,
            opacity,
            edges,
            base_levels: None,
            pitch: cells.width() as usize,
        };
        self.edit_cells(id, cells, &input);
        Ok(())
    }

    /// Changes the top and bottom levels of the cells of `cells`: both channels are checked
    /// together (every bottom below its non-zero top). A `base_levels` channel the grid does not
    /// have yet is created (0 elsewhere); it needs a level height. Goes into the cell log.
    ///
    /// # Errors
    /// As [`set_tile_cells`](Self::set_tile_cells).
    pub fn set_tile_cell_levels(
        &mut self,
        id: ObstacleId,
        cells: CellRect,
        solid: Option<&[u8]>,
        base_levels: Option<&[u8]>,
    ) -> Result<(), ObstacleError> {
        let grid = grid_of(&mut self.obstacles, id)?.grid;
        grid.check_rect(&cells)?;
        if solid.is_none() && base_levels.is_none() {
            return Ok(());
        }
        grid.check_level_edit(&self.bytes, &cells, solid, base_levels)?;
        let input = CellChannels {
            solid,
            base_levels,
            opacity: None,
            edges: None,
            pitch: cells.width() as usize,
        };
        self.edit_cells(id, cells, &input);
        Ok(())
    }

    fn edit_cells(&mut self, id: ObstacleId, cells: CellRect, input: &CellChannels<'_>) {
        let s = self
            .obstacles
            .get_mut(id)
            .expect("callers checked the handle");
        let before = partial_active(s);
        s.grid.write_cells(&mut self.bytes, &cells, input);
        self.partial_active = self.partial_active - before + partial_active(s);
        self.edit.log.push(id, cells);
    }

    /// Replaces the tile grid like [`set_tile_grid`](Self::set_tile_grid), unless the obstacle
    /// is a grid and the description differs from it in nothing but cell bytes: then only the
    /// changed cells are written and logged (one rectangle per changed
    /// [`COMPARE_BLOCK`](super::COMPARE_BLOCK) square, at most [`MAX_COMPARE_RECTS`], past that
    /// one around all) and neither revision nor dirty tracking move. The comparison costs one
    /// pass over the cells, so it pays when a host re-sends a grid that mostly did not change.
    ///
    /// # Errors
    /// As [`set_tile_grid`](Self::set_tile_grid).
    pub fn set_tile_grid_compare(
        &mut self,
        id: ObstacleId,
        desc: &TileGridDesc<'_>,
    ) -> Result<(), ObstacleError> {
        let current = self.obstacles.get(id).ok_or_else(|| invalid(id))?;
        desc.validate()?;
        if !(current.is_tile_grid() && same_grid_but_cells(&current.common, &current.grid, desc)) {
            return self.set_tile_grid(id, desc);
        }
        let mut rects = [CellRect::default(); MAX_COMPARE_RECTS];
        let count = changed_cell_rects(
            &current.grid,
            &self.bytes,
            desc,
            &mut rects,
            &mut self.edit.row,
        );
        let columns = desc.columns as usize;
        for r in &rects[..count] {
            let first = r.y0 as usize * columns + r.x0 as usize;
            fn at(c: Option<&[u8]>, first: usize) -> Option<&[u8]> {
                c.map(|c| &c[first..])
            }
            let input = CellChannels {
                solid: at(Some(desc.solid), first),
                base_levels: at(desc.base_levels, first),
                opacity: at(desc.opacity, first),
                edges: at(desc.edges, first),
                pitch: columns,
            };
            self.edit_cells(id, *r, &input);
        }
        Ok(())
    }

    // ---- patterns ----

    /// Assigns a pattern (`0` = none) to a shape or to a tile grid (then to every cell the table
    /// does not override). The pattern must be live; a polygon with a pattern has at most
    /// [`MAX_PATTERN_VERTICES`](super::MAX_PATTERN_VERTICES) corners. A change counts like
    /// `set_shape`; assigning what is already assigned changes nothing.
    ///
    /// # Errors
    /// Stale handle, a pattern that is not live, or a polygon with too many corners.
    pub fn set_pattern(
        &mut self,
        id: ObstacleId,
        pattern: u32,
        patterns: &impl PatternSource,
    ) -> Result<(), ObstacleError> {
        let s = self.obstacles.get(id).ok_or_else(|| invalid(id))?;
        check_assignment(s, pattern, patterns)?;
        self.apply_pattern(id, pattern, patterns);
        Ok(())
    }

    fn apply_pattern(&mut self, id: ObstacleId, pattern: u32, patterns: &impl PatternSource) {
        if self
            .obstacles
            .get(id)
            .is_some_and(|s| s.patterns.pattern == pattern)
        {
            return;
        }
        self.retally(id, |s, _, words| {
            s.patterns.pattern = pattern;
            s.patterns.refs = s
                .patterns
                .count_live(words.slice(s.patterns.table), patterns);
        });
        self.mark_changed(id);
    }

    /// Assigns many patterns at once (`ids[i]` gets `patterns[i]`). Checks every element and
    /// that the obstacles are distinct first, then changes.
    ///
    /// # Errors
    /// Differing lengths, or [`ObstacleError::Batch`] naming the first bad element; nothing
    /// changes.
    pub fn set_patterns(
        &mut self,
        ids: impl Column<ObstacleId>,
        pattern_ids: impl Column<u32>,
        patterns: &impl PatternSource,
    ) -> Result<(), ObstacleError> {
        same_length("patterns", ids.len(), pattern_ids.len())?;
        for (i, (id, p)) in ids.iter().zip(pattern_ids.iter()).enumerate() {
            let Some(s) = self.obstacles.get(*id) else {
                return Err(batch("ids", i, invalid(*id)));
            };
            check_assignment(s, *p, patterns).map_err(|e| batch("patterns", i, e))?;
        }
        if let Some(index) = crate::first_repeat(&ids, &mut self.sorted) {
            return Err(batch(
                "ids",
                index,
                ObstacleError::Unsupported("an obstacle handle appears twice"),
            ));
        }
        for (id, p) in ids.iter().zip(pattern_ids.iter()) {
            self.apply_pattern(*id, *p, patterns);
        }
        Ok(())
    }

    /// Replaces the pattern table of a tile grid (at most
    /// [`MAX_PATTERN_TABLE`](super::MAX_PATTERN_TABLE) entries, each `0` = none or a live
    /// pattern). A change counts like `set_shape`.
    ///
    /// # Errors
    /// Stale handle, not a tile grid, too many entries, or an entry that is not live.
    pub fn set_tile_pattern_table(
        &mut self,
        id: ObstacleId,
        table: &[u32],
        patterns: &impl PatternSource,
    ) -> Result<(), ObstacleError> {
        let old = grid_of(&mut self.obstacles, id)?.patterns.table;
        check_table(table, patterns)?;
        let span = self.words.replace(old, table);
        self.retally(id, |s, _, words| {
            s.patterns.table = span;
            s.patterns.refs = s.patterns.count_live(words.slice(span), patterns);
        });
        self.mark_changed(id);
        self.compact_if_needed();
        Ok(())
    }

    /// Sets the table index of the cells of `cells` (`width * height` bytes, row-major): `0` =
    /// the grid's pattern, `k` = entry `k - 1` of the table. Goes into the cell log like
    /// [`set_tile_cells`](Self::set_tile_cells), so only the rectangle is redrawn; unchanged
    /// indices log nothing.
    ///
    /// # Errors
    /// Stale handle, not a tile grid, a bad rectangle or length, or an index above the table
    /// size.
    pub fn set_tile_pattern_cells(
        &mut self,
        id: ObstacleId,
        cells: CellRect,
        indices: &[u8],
    ) -> Result<(), ObstacleError> {
        let s = grid_of(&mut self.obstacles, id)?;
        s.grid.check_rect(&cells)?;
        if indices.len() != cells.area() {
            return Err(ObstacleError::ChannelLength {
                field: "indices",
                expected: cells.area(),
                got: indices.len(),
            });
        }
        let table = self.words.slice(s.patterns.table).len();
        if let Some(i) = indices.iter().position(|&k| usize::from(k) > table) {
            return Err(ObstacleError::PatternIndex {
                cell: i,
                index: indices[i],
                table,
            });
        }
        let columns = s.grid.columns;
        let stored = self.bytes.slice(s.patterns.cells);
        let differs = indices.iter().enumerate().any(|(i, &k)| {
            k != stored
                .get(cells.cell_index(columns, i))
                .copied()
                .unwrap_or(0)
        });
        if differs {
            if s.patterns.cells.is_empty() {
                let total = columns as usize * s.grid.rows as usize;
                s.patterns.cells = self.bytes.push_fill(total, 0);
            }
            let dst = self.bytes.slice_mut(s.patterns.cells);
            let w = cells.width() as usize;
            for (y, row) in indices.chunks_exact(w).enumerate() {
                let start = cells.cell_index(columns, y * w);
                dst[start..start + w].copy_from_slice(row);
            }
            self.edit.log.push(id, cells);
        }
        Ok(())
    }

    /// An active obstacle has a live pattern (patterns of destroyed obstacles and destroyed
    /// patterns do not count).
    pub fn any_pattern(&self) -> bool {
        self.iter()
            .any(|(_, s)| s.is_active() && s.patterns.refs > 0)
    }

    /// The value the obstacle writes to the mask: the scalar opacity (times the cell's), or with
    /// a live pattern in effect `clamp(mean * that, 1/255, 254/255)` (a value of 254.5/255 or
    /// more ends the ray before it reads the pattern, one that rounds to 0 drops the obstacle
    /// from the mask). A destroyed pattern gives the scalar. `cell_index` is
    /// `row * columns + column` and only used for tile grids. `None` for a stale handle.
    pub fn mask_opacity(
        &self,
        id: ObstacleId,
        cell_index: usize,
        patterns: &impl PatternSource,
    ) -> Option<f32> {
        let s = self.obstacles.get(id)?;
        Some(mask_opacity(
            s,
            self.bytes.slice(s.grid.opacity),
            self.words.slice(s.patterns.table),
            self.bytes.slice(s.patterns.cells),
            cell_index,
            patterns,
        ))
    }

    /// Active obstacles (all with `include_inactive`) whose pattern or table still names a
    /// destroyed pattern.
    pub fn count_destroyed_pattern_users(
        &self,
        patterns: &impl PatternSource,
        include_inactive: bool,
    ) -> u32 {
        let dead = |id: u32| patterns.mean_opacity(id).is_none();
        self.iter()
            .filter(|(_, s)| {
                (include_inactive || s.is_active())
                    && s.patterns.names(self.words.slice(s.patterns.table), dead)
            })
            .count() as u32
    }

    /// The host calls this after the pattern store destroyed the patterns `ids`. The assignments
    /// stay but act like none; every obstacle (and for a grid every cell) that loses its
    /// pattern changes like after `set_shape`: a shape or the grid's own pattern is a change of
    /// the obstacle, table entries only log the cells that point at them.
    pub fn on_patterns_destroyed(&mut self, ids: &[u32]) {
        let destroyed = |id: u32| ids.contains(&id);
        self.edit.affected.clear();
        for (handle, s) in self.obstacles.iter() {
            if s.patterns
                .names(self.words.slice(s.patterns.table), destroyed)
            {
                self.edit.affected.push(handle);
            }
        }
        for k in 0..self.edit.affected.len() {
            let handle = self.edit.affected[k];
            let (lost, whole, rect) = {
                let s = self.obstacles.get(handle).expect("collected above");
                let table = self.words.slice(s.patterns.table);
                let whole =
                    !s.is_tile_grid() || (s.patterns.pattern != 0 && destroyed(s.patterns.pattern));
                let rect = if whole {
                    None
                } else {
                    cells_of_destroyed(
                        self.bytes.slice(s.patterns.cells),
                        table,
                        s.grid.columns,
                        ids,
                    )
                };
                (s.patterns.count_in(table, ids), whole, rect)
            };
            self.retally(handle, |s, _, _| {
                s.patterns.refs -= lost.min(s.patterns.refs);
            });
            if whole {
                self.mark_changed(handle); // every cell without an own index lost its pattern
            } else if let Some(rect) = rect {
                self.edit.log.push(handle, rect); // only table entries: the cells that point at one
            }
        }
    }
}
