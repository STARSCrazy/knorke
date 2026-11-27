// Origin: chiaro @ 9bc5b9b, src/rendering/GpuPatternObstacle.hpp (the records), the CPU parts of
// src/rendering/PatternOccluders.cpp (InDomain, MakePatternParams, BuildData, BuildIdTriangles and
// the records of Prepare) and of src/rendering/PatternAtlas.cpp (RowsFor, the texels and records
// of Prepare).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Pattern obstacles on the GPU (P28, Chiaro E127): what the row pattern pass and the row lights
//! read (`lightrow_pattern.hlsli`).
//!
//! The pattern obstacles in the mask's domain become one [`GpuPatternObstacle`] record each,
//! with their polygon corners and tile cells in one word buffer (`PatternData`); every live
//! pattern becomes one [`GpuPattern`] record and its alpha a region of the R8 pattern atlas
//! (alpha at the [`PatternStore`] place, the column means below the alpha band). An id texture
//! at mask size holds `1 + record index` per texel; the row pattern pass reads it, finds the
//! obstacle's record and intersects its geometry to sample the pattern there.
//!
//! | Module | Content |
//! |---|---|
//! | this one | records, uniform blocks, atlas bytes, the frame's pattern set, without a GPU |
//! | `gpu` (feature `gpu`) | [`PatternOccluders`]: atlas texture, buffers, id texture and its passes |
//!
//! # Redesign against the C++ version
//!
//! - The atlas texture is an [`AtlasTexture`](crate::atlas::AtlasTexture) like the profile and
//!   mask atlases. It is sized to [`PatternStore::atlas_rows`] of the current store; Chiaro
//!   only grew it until the last pattern was destroyed. A new size only follows a new store
//!   version (create or destroy), never a steady frame. The rows enter the sampling as
//!   `1 / height`, so only a store that shrank can differ, and nearest sampling of texel centres
//!   does not change with it (guess, not measured).
//! - Records follow the slot order of the stores (Chiaro: the dense order of its slot maps).
//!   Record indices are internal; only obstacles overlapping in the id texture could resolve
//!   differently ("last writer wins" in draw order).
//! - Rotations use `det::sin_cos` in `f32` like the obstacle triangles (Chiaro: `double`). The
//!   rotated box (30°) and polygon (17°) of `tests/gpu_patterns.rs` give mask and light buffer
//!   bit-identical to Chiaro; other angles may differ by one ulp in the record.
//! - Chiaro's more-than-65535 warning (a log line) is gone; the obstacles beyond keep their mean
//!   opacity without the pattern.

#[cfg(feature = "gpu")]
mod gpu;

#[cfg(feature = "gpu")]
pub use gpu::{
    PATTERN_ID_FORMAT, PATTERN_PROGRAMS, PatternMaps, PatternOccluders, PatternPrograms,
};

use crate::obstacle::{ObstacleId, ObstacleKind, StoredObstacle};
use crate::occluder_raster::widen_circle;
use crate::pattern_store::{ATLAS_SIZE, MAX_REGION, MEANS_PER_ROW, PatternStore};
use crate::shadow::{MaskAnchor, OcclusionDomain};
use crate::{ObstacleStore, Projection};
use knorke_math::{At, Vec2, World2, det, uniform_block};

/// Most pattern obstacles per frame: the id texture holds `1 + index` in 16 bits; the rest casts
/// its mean opacity without the pattern.
pub const MAX_PATTERN_OBSTACLES: u32 = 65535;
/// [`GpuPatternObstacle::kind`]`[1]` bit: `heights[0..1]` is a finite span; without it the
/// column mean is read (no `v`).
pub const PATTERN_FINITE_HEIGHT: u32 = 1;
/// [`GpuPatternObstacle::data`]`[3]` of a tile grid without cell indices.
pub const PATTERN_NO_INDICES: u32 = u32::MAX;

/// [`GpuPatternObstacle::kind`]`[0]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PatternKind {
    /// `shape[0..1]` half extents about the origin.
    Box = 0,
    /// `data[0]` first word of the corners, `data[1]` corner count.
    Polygon = 1,
    /// `shape[0]` radius about the origin.
    Circle = 2,
    /// Cells in `PatternData`.
    TileGrid = 3,
}

uniform_block! {
    /// One live pattern (`StructuredBuffer<GpuPattern> Patterns`, `pattern.hlsli`).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct GpuPattern[48] {
        /// Atlas texel x, y of the region's top-left, width, height.
        pub rect: [u32; 4],
        /// Atlas texel x, y of column 0's mean; 0, 0.
        pub mean: [u32; 4],
        /// Repeat width, repeat height (0: stretched), 0, 0.
        pub repeat: [f32; 4],
    }
}

uniform_block! {
    /// One pattern obstacle of the frame (`StructuredBuffer<GpuPatternObstacle>
    /// PatternObstacles`); the id texture holds `1 + its index`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct GpuPatternObstacle[96] {
        /// [`PatternKind`]; [`PATTERN_FINITE_HEIGHT`]; pattern as `1 + record index`, 0 none; 0.
        pub kind: [u32; 4],
        /// World origin x, y; cos and sin of the rotation.
        pub transform: [f32; 4],
        /// Box: half extents; circle: radius; tile grid: cell size x, y, edge width.
        pub shape: [f32; 4],
        /// Base, top (finite only), tile grid level height, the obstacle's opacity.
        pub heights: [f32; 4],
        /// Polygon: first corner word, corner count. Tile grid: columns, rows, first cell word,
        /// first index word or [`PATTERN_NO_INDICES`].
        pub data: [u32; 4],
        /// Tile grid: first table word, entries; 0, 0.
        pub table: [u32; 4],
    }
}

uniform_block! {
    /// `PatternParams` of the `*_pattern` programs: mask texel to world ground point, atlas size.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct PatternParams[48] {
        /// World x = this . (mask x, mask y, 1); w 0.
        pub mask_to_world_x: [f32; 4],
        /// World y likewise.
        pub mask_to_world_y: [f32; 4],
        /// 1 / width, 1 / height, width, height of the atlas in texels.
        pub atlas_size: [f32; 4],
    }
}

uniform_block! {
    /// `TilePatternIdParams` of `tile_pattern_id.frag`: the strip block and the grid's id.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct TilePatternIdParams[32] {
        /// As the strip part of `tile.frag`'s block.
        pub strip: [f32; 4],
        /// `1 + record index` of the grid; 0, 0, 0.
        pub id: [u32; 4],
    }
}

/// The mask texel of a world ground point (`occluder.vert`), in `f64` as in Chiaro.
fn world_to_mask(p: &Projection, d: &OcclusionDomain, a: &MaskAnchor, w: [f64; 2]) -> [f64; 2] {
    let (o, x, y) = (p.origin.raw(), p.x_axis, p.y_axis);
    let sx = f64::from(o.x) + w[0] * f64::from(x.x) + w[1] * f64::from(y.x);
    let sy = f64::from(o.y) + w[0] * f64::from(x.y) + w[1] * f64::from(y.y);
    let m = f64::from(d.margin);
    [
        (sx + m) * f64::from(d.texels_per_scene[0]) - f64::from(a.remainder[0]),
        (sy + m) * f64::from(d.texels_per_scene[1]) - f64::from(a.remainder[1]),
    ]
}

/// `true` if a pattern obstacle lies (partly) in the mask grown by a texel: its local bounds
/// placed in the world.
pub fn in_domain(
    store: &ObstacleStore,
    id: ObstacleId,
    min_radius: f32,
    p: &Projection,
    d: &OcclusionDomain,
    a: &MaskAnchor,
) -> bool {
    let Some(o) = store.get(id) else {
        return false;
    };
    let (lo, hi) = if o.is_tile_grid() {
        let hi = [
            o.grid.columns as f32 * o.grid.cell_size.x,
            o.grid.rows as f32 * o.grid.cell_size.y,
        ];
        ([0.0; 2], hi)
    } else if o.kind == ObstacleKind::Circle {
        let r = det::max(o.radius, min_radius);
        ([-r, -r], [r, r])
    } else {
        let outline = store.outline(id);
        let Some(first) = outline.first() else {
            return false;
        };
        let (mut lo, mut hi) = ([first.x, first.y], [first.x, first.y]);
        for v in &outline[1..] {
            lo = [det::min(lo[0], v.x), det::min(lo[1], v.y)];
            hi = [det::max(hi[0], v.x), det::max(hi[1], v.y)];
        }
        (lo, hi)
    };
    let (s, c) = rotation(o);
    let (s, c) = (f64::from(s), f64::from(c));
    let pos = o.common.position.raw();
    let (mut x0, mut y0, mut x1, mut y1) = (1e300f64, 1e300f64, -1e300f64, -1e300f64);
    for k in 0..4 {
        let lx = f64::from(if k & 1 != 0 { hi[0] } else { lo[0] });
        let ly = f64::from(if k & 2 != 0 { hi[1] } else { lo[1] });
        let world = [
            f64::from(pos.x) + c * lx - s * ly,
            f64::from(pos.y) + s * lx + c * ly,
        ];
        let [mx, my] = world_to_mask(p, d, a, world);
        // Plain comparisons: the bounds are finite.
        x0 = if mx < x0 { mx } else { x0 };
        y0 = if my < y0 { my } else { y0 };
        x1 = if mx > x1 { mx } else { x1 };
        y1 = if my > y1 { my } else { y1 };
    }
    x1 >= -1.0 && y1 >= -1.0 && x0 <= f64::from(d.width) + 1.0 && y0 <= f64::from(d.height) + 1.0
}

/// `(sin, cos)` of the obstacle's rotation, as the obstacle triangles use it.
fn rotation(o: &StoredObstacle) -> (f32, f32) {
    det::sin_cos(o.common.rotation * (core::f32::consts::PI / 180.0))
}

/// The [`PatternParams`] of a frame: the inverse of [`world_to_mask`] and the atlas size.
pub fn pattern_params(
    p: &Projection,
    d: &OcclusionDomain,
    a: &MaskAnchor,
    atlas: (u32, u32),
) -> PatternParams {
    // mask = A world + c
    let (tx, ty) = (
        f64::from(d.texels_per_scene[0]),
        f64::from(d.texels_per_scene[1]),
    );
    let (o, x, y) = (p.origin.raw(), p.x_axis, p.y_axis);
    let a00 = tx * f64::from(x.x);
    let a01 = tx * f64::from(y.x);
    let a10 = ty * f64::from(x.y);
    let a11 = ty * f64::from(y.y);
    let m = f64::from(d.margin);
    let c0 = (f64::from(o.x) + m) * tx - f64::from(a.remainder[0]);
    let c1 = (f64::from(o.y) + m) * ty - f64::from(a.remainder[1]);
    let det = a00 * a11 - a01 * a10;
    let inv = if det != 0.0 { 1.0 / det } else { 0.0 };
    let (i00, i01, i10, i11) = (a11 * inv, -a01 * inv, -a10 * inv, a00 * inv);
    let (w, h) = (atlas.0 as f32, atlas.1 as f32);
    PatternParams {
        mask_to_world_x: [i00 as f32, i01 as f32, (-(i00 * c0 + i01 * c1)) as f32, 0.0],
        mask_to_world_y: [i10 as f32, i11 as f32, (-(i10 * c0 + i11 * c1)) as f32, 0.0],
        atlas_size: [1.0 / w, 1.0 / h, w, h],
    }
}

/// Atlas texel x, y of the column means of the `i`-th pattern in store order.
fn mean_place(i: u32, alpha_rows: u32) -> [u32; 2] {
    [
        (i % MEANS_PER_ROW) * MAX_REGION,
        alpha_rows + i / MEANS_PER_ROW,
    ]
}

/// Writes the atlas of `rows` rows (`ATLAS_SIZE` wide, R8) into `out`: every pattern's alpha at
/// its store place, then its column means (Chiaro `PatternAtlas::Prepare`).
pub fn fill_pattern_atlas(store: &PatternStore, rows: u32, out: &mut [u8]) {
    let width = ATLAS_SIZE as usize;
    out[..width * rows as usize].fill(0);
    let (_, alpha_rows) = store.atlas_rows();
    for (i, (_, p)) in store.iter().enumerate() {
        let w = p.width as usize;
        for y in 0..p.height as usize {
            let at = (p.atlas_y as usize + y) * width + p.atlas_x as usize;
            out[at..at + w].copy_from_slice(&p.alpha[y * w..(y + 1) * w]);
        }
        let [mx, my] = mean_place(i as u32, alpha_rows);
        let at = my as usize * width + mx as usize;
        out[at..at + w].copy_from_slice(p.column_mean);
    }
}

/// One [`GpuPattern`] per live pattern in store order into `records`; `index` gets
/// `(raw id, 1 + record index)` sorted by id.
pub fn pattern_records(
    store: &PatternStore,
    records: &mut Vec<GpuPattern>,
    index: &mut Vec<(u32, u32)>,
) {
    let (_, alpha_rows) = store.atlas_rows();
    records.clear();
    index.clear();
    for (i, (id, p)) in store.iter().enumerate() {
        let [mx, my] = mean_place(i as u32, alpha_rows);
        records.push(GpuPattern {
            rect: [p.atlas_x, p.atlas_y, p.width, p.height],
            mean: [mx, my, 0, 0],
            repeat: [p.repeat_width, p.repeat_height, 0.0, 0.0],
        });
        index.push((crate::pattern_store::raw_id(id), i as u32 + 1));
    }
    index.sort_unstable();
}

/// `1 + record index` of a raw pattern id in a sorted index; 0 for none or one not live.
pub fn index_of(index: &[(u32, u32)], raw: u32) -> u32 {
    if raw == 0 {
        return 0;
    }
    index
        .binary_search_by_key(&raw, |e| e.0)
        .map_or(0, |i| index[i].1)
}

/// The pattern obstacles of one frame and everything built from them; reused across frames.
#[derive(Clone, Debug, Default)]
pub struct PatternSet {
    /// The frame's pattern obstacles in record order, with the revision they had.
    pub members: Vec<(ObstacleId, u32)>,
    /// `PatternData`: polygon corners, tile cell words, cell indices, tables.
    pub words: Vec<u32>,
    offsets: Vec<u32>,
    index_words: Vec<u32>,
    table_words: Vec<u32>,
    /// One record per member.
    pub records: Vec<GpuPatternObstacle>,
    /// World triangles of the shape members.
    pub id_vertices: Vec<At<World2, Vec2>>,
    /// `1 + record index` per triangle (the opacity slot of `occluder.vert`).
    pub id_values: Vec<f32>,
    /// Tile grid members and their `1 + record index`.
    pub tile_draws: Vec<(ObstacleId, u32)>,
}

/// An active obstacle with a live pattern (Chiaro `HasPattern`).
fn has_pattern(o: &StoredObstacle) -> bool {
    o.is_active() && o.patterns.refs > 0
}

impl PatternSet {
    /// Collects the pattern obstacles in the domain (a frame that draws the mask).
    pub fn collect(
        &mut self,
        obstacles: &ObstacleStore,
        min_radius: f32,
        p: &Projection,
        d: &OcclusionDomain,
        a: &MaskAnchor,
    ) {
        self.members.clear();
        if !obstacles.any_pattern() {
            return;
        }
        for (id, o) in obstacles.iter() {
            if !has_pattern(o) || !in_domain(obstacles, id, min_radius, p, d, a) {
                continue;
            }
            if self.members.len() >= MAX_PATTERN_OBSTACLES as usize {
                break; // the rest keep their mean opacity, without the pattern
            }
            self.members.push((id, o.revision));
        }
    }

    /// Builds `words` for the members (Chiaro `BuildData`); `index` from [`fill_pattern_atlas`].
    pub fn build_data(&mut self, obstacles: &ObstacleStore, index: &[(u32, u32)]) {
        let n = self.members.len();
        self.words.clear();
        self.offsets.clear();
        self.offsets.resize(n, 0);
        self.index_words.clear();
        self.index_words.resize(n, PATTERN_NO_INDICES);
        self.table_words.clear();
        self.table_words.resize(n, 0);
        for (m, &(id, _)) in self.members.iter().enumerate() {
            let Some(o) = obstacles.get(id) else {
                continue;
            };
            if o.kind == ObstacleKind::Polygon && !o.box_shape {
                self.offsets[m] = self.words.len() as u32;
                for v in obstacles.outline(id) {
                    self.words.extend([v.x.to_bits(), v.y.to_bits()]);
                }
            } else if let Some(t) = obstacles.tile(id) {
                // value, bottom, raw cell opacity, edges (0 as 255): not the mask texel.
                self.offsets[m] = self.words.len() as u32;
                let cells = t.solid.len();
                for i in 0..cells {
                    let bottom = t.base_levels.get(i).map_or(0, |&b| u32::from(b));
                    let opacity = t.opacity.get(i).map_or(255, |&b| u32::from(b));
                    let edges = t
                        .edges
                        .get(i)
                        .map_or(255, |&e| if e == 0 { 255 } else { u32::from(e) });
                    self.words
                        .push(u32::from(t.solid[i]) | bottom << 8 | opacity << 16 | edges << 24);
                }
                if !t.pattern_cells.is_empty() {
                    let first = self.words.len();
                    self.index_words[m] = first as u32;
                    self.words.resize(first + cells.div_ceil(4), 0);
                    for (i, &c) in t.pattern_cells.iter().enumerate() {
                        self.words[first + i / 4] |= u32::from(c) << (8 * (i % 4));
                    }
                }
                self.table_words[m] = self.words.len() as u32;
                self.words
                    .extend(t.pattern_table.iter().map(|&id| index_of(index, id)));
            }
        }
        if self.words.is_empty() {
            self.words.push(0); // a bound storage buffer must not be empty
        }
    }

    /// The records of the members, rebuilt every frame (obstacles move); needs
    /// [`build_data`](Self::build_data) of the same members.
    pub fn build_records(&mut self, obstacles: &ObstacleStore, index: &[(u32, u32)]) {
        self.records.clear();
        for (m, &(id, _)) in self.members.iter().enumerate() {
            let mut r = GpuPatternObstacle {
                transform: [0.0, 0.0, 1.0, 0.0], // identity rotation
                ..Default::default()
            };
            if let Some(o) = obstacles.get(id).filter(|o| has_pattern(o)) {
                let kind = if o.is_tile_grid() {
                    PatternKind::TileGrid
                } else if o.kind == ObstacleKind::Circle {
                    PatternKind::Circle
                } else if o.box_shape {
                    PatternKind::Box
                } else {
                    PatternKind::Polygon
                };
                let finite = if o.has_finite_height() {
                    PATTERN_FINITE_HEIGHT
                } else {
                    0
                };
                r.kind = [kind as u32, finite, index_of(index, o.patterns.pattern), 0];
                let (s, c) = rotation(o);
                let p = o.common.position.raw();
                r.transform = [p.x, p.y, c, s];
                let top = if o.common.height > 0.0 {
                    o.common.height
                } else {
                    0.0
                };
                r.heights = [o.common.base_height, top, 0.0, o.common.opacity];
                match kind {
                    PatternKind::TileGrid => {
                        let g = &o.grid;
                        r.shape = [g.cell_size.x, g.cell_size.y, g.edge_width, 0.0];
                        r.heights[2] = g.level_height;
                        r.data = [g.columns, g.rows, self.offsets[m], self.index_words[m]];
                        let entries = obstacles.tile(id).map_or(0, |t| t.pattern_table.len());
                        r.table = [self.table_words[m], entries as u32, 0, 0];
                    }
                    PatternKind::Circle => r.shape[0] = o.radius,
                    PatternKind::Box => {
                        // Corner 2 of the box outline is (width / 2, depth / 2).
                        let corner = obstacles.outline(id)[2];
                        r.shape[0] = corner.x;
                        r.shape[1] = corner.y;
                    }
                    PatternKind::Polygon => {
                        r.data[0] = self.offsets[m];
                        r.data[1] = obstacles.outline(id).len() as u32;
                    }
                }
            }
            self.records.push(r);
        }
    }

    /// The world triangles of the shape members with their ids, and the tile grid members
    /// (Chiaro `BuildIdTriangles`): a frame that draws the mask.
    pub fn build_ids(&mut self, obstacles: &ObstacleStore, min_radius: f32) {
        self.id_vertices.clear();
        self.id_values.clear();
        self.tile_draws.clear();
        for (m, &(id, _)) in self.members.iter().enumerate() {
            let Some(o) = obstacles.get(id) else {
                continue;
            };
            let value = m as u32 + 1;
            if o.is_tile_grid() {
                self.tile_draws.push((id, value));
                continue;
            }
            let first = self.id_vertices.len();
            obstacles.append_world_triangles(id, &mut self.id_vertices);
            if o.kind == ObstacleKind::Circle && o.radius < min_radius {
                widen_circle(o, min_radius, first, &mut self.id_vertices); // as in the mask
            }
            // The id in the opacity slot, exact in a float up to MAX_PATTERN_OBSTACLES.
            let triangles = (self.id_vertices.len() - first) / 3;
            self.id_values
                .resize(self.id_values.len() + triangles, value as f32);
        }
    }
}
