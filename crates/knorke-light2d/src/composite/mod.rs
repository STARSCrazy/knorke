//! The composite pass (P27, Chiaro `CompositeStage`): scene colour times light, plus glow, into
//! a linear RGBA16F HDR buffer at scene resolution.
//!
//! | Module | Content | Chiaro source |
//! |---|---|---|
//! | `params` | [`CompositeProgram`], [`composite_layout_of`], [`CompositeParams`] | `CompositeLayout.hpp`, `CompositeStage.cpp` |
//! | `gpu` (feature `gpu`) | `CompositeStage` | `CompositeStage.*` |
//!
//! # Redesign against the C++ version
//!
//! - `Prepare` and `Record` keep their split: `CompositeStage::prepare` may fail and creates the
//!   HDR target and the pipeline of the frame's program, `CompositeStage::record` records one
//!   render pass into the host's command buffer (ADR 0002: never submits, presents or reads
//!   back). Everything the stage reads comes in a `CompositeFrame` of borrowed texture views;
//!   what the light pass produced is a `CompositeLight` (`CompositeLight::from_stage` takes it
//!   from a `LightingStage`).
//! - The colour space of the input is a field of the frame ([`InputColorSpace`]) because the
//!   setting lives in the post settings (`knorke-post`), which this crate does not know. The
//!   host passes it to both.
//! - The medium programs are in the program table, the layout and the pipelines, and the stage
//!   binds the medium inputs a frame hands it (the medium stage, P30).
//! - The uniform block is a `#[repr(C)]` struct with a byte writer; its layout is pinned by
//!   tests against Chiaro's struct and the member offsets in the SPIR-V of the six shaders
//!   (`tests/composite_uniform_layouts.rs`).

mod params;

#[cfg(feature = "gpu")]
mod gpu;

pub use params::{
    COMPOSITE_PROGRAM_COUNT, COMPOSITE_PROGRAMS, CompositeParams, CompositeParamsInput,
    CompositeProgram, InputColorSpace, composite_layout_of, composite_program_of,
};

#[cfg(feature = "gpu")]
pub use gpu::{
    COMPOSITE_INPUT_MISSING, COMPOSITE_NOT_PREPARED, CompositeFrame, CompositeLight,
    CompositeMedium, CompositeResult, CompositeStage, HDR_FORMAT,
};
