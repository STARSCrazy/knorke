//! Layout-stable math and color types of the Knorke Engine.
//!
//! Why this crate exists (ADR 0006): if navigation, lighting, renderer and game each use
//! their own vector type, every boundary converts every element every frame. Here all
//! types are `#[repr(C)]`, contain only `f32`/`i32`/`u8`, have no padding and implement
//! [`bytemuck::Pod`]. A `&[Vec2]` can therefore be viewed as `&[f32]`, as bytes for a GPU
//! upload, or as the identically laid-out vector type of another library — without a copy.
//!
//! Coordinate spaces are separate types ([`At`], [`Space`]): a world position and a screen
//! position cannot be mixed up by accident, but cost nothing at runtime and cast to their
//! raw vectors in bulk.
//!
//! This crate is intentionally small: what a renderer, lighting pass and gameplay code all
//! need to agree on. It is not a general linear-algebra library; games may keep using glam
//! or nalgebra and cast at the boundary (see the `mint` feature, or `bytemuck::cast_slice`
//! for types with the same layout).

mod color;
pub mod det;
mod geometry;
#[cfg(feature = "mint")]
mod mint_interop;
mod pod;
mod snapshot;
mod space;
mod srgb_table;
mod transform;
pub mod uniform;
mod vector;

pub use bytemuck;
pub use color::{LinearRgba, Srgba8};
pub use geometry::{Aabb2, Capsule2, Circle, Ray2, Segment2};
pub use space::{At, Screen, Space, Tile, Transform2, World2};
pub use transform::{Affine2, Mat4};
pub use vector::{IVec2, IVec3, Vec2, Vec3, Vec4};
