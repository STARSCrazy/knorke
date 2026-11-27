//! Obstacle raster (P22): the obstacles of a scene drawn into the occlusion mask.
//!
//! Two kinds of geometry go into the mask, as in Chiaro:
//!
//! - **Triangles** of polygons, boxes and circles ([`TriangleSet`]): world triangles with one
//!   opacity and height interval per triangle in storage buffers; one draw
//!   of 3 vertices per triangle without vertex buffers. The static set is rebuilt and uploaded
//!   only when [`ObstacleStore::static_version`](crate::ObstacleStore::static_version) or the
//!   widening of thin circles changed, the dynamic set in every frame that draws the mask.
//! - **Tile grids** ([`TileSet`]): one RGBA8 cell texture per grid, kept across frames, and one
//!   transformed quad per grid. Edited cells are uploaded as the bounding box of the changed
//!   cells only (the cell log of the store, the same [`CellRect`](crate::shadow::CellRect) the
//!   shadow plan uses).
//!
//! | Module | Content | Chiaro source |
//! |---|---|---|
//! | `params` | [`DomainParams`], [`TileParams`], height intervals, thin circle radius | `OcclusionCommon.hpp`, `OccluderGeometry.cpp` |
//! | `triangles` | [`TriangleSet`], [`NoPatterns`], the triangle buffer layout | `OccluderGeometry.*` |
//! | `tile_cells` | [`TileSet`], [`TileUniforms`], [`write_cell_texels`] | `TileOccluders.*` |
//! | `gpu` (feature `gpu`) | [`OccluderGeometry`], [`TileOccluders`], the programs | `OccluderGeometry.*`, `TileOccluders.*` |
//!
//! The mask texture, its clearing and caching and the update plan's rectangles are P23; this
//! package draws "everything" into whatever render pass the host (or P23)
//! opens on the mask, in the order prepare, record uploads, record draws.
//!
//! # Redesign against the C++ version
//!
//! - The GPU-free half (collecting, laying out, which cells are pending, the texel values) is
//!   separate and tested without a device; the stages only own buffers and textures.
//! - Preparing fails with a [`Diagnostic`](knorke_types::Diagnostic) (no SDL error string);
//!   recording does not fail except for wrong use.
//! - Pipelines come from the host's [`PipelineCache`](knorke_gpu::PipelineCache): the stages
//!   export their program table ([`OCCLUDER_PROGRAMS`]) to be placed at some base index.
//! - Uploads go through the safe copy pass, including the partial cell upload
//!   ([`CopyPass::upload_to_texture_region`](knorke_gpu::CopyPass::upload_to_texture_region)).
//! - Circle widening, the opacity per triangle and the height intervals are plain `Vec`
//!   buffers reused from frame to frame: no allocation once warm.

mod params;
mod tile_cells;
mod triangles;

#[cfg(feature = "gpu")]
mod gpu;

pub use params::{
    DomainParams, INFINITE_TOP, TileParams, interval_channels, obstacle_interval,
    thin_circle_radius,
};
pub use tile_cells::{CELL_BYTES, TileSet, TileSlot, TileUniforms, write_cell_texels};
pub use triangles::{
    NoPatterns, TriangleSet, triangle_buffer_floats, widen_circle, write_triangle_buffer,
};

#[cfg(feature = "gpu")]
pub use gpu::{
    HEIGHT_FORMAT, MASK_FORMAT, NOT_PREPARED, OCCLUDER_PROGRAMS, OccluderGeometry,
    OccluderPrograms, TILE_UPLOAD_LIMIT, TileOccluders,
};
