// Origin: chiaro @ 9bc5b9b, src/shadows/TileGrid.hpp and TileGrid.cpp (cell editing, the cell log,
// compared grids); the creation part of the cells is in `stored.rs` (P13).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Editing the cells of a tile grid and the log of cell changes (P14).
//!
//! Redesign against the C++ version: `SDL_Rect` (signed, checked at run time) becomes the
//! half-open [`CellRect`]; "channel pointer or null" becomes `Option<&[u8]>` with a checked
//! length; errors are typed ([`ObstacleError`]) instead of SDL error strings. The cell log is a
//! fixed ring of [`CellLog::SIZE`] entries and never allocates; it is part of the store snapshot.
//!
//! A cell edit is *not* a store change in the sense of `revision`/`dirty_*`: like in Chiaro, a
//! consumer (the shadow cache, P16) follows the grid through [`CellLog`] and redraws only the
//! rectangle. Replacing the whole grid (`set_tile_grid`) is a normal change.

use super::desc::{ObstacleCommon, TileGridDesc};
use super::error::ObstacleError;
use super::store::{Arena, ObstacleId};
use super::stored::TileCells;
use knorke_types::{Reader, SnapshotError, Writer};

/// A rectangle of cells of a tile grid, half-open: `[x0, x1) x [y0, y1)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellRect {
    /// First column.
    pub x0: u32,
    /// First row.
    pub y0: u32,
    /// One past the last column.
    pub x1: u32,
    /// One past the last row.
    pub y1: u32,
}

impl CellRect {
    /// A rectangle from its corner and size.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x0: x,
            y0: y,
            x1: x.saturating_add(width),
            y1: y.saturating_add(height),
        }
    }

    /// Columns.
    pub const fn width(&self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }

    /// Rows.
    pub const fn height(&self) -> u32 {
        self.y1.saturating_sub(self.y0)
    }

    /// Number of cells.
    pub const fn area(&self) -> usize {
        self.width() as usize * self.height() as usize
    }

    /// The grid index of the `i`-th cell (row-major) of the rectangle.
    pub(super) fn cell_index(&self, columns: u32, i: usize) -> usize {
        let w = self.width() as usize;
        (self.y0 as usize + i / w) * columns as usize + self.x0 as usize + i % w
    }
}

/// One cell change of a grid, as the cell log keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellChange {
    /// The grid that changed.
    pub grid: ObstacleId,
    /// The cells that changed.
    pub cells: CellRect,
}

/// The last [`CellLog::SIZE`] cell changes of a store, numbered by a version that rises with every
/// one. A consumer remembers the version it has seen and asks for everything after it.
#[derive(Clone, Debug)]
pub struct CellLog {
    /// Change `v` is at `v % SIZE`.
    log: [Option<CellChange>; CellLog::SIZE as usize],
    version: u64,
}

impl Default for CellLog {
    fn default() -> Self {
        Self {
            log: [None; CellLog::SIZE as usize],
            version: 0,
        }
    }
}

impl CellLog {
    /// Changes the log keeps.
    pub const SIZE: u64 = 64;

    /// Number of changes so far.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub(super) fn push(&mut self, grid: ObstacleId, cells: CellRect) {
        self.version += 1;
        self.log[(self.version % Self::SIZE) as usize] = Some(CellChange { grid, cells });
    }

    /// Calls `f` for every change after the log had version `since`, oldest first. Returns
    /// `false` (and calls nothing) if some of them are no longer in the log; the consumer then
    /// has to treat every grid as changed.
    pub fn for_each(&self, since: u64, mut f: impl FnMut(&CellChange)) -> bool {
        if self.version.saturating_sub(since) > Self::SIZE {
            return false;
        }
        for v in since.saturating_add(1)..=self.version {
            if let Some(c) = &self.log[(v % Self::SIZE) as usize] {
                f(c);
            }
        }
        true
    }

    pub(super) fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.version);
        for e in &self.log {
            match e {
                Some(c) => {
                    w.put(&c.grid.to_bits());
                    for v in [c.cells.x0, c.cells.y0, c.cells.x1, c.cells.y1] {
                        w.put(&v);
                    }
                }
                None => {
                    w.put(&0u64);
                    for _ in 0..4 {
                        w.put(&0u32);
                    }
                }
            }
        }
    }

    /// Reads a log saved by [`save`](Self::save); allocates nothing.
    pub(super) fn restore(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        let version: u64 = r.get()?;
        for slot in &mut self.log {
            let bits: u64 = r.get()?;
            let cells = CellRect {
                x0: r.get()?,
                y0: r.get()?,
                x1: r.get()?,
                y1: r.get()?,
            };
            *slot = if bits == 0 {
                None
            } else {
                let grid =
                    ObstacleId::from_bits(bits).ok_or(SnapshotError::Invalid("cell log handle"))?;
                Some(CellChange { grid, cells })
            };
        }
        self.version = version;
        Ok(())
    }
}

/// Most rectangles one compared `set_tile_grid_compare` logs (one per changed 16 x 16 block;
/// more: one around all).
pub const MAX_COMPARE_RECTS: usize = 16;
/// Side of the blocks a compared grid replacement groups changed cells by.
pub const COMPARE_BLOCK: u32 = 16;

/// New bytes for the cells of a rectangle: rows of `width` bytes, `pitch` bytes apart; `None`
/// leaves a channel as it is.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct CellChannels<'a> {
    pub solid: Option<&'a [u8]>,
    pub base_levels: Option<&'a [u8]>,
    pub opacity: Option<&'a [u8]>,
    pub edges: Option<&'a [u8]>,
    pub pitch: usize,
}

fn length(field: &'static str, expected: usize, got: usize) -> Result<(), ObstacleError> {
    if expected == got {
        Ok(())
    } else {
        Err(ObstacleError::ChannelLength {
            field,
            expected,
            got,
        })
    }
}

impl TileCells {
    /// `true` if the rectangle is not empty and lies inside the grid.
    pub fn contains_rect(&self, r: &CellRect) -> bool {
        r.x1 > r.x0 && r.y1 > r.y0 && r.x1 <= self.columns && r.y1 <= self.rows
    }

    pub(super) fn check_rect(&self, r: &CellRect) -> Result<(), ObstacleError> {
        if self.contains_rect(r) {
            Ok(())
        } else {
            Err(ObstacleError::CellRect {
                rect: *r,
                columns: self.columns,
                rows: self.rows,
            })
        }
    }

    /// Checks the new bytes of `set_tile_cells` against the grid: lengths, that edges need an
    /// edge width and only the bits 1, 2, 4, 8, and that a new top stays above the stored base
    /// level. Absent channels are not checked.
    pub(super) fn check_cell_edit(
        &self,
        bytes: &Arena<u8>,
        r: &CellRect,
        solid: Option<&[u8]>,
        opacity: Option<&[u8]>,
        edges: Option<&[u8]>,
    ) -> Result<(), ObstacleError> {
        let n = r.area();
        for (field, c) in [("solid", solid), ("opacity", opacity), ("edges", edges)] {
            if let Some(c) = c {
                length(field, n, c.len())?;
            }
        }
        if edges.is_some() && !(self.edge_width > 0.0) {
            return Err(ObstacleError::Unsupported(
                "tile grid edges need edge_width > 0",
            ));
        }
        if let Some(e) = edges
            && let Some(i) = e.iter().position(|&b| b > 15)
        {
            return Err(ObstacleError::CellValue {
                field: "edges",
                cell: i,
                value: e[i],
                rule: "may only use the bits 1, 2, 4 and 8",
            });
        }
        let base = bytes.slice(self.base_levels);
        if let (Some(s), false) = (solid, base.is_empty()) {
            for (i, &top) in s.iter().enumerate() {
                if top != 0 && base[r.cell_index(self.columns, i)] >= top {
                    return Err(ObstacleError::CellValue {
                        field: "solid",
                        cell: i,
                        value: top,
                        rule: "must stay above the stored base level",
                    });
                }
            }
        }
        Ok(())
    }

    /// The same for `set_tile_cell_levels`: both channels together, every bottom below its
    /// non-zero top.
    pub(super) fn check_level_edit(
        &self,
        bytes: &Arena<u8>,
        r: &CellRect,
        solid: Option<&[u8]>,
        base_levels: Option<&[u8]>,
    ) -> Result<(), ObstacleError> {
        let n = r.area();
        for (field, c) in [("solid", solid), ("base_levels", base_levels)] {
            if let Some(c) = c {
                length(field, n, c.len())?;
            }
        }
        if base_levels.is_some() && !(self.level_height > 0.0) {
            return Err(ObstacleError::Unsupported(
                "tile grid base_levels need level_height > 0",
            ));
        }
        let (stored_top, stored_base) = (bytes.slice(self.solid), bytes.slice(self.base_levels));
        for i in 0..n {
            let cell = r.cell_index(self.columns, i);
            let top = solid.map_or(stored_top[cell], |s| s[i]);
            let bottom =
                base_levels.map_or_else(|| stored_base.get(cell).copied().unwrap_or(0), |b| b[i]);
            if top != 0 && bottom >= top {
                return Err(ObstacleError::CellValue {
                    field: "base_levels",
                    cell: i,
                    value: bottom,
                    rule: "must be below the top level",
                });
            }
        }
        Ok(())
    }

    /// Writes the channels into the validated rectangle (a channel the grid does not have yet is
    /// created with its default, which appends to the arena) and keeps the derived counts up to
    /// date.
    pub(super) fn write_cells(
        &mut self,
        bytes: &mut Arena<u8>,
        r: &CellRect,
        input: &CellChannels<'_>,
    ) {
        let total = self.columns as usize * self.rows as usize;
        if input.base_levels.is_some() && self.base_levels.is_empty() {
            self.base_levels = bytes.push_fill(total, 0); // every cell starts at the ground
        }
        if input.opacity.is_some() && self.opacity.is_empty() {
            self.opacity = bytes.push_fill(total, 255); // the same as no channel
        }
        if input.edges.is_some() && self.edges.is_empty() {
            self.edges = bytes.push_fill(total, 0); // whole cells
        }
        let columns = self.columns as usize;
        let (w, h) = (r.width() as usize, r.height() as usize);
        let cell_of = |y: usize, x: usize| (r.y0 as usize + y) * columns + r.x0 as usize + x;
        // Take the old cells out of the counts, write, count the new ones.
        self.count_rect(bytes, r, false);
        let mut recount_top = false;
        let channels = [
            (self.solid, input.solid),
            (self.base_levels, input.base_levels),
            (self.opacity, input.opacity),
            (self.edges, input.edges),
        ];
        for (k, (span, new)) in channels.into_iter().enumerate() {
            let Some(new) = new else { continue };
            let dst = bytes.slice_mut(span);
            for y in 0..h {
                for x in 0..w {
                    let (cell, v) = (cell_of(y, x), new[y * input.pitch + x]);
                    if k == 0 {
                        recount_top |= dst[cell] == self.max_level && v < self.max_level;
                        self.max_level = self.max_level.max(v);
                    }
                    dst[cell] = v;
                }
            }
        }
        self.count_rect(bytes, r, true);
        if recount_top {
            self.max_level = bytes.slice(self.solid).iter().copied().max().unwrap_or(0);
        }
    }

    /// Adds (`add`) or removes the solid cells of a rectangle with partial opacity and with edge
    /// bits to or from the counts.
    fn count_rect(&mut self, bytes: &Arena<u8>, r: &CellRect, add: bool) {
        let (solid, opacity, edges) = (
            bytes.slice(self.solid),
            bytes.slice(self.opacity),
            bytes.slice(self.edges),
        );
        let columns = self.columns as usize;
        let (mut partial, mut edge) = (0u32, 0u32);
        for y in r.y0 as usize..r.y1 as usize {
            for cell in y * columns + r.x0 as usize..y * columns + r.x1 as usize {
                if solid[cell] != 0 {
                    partial += u32::from(opacity.get(cell).is_some_and(|&o| o < 255));
                    edge += u32::from(edges.get(cell).is_some_and(|&e| e != 0));
                }
            }
        }
        if add {
            self.partial_cells += partial;
            self.edge_cells += edge;
        } else {
            self.partial_cells -= partial;
            self.edge_cells -= edge;
        }
    }
}

/// `true` if a validated desc differs from the stored obstacle in nothing but the bytes of its
/// cell channels (compared grid replacement). Floats compare by bits, so `-0` against `0` counts
/// as a change.
pub(super) fn same_grid_but_cells(
    common: &ObstacleCommon,
    g: &TileCells,
    d: &TileGridDesc<'_>,
) -> bool {
    fn same(a: f32, b: f32) -> bool {
        a.to_bits() == b.to_bits()
    }
    let (p, q) = (common, &d.common);
    let (pp, qp) = (p.position.raw(), q.position.raw());
    same(pp.x, qp.x)
        && same(pp.y, qp.y)
        && same(p.rotation, q.rotation)
        && same(p.height, q.height)
        && same(p.base_height, q.base_height)
        && same(p.opacity, q.opacity)
        && p.dynamic == q.dynamic
        && p.enabled == q.enabled
        && p.shadow_footprint == q.shadow_footprint
        && g.columns == d.columns
        && g.rows == d.rows
        && same(g.cell_size.x, d.cell_size.x)
        && same(g.cell_size.y, d.cell_size.y)
        && same(g.level_height, d.level_height)
        && same(g.edge_width, d.edge_width)
        && g.base_levels.is_empty() == d.base_levels.is_none()
        && g.opacity.is_empty() == d.opacity.is_none()
        && g.edges.is_empty() == d.edges.is_none()
}

/// The rectangles of cells whose bytes differ from the desc (same grid, see
/// [`same_grid_but_cells`]): one per changed block, in row-major block order; past
/// [`MAX_COMPARE_RECTS`] one around all of them. Returns how many are in `rects`. `row` is
/// working memory that is reused.
pub(super) fn changed_cell_rects(
    g: &TileCells,
    bytes: &Arena<u8>,
    d: &TileGridDesc<'_>,
    rects: &mut [CellRect; MAX_COMPARE_RECTS],
    row: &mut Vec<CellRect>,
) -> usize {
    let columns = g.columns as usize;
    // The channels as (stored, new) pairs; a channel the grid does not have is absent in the desc too.
    let channels: [(&[u8], Option<&[u8]>); 4] = [
        (bytes.slice(g.solid), Some(d.solid)),
        (bytes.slice(g.base_levels), d.base_levels),
        (bytes.slice(g.opacity), d.opacity),
        (bytes.slice(g.edges), d.edges),
    ];
    let row_differs = |start: usize| {
        channels.iter().any(|(old, new)| {
            new.is_some_and(|n| old[start..start + columns] != n[start..start + columns])
        })
    };
    let cell_differs = |i: usize| {
        channels
            .iter()
            .any(|(old, new)| new.is_some_and(|n| old[i] != n[i]))
    };

    const B: u32 = COMPARE_BLOCK;
    let none = CellRect {
        x0: g.columns,
        y0: g.rows,
        x1: 0,
        y1: 0,
    };
    let grow = |r: CellRect, q: CellRect| CellRect {
        x0: r.x0.min(q.x0),
        y0: r.y0.min(q.y0),
        x1: r.x1.max(q.x1),
        y1: r.y1.max(q.y1),
    };
    let mut count = 0;
    let mut all = none;
    let mut overflow = false;
    let blocks = g.columns.div_ceil(B) as usize;
    let mut y0 = 0;
    while y0 < g.rows {
        let y1 = g.rows.min(y0 + B);
        let mut any = false;
        for y in y0..y1 {
            let start = y as usize * columns;
            if !row_differs(start) {
                continue;
            }
            if !any {
                row.clear();
                row.resize(blocks, none);
                any = true;
            }
            for x in 0..g.columns {
                if cell_differs(start + x as usize) {
                    let b = (x / B) as usize;
                    row[b] = grow(
                        row[b],
                        CellRect {
                            x0: x,
                            y0: y,
                            x1: x + 1,
                            y1: y + 1,
                        },
                    );
                }
            }
        }
        if any {
            for &block in row.iter() {
                if block.x1 == 0 {
                    continue; // unchanged block
                }
                all = grow(all, block);
                overflow |= count == MAX_COMPARE_RECTS;
                if !overflow {
                    rects[count] = block;
                    count += 1;
                }
            }
        }
        y0 += B;
    }
    if overflow {
        rects[0] = all;
        count = 1;
    }
    count
}
