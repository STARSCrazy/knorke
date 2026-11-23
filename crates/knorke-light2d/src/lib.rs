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
//!
//! GPU stages follow later behind a `gpu` feature and take `knorke-gpu` as their only graphics
//! dependency. This crate does not know `knorke-post`, `knorke-lighting`, `knorke-render` or
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

pub mod codec;
pub mod light;
pub mod light_math;
pub mod light_store;
pub mod material;
pub mod projection;
pub mod triangulate;

pub use light::{LightError, LightResources, NoResources};
pub use light_store::LightStore;
pub use projection::{Projection, ProjectionError};
pub use triangulate::{TriangulateError, Triangulator, circle_triangles};
