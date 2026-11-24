// Origin: chiaro @ 9bc5b9b, src/shadows/ObstacleDesc.hpp and ObstacleDesc.cpp (ObstacleCommon,
// ShapeParams, TileGridParams, Init*, Validate*, the box conversion) and the field list of
// CHI_ShapeDesc / CHI_TileGridDesc in include/Chiaro/Chiaro.h.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Descriptions of obstacles and their validation (P13).
//!
//! Redesign against the C++ version: the pair of the public flat descs (`CHI_ShapeDesc`, flag
//! bits) and the engine params (`ShapeParams`, converted at the C boundary) becomes one
//! description per kind. Flag bits are `bool`s; the unknown-flag and unknown-shape-type errors
//! cannot happen any more. A box is [`Shape::Box`] and is stored as its 4-vertex polygon. Bulk
//! data (polygon corners, cell channels) is borrowed: the store copies what it keeps and never
//! holds on to the caller's memory.
//!
//! Heights keep the C++ convention: `height == 0` is infinitely tall.

use super::error::ObstacleError;
use knorke_math::{At, Vec2, World2};

/// Most corners of a polygon.
pub const MAX_VERTICES: usize = 4096;
/// Most columns or rows of a tile grid.
pub const MAX_CELLS: u32 = 4096;

/// What every obstacle has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObstacleCommon {
    /// World position of the obstacle's local origin.
    pub position: At<World2, Vec2>,
    /// Rotation in degrees, counter-clockwise.
    pub rotation: f32,
    /// Height of the top above the ground; `0` is infinitely tall.
    pub height: f32,
    /// Height of the bottom (for obstacles that hover); `0` reaches the ground. Needs a finite
    /// `height` above it.
    pub base_height: f32,
    /// Opacity in `[0, 1]`. An obstacle with opacity 0 is kept but acts as disabled.
    pub opacity: f32,
    /// Moves often: changes do not count as changes of static geometry.
    pub dynamic: bool,
    /// Takes part in masks and fields.
    pub enabled: bool,
    /// Casts a shadow of its footprint (shapes only).
    pub shadow_footprint: bool,
}

impl Default for ObstacleCommon {
    /// At the origin, infinitely tall, opaque, static, enabled, no footprint shadow.
    fn default() -> Self {
        Self {
            position: At::new(Vec2::ZERO),
            rotation: 0.0,
            height: 0.0,
            base_height: 0.0,
            opacity: 1.0,
            dynamic: false,
            enabled: true,
            shadow_footprint: false,
        }
    }
}

impl ObstacleCommon {
    /// Checks the values every obstacle shares.
    ///
    /// # Errors
    /// A non-finite position, rotation or height, a negative or inconsistent base height, an
    /// opacity outside `[0, 1]`.
    pub fn validate(&self) -> Result<(), ObstacleError> {
        let p = self.position.raw();
        if !p.x.is_finite() || !p.y.is_finite() {
            return Err(ObstacleError::NotFinite { field: "position" });
        }
        if !self.rotation.is_finite() {
            return Err(ObstacleError::NotFinite { field: "rotation" });
        }
        if !self.height.is_finite() {
            return Err(ObstacleError::NotFinite { field: "height" });
        }
        if !self.base_height.is_finite() {
            return Err(ObstacleError::NotFinite {
                field: "base_height",
            });
        }
        if self.base_height < 0.0 {
            return Err(ObstacleError::OutOfRange {
                field: "base_height",
                rule: "must be >= 0",
                value: self.base_height,
            });
        }
        if self.base_height > 0.0 && (self.height <= 0.0 || self.base_height >= self.height) {
            return Err(ObstacleError::OutOfRange {
                field: "base_height",
                rule: "must be below a finite height",
                value: self.base_height,
            });
        }
        if !(self.opacity >= 0.0 && self.opacity <= 1.0) {
            return Err(ObstacleError::OutOfRange {
                field: "opacity",
                rule: "must be in [0, 1]",
                value: self.opacity,
            });
        }
        Ok(())
    }
}

/// The form of a shape obstacle, in its local space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape<'a> {
    /// A simple polygon (3 to 4096 corners, any winding).
    Polygon(&'a [Vec2]),
    /// A box of `width` x `depth` around the local origin; stored as a 4-vertex polygon.
    Box {
        /// Extent along x.
        width: f32,
        /// Extent along y.
        depth: f32,
    },
    /// A circle around the local origin.
    Circle {
        /// Radius.
        radius: f32,
    },
}

/// A polygon, box or circle obstacle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeDesc<'a> {
    /// What every obstacle has.
    pub common: ObstacleCommon,
    /// The form.
    pub shape: Shape<'a>,
}

impl<'a> ShapeDesc<'a> {
    /// A polygon with default common values.
    pub fn polygon(vertices: &'a [Vec2]) -> Self {
        Self {
            common: ObstacleCommon::default(),
            shape: Shape::Polygon(vertices),
        }
    }

    /// A box with default common values.
    pub fn rect(width: f32, depth: f32) -> Self {
        Self {
            common: ObstacleCommon::default(),
            shape: Shape::Box { width, depth },
        }
    }

    /// A circle with default common values.
    pub fn circle(radius: f32) -> Self {
        Self {
            common: ObstacleCommon::default(),
            shape: Shape::Circle { radius },
        }
    }

    /// Checks everything except whether the polygon is simple (the triangulation does that).
    ///
    /// # Errors
    /// See [`ObstacleError`].
    pub fn validate(&self) -> Result<(), ObstacleError> {
        self.common.validate()?;
        match self.shape {
            Shape::Polygon(v) => {
                if v.len() < 3 || v.len() > MAX_VERTICES {
                    return Err(ObstacleError::VertexCount { count: v.len() });
                }
            }
            Shape::Box { width, depth } => {
                if !width.is_finite() || !depth.is_finite() {
                    return Err(ObstacleError::NotFinite {
                        field: "width/depth",
                    });
                }
                if width <= 0.0 {
                    return Err(ObstacleError::OutOfRange {
                        field: "width",
                        rule: "must be > 0",
                        value: width,
                    });
                }
                if depth <= 0.0 {
                    return Err(ObstacleError::OutOfRange {
                        field: "depth",
                        rule: "must be > 0",
                        value: depth,
                    });
                }
            }
            Shape::Circle { radius } => {
                if !radius.is_finite() || radius <= 0.0 {
                    return Err(ObstacleError::OutOfRange {
                        field: "radius",
                        rule: "must be finite and > 0",
                        value: radius,
                    });
                }
            }
        }
        Ok(())
    }
}

/// A grid of solid cells. Every channel holds `columns * rows` bytes, row-major.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileGridDesc<'a> {
    /// What every obstacle has (`shadow_footprint` is not supported on grids).
    pub common: ObstacleCommon,
    /// Columns, `1..=4096`.
    pub columns: u32,
    /// Rows, `1..=4096`.
    pub rows: u32,
    /// World size of one cell, finite and > 0.
    pub cell_size: Vec2,
    /// Height of one level; `0`: all cells are as tall as `common.height`.
    pub level_height: f32,
    /// Width of the edge strips of cells; needed for `edges`.
    pub edge_width: f32,
    /// Per cell: `0` empty, otherwise the top level.
    pub solid: &'a [u8],
    /// Per cell: the bottom level (below the top of a solid cell); needs `level_height`.
    pub base_levels: Option<&'a [u8]>,
    /// Per cell: opacity, 255 opaque.
    pub opacity: Option<&'a [u8]>,
    /// Per cell: edge strip bits 1, 2, 4, 8.
    pub edges: Option<&'a [u8]>,
}

impl<'a> TileGridDesc<'a> {
    /// A grid with default common values and one-unit cells; `solid` holds `columns * rows` bytes.
    pub fn new(columns: u32, rows: u32, solid: &'a [u8]) -> Self {
        Self {
            common: ObstacleCommon::default(),
            columns,
            rows,
            cell_size: Vec2::new(1.0, 1.0),
            level_height: 0.0,
            edge_width: 0.0,
            solid,
            base_levels: None,
            opacity: None,
            edges: None,
        }
    }

    /// Checks all values and the channel lengths.
    ///
    /// # Errors
    /// See [`ObstacleError`].
    pub fn validate(&self) -> Result<(), ObstacleError> {
        self.common.validate()?;
        if self.common.shadow_footprint {
            return Err(ObstacleError::Unsupported(
                "shadow_footprint is not supported on tile grids (use shapes)",
            ));
        }
        for (field, value) in [("columns", self.columns), ("rows", self.rows)] {
            if !(1..=MAX_CELLS).contains(&value) {
                return Err(ObstacleError::GridSize { field, value });
            }
        }
        let count = self.columns as usize * self.rows as usize;
        let channels = [
            ("solid", Some(self.solid)),
            ("base_levels", self.base_levels),
            ("opacity", self.opacity),
            ("edges", self.edges),
        ];
        for (field, channel) in channels {
            if let Some(c) = channel
                && c.len() != count
            {
                return Err(ObstacleError::ChannelLength {
                    field,
                    expected: count,
                    got: c.len(),
                });
            }
        }
        let cs = self.cell_size;
        if !cs.x.is_finite() || !cs.y.is_finite() || cs.x <= 0.0 || cs.y <= 0.0 {
            return Err(ObstacleError::OutOfRange {
                field: "cell_size",
                rule: "must be finite and > 0",
                value: if cs.x.is_finite() && cs.x > 0.0 {
                    cs.y
                } else {
                    cs.x
                },
            });
        }
        if !self.level_height.is_finite() || self.level_height < 0.0 {
            return Err(ObstacleError::OutOfRange {
                field: "level_height",
                rule: "must be finite and >= 0",
                value: self.level_height,
            });
        }
        if !self.edge_width.is_finite() || self.edge_width < 0.0 {
            return Err(ObstacleError::OutOfRange {
                field: "edge_width",
                rule: "must be finite and >= 0",
                value: self.edge_width,
            });
        }
        if self.edges.is_some() && self.edge_width <= 0.0 {
            return Err(ObstacleError::Unsupported(
                "tile grid edges need edge_width > 0",
            ));
        }
        if self.base_levels.is_some() && self.level_height <= 0.0 {
            return Err(ObstacleError::Unsupported(
                "tile grid base_levels need level_height > 0",
            ));
        }
        if let Some(edges) = self.edges {
            for (cell, &value) in edges.iter().enumerate() {
                if value > 15 {
                    return Err(ObstacleError::CellValue {
                        field: "edges",
                        cell,
                        value,
                        rule: "has bits other than 1, 2, 4 and 8",
                    });
                }
            }
        }
        if let Some(base) = self.base_levels {
            for (cell, (&b, &top)) in base.iter().zip(self.solid).enumerate() {
                if top != 0 && b >= top {
                    return Err(ObstacleError::CellValue {
                        field: "base_levels",
                        cell,
                        value: b,
                        rule: "is at or above the top of its cell",
                    });
                }
            }
        }
        Ok(())
    }
}

/// A new pose of an obstacle, for [`ObstacleStore::set_transforms`](super::ObstacleStore::set_transforms).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObstaclePose {
    /// World position.
    pub position: At<World2, Vec2>,
    /// Rotation in degrees, counter-clockwise.
    pub rotation: f32,
}

impl ObstaclePose {
    /// Checks that all values are finite.
    ///
    /// # Errors
    /// [`ObstacleError::NotFinite`].
    pub fn validate(&self) -> Result<(), ObstacleError> {
        let p = self.position.raw();
        if p.x.is_finite() && p.y.is_finite() && self.rotation.is_finite() {
            Ok(())
        } else {
            Err(ObstacleError::NotFinite {
                field: "position/rotation",
            })
        }
    }
}
