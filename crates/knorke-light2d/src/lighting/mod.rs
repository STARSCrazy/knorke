//! The light pass (P25, Chiaro `LightingStage`): the lights of a
//! [`LightList`](crate::light_culling::LightList) into an RGBA16F light buffer, with the
//! program of each kind of draw, its binding layout and the uniform blocks of the shaders.
//!
//! | Module | Content | Chiaro source |
//! |---|---|---|
//! | `params` | uniform blocks ([`LightPassParams`], [`SurfaceParams`], …), [`DetailMode`], [`program_for`] | `LightingStage.cpp`, `Frame.hpp` |
//! | `plan` | [`LightDraws`]: split of the light list, buffer order, draw order | `LightingStage.cpp` |
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
//! - Uniform blocks are `#[repr(C)]` structs with byte writers; their layouts are pinned by
//!   tests against Chiaro's structs and against the member offsets in the SPIR-V of the
//!   shaders (`tests/light_uniform_layouts.rs`).
//! - Not ported here: the ambient zone pass (P26), the ring lights of shockwaves, the medium
//!   world map and pattern obstacles (P28, P29). Their programs exist; the stage sets neither
//!   `Medium` nor `Pattern`.
//! - Shadowed lights with a caster filter (E53) need the caster maps of their class in
//!   `ShadowMaps::casters`. The occlusion stage (P23) does not build them yet; such frames fail
//!   with a diagnostic instead of drawing with the wrong shadows.

mod params;
mod plan;

#[cfg(feature = "gpu")]
mod gpu;

pub use params::{
    DetailMode, DirectionalParams, LightPass, LightPassParams, LightSurfacePassParams,
    ProgramModes, ShadowDomain, SurfaceParams, SurfaceShadowParams, ambient_clear, detail_mode_for,
    height_tolerance, program_for,
};
pub use plan::{DrawProgram, LIGHT_DRAW_COUNT, LightDraw, LightDraws};

#[cfg(feature = "gpu")]
pub use gpu::{
    CasterMaps, LIGHT_FORMAT, LIGHTING_ATLAS_MISSING, LIGHTING_CASTERS_MISSING,
    LIGHTING_INPUT_MISSING, LIGHTING_NOT_PREPARED, LightingFrame, LightingRecord, LightingResult,
    LightingStage, PureAmbient, ShadowMaps,
};
