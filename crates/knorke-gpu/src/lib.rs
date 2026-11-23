//! GPU foundation of the Knorke Engine: a safe wrapper around the SDL3 GPU API
//! (ADR 0014). Used by 2D light, post-processing and a later 3D renderer.
//!
//! Without features the crate holds only GPU-free logic: [`LiveCounter`] (leak checks)
//! and the staging size arithmetic in [`staging`]. Feature `sdl` adds the SDL3 wrappers:
//! [`Device`], [`GpuHandle`], [`RenderTarget`], [`Samplers`], [`UploadBuffer`]. Feature
//! `sdl-build-static` builds SDL3 from source and links it statically.
//!
//! **Ownership (ADR 0002):** the host owns device, swapchain, command buffers and the frame
//! clock. Objects here borrow a [`Device`] for a lifetime `'d`, record into command buffers
//! and copy passes the host lends them, and never submit, present or read back.
//!
//! **`unsafe` (ADR 0005):** this crate is the only place with `unsafe` for the GPU. The
//! crate denies `unsafe_code`; the SDL modules allow it locally and justify each block with
//! a `// SAFETY:` comment.

mod live;
pub mod staging;

pub use live::{LiveCounter, global_live_gpu_objects};

// FFI boundary to SDL3: the only modules allowed to use `unsafe` (ADR 0005).
#[cfg(feature = "sdl")]
#[allow(unsafe_code)]
mod handle;
#[cfg(feature = "sdl")]
#[allow(unsafe_code)]
mod render_target;
#[cfg(feature = "sdl")]
#[allow(unsafe_code)]
mod samplers;
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

/// The raw SDL3 bindings this crate is built on (`sdl3-sys`), for the host's own calls.
#[cfg(feature = "sdl")]
pub use sdl3_sys as sys;
