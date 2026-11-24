//! Shadow cache plan (P16): which parts of the occlusion mask and its distance field a frame must
//! redraw. GPU-free; the GPU mask, field and cache that consume the plan are P23.
//!
//! | Module | Content | Chiaro source |
//! |---|---|---|
//! | `domain` | [`OcclusionDomain`], [`MaskAnchor`] | `OcclusionDomain.*` |
//! | `dirty_rects` | [`DirtyRect`], [`DirtyRectList`], obstacle and cell rectangles | `DirtyRects.*` |
//! | `plan` | [`MaskUpdatePlan`], [`MaskUpdate`], [`MaskPlanInput`], [`CellLog`] | `MaskUpdatePlan.*` |
//!
//! # Rules (E21)
//!
//! Changed mask rectangles of a frame, in mask texels:
//! - the strips that a camera pan by `snapped - snapped_prev` exposes;
//! - per enabled dynamic obstacle whose revision, position or rotation changed: its previous and
//!   its current rectangle (one bounding box if that is no larger than both); a new or
//!   re-enabled one adds its current rectangle, a destroyed or disabled one its previous
//!   rectangle once; an unchanged one adds nothing;
//! - per logged tile cell change since the recorded frame ([`CellLog`]), the rectangle of its
//!   cells; a log that lost changes rebuilds fully;
//! - with a sprite mask, its previous scene rectangle and the current one grown by
//!   [`SPRITE_PAN_ALLOWANCE`].
//!
//! Field rectangles are these, plus the static and unchanged dynamic obstacles that a pan pushed
//! out of the mask, grown by `field_grow`; seed rectangles are the field rectangles grown by
//! `seed_grow` and merged again. Full rebuild: no comparable recorded frame, a changed
//! [`MaskLayout`] (it contains [`ObstacleStore::static_version`](crate::ObstacleStore::static_version),
//! so every static change rebuilds), a pan of more than half the mask, more than
//! [`DirtyRectList::MAX_DIRTY_RECTS`] changed rectangles, or field rectangles above
//! `full_rebuild_ratio` of the mask.
//!
//! # Redesign against the C++ version
//!
//! - Footprints are matched by slot index (the store iterates in slot order, so no sort is
//!   needed); a slot reused by another generation counts as removed plus new.
//! - [`ObstacleStore::revision`](crate::ObstacleStore::revision) gives a fast path: same
//!   layout, no pan, no sprite mask and an unchanged revision and cell version decide
//!   [`MaskUpdate::Reused`] without walking the obstacles.
//! - The dirty [`BitSet`](knorke_types::BitSet)s of the store are not consumed here: the plan
//!   compares its own footprints exactly (any number of passive observers can do that), so it
//!   needs no `&mut ObstacleStore` and does not call `clear_dirty`. They remain for the single
//!   owner that P23 introduces.
//! - The tile cell log (`ObstacleStore::ForEachCellChange`) is the trait [`CellLog`], because
//!   cell editing arrives with P14; [`NoCellLog`] stands in until then.
//! - The plan is a cache of what the GPU textures hold, not simulation state: it has no
//!   snapshot. After a restore or new textures call [`MaskUpdatePlan::invalidate`] (or set
//!   [`MaskPlanInput::force_full`]).
//! - Projection math is done in `f64` with explicit comparisons and rotations through
//!   [`knorke_math::det`], so the rectangles are identical on every platform.

mod dirty_rects;
mod domain;
mod plan;

pub use dirty_rects::{
    CellRect, DirtyRect, DirtyRectList, TEXEL_COORD_LIMIT, clamp_texel_coord, obstacle_grid_rect,
    tile_cells_grid_rect,
};
pub use domain::{MaskAnchor, OcclusionDomain};
pub use plan::{
    CellLog, DynamicFootprint, FULL_REBUILD_RATIO, MaskLayout, MaskPlanInput, MaskUpdate,
    MaskUpdateLimits, MaskUpdatePlan, NoCellLog, SPRITE_PAN_ALLOWANCE, sprite_grid_rect,
};
