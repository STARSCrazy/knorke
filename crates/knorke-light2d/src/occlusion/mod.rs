//! Occlusion mask and its cache (P23): the obstacles of P22 drawn into a mask that is kept
//! across frames, read by the light rows and sun maps (ADR 0019).
//!
//! | Module | Content | Chiaro source |
//! |---|---|---|
//! | `params` | [`RectParams`], [`ShiftCopy`] | `SdfShadows.*`, `OcclusionMask.cpp` |
//! | `gpu` (feature `gpu`) | [`OcclusionMask`], [`MASK_PROGRAMS`] | `OcclusionMask.*` |
//!
//! # Cache
//!
//! [`MaskUpdatePlan`](crate::shadow::MaskUpdatePlan) (P16) decides each frame between reuse, a
//! local update (shifted on a camera pan by whole texels, changed rectangles cleared and redrawn)
//! and a full rebuild; see [`OcclusionMask`].
//!
//! # Redesign against the C++ version
//!
//! - Begin, Plan, PreparePipelines are one [`OcclusionMask::prepare`]; `RecordMask` is
//!   [`OcclusionMask::record`] on a lent command buffer (ADR 0002). Errors are
//!   [`Diagnostic`](knorke_types::Diagnostic)s.
//! - No distance field: Chiaro's jump flooding field (`SdfShadows`), which its light programs
//!   sphere traced, is gone with them (plan 17.13 L7); the rows read the mask itself.
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
//!   drawn fully every frame, as in Chiaro.
//! - The stage has no snapshot (it is a cache of GPU contents); after a restore call
//!   [`OcclusionMask::invalidate`].

mod params;

#[cfg(feature = "gpu")]
mod gpu;

pub use params::{RectParams, ShiftCopy};

#[cfg(feature = "gpu")]
pub use gpu::{MASK_PROGRAMS, MaskPrograms, MaskRecord, NOT_PREPARED, OcclusionMask};
