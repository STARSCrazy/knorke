//! Pixel-art 3D software renderer of the Knorke Engine: grid rooms (Doom-like walls, floors,
//! doors), sprites, triangle meshes, particles and backdrops, drawn by one triangle rasterizer
//! into a small frame with a deliberately pixelated look (palette, dithering, outlines, texture
//! levels that keep dark lines).
//!
//! The crate is standalone (ADR 0001): no light pass, no threads, no clocks. Frame buffers are
//! the renderer's own, reused state; the host reads them after `end`. With a light pass,
//! [`Renderer::light_samples`] hands out G-buffer columns and shadow casters, and
//! [`Renderer::end_lit`] takes one light value per sample back. `knorke-integration` joins it
//! with `knorke-lighting` that way (ADR 0013).
//!
//! Origin: pixelraum @ 81927d8 (`D:\Entwicklung\pixelraum`, same author), relicensed from MIT to
//! MIT OR Apache-2.0. The C interface (`capi.rs`, `include/pixelraum.h`) stays in pixelraum for
//! now (plan 17).

// `!(x > 0.0)` rejects NaN on purpose (ported pixelraum checks); the raster/shadow kernels pass
// their state as plain arguments in the hot loop.
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::too_many_arguments)]

mod anim;
pub mod backdrop;
pub mod blend;
pub mod camera;
pub mod grid;
mod math;
mod palette;
pub mod particles;
pub mod pick;
pub mod png;
mod raster;
pub mod renderer;
mod srgb;
pub mod texture;

pub use backdrop::{
    BACKDROP_BAND, BACKDROP_CLAMP_BOTTOM, BACKDROP_CLAMP_TOP, BACKDROP_FOG, BACKDROP_SPOT,
    BACKDROP_TINT, Backdrop,
};
pub use blend::{BLEND_ADD, BLEND_ALPHA, BLEND_NONE};
pub use camera::Camera;
pub use grid::{Cell, GridMap, Wall};
pub use knorke_math::Vec3;
pub use pick::Pick;
pub use renderer::{
    FrameLight, LightSamples, Options, ParticleLight, Renderer, SOLID_CASTER, SPRITE_BILLBOARD,
    SPRITE_FLIP_X, SPRITE_NO_SHADOW, ShadowBillboard, ShadowCaster, Sprite, Stats, Vertex,
    anim_frame, quantize_time, sample,
};
pub use texture::Texture;

/// Version of the pixelraum code base this crate was migrated from, as 0xMMmmpp.
pub const PIXELRAUM_VERSION: u32 = 0x000900;
