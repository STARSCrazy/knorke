//! Grid map: rooms made of square cells, like Wolfenstein/Doom levels, but turned into real 3D quads.
//!
//! Cell (x, z) covers world x in [x, x+1] * cell_size and z in [z, z+1] * cell_size.
//! Sides: 0 = north (-z), 1 = east (+x), 2 = south (+z), 3 = west (-x).
//! A wall on a side faces into its cell, so it is seen from inside that cell. With `back_texture` it is
//! also drawn facing the neighbour cell (two-sided), unless the neighbour has its own wall on that edge.
//! Doors: a wall can carry a leaf (a sub-rectangle) that slides up, sideways, or swings on a hinge.

use crate::math::{Vec3, Vec3Ext};

/// Wall side facing north (-z).
pub const NORTH: usize = 0;
/// Wall side facing east (+x).
pub const EAST: usize = 1;
/// Wall side facing south (+z).
pub const SOUTH: usize = 2;
/// Wall side facing west (-x).
pub const WEST: usize = 3;
/// Face kinds beyond the four wall sides (picking and raycasts).
pub const SIDE_FLOOR: usize = 4;
/// `GridHit::side` / `Pick::side` of a ceiling.
pub const SIDE_CEILING: usize = 5;
/// `GridHit::side` / `Pick::side` of a step face between two heights.
pub const SIDE_STEP: usize = 6;

/// Door modes (`Wall::door_mode`).
pub const DOOR_UP: i32 = 0;
/// Door leaf slides to the left.
pub const DOOR_SLIDE_LEFT: i32 = 1;
/// Door leaf slides to the right.
pub const DOOR_SLIDE_RIGHT: i32 = 2;
/// Door leaf swings on its left edge.
pub const DOOR_SWING_LEFT: i32 = 3;
/// Door leaf swings on its right edge.
pub const DOOR_SWING_RIGHT: i32 = 4;

/// Which part of a wall a pick or raycast found (bit flags). PART_WALL: the wall or the frame around
/// a separate leaf (also a whole-wall door without `leaf_texture`); PART_LEAF: the separate leaf.
pub const PART_WALL: i32 = 0;
/// `part` of a door leaf.
pub const PART_LEAF: i32 = 1;
/// Picking only: the face seen from the neighbour cell (back texture, back of a leaf).
pub const PART_BACK: i32 = 2;

/// Result of `GridMap::raycast`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridHit {
    /// Cell x of the face.
    pub cell_x: i32,
    /// Cell z of the face.
    pub cell_z: i32,
    /// Storey of the face: the level of the span it belongs to (SIDE_STEP: the span the ray was in).
    pub level: i32,
    /// NORTH..WEST for walls, SIDE_FLOOR, SIDE_CEILING or SIDE_STEP.
    pub side: i32,
    /// PART_WALL or PART_LEAF (walls only; 0 otherwise).
    pub part: i32,
    /// World units along the ray.
    pub distance: f32,
    /// World position of the hit.
    pub point: Vec3,
}

/// One wall of a cell, optionally with a door leaf.
///
/// Wall coordinates: u runs 0..1 along the wall, left to right as seen from inside the cell; heights are
/// fractions 0 (floor) .. 1 (ceiling) of the wall at that u (sloped cells: the wall is a trapezoid).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wall {
    /// Texture id, or -1 for no wall. With a separate leaf this is the frame around the leaf.
    pub texture: i32,
    /// Door opening 0 (closed) .. 1 (fully open), see `door_mode`.
    pub open: f32,
    /// Opacity 0..1 of frame and leaf; below 1 drawn see-through (BLEND_ALPHA), no shadow, not pickable.
    pub alpha: f32,
    /// Texture of the same wall seen from the neighbour cell, -1 = none (one-sided). Not mirrored: u is
    /// flipped so the picture reads left to right from that side too. Ignored where the neighbour has a
    /// wall of its own on this edge (that one wins).
    pub back_texture: i32,
    /// Separate door leaf, -1 = none: then the whole wall is the leaf (texture `texture`) and the leaf
    /// rectangle is ignored. With a leaf, the frame is drawn around the leaf rectangle only.
    pub leaf_texture: i32,
    /// Leaf rectangle: u from `leaf_u0` to `leaf_u1`, height from the floor up to `leaf_top`.
    /// leaf_u1 <= leaf_u0 means 0..1 (the whole width), leaf_top <= 0 means 1, so a zeroed rect is the whole wall.
    pub leaf_u0: f32,
    /// Right edge of the leaf rectangle (see `leaf_u0`).
    pub leaf_u1: f32,
    /// Top of the leaf rectangle (see `leaf_u0`).
    pub leaf_top: f32,
    /// DOOR_UP (default), DOOR_SLIDE_LEFT/RIGHT, DOOR_SWING_LEFT/RIGHT.
    pub door_mode: i32,
    /// Swing doors: angle in degrees at open = 1 (|angle| clamped to 180). Positive swings into this cell,
    /// negative into the neighbour.
    pub swing_angle: f32,
}

impl Default for Wall {
    fn default() -> Self {
        Wall {
            texture: -1,
            open: 0.0,
            alpha: 1.0,
            back_texture: -1,
            leaf_texture: -1,
            leaf_u0: 0.0,
            leaf_u1: 1.0,
            leaf_top: 1.0,
            door_mode: DOOR_UP,
            swing_angle: 90.0,
        }
    }
}

impl Wall {
    /// Leaf rectangle (u0, u1, top), normalised; the whole wall without a separate leaf.
    pub fn leaf_rect(&self) -> (f32, f32, f32) {
        if self.leaf_texture < 0 {
            return (0.0, 1.0, 1.0);
        }
        let (mut u0, mut u1) = (self.leaf_u0.clamp(0.0, 1.0), self.leaf_u1.clamp(0.0, 1.0));
        if !(u1 > u0) {
            (u0, u1) = (0.0, 1.0);
        }
        let top = if self.leaf_top > 0.0 {
            self.leaf_top.min(1.0)
        } else {
            1.0
        };
        (u0, u1, top)
    }

    fn is_swing(&self) -> bool {
        self.door_mode == DOOR_SWING_LEFT || self.door_mode == DOOR_SWING_RIGHT
    }

    /// Opening 0..1 (NaN = closed).
    fn opening(&self) -> f32 {
        if self.open > 0.0 {
            self.open.min(1.0)
        } else {
            0.0
        }
    }

    /// Swing angle in radians for the current opening; 0 for other modes.
    fn swing(&self) -> f32 {
        if !self.is_swing() || !self.swing_angle.is_finite() {
            return 0.0;
        }
        (self.opening() * self.swing_angle.clamp(-180.0, 180.0)).to_radians()
    }

    /// Part reported for the leaf.
    fn leaf_part(&self) -> i32 {
        if self.leaf_texture >= 0 {
            PART_LEAF
        } else {
            PART_WALL
        }
    }

    /// Does the leaf, where it is now, cover wall point (u, t) of its plane? (Not for swung leaves.)
    fn leaf_covers(&self, u: f32, t: f32, f: f32, c: f32, y: f32) -> bool {
        let (u0, u1, top) = self.leaf_rect();
        let open = self.opening();
        if self.leaf_texture >= 0 && (u < u0 || u > u1 || t > top) {
            return false;
        }
        match self.door_mode {
            DOOR_SWING_LEFT | DOOR_SWING_RIGHT => self.swing() == 0.0,
            _ if open >= 1.0 => false,
            DOOR_SLIDE_LEFT => u <= u1 - (u1 - u0) * open,
            DOOR_SLIDE_RIGHT => u >= u0 + (u1 - u0) * open,
            _ => y >= f + (c - f) * (open * top),
        }
    }

    /// Does the wall block a ray crossing its plane at (u, height y; floor f, ceiling c there)?
    fn blocks(&self, u: f32, f: f32, c: f32, y: f32) -> Option<i32> {
        let t = if c > f { (y - f) / (c - f) } else { 0.0 };
        let (u0, u1, top) = self.leaf_rect();
        if self.leaf_texture >= 0 && (u < u0 || u > u1 || t > top) {
            return Some(PART_WALL);
        }
        self.leaf_covers(u, t, f, c, y).then(|| self.leaf_part())
    }
}

/// Inward normal per side (into the cell).
const INWARD: [Vec3; 4] = [
    Vec3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    },
    Vec3 {
        x: -1.0,
        y: 0.0,
        z: 0.0,
    },
    Vec3 {
        x: 0.0,
        y: 0.0,
        z: -1.0,
    },
    Vec3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    },
];

/// a + (b - a) * t, exact at t = 0 and t = 1.
fn mix(a: f32, b: f32, t: f32) -> f32 {
    if t == 0.0 {
        a
    } else if t == 1.0 {
        b
    } else {
        a + (b - a) * t
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// One grid cell of one storey: heights, textures and four walls.
pub struct Cell {
    /// Floor height in world units.
    pub floor_height: f32,
    /// Ceiling height in world units.
    pub ceiling_height: f32,
    /// Texture ids, -1 for none (no ceiling shows the sky or the background).
    pub floor_texture: i32,
    /// Ceiling texture id, -1 = open sky.
    pub ceiling_texture: i32,
    /// Texture of the step faces this cell shows towards lower/higher neighbours; -1 uses the floor texture.
    pub side_texture: i32,
    /// Light level 0..1 of everything in this cell.
    pub light: f32,
    /// Walls by side (NORTH, EAST, SOUTH, WEST).
    pub walls: [Wall; 4],
    /// Slopes: offsets (cell sizes) added to floor_height / ceiling_height at the corners NW, NE, SE, SW.
    /// A sloped surface is two triangles split along NW-SE. Where a corner's floor is at or above its
    /// ceiling, the ceiling is clamped to the floor there; all four corners like that make the cell solid.
    pub floor_corners: [f32; 4],
    /// Ceiling corner offsets NW, NE, SE, SW (see `floor_corners`).
    pub ceiling_corners: [f32; 4],
}

impl Cell {
    /// True if no corner has room between floor and ceiling.
    pub fn is_solid(&self) -> bool {
        (0..4).all(|i| {
            self.ceiling_height + self.ceiling_corners[i]
                <= self.floor_height + self.floor_corners[i]
        })
    }

    /// True if a floor or ceiling corner is offset.
    pub fn is_sloped(&self) -> bool {
        self.floor_corners
            .iter()
            .chain(&self.ceiling_corners)
            .any(|&o| o != 0.0)
    }

    /// World heights of floor and ceiling at the corners NW, NE, SE, SW (ceiling clamped to the floor).
    pub fn corner_heights(&self, cell_size: f32) -> ([f32; 4], [f32; 4]) {
        let f = self
            .floor_corners
            .map(|o| (self.floor_height + o) * cell_size);
        let c = std::array::from_fn(|i| {
            ((self.ceiling_height + self.ceiling_corners[i]) * cell_size).max(f[i])
        });
        (f, c)
    }
}

/// Height inside a cell from its corner heights (NW, NE, SE, SW) at local u = x, v = z in 0..1,
/// on the two triangles split along NW-SE.
fn corner_lerp(h: [f32; 4], u: f32, v: f32) -> f32 {
    if u >= v {
        h[0] + (h[1] - h[0]) * u + (h[2] - h[1]) * v
    } else {
        h[0] + (h[2] - h[3]) * u + (h[3] - h[0]) * v
    }
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            floor_height: 0.0,
            ceiling_height: 1.0,
            floor_texture: -1,
            ceiling_texture: -1,
            side_texture: -1,
            light: 1.0,
            walls: [Wall::default(); 4],
            floor_corners: [0.0; 4],
            ceiling_corners: [0.0; 4],
        }
    }
}

/// A grid of columns, each `levels` cells high (storeys). The cell at level k of column (x, z) is one
/// open span floor..ceiling; a solid cell is an absent span. Spans of a column are used bottom to top:
/// a span's floor is raised to the ceiling of the highest open span below it where it would reach into
/// it (per corner), so spans never overlap; a span that lies wholly inside or below a lower one is
/// absent. Everything outside the spans is solid. Cells are stored level by level:
/// index (level * depth + z) * width + x, so level 0 is the first `width * depth` cells.
#[derive(Clone, Debug)]
pub struct GridMap {
    /// Cells along x.
    pub width: usize,
    /// Cells along z.
    pub depth: usize,
    /// Storeys per column, at least 1.
    pub levels: usize,
    /// Edge length of a cell in world units.
    pub cell_size: f32,
    /// Cells, level by level, row by row.
    pub cells: Vec<Cell>,
}

/// One open span of a column: the cell's level and its effective corner heights (world units,
/// NW, NE, SE, SW) after the ordering rule of `GridMap`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    /// Level (storey) of the cell.
    pub level: usize,
    /// Floor corner heights NW, NE, SE, SW.
    pub floor: [f32; 4],
    /// Ceiling corner heights NW, NE, SE, SW.
    pub ceiling: [f32; 4],
}

impl Span {
    /// Floor and ceiling at local u = x, v = z in 0..1 (clamped).
    pub fn heights(&self, u: f32, v: f32) -> (f32, f32) {
        let (u, v) = (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
        (
            corner_lerp(self.floor, u, v),
            corner_lerp(self.ceiling, u, v),
        )
    }

    /// Floor or ceiling not level.
    fn uneven(&self) -> bool {
        self.floor.iter().any(|&h| h != self.floor[0])
            || self.ceiling.iter().any(|&h| h != self.ceiling[0])
    }
}

/// The open spans of one column, bottom to top (`GridMap::spans`).
pub struct Spans<'a> {
    grid: &'a GridMap,
    base: usize,
    level: usize,
    end: usize,
    below: Option<[f32; 4]>,
}

impl Iterator for Spans<'_> {
    type Item = Span;
    fn next(&mut self) -> Option<Span> {
        let s = self.grid.cell_size;
        while self.level < self.end {
            let level = self.level;
            self.level += 1;
            let c = &self.grid.cells[self.base + level * self.grid.width * self.grid.depth];
            if c.is_solid() {
                continue;
            }
            let (mut f, mut ch) = c.corner_heights(s);
            if let Some(b) = self.below {
                // ordering rule: never below the ceiling of the open span underneath
                for i in 0..4 {
                    f[i] = f[i].max(b[i]);
                    ch[i] = ch[i].max(f[i]);
                }
                if (0..4).all(|i| ch[i] <= f[i]) {
                    continue;
                }
            }
            self.below = Some(ch);
            return Some(Span {
                level,
                floor: f,
                ceiling: ch,
            });
        }
        None
    }
}

/// A textured quad: corners top-left, top-right, bottom-right, bottom-left as seen from the front.
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    /// Corners in world space.
    pub corners: [Vec3; 4],
    /// Texture coordinates of the corners.
    pub uv: [[f32; 2]; 4],
    /// Texture id.
    pub texture: i32,
    /// Light level 0..1.
    pub light: f32,
    /// Unit normal of the front face.
    pub normal: Vec3,
    /// Repeat the texture (step faces) instead of clamping it to the face.
    pub wrap: bool,
    /// Which face of its cell this is (NORTH..WEST, SIDE_FLOOR, SIDE_CEILING, SIDE_STEP); -1 = not a grid face.
    pub side: i32,
    /// Opacity; below 1 the quad is see-through (grid walls only).
    pub alpha: f32,
    /// PART_WALL / PART_LEAF, plus PART_BACK for faces seen from the neighbour cell (grid walls only).
    pub part: i32,
}

impl GridMap {
    /// One storey: every cell an empty room (as `Cell::default`).
    pub fn new(width: usize, depth: usize, cell_size: f32) -> GridMap {
        GridMap::with_levels(width, depth, 1, cell_size)
    }

    /// `levels` storeys (at least 1): level 0 as in `new`, the levels above start solid (absent).
    pub fn with_levels(width: usize, depth: usize, levels: usize, cell_size: f32) -> GridMap {
        let levels = levels.max(1);
        let solid = Cell {
            ceiling_height: 0.0,
            ..Cell::default()
        };
        let mut cells = vec![Cell::default(); width * depth];
        cells.resize(width * depth * levels, solid);
        GridMap {
            width,
            depth,
            levels,
            cell_size,
            cells,
        }
    }

    fn index(&self, x: i64, z: i64, level: usize) -> Option<usize> {
        if x < 0
            || z < 0
            || x as usize >= self.width
            || z as usize >= self.depth
            || level >= self.levels
        {
            None
        } else {
            Some((level * self.depth + z as usize) * self.width + x as usize)
        }
    }

    /// Cell (x, z) of level 0.
    pub fn cell(&self, x: i64, z: i64) -> Option<&Cell> {
        self.cell_at(x, z, 0)
    }

    /// Cell (x, z) of level 0.
    pub fn cell_mut(&mut self, x: usize, z: usize) -> Option<&mut Cell> {
        self.cell_at_mut(x, z, 0)
    }

    /// Cell (x, z) of storey `level`, None outside the map.
    pub fn cell_at(&self, x: i64, z: i64, level: usize) -> Option<&Cell> {
        self.index(x, z, level).map(|i| &self.cells[i])
    }

    /// Cell (x, z) of storey `level`, mutable; None outside the map.
    pub fn cell_at_mut(&mut self, x: usize, z: usize, level: usize) -> Option<&mut Cell> {
        self.index(x as i64, z as i64, level)
            .map(|i| &mut self.cells[i])
    }

    /// The open spans of column (x, z), bottom to top; none outside the map.
    pub fn spans(&self, x: i64, z: i64) -> Spans<'_> {
        match self.index(x, z, 0) {
            Some(base) => Spans {
                grid: self,
                base,
                level: 0,
                end: self.levels,
                below: None,
            },
            None => Spans {
                grid: self,
                base: 0,
                level: 0,
                end: 0,
                below: None,
            },
        }
    }

    /// The span of level `level` in column (x, z); None if it is absent (solid or hidden by the ordering rule).
    pub fn span(&self, x: i64, z: i64, level: usize) -> Option<Span> {
        if level >= self.levels {
            return None;
        }
        let mut it = self.spans(x, z);
        it.end = it.end.min(level + 1);
        it.find(|s| s.level == level)
    }

    /// Floor and ceiling height (world units) of the span at `level` of column (cx, cz) at world point p
    /// (x/z clamped to the cell); None if that span is absent.
    pub fn surface_at(&self, cx: i64, cz: i64, level: usize, p: Vec3) -> Option<(f32, f32)> {
        let sp = self.span(cx, cz, level)?;
        Some(self.span_heights(&sp, cx, cz, p))
    }

    fn span_heights(&self, sp: &Span, cx: i64, cz: i64, p: Vec3) -> (f32, f32) {
        let s = self.cell_size;
        sp.heights(p.x / s - cx as f32, p.z / s - cz as f32)
    }

    /// The span of column (cx, cz) that contains world point p (floor <= p.y <= ceiling at p; the
    /// lower one where two touch): its level, floor and ceiling there.
    pub fn span_containing(&self, cx: i64, cz: i64, p: Vec3) -> Option<(usize, f32, f32)> {
        self.spans(cx, cz).find_map(|sp| {
            let (f, c) = self.span_heights(&sp, cx, cz, p);
            (p.y >= f && p.y <= c).then_some((sp.level, f, c))
        })
    }

    /// The span of column (x, z) that faces most of the interval lo..hi (heights at the two ends of a
    /// shared edge, corners `ni`, `nj` of that column): largest overlap at the middle of the edge, the
    /// lower one on ties, even if none overlaps. None for a column without spans.
    fn facing_span(
        &self,
        x: i64,
        z: i64,
        ni: usize,
        nj: usize,
        lo: [f32; 2],
        hi: [f32; 2],
    ) -> Option<Span> {
        let mut best: Option<(f32, Span)> = None;
        for sp in self.spans(x, z) {
            let top = (sp.ceiling[ni] + sp.ceiling[nj]).min(hi[0] + hi[1]);
            let bottom = (sp.floor[ni] + sp.floor[nj]).max(lo[0] + lo[1]);
            let o = top - bottom;
            if best.is_none_or(|(b, _)| o > b) {
                best = Some((o, sp));
            }
        }
        best.map(|(_, sp)| sp)
    }

    /// First crossing of the sloped floor or ceiling of span `sp` in column (cx, cz) for t in [t0, t1]:
    /// textured faces only, or with `all` also untextured ones (holes).
    #[allow(clippy::too_many_arguments)]
    fn slope_hit(
        &self,
        cx: i64,
        cz: i64,
        sp: &Span,
        c: &Cell,
        origin: Vec3,
        d: Vec3,
        t0: f32,
        mut t1: f32,
        all: bool,
    ) -> Option<(f32, usize)> {
        let s = self.cell_size;
        // past the lowest floor corner (going down) or highest ceiling corner (going up) nothing can be hit;
        // this also keeps t1 finite for vertical rays
        let (fc, cc) = (sp.floor, sp.ceiling);
        let bound = if d.y < 0.0 {
            fc.iter().fold(f32::INFINITY, |m, &h| m.min(h))
        } else {
            cc.iter().fold(f32::NEG_INFINITY, |m, &h| m.max(h))
        };
        if d.y != 0.0 {
            t1 = t1.min(((bound + s * d.y.signum() - origin.y) / d.y).max(t0));
        }
        // split where the ray crosses the NW-SE diagonal (u == v)
        let mut mid = t1;
        if d.x != d.z {
            let t = ((cx - cz) as f32 * s - (origin.x - origin.z)) / (d.x - d.z);
            if t > t0 && t < t1 {
                mid = t;
            }
        }
        let cuts = [t0, mid, t1];
        let gap = |t: f32| {
            let p = origin + d * t;
            let (f, ch) = self.span_heights(sp, cx, cz, p);
            (p.y - f, ch - p.y)
        };
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let ((fa, ca), (fb, cb)) = (gap(a), gap(b));
            let cross =
                |ga: f32, gb: f32| (ga >= 0.0 && gb < 0.0).then(|| a + (b - a) * ga / (ga - gb));
            let floor = if all || c.floor_texture >= 0 {
                cross(fa, fb)
            } else {
                None
            };
            let ceil = if all || c.ceiling_texture >= 0 {
                cross(ca, cb)
            } else {
                None
            };
            match (floor, ceil) {
                (Some(f), Some(cl)) => {
                    return Some(if f <= cl {
                        (f, SIDE_FLOOR)
                    } else {
                        (cl, SIDE_CEILING)
                    });
                }
                (Some(f), None) => return Some((f, SIDE_FLOOR)),
                (None, Some(cl)) => return Some((cl, SIDE_CEILING)),
                _ => {}
            }
        }
        None
    }

    /// First grid face along a ray (sprites and meshes are ignored), within `max_dist` world units.
    /// Walls block from both sides (the hit names the wall's own cell, level and side); the open lower
    /// part of a door lets the ray through. The ray starts in the span containing the origin (else the
    /// nearest span of that column). Floors and ceilings count where textured; an untextured one is a
    /// hole: the ray goes on into the span of the same column it meets there, if any. Crossing an edge,
    /// the ray enters the neighbour span that contains its height; with none it hits the solid
    /// (SIDE_STEP) if it was inside its own span. Leaving the map is a miss.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_dist: f32) -> Option<GridHit> {
        let len = dir.length();
        let s = self.cell_size;
        if !(len > 0.0) || !(s > 0.0) || !(max_dist >= 0.0) {
            return None;
        }
        let d = dir * (1.0 / len);
        let mut cx = (origin.x / s).floor() as i64;
        let mut cz = (origin.z / s).floor() as i64;
        let axis = |o: f32, dv: f32, c: i64| -> (f32, f32, i64) {
            // (t of the next boundary, t per cell, step)
            if dv > 0.0 {
                (((c + 1) as f32 * s - o) / dv, s / dv, 1)
            } else if dv < 0.0 {
                ((c as f32 * s - o) / dv, -s / dv, -1)
            } else {
                (f32::INFINITY, f32::INFINITY, 0)
            }
        };
        let (mut tx, dtx, sx) = axis(origin.x, d.x, cx);
        let (mut tz, dtz, sz) = axis(origin.z, d.z, cz);
        let mut t0 = 0.0f32;
        let hit = |cx: i64, cz: i64, level: usize, side: usize, t: f32| GridHit {
            cell_x: cx as i32,
            cell_z: cz as i32,
            level: level as i32,
            side: side as i32,
            part: 0,
            distance: t,
            point: origin + d * t,
        };
        let part_hit = |cx: i64, cz: i64, level: usize, side: usize, part: i32, t: f32| GridHit {
            part,
            ..hit(cx, cz, level, side, t)
        };
        let multi = self.levels > 1;
        // the span to start in: the one containing the origin, else the nearest
        let mut level = self.nearest_span(cx, cz, origin)?.level;
        loop {
            let c = self.cell_at(cx, cz, level)?;
            let sp = self.span(cx, cz, level)?;
            let (fh, ch) = (sp.floor[0], sp.ceiling[0]);
            let sloped = c.is_sloped() || sp.uneven();
            let t1 = tx.min(tz);
            // floor and ceiling inside this cell
            let plane = if sloped {
                self.slope_hit(cx, cz, &sp, c, origin, d, t0, t1, false)
            } else if d.y < 0.0 && c.floor_texture >= 0 {
                Some(((fh - origin.y) / d.y, SIDE_FLOOR))
            } else if d.y > 0.0 && c.ceiling_texture >= 0 {
                Some(((ch - origin.y) / d.y, SIDE_CEILING))
            } else {
                None
            };
            let mut first = plane
                .filter(|&(t, _)| t >= t0 && t <= t1)
                .map(|(t, side)| hit(cx, cz, level, side, t.max(0.0)));
            // swung leaves reaching into this cell (from this cell or a neighbour)
            if let Some(sw) = self.swing_hit(cx, cz, origin, d, t0, t1)
                && first.is_none_or(|f| sw.distance < f.distance)
            {
                first = Some(sw);
            }
            // a hole (untextured floor or ceiling) before that: go on in the span met there
            if multi {
                let any = if sloped {
                    self.slope_hit(cx, cz, &sp, c, origin, d, t0, t1, true)
                } else if d.y < 0.0 {
                    Some(((fh - origin.y) / d.y, SIDE_FLOOR))
                } else if d.y > 0.0 {
                    Some(((ch - origin.y) / d.y, SIDE_CEILING))
                } else {
                    None
                };
                if let Some((th, side)) = any.filter(|&(t, _)| t >= t0 && t <= t1) {
                    let textured = if side == SIDE_FLOOR {
                        c.floor_texture >= 0
                    } else {
                        c.ceiling_texture >= 0
                    };
                    if !textured && first.is_none_or(|f| th < f.distance) {
                        let p = origin + d * th;
                        let next = self.spans(cx, cz).find(|o| {
                            let (f, ch) = self.span_heights(o, cx, cz, p);
                            o.level != level
                                && (o.level < level) == (side == SIDE_FLOOR)
                                && p.y >= f - 1e-4 * s
                                && p.y <= ch + 1e-4 * s
                        });
                        if let Some(n) = next {
                            level = n.level;
                            t0 = th;
                            continue;
                        }
                    }
                }
            }
            if let Some(h) = first {
                return (h.distance <= max_dist).then_some(h);
            }
            if t1 > max_dist {
                return None;
            }
            let side = if tx < tz {
                if sx > 0 { EAST } else { WEST }
            } else if sz > 0 {
                SOUTH
            } else {
                NORTH
            };
            let y = origin.y + d.y * t1;
            let edge = origin + d * t1;
            let (fh, ch) = if sloped {
                self.span_heights(&sp, cx, cz, edge)
            } else {
                (fh, ch)
            };
            let inside = y >= fh && y <= ch;
            let wall = c.walls[side];
            if inside
                && wall.texture >= 0
                && let Some(part) = wall.blocks(self.edge_u(cx, cz, side, edge), fh, ch, y)
            {
                return Some(part_hit(cx, cz, level, side, part, t1));
            }
            let (nx, nz) = if tx < tz {
                (cx + sx, cz)
            } else {
                (cx, cz + sz)
            };
            self.cell(nx, nz)?;
            // the neighbour span at the ray's height; the nearest one if the ray is outside them all
            let mut found: Option<(usize, f32, f32)> = None;
            let mut near: Option<(f32, usize)> = None;
            for nsp in self.spans(nx, nz) {
                let nc = self.cell_at(nx, nz, nsp.level)?;
                let (nfh, nch) = if nc.is_sloped() || nsp.uneven() {
                    self.span_heights(&nsp, nx, nz, edge)
                } else {
                    (nsp.floor[0], nsp.ceiling[0])
                };
                if y >= nfh && y <= nch {
                    found = Some((nsp.level, nfh, nch));
                    break;
                }
                let dist = (nfh - y).max(y - nch);
                if near.is_none_or(|(b, _)| dist < b) {
                    near = Some((dist, nsp.level));
                }
            }
            if inside {
                match found {
                    Some((nl, nfh, nch)) => {
                        // the neighbour's wall on the shared edge blocks from behind as well (line of sight)
                        let back = (side + 2) % 4;
                        let bw = self.cell_at(nx, nz, nl)?.walls[back];
                        if bw.texture >= 0
                            && nch > nfh
                            && let Some(part) =
                                bw.blocks(self.edge_u(nx, nz, back, edge), nfh, nch, y)
                        {
                            return Some(part_hit(nx, nz, nl, back, part, t1));
                        }
                    }
                    None => return Some(hit(cx, cz, level, SIDE_STEP, t1)),
                }
            }
            level = match (found, near) {
                (Some((l, _, _)), _) | (None, Some((_, l))) => l,
                (None, None) => return None,
            };
            if tx < tz {
                tx += dtx;
            } else {
                tz += dtz;
            }
            cx = nx;
            cz = nz;
            t0 = t1;
        }
    }

    /// The span of column (cx, cz) containing p, else the one nearest to p.y.
    fn nearest_span(&self, cx: i64, cz: i64, p: Vec3) -> Option<Span> {
        let mut best: Option<(f32, Span)> = None;
        for sp in self.spans(cx, cz) {
            let (f, c) = self.span_heights(&sp, cx, cz, p);
            let dist = (f - p.y).max(p.y - c).max(0.0);
            if best.is_none_or(|(b, _)| dist < b) {
                best = Some((dist, sp));
            }
            if dist == 0.0 {
                break;
            }
        }
        best.map(|(_, sp)| sp)
    }

    /// Position 0..1 of world point p along side `side` of cell (cx, cz), left to right seen from inside.
    fn edge_u(&self, cx: i64, cz: i64, side: usize, p: Vec3) -> f32 {
        let s = self.cell_size;
        let (lx, lz) = (p.x / s - cx as f32, p.z / s - cz as f32);
        match side {
            NORTH => lx,
            EAST => lz,
            SOUTH => 1.0 - lx,
            _ => 1.0 - lz,
        }
    }

    /// World corners NW, NE, SE, SW of cell (x, z) at height 0.
    fn corners(&self, x: i64, z: i64) -> [Vec3; 4] {
        let s = self.cell_size;
        let (x0, z0) = (x as f32 * s, z as f32 * s);
        let (x1, z1) = (x0 + s, z0 + s);
        [
            Vec3::new(x0, 0.0, z0),
            Vec3::new(x1, 0.0, z0),
            Vec3::new(x1, 0.0, z1),
            Vec3::new(x0, 0.0, z1),
        ]
    }

    /// The swung leaf of wall `side` of cell (x, z, level): corners top-left, top-right, bottom-right, bottom-left
    /// of its front (the face that looks into the cell when closed), its front normal and the leaf's u at the
    /// left and right corners. Rectangular, with the floor and leaf-top heights at the hinge. None if the
    /// wall has no swung leaf right now.
    pub fn swing_leaf(
        &self,
        x: i64,
        z: i64,
        level: usize,
        side: usize,
    ) -> Option<([Vec3; 4], Vec3, [f32; 2])> {
        let c = self.cell_at(x, z, level)?;
        let w = c.walls[side];
        let angle = w.swing();
        if w.texture < 0 || angle == 0.0 || c.is_solid() {
            return None;
        }
        let s = self.cell_size;
        let sp = self.span(x, z, level)?;
        let (fc, cc) = (sp.floor, sp.ceiling);
        let p = self.corners(x, z);
        let (i, j) = (side, (side + 1) % 4);
        let (u0, u1, top) = w.leaf_rect();
        let left = w.door_mode == DOOR_SWING_LEFT;
        let uh = if left { u0 } else { u1 };
        let base = p[i] + (p[j] - p[i]) * uh;
        let f = mix(fc[i], fc[j], uh);
        let top_y = f + (mix(cc[i], cc[j], uh) - f) * top;
        let e = (p[j] - p[i]) * (1.0 / s);
        let n = INWARD[side];
        let (sn, cs) = angle.sin_cos();
        let width = (u1 - u0) * s;
        let at = |p: Vec3, y: f32| Vec3::new(p.x, y, p.z);
        if left {
            let far = base + (e * cs + n * sn) * width;
            Some((
                [at(base, top_y), at(far, top_y), at(far, f), at(base, f)],
                n * cs - e * sn,
                [0.0, 1.0],
            ))
        } else {
            let far = base + (e * (-cs) + n * sn) * width;
            Some((
                [at(far, top_y), at(base, top_y), at(base, f), at(far, f)],
                n * cs + e * sn,
                [0.0, 1.0],
            ))
        }
    }

    /// Nearest swung leaf crossed for t in [t0, t1] by a ray in cell (cx, cz). A leaf is at most one cell
    /// long and hinged on its own cell's edge, so only this cell and its eight neighbours can own one.
    fn swing_hit(
        &self,
        cx: i64,
        cz: i64,
        origin: Vec3,
        d: Vec3,
        t0: f32,
        t1: f32,
    ) -> Option<GridHit> {
        let mut best: Option<GridHit> = None;
        for oz in cz - 1..=cz + 1 {
            for ox in cx - 1..=cx + 1 {
                for level in 0..self.levels {
                    let Some(c) = self.cell_at(ox, oz, level) else {
                        continue;
                    };
                    for side in 0..4 {
                        if !c.walls[side].is_swing() || c.walls[side].texture < 0 {
                            continue;
                        }
                        let Some((q, n, _)) = self.swing_leaf(ox, oz, level, side) else {
                            continue;
                        };
                        let den = d.dot(n);
                        if den == 0.0 {
                            continue;
                        }
                        let t = (q[0] - origin).dot(n) / den;
                        if !(t >= t0 && t <= t1) || best.is_some_and(|b| b.distance <= t) {
                            continue;
                        }
                        let p = origin + d * t;
                        let (ea, eb) = (q[1] - q[0], q[3] - q[0]);
                        let (a, b) = (
                            (p - q[0]).dot(ea) / ea.dot(ea),
                            (p - q[0]).dot(eb) / eb.dot(eb),
                        );
                        if (0.0..=1.0).contains(&a) && (0.0..=1.0).contains(&b) {
                            best = Some(GridHit {
                                cell_x: ox as i32,
                                cell_z: oz as i32,
                                level: level as i32,
                                side: side as i32,
                                part: c.walls[side].leaf_part(),
                                distance: t.max(0.0),
                                point: p,
                            });
                        }
                    }
                }
            }
        }
        best
    }

    /// Quads of wall `side` of cell (x, z): the wall (or frame and leaf) facing into the cell, and with
    /// `back_texture` the same faces seen from the neighbour. Swung leaves are always drawn from both sides.
    fn wall_quads(
        &self,
        x: usize,
        z: usize,
        level: usize,
        sp: &Span,
        side: usize,
        out: &mut Vec<Quad>,
    ) {
        let c = &self.cells[(level * self.depth + z) * self.width + x];
        let wall = c.walls[side];
        let (fc, cc) = (sp.floor, sp.ceiling);
        let p = self.corners(x as i64, z as i64);
        let (i, j) = (side, (side + 1) % 4);
        let (a, b, n) = (p[i], p[j], INWARD[side]);
        const OFFSETS: [(i64, i64); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
        let (nx, nz) = (x as i64 + OFFSETS[side].0, z as i64 + OFFSETS[side].1);
        // the neighbour span facing this wall (most height in common); none in a solid column
        let facing = self.facing_span(
            nx,
            nz,
            (side + 3) % 4,
            (side + 2) % 4,
            [fc[i], fc[j]],
            [cc[i], cc[j]],
        );
        let nb = match facing {
            Some(f) => self.cell_at(nx, nz, f.level),
            None => self.cell_at(nx, nz, level),
        };
        // the back face shows only towards an open neighbour without a wall of its own on this edge
        let nb_free = facing
            .and(nb)
            .filter(|nb| nb.walls[(side + 2) % 4].texture < 0);
        let back_light = nb.map_or(c.light, |nb| nb.light);
        let two_sided = wall.back_texture >= 0 && nb_free.is_some();
        // wall point at u (along the wall) and t (0 = floor, 1 = ceiling at that u)
        let pt = |u: f32, t: f32| {
            let f = mix(fc[i], fc[j], u);
            let y = if t == 0.0 {
                f
            } else if t == 1.0 {
                mix(cc[i], cc[j], u)
            } else {
                f + (mix(cc[i], cc[j], u) - f) * t
            };
            Vec3::new(mix(a.x, b.x, u), y, mix(a.z, b.z, u))
        };
        let quad = |corners: [Vec3; 4],
                    uv: [[f32; 2]; 4],
                    texture: i32,
                    normal: Vec3,
                    light: f32,
                    part: i32| Quad {
            corners,
            uv,
            texture,
            light,
            normal,
            wrap: false,
            side: side as i32,
            alpha: wall.alpha,
            part,
        };
        // a planar piece u ul..ur, t tb..tt; the back copy has u flipped so its picture is not mirrored
        let mut piece = |ul: f32,
                         ur: f32,
                         tb: f32,
                         tt: f32,
                         uv: [[f32; 2]; 4],
                         texture: i32,
                         back_texture: i32,
                         part: i32| {
            if !(ur > ul && tt > tb) {
                return;
            }
            let k = [pt(ul, tt), pt(ur, tt), pt(ur, tb), pt(ul, tb)];
            out.push(quad(k, uv, texture, n, c.light, part));
            if two_sided {
                let f = |q: [f32; 2]| [1.0 - q[0], q[1]];
                out.push(quad(
                    [k[1], k[0], k[3], k[2]],
                    [f(uv[1]), f(uv[0]), f(uv[3]), f(uv[2])],
                    back_texture,
                    -n,
                    back_light,
                    part | PART_BACK,
                ));
            }
        };
        let (u0, u1, top) = wall.leaf_rect();
        let leaf_tex = if wall.leaf_texture >= 0 {
            wall.leaf_texture
        } else {
            wall.texture
        };
        let leaf_back = if wall.leaf_texture >= 0 {
            wall.leaf_texture
        } else {
            wall.back_texture
        };
        if wall.leaf_texture >= 0 {
            // frame around the leaf rectangle, in wall texture coordinates (v = 0 at the ceiling)
            let fr = |ul: f32, ur: f32, tb: f32, tt: f32| {
                [
                    [ul, 1.0 - tt],
                    [ur, 1.0 - tt],
                    [ur, 1.0 - tb],
                    [ul, 1.0 - tb],
                ]
            };
            piece(
                0.0,
                u0,
                0.0,
                1.0,
                fr(0.0, u0, 0.0, 1.0),
                wall.texture,
                wall.back_texture,
                PART_WALL,
            );
            piece(
                u1,
                1.0,
                0.0,
                1.0,
                fr(u1, 1.0, 0.0, 1.0),
                wall.texture,
                wall.back_texture,
                PART_WALL,
            );
            piece(
                u0,
                u1,
                top,
                1.0,
                fr(u0, u1, top, 1.0),
                wall.texture,
                wall.back_texture,
                PART_WALL,
            );
        }
        let open = wall.opening();
        let part = wall.leaf_part();
        if open >= 1.0 && !wall.is_swing() {
            return;
        }
        let w = u1 - u0;
        match wall.door_mode {
            DOOR_SLIDE_LEFT => piece(
                u0,
                u1 - w * open,
                0.0,
                top,
                [[open, 0.0], [1.0, 0.0], [1.0, 1.0], [open, 1.0]],
                leaf_tex,
                leaf_back,
                part,
            ),
            DOOR_SLIDE_RIGHT => piece(
                u0 + w * open,
                u1,
                0.0,
                top,
                [[0.0, 0.0], [1.0 - open, 0.0], [1.0 - open, 1.0], [0.0, 1.0]],
                leaf_tex,
                leaf_back,
                part,
            ),
            DOOR_SWING_LEFT | DOOR_SWING_RIGHT if wall.swing() != 0.0 => {
                let Some((k, nl, _)) = self.swing_leaf(x as i64, z as i64, level, side) else {
                    return;
                };
                let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
                let inward = wall.swing() > 0.0;
                let light = if inward { c.light } else { back_light };
                out.push(quad(k, uv, leaf_tex, nl, light, part));
                // the back of the leaf: the same picture seen from behind (mirrored, like a painted board);
                // a whole-wall leaf with a back texture shows that one, unmirrored
                let (bt, buv) = if wall.leaf_texture < 0 && wall.back_texture >= 0 {
                    (wall.back_texture, uv)
                } else {
                    (leaf_tex, [uv[1], uv[0], uv[3], uv[2]])
                };
                out.push(quad(
                    [k[1], k[0], k[3], k[2]],
                    buv,
                    bt,
                    -nl,
                    light,
                    part | PART_BACK,
                ));
            }
            // DOOR_UP, unknown modes and closed swing doors: the leaf slides up, its top part hides in the frame
            _ => {
                let open = if wall.is_swing() { 0.0 } else { open };
                piece(
                    u0,
                    u1,
                    open * top,
                    top,
                    [[0.0, open], [1.0, open], [1.0, 1.0], [0.0, 1.0]],
                    leaf_tex,
                    leaf_back,
                    part,
                )
            }
        }
    }

    /// All quads of the cell at (x, z, level); nothing if its span is absent.
    pub fn cell_quads(&self, x: usize, z: usize, level: usize, out: &mut Vec<Quad>) {
        let Some(c) = self.cell_at(x as i64, z as i64, level) else {
            return;
        };
        if c.is_solid() {
            return;
        }
        let Some(sp) = self.span(x as i64, z as i64, level) else {
            return;
        };
        let s = self.cell_size;
        let x0 = x as f32 * s;
        let x1 = x0 + s;
        let z0 = z as f32 * s;
        let z1 = z0 + s;
        let (fc, cc) = (sp.floor, sp.ceiling);
        let (fh, ch) = (fc[0], cc[0]);
        // corners NW, NE, SE, SW at height 0
        let p = [
            Vec3::new(x0, 0.0, z0),
            Vec3::new(x1, 0.0, z0),
            Vec3::new(x1, 0.0, z1),
            Vec3::new(x0, 0.0, z1),
        ];
        let at = |p: Vec3, y: f32| Vec3::new(p.x, y, p.z);
        let full = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let sloped_floor = fc.iter().any(|&h| h != fh);
        let sloped_ceiling = cc.iter().any(|&h| h != ch);
        // One triangle as a quad with its last corner doubled (the second half is empty).
        let tri = |v: [Vec3; 3], uv: [[f32; 2]; 3], texture: i32, side: usize| Quad {
            corners: [v[0], v[1], v[2], v[2]],
            uv: [uv[0], uv[1], uv[2], uv[2]],
            texture,
            light: c.light,
            normal: (v[2] - v[0]).cross(v[1] - v[0]).normalized(),
            wrap: false,
            side: side as i32,
            alpha: 1.0,
            part: PART_WALL,
        };
        if c.floor_texture >= 0 {
            if sloped_floor {
                // Seen from above, NW, NE, SE, SW; split along NW-SE.
                let v = std::array::from_fn::<Vec3, 4, _>(|i| at(p[i], fc[i]));
                for [a, b, d] in [[0, 1, 2], [0, 2, 3]] {
                    out.push(tri(
                        [v[a], v[b], v[d]],
                        [full[a], full[b], full[d]],
                        c.floor_texture,
                        SIDE_FLOOR,
                    ));
                }
            } else {
                // Seen from above, top of the image towards north.
                out.push(Quad {
                    corners: [at(p[0], fh), at(p[1], fh), at(p[2], fh), at(p[3], fh)],
                    uv: full,
                    texture: c.floor_texture,
                    light: c.light,
                    normal: Vec3::new(0.0, 1.0, 0.0),
                    wrap: false,
                    side: SIDE_FLOOR as i32,
                    alpha: 1.0,
                    part: PART_WALL,
                });
            }
        }
        if c.ceiling_texture >= 0 {
            if sloped_ceiling {
                // Seen from below, SW, SE, NE, NW (uv as the flat ceiling); split along NW-SE.
                let v = std::array::from_fn::<Vec3, 4, _>(|i| at(p[i], cc[i]));
                let uv = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]; // NW, NE, SE, SW
                for [a, b, d] in [[3, 2, 0], [2, 1, 0]] {
                    out.push(tri(
                        [v[a], v[b], v[d]],
                        [uv[a], uv[b], uv[d]],
                        c.ceiling_texture,
                        SIDE_CEILING,
                    ));
                }
            } else {
                // Seen from below, top of the image towards south.
                out.push(Quad {
                    corners: [at(p[3], ch), at(p[2], ch), at(p[1], ch), at(p[0], ch)],
                    uv: full,
                    texture: c.ceiling_texture,
                    light: c.light,
                    normal: Vec3::new(0.0, -1.0, 0.0),
                    wrap: false,
                    side: SIDE_CEILING as i32,
                    alpha: 1.0,
                    part: PART_WALL,
                });
            }
        }
        // Inward normal per side; the edge of side i runs from corner i to corner i + 1 (left to right
        // as seen from inside the cell).
        let normals = INWARD;
        let offsets = [(0i64, -1i64), (1, 0), (0, 1), (-1, 0)];
        for side in 0..4 {
            let (i, j) = (side, (side + 1) % 4);
            let (a, b, n) = (p[i], p[j], normals[side]);
            let wall = c.walls[side];
            if wall.texture >= 0 {
                self.wall_quads(x, z, level, &sp, side, out);
                if wall.opening() <= 0.0 {
                    continue;
                }
            }
            // Steps: the solid of the neighbour column seen from this span on the shared edge, i.e. this
            // span's height minus the neighbour's open spans. A piece below a neighbour span (its floor
            // slab) has that span's side texture; the piece above the topmost span has the topmost's.
            let (dx, dz) = offsets[side];
            // the neighbour's corners on the shared edge: ours i is its (i + 3) % 4, ours j its (i + 2) % 4
            let (ni, nj) = ((side + 3) % 4, (side + 2) % 4);
            let face = |corners: [Vec3; 4], uv: [[f32; 2]; 4], tex: i32| Quad {
                corners,
                uv,
                texture: tex,
                light: c.light,
                normal: n,
                wrap: true,
                side: SIDE_STEP as i32,
                alpha: 1.0,
                part: PART_WALL,
            };
            // A face from bottom to top on the edge where gap > 0; split where the gap changes sign.
            let mut riser = |top: [f32; 2], bottom: [f32; 2], gap: [f32; 2], tex: i32| {
                let vh = [(top[0] - bottom[0]) / s, (top[1] - bottom[1]) / s];
                if gap[0] >= 0.0 && gap[1] >= 0.0 && (gap[0] > 0.0 || gap[1] > 0.0) {
                    out.push(face(
                        [
                            at(a, top[0]),
                            at(b, top[1]),
                            at(b, bottom[1]),
                            at(a, bottom[0]),
                        ],
                        [[0.0, 0.0], [1.0, 0.0], [1.0, vh[1]], [0.0, vh[0]]],
                        tex,
                    ));
                } else if gap[0] * gap[1] < 0.0 {
                    let k = gap[0] / (gap[0] - gap[1]);
                    let m = at(a + (b - a) * k, bottom[0] + (bottom[1] - bottom[0]) * k);
                    if gap[0] > 0.0 {
                        out.push(face(
                            [at(a, top[0]), m, m, at(a, bottom[0])],
                            [[0.0, 0.0], [k, 0.0], [k, 0.0], [0.0, vh[0]]],
                            tex,
                        ));
                    } else {
                        out.push(face(
                            [m, at(b, top[1]), at(b, bottom[1]), m],
                            [[k, 0.0], [1.0, 0.0], [1.0, vh[1]], [k, 0.0]],
                            tex,
                        ));
                    }
                }
            };
            let (fa, fb, ca, cb) = (fc[i], fc[j], cc[i], cc[j]);
            let (nx, nz) = (x as i64 + dx, z as i64 + dz);
            // ceiling of the previous neighbour span on the edge, and the texture of the last one
            let mut below: Option<[f32; 2]> = None;
            let mut last_tex = -1;
            for nsp in self.spans(nx, nz) {
                let nb =
                    &self.cells[(nsp.level * self.depth + nz as usize) * self.width + nx as usize];
                let tex = if nb.side_texture >= 0 {
                    nb.side_texture
                } else {
                    nb.floor_texture
                };
                let (nf, nc) = (nsp.floor, nsp.ceiling);
                if tex >= 0 {
                    match below {
                        None => riser(
                            [nf[ni].min(ca), nf[nj].min(cb)],
                            [fa, fb],
                            [nf[ni] - fa, nf[nj] - fb],
                            tex,
                        ),
                        Some(pc) => {
                            let top = [nf[ni].min(ca), nf[nj].min(cb)];
                            let bottom = [pc[0].max(fa), pc[1].max(fb)];
                            riser(top, bottom, [top[0] - bottom[0], top[1] - bottom[1]], tex);
                        }
                    }
                }
                below = Some([nc[ni], nc[nj]]);
                last_tex = tex;
            }
            // a solid neighbour column shows nothing without a wall
            if let (Some(pc), true) = (below, last_tex >= 0) {
                riser(
                    [ca, cb],
                    [pc[0].max(fa), pc[1].max(fb)],
                    [ca - pc[0], cb - pc[1]],
                    last_tex,
                );
            }
        }
    }
}

/// Texture id of the opaque, untextured blocker quads from `GridMap::shadow_quads` (never drawn).
pub const SHADOW_SOLID: i32 = -2;

/// `Quad::side` of shadow-only quads that only the sun needs (roofs, ceiling caps, the outer
/// parts of border boxes): they give thickness against the sun's shadow bias; a point light sees
/// them only behind the faces and ceilings that already block it. Other shadow-only quads: -1.
pub const SIDE_SUN_ONLY: i32 = -2;

/// Height of the shadow-only cap above a textured ceiling, in cell sizes (see `shadow_quads`).
pub const CEILING_CAP: f32 = 1.0 / 16.0;

impl GridMap {
    /// Top of solid column (x, z) as a light blocker (world units): the highest of its own floor
    /// corners (level 0) and of the ceiling corners of the topmost spans of its open neighbours on
    /// the shared edges.
    pub fn solid_top(&self, x: usize, z: usize) -> f32 {
        let c = &self.cells[z * self.width + x];
        let s = self.cell_size;
        let (f, _) = c.corner_heights(s);
        let mut top = f.iter().fold(f32::NEG_INFINITY, |m, &h| m.max(h));
        for (side, (dx, dz)) in [(0i64, -1i64), (1, 0), (0, 1), (-1, 0)]
            .into_iter()
            .enumerate()
        {
            if let Some(nsp) = self.spans(x as i64 + dx, z as i64 + dz).last() {
                let nc = nsp.ceiling;
                top = top.max(nc[(side + 3) % 4]).max(nc[(side + 2) % 4]);
            }
        }
        top
    }

    /// True if column (x, z) is inside the map and has no open span.
    fn column_solid(&self, x: i64, z: i64) -> bool {
        self.index(x, z, 0).is_some() && self.spans(x, z).next().is_none()
    }

    /// Shadow-only quads of one column (all its levels) that close the volume for light; they are
    /// never drawn.
    /// - A solid column (no open span) is a block from its lowest floor corner (level 0) up to
    ///   `solid_top` (at least as tall as the rooms around it; a taller building = a solid column with
    ///   a higher floor_height). It gets a roof and a face on every side where the neighbour does not
    ///   cover it: towards open columns from the floor of their lowest span, towards lower solid
    ///   columns from their top, at the map border from the lowest of those and its own floor.
    ///   Texture `SHADOW_SOLID`. Rock that sticks out nowhere adds nothing.
    /// - Each span with a textured ceiling gets a cap up to `CEILING_CAP` cells above the ceiling
    ///   (same texture and coordinates, so holes in the ceiling stay holes), but never above the
    ///   floor of the span over it. The cap gives the ceiling a thickness: a wall just below it is
    ///   then far enough behind the blocker for the shadow bias. It is raised only at corners between
    ///   closed sides (the neighbour span facing it is roofed, or there is none, or the map border),
    ///   so the roof edge over an open neighbour throws the same shadow as before.
    /// - Such a roofed span at the map border is closed towards the outside by a box one cell
    ///   deep (floor to cap: inner face, roof, outer face). Open-air spans at the border stay
    ///   open: the world outside is open land.
    ///
    /// The solid between spans needs nothing extra: its faces are the step faces and the textured
    /// floors and ceilings around it, which cast shadows themselves.
    ///
    /// A block face is left out where a closed, opaque wall of the open neighbour already covers
    /// it (`opaque(texture)`: the texture has no transparent texel); it then starts at the top of
    /// that wall. Roofs, caps and the outer parts of border boxes have `side` `SIDE_SUN_ONLY`,
    /// the rest -1.
    pub fn shadow_quads(
        &self,
        x: usize,
        z: usize,
        opaque: &dyn Fn(i32) -> bool,
        out: &mut Vec<Quad>,
    ) {
        let c = &self.cells[z * self.width + x];
        let s = self.cell_size;
        let (x0, z0) = (x as f32 * s, z as f32 * s);
        let p = [
            Vec3::new(x0, 0.0, z0),
            Vec3::new(x0 + s, 0.0, z0),
            Vec3::new(x0 + s, 0.0, z0 + s),
            Vec3::new(x0, 0.0, z0 + s),
        ];
        let at = |p: Vec3, y: f32| Vec3::new(p.x, y, p.z);
        let offsets = [(0i64, -1i64), (1, 0), (0, 1), (-1, 0)];
        let solid = |corners: [Vec3; 4], normal: Vec3, side: i32| Quad {
            corners,
            uv: [[0.0; 2]; 4],
            texture: SHADOW_SOLID,
            light: 1.0,
            normal,
            wrap: false,
            side,
            alpha: 1.0,
            part: 0,
        };
        // outward normal of side i of this cell
        let outward = [
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(-1.0, 0.0, 0.0),
        ];
        let mut spans = self.spans(x as i64, z as i64).peekable();
        if spans.peek().is_none() {
            let top = self.solid_top(x, z);
            let (f, _) = c.corner_heights(s);
            // where each neighbour stops covering this side: open columns at the floor of their
            // lowest span, solid ones at their top; the map border at the lowest of those and of
            // this cell's floor
            let mut bottom: [Option<f32>; 4] = [None; 4];
            let mut ground = f.iter().fold(f32::INFINITY, |m, &h| m.min(h));
            for side in 0..4 {
                let (dx, dz) = offsets[side];
                let (nx, nz) = (x as i64 + dx, z as i64 + dz);
                let mut covered = None;
                bottom[side] = self.index(nx, nz, 0).map(|_| {
                    let Some(nsp) = self.spans(nx, nz).next() else {
                        return self.solid_top(nx as usize, nz as usize);
                    };
                    let nb = &self.cells
                        [(nsp.level * self.depth + nz as usize) * self.width + nx as usize];
                    let (nf, nc) = (nsp.floor, nsp.ceiling);
                    let (ni, nj) = ((side + 3) % 4, (side + 2) % 4);
                    let w = nb.walls[(side + 2) % 4];
                    if w.texture >= 0 && w.alpha >= 1.0 && w.open <= 0.0 && opaque(w.texture) {
                        covered = Some(nc[ni].min(nc[nj]));
                    }
                    nf[ni].min(nf[nj])
                });
                if let Some(b) = bottom[side] {
                    ground = ground.min(b);
                }
                if covered.is_some() {
                    bottom[side] = covered;
                }
            }
            let mut any = false;
            for side in 0..4 {
                let b = bottom[side].unwrap_or(ground);
                let (nx, nz) = (x as i64 + offsets[side].0, z as i64 + offsets[side].1);
                any |= self.index(nx, nz, 0).is_some() && !self.column_solid(nx, nz);
                if !(b < top) {
                    continue;
                }
                let (pa, pb) = (p[side], p[(side + 1) % 4]);
                out.push(solid(
                    [at(pb, top), at(pa, top), at(pa, b), at(pb, b)],
                    outward[side],
                    -1,
                ));
                any = true;
            }
            if any {
                out.push(solid(
                    [at(p[0], top), at(p[1], top), at(p[2], top), at(p[3], top)],
                    Vec3::new(0.0, 1.0, 0.0),
                    SIDE_SUN_ONLY,
                ));
            }
            return;
        }
        while let Some(sp) = spans.next() {
            let c = &self.cells[(sp.level * self.depth + z) * self.width + x];
            if c.ceiling_texture < 0 {
                continue;
            }
            let (fc, cc) = (sp.floor, sp.ceiling);
            // A side is closed when the neighbour span facing it is roofed, when there is none (solid),
            // or outside the map. The cap is raised only at corners between two closed sides: over an
            // open side it stays at the ceiling, so the shadow the roof edge throws outside does not grow.
            let closed: [bool; 4] = std::array::from_fn(|side| {
                let (dx, dz) = offsets[side];
                let (nx, nz) = (x as i64 + dx, z as i64 + dz);
                let (i, j) = (side, (side + 1) % 4);
                match self.facing_span(
                    nx,
                    nz,
                    (side + 3) % 4,
                    (side + 2) % 4,
                    [fc[i], fc[j]],
                    [cc[i], cc[j]],
                ) {
                    Some(f) => {
                        self.cells[(f.level * self.depth + nz as usize) * self.width + nx as usize]
                            .ceiling_texture
                            >= 0
                    }
                    None => true,
                }
            });
            let cap = CEILING_CAP * s;
            // never into the span above
            let room: [f32; 4] = match spans.peek() {
                Some(up) => std::array::from_fn(|k| (up.floor[k] - cc[k]).clamp(0.0, cap)),
                None => [cap; 4],
            };
            let top: [f32; 4] = std::array::from_fn(|k| {
                cc[k]
                    + if closed[k] && closed[(k + 3) % 4] {
                        room[k]
                    } else {
                        0.0
                    }
            });
            if top != cc {
                // Same split and texture coordinates as the ceiling in `cell_quads`.
                let v = std::array::from_fn::<Vec3, 4, _>(|i| at(p[i], top[i]));
                let uv = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]; // NW, NE, SE, SW
                let n = Vec3::new(0.0, 1.0, 0.0);
                for [a, b, d] in [[3, 2, 0], [2, 1, 0]] {
                    out.push(Quad {
                        corners: [v[a], v[b], v[d], v[d]],
                        uv: [uv[a], uv[b], uv[d], uv[d]],
                        texture: c.ceiling_texture,
                        light: c.light,
                        normal: n,
                        wrap: false,
                        side: SIDE_SUN_ONLY,
                        alpha: 1.0,
                        part: 0,
                    });
                }
            }
            // the map border: a closed box one cell deep, floor to cap
            for side in 0..4 {
                let (dx, dz) = offsets[side];
                if self.index(x as i64 + dx, z as i64 + dz, 0).is_some() {
                    continue;
                }
                let (i, j) = (side, (side + 1) % 4);
                let (a, b) = (p[i], p[j]);
                let o = outward[side] * s;
                out.push(solid(
                    [at(b, top[j]), at(a, top[i]), at(a, fc[i]), at(b, fc[j])],
                    outward[side],
                    -1,
                ));
                out.push(solid(
                    [
                        at(a, top[i]),
                        at(b, top[j]),
                        at(b + o, top[j]),
                        at(a + o, top[i]),
                    ],
                    Vec3::new(0.0, 1.0, 0.0),
                    SIDE_SUN_ONLY,
                ));
                out.push(solid(
                    [
                        at(a + o, top[i]),
                        at(b + o, top[j]),
                        at(b + o, fc[j]),
                        at(a + o, fc[i]),
                    ],
                    outward[side],
                    SIDE_SUN_ONLY,
                ));
            }
        }
    }
}

#[cfg(test)]
mod shadow_quad_tests {
    use super::*;

    fn map() -> GridMap {
        // 3 x 1: solid | room (ceiling 1, textured) | solid with floor 3 (tall building)
        let mut g = GridMap::new(3, 1, 1.0);
        for x in [0, 2] {
            g.cell_mut(x, 0).unwrap().ceiling_height = 0.0;
        }
        g.cell_mut(2, 0).unwrap().floor_height = 3.0;
        let r = g.cell_mut(1, 0).unwrap();
        r.floor_texture = 0;
        r.ceiling_texture = 1;
        g
    }

    #[test]
    fn solid_cells_become_blocks() {
        let g = map();
        let mut q = Vec::new();
        g.shadow_quads(0, 0, &|_| true, &mut q);
        // as tall as the room next to it: faces on all four sides (three at the border), roof at 1
        assert_eq!(q.len(), 5);
        assert!(q.iter().all(|q| q.texture == SHADOW_SOLID));
        let east: Vec<&Quad> = q
            .iter()
            .filter(|q| q.corners.iter().all(|c| c.x == 1.0))
            .collect();
        assert_eq!(east.len(), 1);
        assert_eq!(east[0].corners.map(|c| c.y), [1.0, 1.0, 0.0, 0.0]);
        assert!(q[4].corners.iter().all(|c| c.y == 1.0));
        q.clear();
        // the tall building: its own floor height 3 is the top
        g.shadow_quads(2, 0, &|_| true, &mut q);
        assert_eq!(q.len(), 5);
        assert!(q[4].corners.iter().all(|c| c.y == 3.0));
        // the room's own closed, opaque wall already covers the face towards it
        let mut walled = map();
        walled.cell_mut(1, 0).unwrap().walls[WEST] = Wall {
            texture: 2,
            ..Wall::default()
        };
        for (opaque, n) in [(true, 4), (false, 5)] {
            q.clear();
            walled.shadow_quads(0, 0, &|_| opaque, &mut q);
            assert_eq!(q.len(), n, "opaque wall: {opaque}");
        }
        // a lower solid neighbour: the face starts at its top
        let mut g = GridMap::new(3, 1, 1.0);
        g.cells.iter_mut().for_each(|c| c.ceiling_height = 0.0);
        g.cell_mut(1, 0).unwrap().floor_height = 2.0;
        g.cell_mut(2, 0).unwrap().floor_height = 0.5;
        q.clear();
        g.shadow_quads(1, 0, &|_| true, &mut q);
        let east: Vec<&Quad> = q
            .iter()
            .filter(|q| q.corners.iter().all(|c| c.x == 2.0))
            .collect();
        assert_eq!(east[0].corners.map(|c| c.y), [2.0, 2.0, 0.5, 0.5]);
        // flat rock adds nothing
        q.clear();
        g.cell_mut(1, 0).unwrap().floor_height = 0.0;
        g.cell_mut(2, 0).unwrap().floor_height = 0.0;
        g.shadow_quads(0, 0, &|_| true, &mut q);
        assert!(q.is_empty());
    }

    #[test]
    fn roofed_cell_gets_cap_and_closed_border() {
        let g = map();
        let mut q = Vec::new();
        g.shadow_quads(1, 0, &|_| true, &mut q);
        // cap (two triangles, raised everywhere: all sides closed) plus a box at the north and
        // south map border (inner face, roof, outer face)
        assert_eq!(q.len(), 8);
        assert!(
            q[..2]
                .iter()
                .all(|q| q.texture == 1 && q.corners.iter().all(|c| c.y == 1.0 + CEILING_CAP))
        );
        assert!(q[2..].iter().all(|q| q.texture == SHADOW_SOLID));
        // an open-air neighbour in the east: the cap stays at the ceiling on that side
        let mut g = map();
        let e = g.cell_mut(2, 0).unwrap();
        e.floor_height = 0.0;
        e.ceiling_height = 2.0;
        q.clear();
        g.shadow_quads(1, 0, &|_| true, &mut q);
        for c in q[..2].iter().flat_map(|q| q.corners) {
            assert_eq!(c.y, if c.x == 2.0 { 1.0 } else { 1.0 + CEILING_CAP });
        }
        // open-air cells add nothing
        let mut g = map();
        g.cell_mut(1, 0).unwrap().ceiling_texture = -1;
        q.clear();
        g.shadow_quads(1, 0, &|_| true, &mut q);
        assert!(q.is_empty());
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;

    /// A varied, random-ish one-level map: heights, slopes, solid cells, walls, doors, back faces.
    pub(crate) fn varied_map() -> GridMap {
        let mut g = GridMap::new(9, 8, 1.0);
        let mut seed = 12345u32;
        let mut rnd = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        for c in g.cells.iter_mut() {
            let r = rnd();
            c.floor_height = (rnd() * 4.0).floor() * 0.25 - 0.25;
            c.ceiling_height = c.floor_height + 0.5 + (rnd() * 4.0).floor() * 0.5;
            if r < 0.15 {
                c.ceiling_height = c.floor_height;
            }
            c.floor_texture = if rnd() < 0.85 { 1 } else { -1 };
            c.ceiling_texture = if rnd() < 0.5 { 2 } else { -1 };
            c.side_texture = if rnd() < 0.5 { 3 } else { -1 };
            c.light = rnd();
            if rnd() < 0.3 {
                for k in 0..4 {
                    c.floor_corners[k] = (rnd() * 3.0).floor() * 0.25;
                    c.ceiling_corners[k] = (rnd() * 3.0).floor() * 0.25 - 0.25;
                }
            }
            for side in 0..4 {
                if rnd() < 0.3 {
                    let w = &mut c.walls[side];
                    w.texture = 4;
                    w.back_texture = if rnd() < 0.5 { 5 } else { -1 };
                    w.open = if rnd() < 0.3 { rnd() } else { 0.0 };
                    w.door_mode = (rnd() * 5.0).floor() as i32;
                    w.swing_angle = rnd() * 300.0 - 150.0;
                    if rnd() < 0.4 {
                        w.leaf_texture = 6;
                        w.leaf_u0 = 0.2;
                        w.leaf_u1 = 0.8;
                        w.leaf_top = 0.7;
                    }
                    w.alpha = if rnd() < 0.2 { 0.5 } else { 1.0 };
                }
            }
        }
        g
    }

    pub(crate) fn hash_quads(q: &[Quad], h: &mut u64) {
        let mut mixin = |v: u64| {
            *h ^= v;
            *h = h.wrapping_mul(0x100000001b3);
        };
        for q in q {
            for c in q.corners {
                mixin(c.x.to_bits() as u64);
                mixin(c.y.to_bits() as u64);
                mixin(c.z.to_bits() as u64);
            }
            for uv in q.uv {
                mixin(uv[0].to_bits() as u64);
                mixin(uv[1].to_bits() as u64);
            }
            for v in [q.normal.x, q.normal.y, q.normal.z, q.light, q.alpha] {
                mixin(v.to_bits() as u64);
            }
            mixin(q.texture as u64);
            mixin(q.side as u64);
            mixin(q.part as u64);
            mixin(q.wrap as u64);
        }
        mixin(q.len() as u64);
    }

    /// Hashes of all `cell_quads` and `shadow_quads` of `varied_map`, and of a fan of raycasts, taken on
    /// main before storeys existed: one-level grids must give exactly the same output.
    #[test]
    fn one_level_output_matches_snapshot() {
        let g = varied_map();
        let (mut hc, mut hs) = (0xcbf29ce484222325u64, 0xcbf29ce484222325u64);
        for z in 0..g.depth {
            for x in 0..g.width {
                let mut q = Vec::new();
                g.cell_quads(x, z, 0, &mut q);
                hash_quads(&q, &mut hc);
                q.clear();
                g.shadow_quads(x, z, &|t| t != 5, &mut q);
                hash_quads(&q, &mut hs);
            }
        }
        let mut hr = 0xcbf29ce484222325u64;
        for i in 0..400 {
            let a = i as f32 * 0.137;
            let o = Vec3::new(
                0.5 + (i % 9) as f32,
                0.3 + (i % 5) as f32 * 0.2,
                0.5 + (i % 8) as f32,
            );
            let d = Vec3::new(a.cos(), (i as f32 * 0.31).sin() * 0.7, a.sin());
            let r = g.raycast(o, d, 30.0);
            let v = r.map_or(0u64, |h| {
                (h.cell_x as u64)
                    ^ ((h.cell_z as u64) << 8)
                    ^ ((h.side as u64) << 16)
                    ^ ((h.part as u64) << 24)
                    ^ ((h.distance.to_bits() as u64) << 32)
            });
            hr ^= v;
            hr = hr.wrapping_mul(0x100000001b3);
        }
        println!("snapshot {hc:#x} {hs:#x} {hr:#x}");
        assert_eq!(
            (hc, hs, hr),
            (0xa2a48f9d54fbce92, 0xdb47fc9dd89a2f16, 0x4aa685d26a8b16e5)
        );
    }
}

#[cfg(test)]
mod storey_tests {
    use super::*;

    fn open(floor: f32, ceiling: f32, tex: i32) -> Cell {
        Cell {
            floor_height: floor,
            ceiling_height: ceiling,
            floor_texture: tex,
            side_texture: tex,
            ..Cell::default()
        }
    }

    /// Step faces of cell (x, z, level) on side `side`: (texture, height bottom-left, height top-left).
    fn steps(g: &GridMap, x: usize, z: usize, level: usize, side: usize) -> Vec<(i32, f32, f32)> {
        let mut q = Vec::new();
        g.cell_quads(x, z, level, &mut q);
        let n = INWARD[side];
        q.iter()
            .filter(|q| q.side == SIDE_STEP as i32 && q.normal == n)
            .map(|q| (q.texture, q.corners[3].y, q.corners[0].y))
            .collect()
    }

    /// 2 x 1: an open cell 0..3 in the west, in the east a column with spans 0..1 (texture 1) and
    /// 1.5..2.5 (texture 2): a half-height neighbour with a balcony above.
    fn balcony() -> GridMap {
        let mut g = GridMap::with_levels(2, 1, 2, 1.0);
        *g.cell_mut(0, 0).unwrap() = open(0.0, 3.0, 9);
        *g.cell_mut(1, 0).unwrap() = open(0.0, 1.0, 1);
        *g.cell_at_mut(1, 0, 1).unwrap() = open(1.5, 2.5, 2);
        g
    }

    #[test]
    fn faces_between_spans_have_the_upper_spans_texture() {
        let g = balcony();
        // seen from the open cell: the slab 1..1.5 (the floor slab of the balcony, its texture) and the
        // solid above the balcony 2.5..3 (texture of the topmost span)
        assert_eq!(steps(&g, 0, 0, 0, EAST), vec![(2, 1.0, 1.5), (2, 2.5, 3.0)]);
        // from the low room and the balcony the open cell covers everything: no faces
        assert!(steps(&g, 1, 0, 0, WEST).is_empty());
        assert!(steps(&g, 1, 0, 1, WEST).is_empty());
        // an untextured upper span: its slab shows nothing (as an untextured step on one level)
        let mut g = balcony();
        g.cell_at_mut(1, 0, 1).unwrap().side_texture = -1;
        g.cell_at_mut(1, 0, 1).unwrap().floor_texture = -1;
        assert!(steps(&g, 0, 0, 0, EAST).is_empty());
        // a neighbour span wholly above: one face up to my ceiling; one wholly below: the solid above it
        let mut g = GridMap::with_levels(2, 1, 2, 1.0);
        *g.cell_mut(0, 0).unwrap() = open(0.0, 1.0, 9);
        *g.cell_mut(1, 0).unwrap() = Cell {
            ceiling_height: 0.0,
            ..Cell::default()
        };
        *g.cell_at_mut(1, 0, 1).unwrap() = open(2.0, 3.0, 2);
        assert_eq!(steps(&g, 0, 0, 0, EAST), vec![(2, 0.0, 1.0)]);
        *g.cell_mut(0, 0).unwrap() = open(3.5, 4.0, 9);
        assert_eq!(steps(&g, 0, 0, 0, EAST), vec![(2, 3.5, 4.0)]);
    }

    #[test]
    fn sloped_slab_between_spans_is_a_triangle() {
        // the gable: an open cell 0..4 next to a room 0..1.5 with a roof span whose floor rises from 1.5
        // (north) to 2.5 (south) on the shared edge
        let mut g = GridMap::with_levels(2, 1, 2, 1.0);
        *g.cell_mut(0, 0).unwrap() = open(0.0, 4.0, 9);
        *g.cell_mut(1, 0).unwrap() = open(0.0, 1.5, 1);
        *g.cell_at_mut(1, 0, 1).unwrap() = Cell {
            floor_corners: [0.0, 0.0, 1.0, 1.0],
            ..open(1.5, 4.0, 2)
        };
        let mut q = Vec::new();
        g.cell_quads(0, 0, 0, &mut q);
        let s: Vec<&Quad> = q.iter().filter(|q| q.side == SIDE_STEP as i32).collect();
        assert_eq!(s.len(), 1);
        // corners top-left, top-right, bottom-right, bottom-left seen from the west: no height at the north end
        let ys = s[0].corners.map(|c| (c.z, c.y));
        assert_eq!(ys, [(0.0, 1.5), (1.0, 2.5), (1.0, 1.5), (0.0, 1.5)]);
        assert_eq!(s[0].texture, 2);
    }

    #[test]
    fn overlapping_and_unordered_spans() {
        let mut g = GridMap::with_levels(1, 1, 3, 1.0);
        *g.cell_mut(0, 0).unwrap() = open(0.0, 2.0, 1);
        // reaches into the span below: its floor is raised to 2
        *g.cell_at_mut(0, 0, 1).unwrap() = open(1.0, 3.0, 2);
        // wholly inside the spans below: absent
        *g.cell_at_mut(0, 0, 2).unwrap() = open(0.5, 2.5, 3);
        let sp: Vec<Span> = g.spans(0, 0).collect();
        assert_eq!(sp.len(), 2);
        assert_eq!(
            (sp[1].level, sp[1].floor, sp[1].ceiling),
            (1, [2.0; 4], [3.0; 4])
        );
        assert!(g.span(0, 0, 2).is_none());
        let mut q = Vec::new();
        g.cell_quads(0, 0, 2, &mut q);
        assert!(q.is_empty(), "an absent span draws nothing");
        g.cell_quads(0, 0, 1, &mut q);
        assert!(
            q.iter()
                .any(|q| q.side == SIDE_FLOOR as i32 && q.corners.iter().all(|c| c.y == 2.0))
        );
        // given top to bottom: the lower level wins, the upper one (below it) is absent
        let mut g = GridMap::with_levels(1, 1, 2, 1.0);
        *g.cell_mut(0, 0).unwrap() = open(2.0, 3.0, 1);
        *g.cell_at_mut(0, 0, 1).unwrap() = open(0.0, 1.0, 2);
        assert_eq!(g.spans(0, 0).map(|s| s.level).collect::<Vec<_>>(), vec![0]);
        // per corner: a sloped upper floor is raised only where it dips into the span below
        let mut g = GridMap::with_levels(1, 1, 2, 1.0);
        *g.cell_mut(0, 0).unwrap() = open(0.0, 1.0, 1);
        *g.cell_at_mut(0, 0, 1).unwrap() = Cell {
            floor_corners: [-0.5, -0.5, 0.5, 0.5],
            ..open(1.0, 3.0, 2)
        };
        assert_eq!(g.span(0, 0, 1).unwrap().floor, [1.0, 1.0, 1.5, 1.5]);
        // surface_at and span_containing follow the rule
        let p = Vec3::new(0.5, 1.0, 0.5);
        assert_eq!(g.surface_at(0, 0, 1, p), Some((1.25, 3.0)));
        assert_eq!(
            g.span_containing(0, 0, p),
            Some((0, 0.0, 1.0)),
            "the lower span where two touch"
        );
        assert_eq!(
            g.span_containing(0, 0, Vec3::new(0.5, 2.0, 0.5))
                .map(|s| s.0),
            Some(1)
        );
        assert!(g.span_containing(0, 0, Vec3::new(0.5, 3.5, 0.5)).is_none());
    }

    #[test]
    fn new_levels_start_solid() {
        let g = GridMap::with_levels(2, 3, 4, 1.0);
        assert_eq!((g.levels, g.cells.len()), (4, 24));
        assert!(g.cell(1, 2).is_some_and(|c| !c.is_solid()));
        assert!((1..4).all(|l| g.cell_at(1, 2, l).is_some_and(|c| c.is_solid())));
        assert!(g.cell_at(0, 0, 4).is_none());
        assert_eq!(GridMap::with_levels(1, 1, 0, 1.0).levels, 1);
    }

    /// 2 x 1, two levels; the upper span of (0, 0) 1.25..2.25 has an east wall; (1, 0) has spans
    /// 0..1 and (with `balcony`) 1.25..2.25.
    fn upper_wall(wall: Wall, balcony: bool) -> GridMap {
        let mut g = GridMap::with_levels(2, 1, 2, 1.0);
        *g.cell_mut(0, 0).unwrap() = open(0.0, 1.0, 1);
        *g.cell_mut(1, 0).unwrap() = Cell {
            light: 0.25,
            ..open(0.0, 1.0, 1)
        };
        let up = g.cell_at_mut(0, 0, 1).unwrap();
        *up = open(1.25, 2.25, 2);
        up.walls[EAST] = wall;
        if balcony {
            *g.cell_at_mut(1, 0, 1).unwrap() = Cell {
                light: 0.5,
                ..open(1.25, 2.25, 2)
            };
        }
        g
    }

    #[test]
    fn doors_and_back_faces_on_an_upper_level() {
        let w = Wall {
            texture: 7,
            back_texture: 8,
            ..Wall::default()
        };
        let mut q = Vec::new();
        upper_wall(w, true).cell_quads(0, 0, 1, &mut q);
        let walls: Vec<&Quad> = q.iter().filter(|q| q.side == EAST as i32).collect();
        assert_eq!(walls.len(), 2);
        assert!(
            walls[0].corners.iter().all(|c| c.y >= 1.25 && c.y <= 2.25),
            "the wall covers its span"
        );
        assert_eq!(
            (walls[1].texture, walls[1].part, walls[1].light),
            (8, PART_BACK, 0.5),
            "back face lit as the balcony"
        );
        // without the balcony the back face goes with the low room (most height in common, though none:
        // as on one level, where a back face is drawn whenever the neighbour is open)
        q.clear();
        upper_wall(w, false).cell_quads(0, 0, 1, &mut q);
        let walls: Vec<&Quad> = q.iter().filter(|q| q.side == EAST as i32).collect();
        assert_eq!((walls.len(), walls[1].light), (2, 0.25));
        // a solid column behind it: no back face
        let mut g = upper_wall(w, false);
        g.cell_mut(1, 0).unwrap().ceiling_height = 0.0;
        q.clear();
        g.cell_quads(0, 0, 1, &mut q);
        assert_eq!(q.iter().filter(|q| q.side == EAST as i32).count(), 1);
        // a door half up: the ray from the balcony passes below the leaf and is stopped above it
        let door = Wall {
            texture: 7,
            open: 0.5,
            ..Wall::default()
        };
        let g = upper_wall(door, true);
        let below = g.raycast(Vec3::new(1.5, 1.4, 0.5), Vec3::new(-1.0, 0.0, 0.0), 5.0);
        assert!(
            below.is_none(),
            "through the open part into the room and out of the map: {below:?}"
        );
        let above = g
            .raycast(Vec3::new(1.5, 2.0, 0.5), Vec3::new(-1.0, 0.0, 0.0), 5.0)
            .unwrap();
        assert_eq!(
            (above.cell_x, above.level, above.side, above.part),
            (0, 1, EAST as i32, PART_WALL)
        );
        assert!((above.distance - 0.5).abs() < 1e-5);
        // a swing door on the upper level swings at the height of that level
        let swing = Wall {
            texture: 7,
            open: 1.0,
            door_mode: DOOR_SWING_LEFT,
            ..Wall::default()
        };
        let g = upper_wall(swing, true);
        let (k, _, _) = g.swing_leaf(0, 0, 1, EAST).unwrap();
        assert_eq!((k[0].y, k[3].y), (2.25, 1.25));
        assert!(g.swing_leaf(0, 0, 0, EAST).is_none());
        q.clear();
        g.cell_quads(0, 0, 1, &mut q);
        assert_eq!(
            q.iter().filter(|q| q.side == EAST as i32).count(),
            2,
            "leaf front and back"
        );
        // the swung leaf (now along the north edge, inside the room) stops a ray going north there
        let h = g
            .raycast(Vec3::new(0.5, 1.75, 0.9), Vec3::new(0.0, 0.0, -1.0), 5.0)
            .unwrap();
        assert_eq!((h.level, h.side, h.part), (1, EAST as i32, PART_WALL));
    }

    #[test]
    fn shadow_cap_stays_below_the_span_above() {
        // a room 0..1 with a textured ceiling and a room right on top (floor 1): no cap into it
        let mut g = GridMap::with_levels(1, 1, 2, 1.0);
        *g.cell_mut(0, 0).unwrap() = Cell {
            ceiling_texture: 3,
            ..open(0.0, 1.0, 1)
        };
        *g.cell_at_mut(0, 0, 1).unwrap() = Cell {
            ceiling_texture: 3,
            ..open(1.0, 2.0, 1)
        };
        let mut q = Vec::new();
        g.shadow_quads(0, 0, &|_| true, &mut q);
        // the lower room: no cap (nothing between the spans), border boxes up to its ceiling; the upper
        // room: cap and border boxes
        let caps: Vec<&Quad> = q.iter().filter(|q| q.texture == 3).collect();
        assert_eq!(caps.len(), 2);
        assert!(
            caps.iter()
                .all(|q| q.corners.iter().all(|c| c.y == 2.0 + CEILING_CAP))
        );
        assert_eq!(
            q.iter().filter(|q| q.texture == SHADOW_SOLID).count(),
            2 * 4 * 3
        );
        // a slab between: the cap grows into it, but not past the floor above
        g.cell_at_mut(0, 0, 1).unwrap().floor_height = 1.03;
        q.clear();
        g.shadow_quads(0, 0, &|_| true, &mut q);
        let low_cap = q.iter().find(|q| q.texture == 3).unwrap();
        assert!(low_cap.corners.iter().all(|c| (c.y - 1.03).abs() < 1e-6));
    }

    #[test]
    fn raycast_starts_in_the_span_of_the_origin() {
        let g = balcony();
        // from the balcony west: it leaves into the open cell (it contains 2.0) and goes out of the map
        assert!(
            g.raycast(Vec3::new(1.5, 2.0, 0.5), Vec3::new(-1.0, 0.0, 0.0), 5.0)
                .is_none()
        );
        // from the open cell east at 1.2: the slab between the spans
        let h = g
            .raycast(Vec3::new(0.5, 1.2, 0.5), Vec3::new(1.0, 0.0, 0.0), 5.0)
            .unwrap();
        assert_eq!((h.cell_x, h.level, h.side), (0, 0, SIDE_STEP as i32));
        // at 2.0 into the balcony, then out of the map
        assert!(
            g.raycast(Vec3::new(0.5, 2.0, 0.5), Vec3::new(1.0, 0.0, 0.0), 5.0)
                .is_none()
        );
        // down from the balcony: its textured floor
        let h = g
            .raycast(Vec3::new(1.5, 2.0, 0.5), Vec3::new(0.0, -1.0, 0.0), 5.0)
            .unwrap();
        assert_eq!((h.cell_x, h.level, h.side), (1, 1, SIDE_FLOOR as i32));
        assert!((h.distance - 0.5).abs() < 1e-6);
    }
}
