//! Occlusion mask, distance field and their cache (P23): the obstacles of P22 drawn into a mask
//! that is kept across frames, and the jump flooding distance field the shadow-casting lights
//! sphere trace through (P25).
//!
//! | Module | Content | Chiaro source |
//! |---|---|---|
//! | `params` | [`FIELD_BOUND`], [`jfa_steps`], [`UPDATE_LIMITS`], [`JfaParams`], [`RectParams`], [`ShiftCopy`] | `SdfShadows.*`, `OcclusionMask.cpp` |
//! | `gpu` (feature `gpu`) | [`OcclusionMask`], [`MASK_PROGRAMS`] | `OcclusionMask.*`, `SdfShadows.*` |
//!
//! # Field
//!
//! Jump flooding turns the mask into an R16F field of texel distances to the nearest obstacle
//! texel (opacity above `0.5 / 255`), clamped to [`FIELD_BOUND`]: a seed pass into RG32F, the
//! steps `16, 8, 4, 2, 1` plus one extra `1`, and a distance pass. A local update runs the same
//! passes over rectangles ([`RectParams`], `rect.vert`): seed and step passes over the seed
//! rectangles of the plan, the distance pass over its field rectangles. A field texel depends on
//! seeds at most [`jfa_span`] texels away, and the seed rectangles reach that far plus two, so
//! the seed targets (never shifted) may hold anything outside them.
//!
//! # Cache
//!
//! [`MaskUpdatePlan`](crate::shadow::MaskUpdatePlan) (P16) decides each frame between reuse, a
//! local update (shifted on a camera pan by whole texels, changed rectangles cleared and redrawn)
//! and a full rebuild; see [`OcclusionMask`].
//!
//! # Redesign against the C++ version
//!
//! - Begin, Plan, PreparePipelines and PrepareField are one [`OcclusionMask::prepare`];
//!   `RecordMask` and the field's `Record` are one [`OcclusionMask::record`] on a lent command
//!   buffer (ADR 0002). Errors are [`Diagnostic`](knorke_types::Diagnostic)s.
//! - The stage computes the [`DomainParams`](crate::occluder_raster::DomainParams) of the frame
//!   itself and hands the same values to the obstacle stages of P22.
//! - The dirty [`BitSet`](knorke_types::BitSet)s of the obstacle store are **not** consumed
//!   here either. Decision: the stage, like the plan, reads `&ObstacleStore` only. Everything it
//!   needs is exact without them: the plan compares its own footprints per dynamic obstacle, the
//!   static geometry follows `static_version`. Consuming them would need `&mut ObstacleStore` in
//!   the render path and make the mask the single reader of a shared store (the light stage,
//!   the caster classes of E53 and tools read it too), and a missed `clear_dirty` (a frame that
//!   fails) would silently lose changes. They stay a host-side convenience until a consumer that
//!   needs exactly "changed since I last looked" appears (plan 17).
//! - The application sprite mask (`occlusion_mask.frag`, E21) is drawn by
//!   [`OcclusionMask::record_with_sprite`] after the obstacles, into the scene rectangle of the
//!   mask (since P31; the plan redraws that rectangle every frame).
//! - The caster class masks of filtered shadows (E53, [`OcclusionMask::prepare_casters`]) are
//!   drawn fully every frame, as in Chiaro (E76: an incremental caster field only if measured).
//! - The stage has no snapshot (it is a cache of GPU contents); after a restore call
//!   [`OcclusionMask::invalidate`].

mod params;

#[cfg(feature = "gpu")]
mod gpu;

pub use params::{
    FIELD_BOUND, JfaParams, RectParams, ShiftCopy, UPDATE_LIMITS, jfa_span, jfa_steps,
};

#[cfg(feature = "gpu")]
pub use gpu::{
    FIELD_FORMAT, MASK_PROGRAMS, MaskPrograms, MaskRecord, NOT_PREPARED, OcclusionMask, SEED_FORMAT,
};
