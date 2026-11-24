// Origin: chiaro @ 9bc5b9b, src/shadows/StoredObstacle.hpp and StoredObstacle.cpp, plus
// MakeTileCells / CountCells of src/shadows/TileGrid.cpp (the creation part of the cells; editing
// cells and the cell log are P14).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! An obstacle as stored: owned copies of the shape, no caller memory (P13).
//!
//! Redesign against the C++ version: triangles and outlines are not owned per obstacle but live
//! in two arenas of the [`ObstacleStore`](super::ObstacleStore); an obstacle only holds a [`Span`]
//! into each (P13b, so a restore allocates nothing, ADR 0010). Read them through
//! [`ObstacleStore::triangles`](super::ObstacleStore::triangles) and
//! [`ObstacleStore::outline`](super::ObstacleStore::outline). The pattern fields of
//! `StoredObstacle` live in [`PatternAssignment`] (P14). The cell channels of a tile grid and the
//! pattern table and cell indices are in byte and word arenas of the store as well (P14), so a
//! restore allocates nothing for them either; read them through
//! [`ObstacleStore::tile`](super::ObstacleStore::tile).
//!
//! The snapshot header of an obstacle holds the description and the spans. The store saves the
//! arenas exactly and checks on restore that the stored triangles are those of the description,
//! so a tampered stream is [`SnapshotError::Invalid`].

use super::desc::{ObstacleCommon, Shape, ShapeDesc, TileGridDesc};
use super::error::ObstacleError;
use super::patterns::PatternAssignment;
use crate::triangulate::{Triangulator, circle_triangles};
use knorke_math::{Vec2, det};
use knorke_types::{Reader, SnapshotError, SnapshotValue, Writer};

/// A range of an arena of the store: `len` elements from `start`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    /// First element.
    pub start: u32,
    /// Number of elements.
    pub len: u32,
}

impl Span {
    /// No elements.
    pub const EMPTY: Self = Self { start: 0, len: 0 };

    /// `true` for no elements.
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

impl SnapshotValue for Span {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.start);
        w.put(&self.len);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Self {
            start: r.get()?,
            len: r.get()?,
        })
    }
}

/// What an obstacle is made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObstacleKind {
    /// A simple polygon (also a box).
    Polygon,
    /// A circle.
    Circle,
    /// A grid of cells.
    TileGrid,
}

/// The cells of a tile grid. Every channel is a [`Span`] of the byte arena of the store holding
/// `columns * rows` bytes, or [`Span::EMPTY`] when the channel is unused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileCells {
    /// Columns.
    pub columns: u32,
    /// Rows.
    pub rows: u32,
    /// World size of one cell.
    pub cell_size: Vec2,
    /// Height of one level, `0` for none.
    pub level_height: f32,
    /// Width of the edge strips.
    pub edge_width: f32,
    /// Top level per cell, `0` empty.
    pub solid: Span,
    /// Bottom level per cell, or empty.
    pub base_levels: Span,
    /// Opacity per cell (255 opaque), or empty.
    pub opacity: Span,
    /// Edge bits per cell, or empty.
    pub edges: Span,
    /// Highest solid value, for the finite top with `level_height`.
    pub max_level: u8,
    /// Solid cells with an opacity below 255.
    pub partial_cells: u32,
    /// Solid cells with edge bits.
    pub edge_cells: u32,
}

impl Default for TileCells {
    fn default() -> Self {
        Self {
            columns: 0,
            rows: 0,
            cell_size: Vec2::new(1.0, 1.0),
            level_height: 0.0,
            edge_width: 0.0,
            solid: Span::EMPTY,
            base_levels: Span::EMPTY,
            opacity: Span::EMPTY,
            edges: Span::EMPTY,
            max_level: 0,
            partial_cells: 0,
            edge_cells: 0,
        }
    }
}

impl TileCells {
    /// Back to "no grid".
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    /// Takes over a validated description whose channels the store put into its byte arena
    /// (`channels` = solid, base levels, opacity, edges) and counts the derived values.
    pub(super) fn assign(&mut self, desc: &TileGridDesc<'_>, channels: [Span; 4]) {
        self.columns = desc.columns;
        self.rows = desc.rows;
        self.cell_size = desc.cell_size;
        self.level_height = desc.level_height;
        self.edge_width = desc.edge_width;
        [self.solid, self.base_levels, self.opacity, self.edges] = channels;
        self.count(
            desc.solid,
            desc.opacity.unwrap_or(&[]),
            desc.edges.unwrap_or(&[]),
        );
    }

    /// Counts the derived values of the cells from scratch (absent channels are empty slices).
    pub(super) fn count(&mut self, solid: &[u8], opacity: &[u8], edges: &[u8]) {
        self.max_level = 0;
        self.partial_cells = 0;
        self.edge_cells = 0;
        for (i, &s) in solid.iter().enumerate() {
            if s == 0 {
                continue;
            }
            if s > self.max_level {
                self.max_level = s;
            }
            self.partial_cells += u32::from(opacity.get(i).is_some_and(|&o| o < 255));
            self.edge_cells += u32::from(edges.get(i).is_some_and(|&e| e != 0));
        }
    }
}

/// An obstacle as stored: owned copies of the shape.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredObstacle {
    /// Polygon, circle or tile grid.
    pub kind: ObstacleKind,
    /// What every obstacle has.
    pub common: ObstacleCommon,
    /// Polygon and circle: local-space triangle list (three vertices per triangle) in the
    /// triangle arena of the store.
    pub triangles: Span,
    /// Tile grids only.
    pub grid: TileCells,
    /// Grows on every replacement of the obstacle (not on a move).
    pub revision: u32,
    /// Circles: the radius (thin poles are widened in the mask).
    pub radius: f32,
    /// Polygons: the local corners of the description, in the outline arena of the store.
    pub outline: Span,
    /// Polygons: the description was a box; the outline holds its four corners.
    pub box_shape: bool,
    /// Patterns assigned to the obstacle or its cells (P14).
    pub patterns: PatternAssignment,
}

impl StoredObstacle {
    pub(super) fn empty() -> Self {
        Self {
            kind: ObstacleKind::Polygon,
            common: ObstacleCommon::default(),
            triangles: Span::EMPTY,
            grid: TileCells::default(),
            revision: 1,
            radius: 0.0,
            outline: Span::EMPTY,
            box_shape: false,
            patterns: PatternAssignment::default(),
        }
    }

    /// `true` for tile grids.
    pub fn is_tile_grid(&self) -> bool {
        self.kind == ObstacleKind::TileGrid
    }

    /// `true` if the obstacle takes part in masks and fields: enabled with an opacity above 0.
    /// An obstacle with opacity 0 is kept but treated as disabled.
    pub fn is_active(&self) -> bool {
        self.common.enabled && self.common.opacity > 0.0
    }

    /// `true` if the obstacle has a finite top: tile grids with levels, other obstacles with
    /// `height > 0`.
    pub fn has_finite_height(&self) -> bool {
        (self.is_tile_grid() && self.grid.level_height > 0.0) || self.common.height > 0.0
    }

    /// Opacity below 1, a solid cell with an opacity below 255 or a live pattern.
    pub fn is_partial(&self) -> bool {
        self.common.opacity < 1.0 || self.grid.partial_cells > 0 || self.patterns.refs > 0
    }

    /// Finite top of the obstacle, `0` if infinitely tall.
    pub fn finite_top(&self) -> f32 {
        if self.is_tile_grid() && self.grid.level_height > 0.0 {
            return f32::from(self.grid.max_level) * self.grid.level_height;
        }
        det::max(self.common.height, 0.0)
    }

    /// Takes over a validated shape whose triangles and outline the store put into its arenas.
    /// Keeps `revision`.
    pub(super) fn assign_shape(&mut self, desc: &ShapeDesc<'_>, triangles: Span, outline: Span) {
        if self.is_tile_grid() {
            // A grid becomes a shape: the assignment goes (the store released its spans).
            self.patterns = PatternAssignment::default();
        }
        self.common = desc.common;
        self.triangles = triangles;
        self.outline = outline;
        self.grid.reset();
        match desc.shape {
            Shape::Circle { radius } => {
                self.kind = ObstacleKind::Circle;
                self.radius = radius;
                self.box_shape = false;
            }
            Shape::Polygon(_) | Shape::Box { .. } => {
                self.kind = ObstacleKind::Polygon;
                self.radius = 0.0;
                self.box_shape = matches!(desc.shape, Shape::Box { .. });
            }
        }
    }

    /// Takes over a validated tile grid whose channels the store put into its byte arena. Keeps
    /// `revision`. The pattern assignment stays with a grid as far as it fits: the cell indices
    /// only for the same size (the store released their span before).
    pub(super) fn assign_tile_grid(&mut self, desc: &TileGridDesc<'_>, channels: [Span; 4]) {
        if !self.is_tile_grid() {
            self.patterns = PatternAssignment::default();
        } else if self.grid.columns != desc.columns || self.grid.rows != desc.rows {
            self.patterns.cells = Span::EMPTY;
        }
        self.kind = ObstacleKind::TileGrid;
        self.common = desc.common;
        self.triangles = Span::EMPTY;
        self.outline = Span::EMPTY;
        self.radius = 0.0;
        self.box_shape = false;
        self.grid.assign(desc, channels);
    }

    /// Becomes `next` (a freshly built shape) but keeps its own revision and, shape to shape,
    /// the pattern assignment. (The store released the data of a grid that stops being one.)
    pub(super) fn replace_with(&mut self, mut next: StoredObstacle) {
        next.revision = self.revision;
        if !self.is_tile_grid() && !next.is_tile_grid() {
            next.patterns = self.patterns;
        }
        *self = next;
    }
}

/// Validates a shape and builds its local triangles into `triangles` (replaced on success, left
/// untouched on error). Returns the four local corners of a box (unused for other shapes).
pub(super) fn prepare_shape(
    tri: &mut Triangulator,
    triangles: &mut Vec<Vec2>,
    desc: &ShapeDesc<'_>,
) -> Result<[Vec2; 4], ObstacleError> {
    desc.validate()?;
    let mut corners = [Vec2::ZERO; 4];
    match desc.shape {
        Shape::Circle { radius } => circle_triangles(radius, triangles),
        Shape::Polygon(v) => tri.triangulate(v, triangles)?,
        Shape::Box { width, depth } => {
            let (hx, hy) = (width * 0.5, depth * 0.5);
            corners = [
                Vec2::new(-hx, -hy),
                Vec2::new(hx, -hy),
                Vec2::new(hx, hy),
                Vec2::new(-hx, hy),
            ];
            tri.triangulate(&corners[..], triangles)?;
        }
    }
    Ok(corners)
}

/// The corners a validated shape stores as its outline.
pub(super) fn outline_of<'a>(desc: &ShapeDesc<'a>, box_corners: &'a [Vec2; 4]) -> &'a [Vec2] {
    match desc.shape {
        Shape::Polygon(v) => v,
        Shape::Box { .. } => box_corners,
        Shape::Circle { .. } => &[],
    }
}

// ---- snapshots (ADR 0010) ----

impl SnapshotValue for ObstacleCommon {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.position);
        for v in [self.rotation, self.height, self.base_height, self.opacity] {
            w.put(&v);
        }
        w.put(&self.dynamic);
        w.put(&self.enabled);
        w.put(&self.shadow_footprint);
    }

    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Self {
            position: r.get()?,
            rotation: r.get()?,
            height: r.get()?,
            base_height: r.get()?,
            opacity: r.get()?,
            dynamic: r.get()?,
            enabled: r.get()?,
            shadow_footprint: r.get()?,
        })
    }
}

/// The description and the spans; the store checks the spans against its arenas on restore.
impl SnapshotValue for StoredObstacle {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&match self.kind {
            ObstacleKind::Polygon => 0u8,
            ObstacleKind::Circle => 1,
            ObstacleKind::TileGrid => 2,
        });
        w.put(&self.common);
        w.put(&self.revision);
        match self.kind {
            ObstacleKind::Polygon => {
                w.put(&self.box_shape);
                w.put(&self.outline);
                w.put(&self.triangles);
            }
            ObstacleKind::Circle => {
                w.put(&self.radius);
                w.put(&self.triangles);
            }
            ObstacleKind::TileGrid => {
                let g = &self.grid;
                w.put(&g.columns);
                w.put(&g.rows);
                w.put(&g.cell_size);
                w.put(&g.level_height);
                w.put(&g.edge_width);
                w.put(&g.solid);
                w.put(&g.base_levels);
                w.put(&g.opacity);
                w.put(&g.edges);
            }
        }
        let p = &self.patterns;
        w.put(&p.pattern);
        w.put(&p.table);
        w.put(&p.cells);
        w.put(&p.refs);
    }

    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let tag: u8 = r.get()?;
        let common: ObstacleCommon = r.get()?;
        let revision: u32 = r.get()?;
        let mut s = Self::empty();
        s.common = common;
        s.revision = revision;
        match tag {
            0 => {
                s.kind = ObstacleKind::Polygon;
                s.box_shape = r.get()?;
                s.outline = r.get()?;
                s.triangles = r.get()?;
            }
            1 => {
                s.kind = ObstacleKind::Circle;
                s.radius = r.get()?;
                s.triangles = r.get()?;
            }
            2 => {
                // The store checks the spans, the values and the counts against its arenas.
                s.kind = ObstacleKind::TileGrid;
                let g = &mut s.grid;
                (g.columns, g.rows) = (r.get()?, r.get()?);
                g.cell_size = r.get()?;
                (g.level_height, g.edge_width) = (r.get()?, r.get()?);
                g.solid = r.get()?;
                g.base_levels = r.get()?;
                g.opacity = r.get()?;
                g.edges = r.get()?;
            }
            _ => return Err(SnapshotError::Invalid("obstacle kind")),
        }
        s.patterns = PatternAssignment {
            pattern: r.get()?,
            table: r.get()?,
            cells: r.get()?,
            refs: r.get()?,
        };
        Ok(s)
    }
}
