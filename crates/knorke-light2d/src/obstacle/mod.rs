//! Obstacles that cast shadows: descriptions, stored form, handle-based store and the dirty
//! tracking the shadow cache builds on (P13).
//!
//! | Module | Content | Chiaro source |
//! |---|---|---|
//! | `desc` | [`ShapeDesc`], [`TileGridDesc`], [`ObstacleCommon`], validation | `ObstacleDesc.*` |
//! | `stored` | [`StoredObstacle`], [`TileCells`] (creation only), world triangles | `StoredObstacle.*` |
//! | `store` | [`ObstacleStore`], batches, counters, versions, dirty sets, snapshot | `ObstacleStore.*` |
//! | `error` | [`ObstacleError`] with codes `knorke.light2d.obstacle.*` | SDL error strings |
//!
//! Not part of this package (later ones extend this module): editing cells of a tile grid and
//! the pattern store (P15). See the store docs for the dirty-tracking design.

mod cells;
mod desc;
mod error;
mod patterns;
mod store;
mod store_edit;
mod stored;

pub use desc::{
    MAX_CELLS, MAX_VERTICES, ObstacleCommon, ObstaclePose, Shape, ShapeDesc, TileGridDesc,
};
pub use cells::{COMPARE_BLOCK, CellChange, CellLog, CellRect, MAX_COMPARE_RECTS};
pub use error::ObstacleError;
pub use patterns::{
    MAX_PATTERN_TABLE, MAX_PATTERN_VERTICES, PatternAssignment, PatternSource,
};
pub use store_edit::TileView;
pub use store::{ObstacleId, ObstacleStore};
pub use stored::{ObstacleKind, Span, StoredObstacle, TileCells};
