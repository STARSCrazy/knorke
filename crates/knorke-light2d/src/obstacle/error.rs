// Origin: chiaro @ 9bc5b9b, src/shadows/ObstacleDesc.cpp, ObstacleStore.cpp (the SDL error strings
// of the validation and of the handle checks).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Why an obstacle or an obstacle operation was refused (P13).
//!
//! Typed errors with stable diagnostic codes `knorke.light2d.obstacle.*` replace the SDL error
//! strings. Polygon errors keep the codes of the triangulation (`knorke.light2d.polygon.*`).

use super::cells::CellRect;
use crate::triangulate::TriangulateError;
use knorke_types::{Diagnostic, DiagnosticCode};

/// Why an obstacle description or an obstacle operation was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum ObstacleError {
    /// A value is NaN or infinite.
    NotFinite {
        /// Field name of the description.
        field: &'static str,
    },
    /// A value is outside its allowed range.
    OutOfRange {
        /// Field name of the description.
        field: &'static str,
        /// The rule, e.g. `must be > 0`.
        rule: &'static str,
        /// The received value.
        value: f32,
    },
    /// A polygon has fewer than 3 or more than 4096 corners.
    VertexCount {
        /// Received corner count.
        count: usize,
    },
    /// Columns or rows of a tile grid are outside `1..=4096`.
    GridSize {
        /// `columns` or `rows`.
        field: &'static str,
        /// The received value.
        value: u32,
    },
    /// A cell channel does not have `columns * rows` bytes.
    ChannelLength {
        /// `solid`, `base_levels`, `opacity` or `edges`.
        field: &'static str,
        /// `columns * rows`.
        expected: usize,
        /// The received length.
        got: usize,
    },
    /// One cell holds a value the grid cannot use.
    CellValue {
        /// Channel name.
        field: &'static str,
        /// Row-major cell index.
        cell: usize,
        /// The byte.
        value: u8,
        /// The rule.
        rule: &'static str,
    },
    /// A combination of settings the obstacle model does not support; the text says why.
    Unsupported(&'static str),
    /// The obstacle is not a tile grid (P14).
    NotTileGrid(u64),
    /// A cell rectangle is empty or reaches outside the grid (P14).
    CellRect {
        /// The rectangle.
        rect: CellRect,
        /// Columns of the grid.
        columns: u32,
        /// Rows of the grid.
        rows: u32,
    },
    /// A pattern id that is not live (never created or destroyed) (P14).
    PatternNotLive {
        /// The pattern id.
        pattern: u32,
    },
    /// A polygon with a pattern has too many corners (P14).
    PatternVertices {
        /// The corner count.
        corners: usize,
    },
    /// A pattern table has too many entries (P14).
    PatternTable {
        /// The entry count.
        count: usize,
    },
    /// A cell's pattern index points past the table (P14).
    PatternIndex {
        /// Index of the cell in the rectangle.
        cell: usize,
        /// The index.
        index: u8,
        /// Entries in the table.
        table: usize,
    },
    /// The polygon cannot be triangulated.
    Polygon(TriangulateError),
    /// An obstacle handle that is stale or was never issued.
    InvalidHandle(u64),
    /// Two parallel inputs of a batch differ in length.
    LengthMismatch {
        /// Which input.
        what: &'static str,
        /// Expected length.
        expected: usize,
        /// Received length.
        got: usize,
    },
    /// One element of a batch was refused; nothing was changed.
    Batch {
        /// Which input, e.g. `descs`.
        field: &'static str,
        /// Index of the element.
        index: usize,
        /// Why.
        source: Box<ObstacleError>,
    },
}

impl ObstacleError {
    /// The stable diagnostic code; a batch error has the code of its cause.
    pub fn code(&self) -> DiagnosticCode {
        match self {
            Self::NotFinite { .. } => DiagnosticCode::new("knorke.light2d.obstacle.not_finite"),
            Self::OutOfRange { .. } | Self::VertexCount { .. } | Self::GridSize { .. } => {
                DiagnosticCode::new("knorke.light2d.obstacle.out_of_range")
            }
            Self::ChannelLength { .. } => {
                DiagnosticCode::new("knorke.light2d.obstacle.channel_length")
            }
            Self::CellValue { .. } => DiagnosticCode::new("knorke.light2d.obstacle.cell_value"),
            Self::Unsupported(_) => DiagnosticCode::new("knorke.light2d.obstacle.unsupported"),
            Self::NotTileGrid(_) => DiagnosticCode::new("knorke.light2d.obstacle.not_tile_grid"),
            Self::CellRect { .. } => DiagnosticCode::new("knorke.light2d.obstacle.cell_rect"),
            Self::PatternNotLive { .. } => {
                DiagnosticCode::new("knorke.light2d.obstacle.pattern_not_live")
            }
            Self::PatternVertices { .. } => {
                DiagnosticCode::new("knorke.light2d.obstacle.pattern_vertices")
            }
            Self::PatternTable { .. } => {
                DiagnosticCode::new("knorke.light2d.obstacle.pattern_table")
            }
            Self::PatternIndex { .. } => {
                DiagnosticCode::new("knorke.light2d.obstacle.pattern_index")
            }
            Self::Polygon(e) => e.to_diagnostic().code,
            Self::InvalidHandle(_) => DiagnosticCode::new("knorke.light2d.obstacle.invalid_handle"),
            Self::LengthMismatch { .. } => {
                DiagnosticCode::new("knorke.light2d.obstacle.length_mismatch")
            }
            Self::Batch { source, .. } => source.code(),
        }
    }

    /// As a diagnostic; a batch error carries the element as its path (`descs[2]`).
    pub fn to_diagnostic(&self) -> Diagnostic {
        let d = Diagnostic::error(self.code(), self.to_string());
        match self {
            Self::Batch { field, index, .. } => d.at(format!("{field}[{index}]")),
            _ => d,
        }
    }
}

impl core::fmt::Display for ObstacleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotFinite { field } => write!(f, "obstacle desc.{field} must be finite"),
            Self::OutOfRange { field, rule, value } => {
                write!(f, "obstacle desc.{field} {rule} (got {value})")
            }
            Self::VertexCount { count } => {
                write!(f, "polygon obstacle needs 3..=4096 vertices (got {count})")
            }
            Self::GridSize { field, value } => {
                write!(f, "tile grid {field} must be in 1..=4096 (got {value})")
            }
            Self::ChannelLength {
                field,
                expected,
                got,
            } => write!(
                f,
                "tile grid {field} needs columns * rows = {expected} bytes (got {got})"
            ),
            Self::CellValue {
                field,
                cell,
                value,
                rule,
            } => write!(f, "tile cell {cell} {field} {rule} (got {value})"),
            Self::Unsupported(what) => write!(f, "{what}"),
            Self::NotTileGrid(bits) => write!(f, "obstacle 0x{bits:016x} is not a tile grid"),
            Self::CellRect {
                rect,
                columns,
                rows,
            } => write!(
                f,
                "tile cell rectangle [{}, {}) x [{}, {}) is empty or outside the {columns}x{rows} grid",
                rect.x0, rect.x1, rect.y0, rect.y1
            ),
            Self::PatternNotLive { pattern } => write!(f, "pattern {pattern} does not exist"),
            Self::PatternVertices { corners } => write!(
                f,
                "a polygon with a pattern has at most {} corners (got {corners}); remove the pattern first",
                super::patterns::MAX_PATTERN_VERTICES
            ),
            Self::PatternTable { count } => write!(
                f,
                "pattern table has {count} entries, the limit is {}",
                super::patterns::MAX_PATTERN_TABLE
            ),
            Self::PatternIndex { cell, index, table } => write!(
                f,
                "tile cell {cell} has pattern index {index}, above the table size of {table}"
            ),
            Self::Polygon(e) => write!(f, "{e}"),
            Self::InvalidHandle(bits) => {
                write!(f, "invalid or destroyed obstacle handle 0x{bits:016x}")
            }
            Self::LengthMismatch {
                what,
                expected,
                got,
            } => write!(f, "{what} has {got} elements, expected {expected}"),
            Self::Batch {
                field,
                index,
                source,
            } => write!(f, "{field}[{index}]: {source}"),
        }
    }
}

impl std::error::Error for ObstacleError {}

impl From<TriangulateError> for ObstacleError {
    fn from(e: TriangulateError) -> Self {
        Self::Polygon(e)
    }
}
