//! GPU foundation of the Knorke Engine: a safe wrapper around the SDL3 GPU API
//! (ADR 0014). Used by 2D light, post-processing and a later 3D renderer.
//!
//! Without features the crate holds only GPU-free logic: [`LiveCounter`] (leak checks),
//! the staging size arithmetic in [`staging`] and the shader pack format in [`kpak`]
//! (reader and writer). Feature `sdl` adds the SDL3 wrappers: [`Device`], [`GpuHandle`],
//! [`RenderTarget`], [`Samplers`], [`UploadBuffer`], [`ShaderLibrary`], [`PipelineCache`],
//! [`bind_program`] and the texture format tables in [`formats`]. Feature
//! `sdl-build-static` builds SDL3 from source and links it statically.
//!
//! **Shaders (ADR 0014 §3):** shaders are not compiled into any library. Each crate with
//! shaders ships one pack per backend format (`knorke-light2d.spirv.kpak`, `….msl.kpak`,
//! `….dxil.kpak`), built by `cargo xtask shaders`. The host picks the pack for its device
//! ([`preferred_format`]) and hands path or bytes to [`ShaderLibrary`]. A crate's optional
//! `embed-shaders` feature passes `include_bytes!` of a pack to [`ShaderLibrary::new`]
//! (zero copy, `&'static`).
//!
//! **Where packs live:** `cargo xtask shaders` writes to `target/shaders/` (not committed;
//! the full Chiaro set is ~17 MB over three formats). The crate's GPU tests use tiny
//! committed packs in `crates/knorke-gpu/tests/data/test.{spirv,msl,dxil}.kpak` (four
//! programs, ~19 KB together), rebuilt with
//! `cargo xtask shaders --src <chiaro>/shaders/compiled --name test --out crates/knorke-gpu/tests/data --only fullscreen.vert,clear.frag,solid.frag,jfa_seed.frag`.
//! Tests against the full pack skip when `target/shaders/` has none.
//!
//! **Ownership (ADR 0002):** the host owns device, swapchain, command buffers and the frame
//! clock. Objects here borrow a [`Device`] for a lifetime `'d`, record into command buffers
//! and copy passes the host lends them, and never submit, present or read back.
//!
//! **`unsafe` (ADR 0005):** this crate is the only place with `unsafe` for the GPU. The
//! crate denies `unsafe_code`; the SDL modules allow it locally and justify each block with
//! a `// SAFETY:` comment.

pub mod kpak;
mod live;
pub mod staging;

pub use live::{LiveCounter, global_live_gpu_objects};

// FFI boundary to SDL3: the only modules allowed to use `unsafe` (ADR 0005).
#[cfg(feature = "sdl")]
#[allow(unsafe_code)]
mod binding;
#[cfg(feature = "sdl")]
pub mod formats;
#[cfg(feature = "sdl")]
#[allow(unsafe_code)]
mod handle;
#[cfg(feature = "sdl")]
#[allow(unsafe_code)]
mod pipeline_cache;
#[cfg(feature = "sdl")]
#[allow(unsafe_code)]
mod render_target;
#[cfg(feature = "sdl")]
#[allow(unsafe_code)]
mod samplers;
#[cfg(feature = "sdl")]
#[allow(unsafe_code)]
mod shader_library;
#[cfg(feature = "sdl")]
#[allow(unsafe_code)]
mod upload;

#[cfg(feature = "sdl")]
pub use handle::{DEVICE_CREATE_FAILED, Device, Gpu, GpuHandle, GpuObject};
#[cfg(feature = "sdl")]
pub use render_target::{RENDER_TARGET_CREATE_FAILED, RENDER_TARGET_ZERO_SIZE, RenderTarget};
#[cfg(feature = "sdl")]
pub use samplers::{Filter, SAMPLER_CREATE_FAILED, Samplers};
#[cfg(feature = "sdl")]
pub use upload::{
    Mapped, UPLOAD_CREATE_FAILED, UPLOAD_LIMIT, UPLOAD_MAP_FAILED, UPLOAD_NOT_RESERVED,
    UploadBuffer,
};

#[cfg(feature = "sdl")]
pub use binding::{BINDING_MISMATCH, Bindings, StageBindings, bind_program, check_bindings};
#[cfg(feature = "sdl")]
pub use pipeline_cache::{
    Blend, PIPELINE_CREATE_FAILED, PIPELINE_STAGE_MISMATCH, PipelineCache, ProgramDesc,
    ProgramIndex, SecondTarget, blend_state,
};
#[cfg(feature = "sdl")]
pub use shader_library::{
    SHADER_CREATE_FAILED, ShaderId, ShaderLibrary, device_formats, preferred_format,
    sdl_shader_format,
};

/// The raw SDL3 bindings this crate is built on (`sdl3-sys`), for the host's own calls.
#[cfg(feature = "sdl")]
pub use sdl3_sys as sys;
