// Origin: chiaro @ 9bc5b9b, src/rendering/TileOccluders.cpp (PrepareTextures: which grids have a
// texture, what is pending, the uniforms, the cell texel values) and TileOccluders.hpp
// (TileTexture).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The CPU half of the tile grid stage: which grids have a cell texture, which cells of it are
//! stale, the cell texels and the uniforms of the draw. The texture itself is the generic `T`
//! (a GPU texture in [`TileOccluders`](super::TileOccluders), `()` in tests).

use super::params::{DomainParams, TileParams, obstacle_interval};
use crate::light_math::DEG_TO_RAD;
use crate::obstacle::{ObstacleId, ObstacleStore, PatternSource, StoredObstacle};
use crate::shadow::CellRect;
use knorke_math::det;
use knorke_math::uniform::write_f32s;

/// Bytes of one cell in the cell texture (`R8G8B8A8_UNORM`): r the raw solid value (non-zero is
/// solid), g the bottom level, b the cell opacity times the obstacle's, a the edge bits (255
/// for a whole cell).
pub const CELL_BYTES: usize = 4;

/// What the draw of one grid pushes: the vertex block placing the quad and the fragment block
/// of `tile.frag`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TileUniforms {
    /// xy world position, z rotation in radians.
    pub placement: [f32; 4],
    /// xy cell size, zw columns and rows.
    pub grid_size: [f32; 4],
    /// x cell height (0: use the obstacle's interval), y top, z -base, w 1 if the height is
    /// finite; then the edge strips of `tile_strip.hlsli`: xy width as a share of the cell per
    /// axis, zw columns and rows (all 0 for whole cells).
    pub heights: [f32; 8],
}

impl TileUniforms {
    /// The uniforms of a tile grid.
    pub fn of(o: &StoredObstacle) -> Self {
        let g = &o.grid;
        let p = o.common.position.raw();
        let [neg_base, top] = obstacle_interval(&o.common);
        let strips = !g.edges.is_empty();
        let (strip_x, strip_y) = if strips {
            (
                det::min(g.edge_width / g.cell_size.x, 1.0),
                det::min(g.edge_width / g.cell_size.y, 1.0),
            )
        } else {
            (0.0, 0.0)
        };
        let finite = if o.has_finite_height() { 1.0 } else { 0.0 };
        Self {
            placement: [p.x, p.y, o.common.rotation * DEG_TO_RAD, 0.0],
            grid_size: [
                g.cell_size.x,
                g.cell_size.y,
                g.columns as f32,
                g.rows as f32,
            ],
            heights: [
                g.level_height,
                top,
                neg_base,
                finite,
                strip_x,
                strip_y,
                if strips { g.columns as f32 } else { 0.0 },
                if strips { g.rows as f32 } else { 0.0 },
            ],
        }
    }

    /// The vertex block of `tile.vert`.
    pub fn params(&self, domain: &DomainParams) -> TileParams {
        TileParams {
            domain: *domain,
            placement: self.placement,
            grid_size: self.grid_size,
        }
    }

    /// The fragment block of `tile.frag` (little endian).
    pub fn heights_bytes(&self) -> [u8; 32] {
        let mut out = [0; 32];
        write_f32s(&mut out, &self.heights);
        out
    }
}

/// One tile grid with a cell texture, kept across frames.
#[derive(Clone, Debug)]
pub struct TileSlot<T> {
    /// The grid.
    pub handle: ObstacleId,
    /// [`StoredObstacle::revision`] the texture holds.
    pub revision: u32,
    /// Columns of the texture.
    pub columns: u32,
    /// Rows of the texture.
    pub rows: u32,
    /// The cell texture; `None` until it is created.
    pub texture: Option<T>,
    /// The cell log version the texture (with its pending upload) holds.
    pub cell_version: u64,
    /// Cells still to upload: the bounding box of everything that changed since the last
    /// upload, all cells after a replacement.
    pub pending: Option<CellRect>,
    /// What the draw pushes.
    pub uniforms: TileUniforms,
    /// Byte offset of the pending cells in the stage's transfer buffer (set by the GPU stage).
    pub staging_offset: u32,
    seen: bool,
}

impl<T> TileSlot<T> {
    fn add_pending(&mut self, r: CellRect) {
        self.pending = Some(self.pending.map_or(r, |p| p.union(&r)));
    }
}

/// The active tile grids of a store with one cell texture each.
#[derive(Clone, Debug)]
pub struct TileSet<T> {
    slots: Vec<TileSlot<T>>,
}

impl<T> Default for TileSet<T> {
    fn default() -> Self {
        Self { slots: Vec::new() }
    }
}

impl<T> TileSet<T> {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// The grids, in the order they were first seen.
    pub fn slots(&self) -> &[TileSlot<T>] {
        &self.slots
    }

    /// Mutable access, for the upload bookkeeping of the GPU stage.
    pub fn slots_mut(&mut self) -> &mut [TileSlot<T>] {
        &mut self.slots
    }

    /// The slot of the grid `handle`.
    pub fn find(&self, handle: ObstacleId) -> Option<&TileSlot<T>> {
        self.slots.iter().find(|s| s.handle == handle)
    }

    /// Number of grids.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// `true` without grids.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Cells whose upload is still pending.
    pub fn any_pending(&self) -> bool {
        self.slots.iter().any(|s| s.pending.is_some())
    }

    /// Brings the slots in line with the active tile grids of `store`: a new grid gets a slot,
    /// a grid whose size changed a new texture (`create(columns, rows)`), a replaced grid
    /// (another revision) has every cell pending, a grid whose cells were edited only the
    /// bounding box of the logged changes (all cells if the log lost some); destroyed, disabled
    /// or retyped grids lose their slot, and with it the texture. The uniforms are refreshed
    /// for every grid.
    ///
    /// Pending cells stay pending until the stage clears them after the upload, even if a
    /// frame fails.
    ///
    /// # Errors
    /// The first error of `create`; the slots of the grids after it are kept as they were.
    pub fn sync<E>(
        &mut self,
        store: &ObstacleStore,
        mut create: impl FnMut(u32, u32) -> Result<T, E>,
    ) -> Result<(), E> {
        for s in &mut self.slots {
            s.seen = false;
        }
        for (handle, o) in store.iter() {
            if !o.is_tile_grid() || !o.is_active() {
                continue;
            }
            let index = match self.slots.iter().position(|s| s.handle == handle) {
                Some(i) => i,
                None => {
                    self.slots.push(TileSlot {
                        handle,
                        revision: 0,
                        columns: 0,
                        rows: 0,
                        texture: None,
                        cell_version: 0,
                        pending: None,
                        uniforms: TileUniforms::default(),
                        staging_offset: 0,
                        seen: false,
                    });
                    self.slots.len() - 1
                }
            };
            let slot = &mut self.slots[index];
            slot.seen = true;
            if slot.texture.is_none() || slot.columns != o.grid.columns || slot.rows != o.grid.rows
            {
                slot.texture = Some(create(o.grid.columns, o.grid.rows)?);
                slot.columns = o.grid.columns;
                slot.rows = o.grid.rows;
                slot.revision = 0; // forces the upload below
            }
            let all = CellRect::new(0, 0, o.grid.columns, o.grid.rows);
            if slot.revision != o.revision {
                slot.add_pending(all);
                slot.revision = o.revision;
            } else if slot.cell_version != store.cell_version() {
                // Edited cells: the bounding box of the changes of this grid.
                let mut changed = None::<CellRect>;
                let known = store.cell_log().for_each(slot.cell_version, |c| {
                    if c.grid == handle {
                        changed = Some(changed.map_or(c.cells, |p| p.union(&c.cells)));
                    }
                });
                if !known {
                    slot.add_pending(all);
                } else if let Some(r) = changed {
                    // A change outside the grid cannot happen; the bound keeps a stale log
                    // entry from writing past the texture.
                    let r = CellRect {
                        x1: r.x1.min(all.x1),
                        y1: r.y1.min(all.y1),
                        ..r
                    };
                    if r.x0 < r.x1 && r.y0 < r.y1 {
                        slot.add_pending(r);
                    }
                }
            }
            slot.cell_version = store.cell_version();
            slot.uniforms = TileUniforms::of(o);
        }
        self.slots.retain(|s| s.seen);
        Ok(())
    }
}

/// Writes the texels of the cells `rect` of the grid `id` (row after row, [`CELL_BYTES`] each)
/// into `out`, which holds at least `rect.area() * CELL_BYTES` bytes. r is the solid value, g
/// the bottom level, b the cell's opacity times the grid's (through
/// [`ObstacleStore::mask_opacity`] for a grid with patterns), a the edge bits or 255.
/// Does nothing for a stale handle or a rectangle outside the grid.
pub fn write_cell_texels(
    store: &ObstacleStore,
    id: ObstacleId,
    rect: &CellRect,
    patterns: &impl PatternSource,
    out: &mut [u8],
) {
    let (Some(view), Some(o)) = (store.tile(id), store.get(id)) else {
        return;
    };
    let columns = view.grid.columns as usize;
    if rect.x1 > view.grid.columns || rect.y1 > view.grid.rows || rect.area() == 0 {
        return;
    }
    let scale = o.common.opacity;
    let scaled = |v: u8| (f32::from(v) * scale).round() as u8;
    let uniform = scaled(255);
    let patterned = o.patterns.refs > 0;
    let mut at = 0;
    for y in rect.y0 as usize..rect.y1 as usize {
        for x in rect.x0 as usize..rect.x1 as usize {
            let i = y * columns + x;
            let opacity = if patterned {
                let m = store.mask_opacity(id, i, patterns).unwrap_or(scale);
                (255.0 * m).round() as u8
            } else if view.opacity.is_empty() {
                uniform
            } else {
                scaled(view.opacity[i])
            };
            let edges = view.edges.get(i).copied().unwrap_or(0);
            out[at] = view.solid[i];
            out[at + 1] = view.base_levels.get(i).copied().unwrap_or(0);
            out[at + 2] = opacity;
            out[at + 3] = if edges == 0 { 255 } else { edges };
            at += CELL_BYTES;
        }
    }
}
