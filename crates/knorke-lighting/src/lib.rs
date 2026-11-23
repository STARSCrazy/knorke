//! Lighting of the Knorke Engine: sun, ambient and point lights with shadow maps, computed per
//! surface sample on the CPU (software path).
//!
//! The crate does not know the renderer (ADR 0001, `cargo xtask deps`). Its inputs are
//! G-buffer-like columns ([`Surfaces`]: kind, world position, normal, billboard axis, pixel per
//! sample), shadow casters as plain triangles ([`Caster`], [`BillboardCaster`]) and a texture
//! query for their alpha test ([`CasterTextures`]). Its output is one linear light factor per
//! sample ([`LightScene::light`]) that the host multiplies with its surface colours.
//! `knorke-integration` joins it with `knorke-render` (ADR 0013); any other host can fill the
//! columns itself.
//!
//! Parallel work runs through the host's [`knorke_types::Parallel`] executor in chunks of fixed
//! length; the result is bit-identical for every executor and thread count. No threads, no
//! clocks: time comes in as seconds from the host ([`LightScene::set_time`]).
//!
//! The GPU path (Rust rewrite of Chiaro, plan section 5) is planned to join this crate later and
//! take the same kind of inputs as textures instead of slices (ADR 0002).
//!
//! Origin: pixelraum @ 81927d8 (`src/light.rs`, `src/shadow.rs`, light parts of
//! `src/renderer/`), same author, relicensed from MIT to MIT OR Apache-2.0.

// `!(x > 0.0)` rejects NaN on purpose (ported pixelraum checks); the raster/shadow kernels pass
// their state as plain arguments in the hot loop.
#![allow(clippy::neg_cmp_op_on_partial_ord, clippy::too_many_arguments)]

mod math;
mod model;
mod scene;
mod shadow;
mod textures;

pub use model::{
    BillboardCaster, Caster, MAX_LIGHTS, PointLight, SOLID_CASTER, Sun, color_scaled, falloff,
    flickered_strength, noise,
};
pub use scene::{LightOptions, LightScene, LightStats, Surfaces, sample};
pub use textures::{AlphaTexture, CasterTextures};
