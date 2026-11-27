//! The light pass (P25, Chiaro `LightingStage`): the lights of a
//! [`LightList`](crate::light_culling::LightList) into an RGBA16F light buffer, drawn by the row
//! and sun programs of [`rows`](crate::rows) (ADR 0019), with the uniform blocks they share.
//!
//! | Module | Content | Chiaro source |
//! |---|---|---|
//! | `params` | uniform blocks ([`SurfaceParams`], [`LightSurfacePassParams`]), [`DetailMode`] | `LightingStage.cpp`, `Frame.hpp` |
//! | `gpu` (feature `gpu`) | `LightingStage` | `LightingStage.*` |
//!
//! # Redesign against the C++ version
//!
//! - `Prepare` and `Record` keep their split: `LightingStage::prepare` may fail and creates
//!   what the frame needs, `LightingStage::record` records into the host's command buffer
//!   (ADR 0002: never submits, presents or reads back). The stage reads everything from a
//!   `LightingFrame`; the occlusion results are a `ShadowMaps` of texture views, the light
//!   atlases are optional texture views (P26 builds them; without one the scene stands in, as
//!   in Chiaro).
//! - Chiaro's light programs, which trace each pixel's shadow through a distance field, are
//!   gone: every light takes its shadow from a row, every sun from a map (plan 17.13 L7).
//! - The ring lights of shockwaves (P21) are their own pass after the lights
//!   ([`ring_light`](crate::ring_light)), from `LightingFrame::ring_lights`.
//! - The medium world map (P29) comes in `LightingFrame::medium_map` and makes every draw a
//!   `_medium` program. Pattern obstacles come in `ShadowMaps::patterns`.
//! - Shadowed lights with a caster filter (E53) need the caster maps of their class in
//!   `ShadowMaps::casters` (built by `OcclusionMask::prepare_casters`); without them the frame
//!   fails with a diagnostic instead of drawing with the wrong shadows.

mod params;

#[cfg(feature = "gpu")]
mod gpu;

pub use params::{
    DetailMode, LightSurfacePassParams, ShadowDomain, SurfaceParams, ambient_clear,
    detail_mode_for, height_tolerance,
};

#[cfg(feature = "gpu")]
pub use gpu::{
    CasterMaps, LIGHT_FORMAT, LIGHTING_ATLAS_MISSING, LIGHTING_CASTERS_MISSING,
    LIGHTING_INPUT_MISSING, LIGHTING_NOT_PREPARED, LightingFrame, LightingRecord, LightingResult,
    LightingStage, MATERIAL_TARGET_FORMAT, PureAmbient, ShadowMaps,
};
#[cfg(feature = "gpu")]
pub(crate) use gpu::{Resources, bind_named};
