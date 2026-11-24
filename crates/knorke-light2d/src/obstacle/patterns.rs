// Origin: chiaro @ 9bc5b9b, src/shadows/ObstaclePatterns.hpp and ObstaclePatterns.cpp, plus the
// pattern fields of src/shadows/StoredObstacle.hpp (the pattern store itself is P15).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Patterns on obstacles (P14): which pattern an obstacle or a tile cell carries, and the value
//! it writes to the mask.
//!
//! A *pattern* is a repeating opacity texture (P15). This module only holds the *assignment*: a
//! raw pattern id (`0` is none) per obstacle, for tile grids a table of up to
//! [`MAX_PATTERN_TABLE`] ids plus one table index per cell. What a pattern *is* (its mean
//! opacity, whether it is still alive) comes through the [`PatternSource`] trait, which the
//! pattern store of P15 will implement; until then a closure does.
//!
//! As in Chiaro, a destroyed pattern stays assigned and acts like none.

use super::cells::CellRect;
use super::desc::{Shape, ShapeDesc};
use super::error::ObstacleError;
use super::stored::{ObstacleKind, Span, StoredObstacle};

/// Most entries of a tile grid's pattern table (the cell byte 0 means "the grid's").
pub const MAX_PATTERN_TABLE: usize = 255;
/// Most corners of a polygon with a pattern (the entry geometry intersects it per ray).
pub const MAX_PATTERN_VERTICES: usize = 64;

/// What the obstacles need to know about patterns.
pub trait PatternSource {
    /// The mean opacity of a live pattern, `None` if there is no such pattern (never created or
    /// destroyed). The id `0` is never asked for.
    fn mean_opacity(&self, id: u32) -> Option<f32>;
}

impl<F: Fn(u32) -> Option<f32>> PatternSource for F {
    fn mean_opacity(&self, id: u32) -> Option<f32> {
        self(id)
    }
}

/// The patterns assigned to one obstacle. The table and the cell indices live in the word and
/// byte arenas of the store; the methods take them as slices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PatternAssignment {
    /// The obstacle's pattern; for a tile grid the one of the cells whose index is 0. `0` is none.
    pub pattern: u32,
    /// Tile grids: up to [`MAX_PATTERN_TABLE`] patterns in the word arena; cell index `k` is
    /// `table[k - 1]`.
    pub table: Span,
    /// Tile grids: `columns * rows` table indices in the byte arena, or empty (all `0`).
    pub cells: Span,
    /// Distinct live patterns in `pattern` and the table, kept by the store.
    pub refs: u32,
}

impl PatternAssignment {
    /// `true` if the pattern or one of the table entries satisfies `pred` (id 0 never does).
    pub fn names(&self, table: &[u32], mut pred: impl FnMut(u32) -> bool) -> bool {
        self.pattern != 0 && pred(self.pattern) || table.iter().any(|&id| id != 0 && pred(id))
    }

    /// The distinct ids among the pattern and the table (0 not counted) that satisfy `pred`.
    /// Allocates nothing: an id counts at its first occurrence.
    fn count_distinct(&self, table: &[u32], mut pred: impl FnMut(u32) -> bool) -> u32 {
        let at = |i: usize| if i == 0 { self.pattern } else { table[i - 1] };
        let mut count = 0;
        for i in 0..=table.len() {
            let id = at(i);
            if id != 0 && (0..i).all(|j| at(j) != id) && pred(id) {
                count += 1;
            }
        }
        count
    }

    /// The distinct live patterns among the obstacle's own and its table's.
    pub fn count_live(&self, table: &[u32], patterns: &impl PatternSource) -> u32 {
        self.count_distinct(table, |id| patterns.mean_opacity(id).is_some())
    }

    /// The distinct patterns among the obstacle's own and its table's that `ids` names (the ones
    /// a destroy takes away).
    pub fn count_in(&self, table: &[u32], ids: &[u32]) -> u32 {
        self.count_distinct(table, |id| ids.contains(&id))
    }
}

/// The smallest rectangle around the cells of a grid `columns` wide whose table entry is
/// destroyed (`ids`); `None` if there is none. Cells without an own index are not looked at:
/// they follow the grid's pattern.
pub(super) fn cells_of_destroyed(
    cells: &[u8],
    table: &[u32],
    columns: u32,
    ids: &[u32],
) -> Option<CellRect> {
    let destroyed = |id: u32| id != 0 && ids.contains(&id);
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for (i, &index) in cells.iter().enumerate() {
        let index = index as usize;
        if index != 0 && index <= table.len() && destroyed(table[index - 1]) {
            let (x, y) = (i as u32 % columns, i as u32 / columns);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x1 > x0).then_some(CellRect { x0, y0, x1, y1 })
}

/// Checks one pattern assignment (`set_pattern`): the pattern must be live and a polygon with a
/// pattern has at most [`MAX_PATTERN_VERTICES`] corners.
pub(super) fn check_assignment(
    stored: &StoredObstacle,
    pattern: u32,
    patterns: &impl PatternSource,
) -> Result<(), ObstacleError> {
    if pattern != 0 && patterns.mean_opacity(pattern).is_none() {
        return Err(ObstacleError::PatternNotLive { pattern });
    }
    let corners = if stored.kind == ObstacleKind::Polygon {
        stored.outline.len as usize
    } else {
        0
    };
    if pattern != 0 && corners > MAX_PATTERN_VERTICES {
        return Err(ObstacleError::PatternVertices { corners });
    }
    Ok(())
}

/// A polygon that keeps a pattern through `set_shape` must still have at most
/// [`MAX_PATTERN_VERTICES`] corners. (A tile grid becomes a shape and drops its assignment, so
/// only a shape keeps a pattern.)
pub(super) fn check_fits_shape(
    current: &StoredObstacle,
    desc: &ShapeDesc<'_>,
) -> Result<(), ObstacleError> {
    if current.is_tile_grid() || current.patterns.pattern == 0 {
        return Ok(());
    }
    if let Shape::Polygon(v) = desc.shape
        && v.len() > MAX_PATTERN_VERTICES
    {
        return Err(ObstacleError::PatternVertices { corners: v.len() });
    }
    Ok(())
}

/// Checks the table of `set_tile_pattern_table`: the count and that every entry is 0 or a live
/// pattern.
pub(super) fn check_table(
    table: &[u32],
    patterns: &impl PatternSource,
) -> Result<(), ObstacleError> {
    if table.len() > MAX_PATTERN_TABLE {
        return Err(ObstacleError::PatternTable { count: table.len() });
    }
    for &id in table {
        if id != 0 && patterns.mean_opacity(id).is_none() {
            return Err(ObstacleError::PatternNotLive { pattern: id });
        }
    }
    Ok(())
}

/// Rounds a non-negative value half up, exactly (no `x + 0.5` rounding error).
fn round_half_up(x: f32) -> u32 {
    let t = x as u32;
    t + u32::from(x - t as f32 >= 0.5)
}

/// The value the obstacle (for a tile grid: the cell `cell_index` = row * columns + column)
/// writes to the mask: the scalar opacity (the opacity times the cell's), or with a live pattern
/// in effect `clamp(mean * that, 1/255, 254/255)` (a value of 254.5/255 or more ends the ray
/// before it reads the pattern, one that rounds to 0 drops the obstacle from the mask). A
/// destroyed pattern gives the scalar.
pub(super) fn mask_opacity(
    o: &StoredObstacle,
    opacity: &[u8],
    table: &[u32],
    pattern_cells: &[u8],
    cell_index: usize,
    patterns: &impl PatternSource,
) -> f32 {
    let mut scalar = o.common.opacity;
    let mut factor = o.common.opacity; // what the mean is multiplied with
    let mut id = o.patterns.pattern;
    if o.is_tile_grid() {
        let cell_opacity = opacity.get(cell_index).copied().unwrap_or(255);
        // The same rounding as the cell texture's opacity channel (TileOccluders), so the scalar stays as it was.
        scalar = round_half_up(f32::from(cell_opacity) * o.common.opacity) as f32 / 255.0;
        factor = o.common.opacity * (f32::from(cell_opacity) / 255.0);
        let index = pattern_cells.get(cell_index).copied().unwrap_or(0) as usize;
        if index != 0 {
            id = table.get(index - 1).copied().unwrap_or(0);
        }
    }
    let mean = if id != 0 {
        patterns.mean_opacity(id)
    } else {
        None
    };
    match mean {
        // The clamp below only applies to a factor above 0: opacity 0 stays 0.
        Some(mean) if factor > 0.0 => (mean * factor).clamp(1.0 / 255.0, 254.0 / 255.0),
        _ => scalar,
    }
}
