// Origin: chiaro @ 9bc5b9b, src/shadows/DirtyRects.hpp and DirtyRects.cpp (DirtyRect,
// DirtyRectList, AddUnion, AddExposedStrips, AddVanishedContent, ObstacleGridRect,
// TileCellsGridRect).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Rectangles of mask texels and the fixed-size list the incremental update collects them in.

use super::domain::OcclusionDomain;
use crate::Projection;
use crate::light_math::DEG_TO_RAD;
use crate::obstacle::{ObstacleId, ObstacleStore, StoredObstacle};
use knorke_math::{At, Vec2, World2, det};

/// Rectangle coordinates stay within ±2^30 texels, so every coordinate and difference fits into
/// `i32`.
pub const TEXEL_COORD_LIMIT: i64 = 1 << 30;

/// Clamps to [`TEXEL_COORD_LIMIT`].
pub fn clamp_texel_coord(v: i64) -> i32 {
    v.clamp(-TEXEL_COORD_LIMIT, TEXEL_COORD_LIMIT) as i32
}

/// A rectangle of mask texels, half-open: `[x0, x1) x [y0, y1)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DirtyRect {
    /// Left edge (inclusive).
    pub x0: i32,
    /// Top edge (inclusive).
    pub y0: i32,
    /// Right edge (exclusive).
    pub x1: i32,
    /// Bottom edge (exclusive).
    pub y1: i32,
}

impl DirtyRect {
    /// The empty rectangle.
    pub const EMPTY: Self = Self::new(0, 0, 0, 0);

    /// `[x0, x1) x [y0, y1)`.
    pub const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// Covers no texel.
    pub fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }

    /// Number of texels.
    pub fn area(&self) -> u64 {
        if self.is_empty() {
            0
        } else {
            (i64::from(self.x1) - i64::from(self.x0)) as u64
                * (i64::from(self.y1) - i64::from(self.y0)) as u64
        }
    }

    /// Bounding box of both.
    pub fn bounding_box(&self, b: &Self) -> Self {
        Self::new(
            self.x0.min(b.x0),
            self.y0.min(b.y0),
            self.x1.max(b.x1),
            self.y1.max(b.y1),
        )
    }

    /// The fold rule of [`DirtyRectList::merge`]: the bounding box is no larger than both together.
    pub fn folds_into_box(&self, b: &Self) -> bool {
        self.bounding_box(b).area() <= self.area() + b.area()
    }

    /// Moved by `(dx, dy)`, clamped; empty stays [`EMPTY`](Self::EMPTY).
    pub fn offset(&self, dx: i32, dy: i32) -> Self {
        if self.is_empty() {
            return Self::EMPTY;
        }
        let (dx, dy) = (i64::from(dx), i64::from(dy));
        Self::new(
            clamp_texel_coord(i64::from(self.x0) + dx),
            clamp_texel_coord(i64::from(self.y0) + dy),
            clamp_texel_coord(i64::from(self.x1) + dx),
            clamp_texel_coord(i64::from(self.y1) + dy),
        )
    }

    /// Grown by `by` texels on every side, clamped (not clipped).
    pub fn grow(&self, by: i32) -> Self {
        let b = i64::from(by);
        Self::new(
            clamp_texel_coord(i64::from(self.x0) - b),
            clamp_texel_coord(i64::from(self.y0) - b),
            clamp_texel_coord(i64::from(self.x1) + b),
            clamp_texel_coord(i64::from(self.y1) + b),
        )
    }

    fn overlaps(&self, b: &Self) -> bool {
        self.x0 < b.x1 && b.x0 < self.x1 && self.y0 < b.y1 && b.y0 < self.y1
    }

    /// Overlapping or sharing a piece of an edge (a common corner alone does not count).
    fn touches(&self, b: &Self) -> bool {
        let x_open = self.x0 < b.x1 && b.x0 < self.x1;
        let y_open = self.y0 < b.y1 && b.y0 < self.y1;
        let x_closed = self.x0 <= b.x1 && b.x0 <= self.x1;
        let y_closed = self.y0 <= b.y1 && b.y0 <= self.y1;
        (x_open && y_closed) || (x_closed && y_open)
    }

    fn intersection(&self, b: &Self) -> Self {
        Self::new(
            self.x0.max(b.x0),
            self.y0.max(b.y0),
            self.x1.min(b.x1),
            self.y1.min(b.y1),
        )
    }

    /// Up to four pieces of `self` outside `b`; returns their count.
    fn subtract(&self, b: &Self, pieces: &mut [Self; 4]) -> usize {
        let a = self;
        let mut n = 0;
        if a.y0 < b.y0 {
            pieces[n] = Self::new(a.x0, a.y0, a.x1, b.y0);
            n += 1;
        }
        if b.y1 < a.y1 {
            pieces[n] = Self::new(a.x0, b.y1, a.x1, a.y1);
            n += 1;
        }
        let (y0, y1) = (a.y0.max(b.y0), a.y1.min(b.y1));
        if a.x0 < b.x0 {
            pieces[n] = Self::new(a.x0, y0, b.x0, y1);
            n += 1;
        }
        if b.x1 < a.x1 {
            pieces[n] = Self::new(b.x1, y0, a.x1, y1);
            n += 1;
        }
        n
    }
}

/// The changed rectangles of one local mask update (E21), in mask texels of the
/// current frame. Storage is a fixed array; adding beyond [`MAX_DIRTY_RECTS`](Self::MAX_DIRTY_RECTS)
/// fails and marks the list as overflowed, which forces a full rebuild. Never allocates.
#[derive(Clone, Debug)]
pub struct DirtyRectList {
    rects: [DirtyRect; Self::MAX_DIRTY_RECTS],
    count: usize,
    overflowed: bool,
}

impl Default for DirtyRectList {
    fn default() -> Self {
        Self {
            rects: [DirtyRect::EMPTY; Self::MAX_DIRTY_RECTS],
            count: 0,
            overflowed: false,
        }
    }
}

impl DirtyRectList {
    /// Capacity of the list.
    pub const MAX_DIRTY_RECTS: usize = 64;

    /// An empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Empties the list and resets the overflow flag.
    pub fn clear(&mut self) {
        self.count = 0;
        self.overflowed = false;
    }

    /// Stores `r` unless it is empty; `false` (and [`overflowed`](Self::overflowed)) if the list
    /// is full.
    pub fn add(&mut self, r: DirtyRect) -> bool {
        if r.is_empty() {
            return true;
        }
        if self.count == Self::MAX_DIRTY_RECTS {
            self.overflowed = true;
            return false;
        }
        self.rects[self.count] = r;
        self.count += 1;
        true
    }

    fn remove(&mut self, i: usize) {
        self.count -= 1;
        self.rects[i] = self.rects[self.count];
    }

    /// Grows every rectangle by `by` texels on each side, clipped to `[0, w) x [0, h)`; empty
    /// results are dropped.
    pub fn expand(&mut self, by: i32, w: i32, h: i32) {
        let b = i64::from(by);
        let mut i = 0;
        while i < self.count {
            let r = self.rects[i];
            let r = DirtyRect::new(
                clamp_texel_coord((i64::from(r.x0) - b).max(0)),
                clamp_texel_coord((i64::from(r.y0) - b).max(0)),
                clamp_texel_coord((i64::from(r.x1) + b).min(i64::from(w))),
                clamp_texel_coord((i64::from(r.y1) + b).min(i64::from(h))),
            );
            if r.is_empty() {
                self.remove(i);
            } else {
                self.rects[i] = r;
                i += 1;
            }
        }
    }

    /// Folds overlapping or touching rectangles into their bounding box while that box is no
    /// larger than the two areas together, then cuts the remaining overlaps apart. Afterwards no
    /// two rectangles overlap and every texel of the input is still covered.
    pub fn merge(&mut self) {
        // 1. Fold neighbours whose bounding box costs no more than the two of them.
        let mut folded = true;
        while folded {
            folded = false;
            let mut i = 0;
            while i < self.count {
                let mut j = i + 1;
                while j < self.count {
                    let (a, b) = (self.rects[i], self.rects[j]);
                    if a.touches(&b) && a.folds_into_box(&b) {
                        self.rects[i] = a.bounding_box(&b);
                        self.remove(j);
                        folded = true;
                        j = i + 1; // the grown box may now meet earlier ones
                    } else {
                        j += 1;
                    }
                }
                i += 1;
            }
        }
        // 2. Cut the smaller of two overlapping rectangles around the larger. Every cut lowers the
        //    total overlap, so this ends; without room for the pieces the pair becomes its
        //    bounding box (one fewer rectangle).
        let mut cut = true;
        while cut {
            cut = false;
            'outer: for i in 0..self.count {
                for j in i + 1..self.count {
                    let (a, b) = (self.rects[i], self.rects[j]);
                    if !a.overlaps(&b) {
                        continue;
                    }
                    let (small, large) = if a.area() <= b.area() { (i, j) } else { (j, i) };
                    let mut pieces = [DirtyRect::EMPTY; 4];
                    let n = self.rects[small].subtract(&self.rects[large], &mut pieces);
                    if n == 0 {
                        self.remove(small);
                    } else if self.count - 1 + n <= Self::MAX_DIRTY_RECTS {
                        self.rects[small] = pieces[0];
                        for p in &pieces[1..n] {
                            self.rects[self.count] = *p;
                            self.count += 1;
                        }
                    } else {
                        self.rects[i] = a.bounding_box(&b);
                        self.remove(j);
                    }
                    cut = true;
                    break 'outer;
                }
            }
        }
    }

    /// Sum of the areas; the covered texel count once [`merge`](Self::merge) has run.
    pub fn area(&self) -> u64 {
        self.rects().iter().map(DirtyRect::area).sum()
    }

    /// The stored rectangles.
    pub fn rects(&self) -> &[DirtyRect] {
        &self.rects[..self.count]
    }

    /// An [`add`](Self::add) failed since the last [`clear`](Self::clear).
    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Adds the union of `a` and `b` by [`merge`](Self::merge)'s fold rule: their bounding box
    /// if it folds, else both.
    pub fn add_union(&mut self, a: DirtyRect, b: DirtyRect) {
        if a.is_empty() || b.is_empty() {
            self.add(if a.is_empty() { b } else { a });
        } else if a.folds_into_box(&b) {
            self.add(a.bounding_box(&b));
        } else {
            self.add(a);
            self.add(b);
        }
    }

    /// Adds the strips of a `w x h` mask that a shift of its contents by `(dx, dy)` exposes
    /// ([`exposed_strips`]). They overlap in the corner; [`merge`](Self::merge) separates them.
    pub fn add_exposed_strips(&mut self, dx: i32, dy: i32, w: i32, h: i32) {
        for strip in exposed_strips([dx, dy], w, h) {
            self.add(strip);
        }
    }

    /// Adds every part of `grid_rects` (moved by `offset`) that lies in `strip`, each on its own.
    pub fn add_parts_in(&mut self, grid_rects: &[DirtyRect], offset: [i32; 2], strip: &DirtyRect) {
        for g in grid_rects {
            self.add(g.offset(offset[0], offset[1]).intersection(strip));
        }
    }
}

/// The strips of a `w x h` mask that a shift of its contents by `(dx, dy)` exposes
/// (`new[x] = old[x - d]`): a full-height strip for `dx`, a full-width strip for `dy`; empty
/// without shift on that axis.
pub(super) fn exposed_strips([dx, dy]: [i32; 2], w: i32, h: i32) -> [DirtyRect; 2] {
    let x = match dx.signum() {
        1 => DirtyRect::new(0, 0, dx.min(w), h),
        -1 => DirtyRect::new((w + dx).max(0), 0, w, h),
        _ => DirtyRect::EMPTY,
    };
    let y = match dy.signum() {
        1 => DirtyRect::new(0, 0, w, dy.min(h)),
        -1 => DirtyRect::new(0, (h + dy).max(0), w, h),
        _ => DirtyRect::EMPTY,
    };
    [x, y]
}

/// The strips just outside a `w x h` mask that the old contents moved into with a shift by
/// `(dx, dy)`: the old mask now spans `[dx, w + dx) x [dy, h + dy)`. Empty without shift on that
/// axis.
pub(super) fn vanished_strips([dx, dy]: [i32; 2], w: i32, h: i32) -> [DirtyRect; 2] {
    let x = match dx.signum() {
        1 => DirtyRect::new(w, dy, w + dx, h + dy),
        -1 => DirtyRect::new(dx, dy, 0, h + dy),
        _ => DirtyRect::EMPTY,
    };
    let y = match dy.signum() {
        1 => DirtyRect::new(dx, h, w + dx, h + dy),
        -1 => DirtyRect::new(dx, dy, w + dx, 0),
        _ => DirtyRect::EMPTY,
    };
    [x, y]
}

/// A rectangle of tile cells, half-open: columns `[x0, x1)`, rows `[y0, y1)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellRect {
    /// First column.
    pub x0: u32,
    /// First row.
    pub y0: u32,
    /// Column past the last.
    pub x1: u32,
    /// Row past the last.
    pub y1: u32,
}

/// Bounding box of world points in grid coordinates, rounded outwards plus one texel.
fn grid_bounds(world: &[At<World2, Vec2>], p: &Projection, d: &OcclusionDomain) -> DirtyRect {
    if world.is_empty() {
        return DirtyRect::EMPTY;
    }
    // grid = (world through the linear part + margin) * texels_per_scene, see MaskAnchor.
    let margin = f64::from(d.margin);
    let (tx, ty) = (
        f64::from(d.texels_per_scene[0]),
        f64::from(d.texels_per_scene[1]),
    );
    let (ax, ay) = (
        [f64::from(p.x_axis.x), f64::from(p.x_axis.y)],
        [f64::from(p.y_axis.x), f64::from(p.y_axis.y)],
    );
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for w in world {
        let w = w.raw();
        let (wx, wy) = (f64::from(w.x), f64::from(w.y));
        let gx = (wx * ax[0] + wy * ay[0] + margin) * tx;
        let gy = (wx * ax[1] + wy * ay[1] + margin) * ty;
        // Explicit comparisons: f64::min/max are not allowed (ADR 0011).
        if gx < min_x {
            min_x = gx;
        }
        if gx > max_x {
            max_x = gx;
        }
        if gy < min_y {
            min_y = gy;
        }
        if gy > max_y {
            max_y = gy;
        }
    }
    let limit = TEXEL_COORD_LIMIT as f64;
    let lo = |v: f64| (v.floor() - 1.0).clamp(-limit, limit) as i32;
    let hi = |v: f64| (v.ceil() + 1.0).clamp(-limit, limit) as i32;
    DirtyRect::new(lo(min_x), lo(min_y), hi(max_x), hi(max_y))
}

/// The world corners of a rectangle of cells, rotated and moved like the tile grid, appended.
fn append_cell_corners(grid: &StoredObstacle, cells: &CellRect, out: &mut Vec<At<World2, Vec2>>) {
    let size = grid.grid.cell_size;
    let (x0, x1) = (size.x * cells.x0 as f32, size.x * cells.x1 as f32);
    let (y0, y1) = (size.y * cells.y0 as f32, size.y * cells.y1 as f32);
    let (s, c) = det::sin_cos(grid.common.rotation * DEG_TO_RAD);
    let p = grid.common.position.raw();
    for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        out.push(At::new(Vec2::new(c * x - s * y + p.x, s * x + c * y + p.y)));
    }
}

/// Bounding box of an obstacle's mask texels in grid coordinates (`grid = mask_texel -
/// MaskAnchor::snapped`, independent of the projection origin): world triangles or the tile grid
/// quad through the linear part of `p` into the mask, rounded outwards plus one texel. `scratch`
/// is reused for the world points.
///
/// A stale `id` gives an empty rectangle.
pub fn obstacle_grid_rect(
    store: &ObstacleStore,
    id: ObstacleId,
    p: &Projection,
    d: &OcclusionDomain,
    scratch: &mut Vec<At<World2, Vec2>>,
) -> DirtyRect {
    scratch.clear();
    let Some(obstacle) = store.get(id) else {
        return DirtyRect::EMPTY;
    };
    if obstacle.is_tile_grid() {
        let all = CellRect::new(0, 0, obstacle.grid.columns, obstacle.grid.rows);
        append_cell_corners(obstacle, &all, scratch);
    } else {
        store.append_world_triangles(id, scratch);
    }
    grid_bounds(scratch, p, d)
}

/// Like [`obstacle_grid_rect`], for the rectangle `cells` of a tile grid only.
pub fn tile_cells_grid_rect(
    grid: &StoredObstacle,
    cells: &CellRect,
    p: &Projection,
    d: &OcclusionDomain,
    scratch: &mut Vec<At<World2, Vec2>>,
) -> DirtyRect {
    scratch.clear();
    append_cell_corners(grid, cells, scratch);
    grid_bounds(scratch, p, d)
}
