//! 2D lighting of the Knorke Engine, ported from Chiaro (ADR 0014): the GPU-free part.
//!
//! This is the CPU logic that does not need a graphics device and can be tested headless:
//!
//! | Module | Content | Chiaro package |
//! |---|---|---|
//! | [`projection`] | world to scene-pixel projection (ortho, isometric) | P5 |
//! | [`triangulate`] | ear-clipping triangulation of simple polygons | P5 |
//! | [`codec`] | byte layout of the surface, material and medium buffers, normal-map conversion | P6 |
//! | [`material`] | material shader math as CPU model (tangent frame, specular lobes) | P6 |
//! | [`light_math`] | attenuation, spot cone, flicker, light-quad culling | P7 |
//! | [`light`], [`light_store`] | light description, validation, handle-based store, snapshots | P7 |
//! | [`obstacle`] | obstacle descriptions, handle-based store, dirty tracking for the shadow cache, snapshots | P13 |
//! | [`animation`], [`area_light`], [`light_culling`], [`gpu_light`] | animation curves and store, area-light model, per-frame light lists | P8 |
//! | [`mask`], [`mask_store`], [`light_mask`], [`resources`] | masks (gobo, window): shapes, store, placement; stores as light resources | P10 |
//! | [`shadow`] | occlusion domain and anchor, dirty rectangles, incremental mask update plan | P16 |
//! | [`occluder_raster`] | obstacles into the occlusion mask: triangle buffers, tile cell textures | P22 |
//! | [`settings`] | settings and quality presets of the light (replace `CullSettings`) | P19 |
//! | [`frame_warnings`] | debug-check findings of a frame as stable diagnostics | P18 |
//! | [`light_program`] (+ `light_pipelines` with `gpu`) | light program family: table of the 92 programs, rules, binding layouts, lazy pipelines | P24 |
//!
//! GPU stages come behind the `gpu` feature and take `knorke-gpu` as their only graphics
//! dependency; today that is only the gate spike `gpu_spike` (to be replaced by P20–P27).
//! This crate does not know `knorke-post`, `knorke-lighting`, `knorke-render` or
//! the AI crates (`cargo xtask deps`).
//!
//! What changed against the C++ library is stated in the module docs: the C ABI and its flag
//! bits, raw pointers and SDL error strings are gone; coordinate spaces, handles from
//! [`knorke_types::Slots`], `Column` inputs, typed errors with stable `knorke.light2d.*`
//! diagnostic codes and snapshots take their place. Maths that influences results uses
//! [`knorke_math::det`], so results are identical on every platform.
//!
//! Origin: chiaro @ 9bc5b9b (zlib license, `LICENSE-CHIARO-ZLIB.txt`); every ported file names its
//! source path.

// `!(x > 0.0)` rejects NaN on purpose, as in the C++ original.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod ambient_zone;
pub mod ambient_zone_culling;
pub mod ambient_zone_store;
pub mod animation;
pub mod area_light;
pub mod codec;
pub mod frame_warnings;
pub mod gpu_light;
#[cfg(feature = "gpu")]
pub mod gpu_spike;
pub mod light;
pub mod light_culling;
pub mod light_mask;
pub mod light_math;
#[cfg(feature = "gpu")]
pub mod light_pipelines;
pub mod light_program;
pub mod light_store;
pub mod mask;
pub mod mask_store;
pub mod material;
pub mod obstacle;
pub mod occluder_raster;
pub mod profile;
pub mod profile_import;
pub mod profile_store;
pub mod projection;
pub mod resources;
pub mod settings;
pub mod shadow;
pub mod triangulate;

pub use animation::{AnimationDesc, AnimationError, AnimationId, AnimationStore};
pub use light::{LightError, LightResources, NoResources};
pub use light_store::LightStore;
pub use mask::MaskError;
pub use mask_store::{MaskId, MaskStore};
pub use obstacle::{ObstacleError, ObstacleId, ObstacleStore};
pub use projection::{Projection, ProjectionError};
pub use resources::StoreResources;
pub use triangulate::{TriangulateError, Triangulator, circle_triangles};
