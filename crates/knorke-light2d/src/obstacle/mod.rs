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
//! the cell change log (P14), patterns on obstacles (P14, P15). See the store docs for the
//! dirty-tracking design.

mod desc;
mod error;
mod store;
mod stored;

pub use desc::{
    MAX_CELLS, MAX_VERTICES, ObstacleCommon, ObstaclePose, Shape, ShapeDesc, TileGridDesc,
};
pub use error::ObstacleError;
pub use store::{ObstacleId, ObstacleStore};
pub use stored::{ObstacleKind, Span, StoredObstacle, TileCells};
