// Origin: chiaro @ 9bc5b9b, src/rendering/AmbientZonePass.hpp, src/rendering/AmbientZonePass.cpp
// (ZonePassParams, ZoneFragParams, PrepareVertices), shaders/src/ambient_zone.vert.hlsl,
// shaders/src/ambient_zone.frag.hlsl.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The ambient zone pass (P26, Chiaro `AmbientZonePass`, E121): one render pass before the light
//! pass that clears the light buffer to the ambient sum (global ambient plus the flat
//! directional sum) and folds the visible zones into it with one instanced draw
//! (`SRC_ALPHA / ONE_MINUS_SRC_ALPHA` into RGB). While a light pass follows and the pure
//! ambient is needed (direction mode), the same values go into a second target, the pure
//! ambient.
//!
//! | Module | Content |
//! |---|---|
//! | this one | the uniform blocks, the zone records and the vertex texture as bytes, without a GPU |
//! | `gpu` (feature `gpu`) | `AmbientZonePass`, owned by the [`LightingStage`](crate::lighting::LightingStage) |
//!
//! # Redesign against the C++ version
//!
//! - The pass is part of the lighting stage as in Chiaro, but its inputs are a [`ZoneInput`]
//!   (store and the frame's [`AmbientZoneList`]) given to
//!   [`LightingStage::prepare_with_zones`](crate::lighting::LightingStage::prepare_with_zones)
//!   instead of fields of a frame struct; `prepare` without zones is unchanged.
//! - The medium stage (P29) does not exist yet: the pure ambient copy is made in direction
//!   mode with a light pass only.
//! - Writing the zone records and the vertex texels is a plain function over a byte slice
//!   ([`write_zones`], [`fill_zone_vertices`]).

use crate::ambient_zone::GPU_ZONE_BYTES;
use crate::ambient_zone_culling::AmbientZoneList;
use crate::ambient_zone_store::AmbientZoneStore;
use crate::projection::Projection;
use knorke_math::uniform_block;

#[cfg(feature = "gpu")]
mod gpu;

#[cfg(feature = "gpu")]
pub use gpu::{
    AMBIENT_TARGET_FORMAT, AmbientZonePass, ZONE_PASS_INPUT_MISSING, ZONE_PASS_NOT_PREPARED,
};

/// Width of the polygon vertex texture in texels (one polygon zone per row).
pub const ZONE_VERTEX_WIDTH: u32 = 64;

/// What the zone pass reads of the frame: the store (polygon vertices) and the zones visible
/// this frame, from `collect_ambient_zones`.
#[derive(Clone, Copy, Debug)]
pub struct ZoneInput<'a> {
    /// The zones of the scene.
    pub store: &'a AmbientZoneStore,
    /// The zones visible this frame, in fold order.
    pub list: &'a AmbientZoneList,
}

impl ZoneInput<'_> {
    /// Zones are drawn: the frame has an ambient zone pass.
    pub fn is_active(&self) -> bool {
        !self.list.zones.is_empty()
    }
}

/// Rows of the vertex texture for `row_count` polygon rows: the next power of two, at least 1.
pub fn zone_vertex_rows(row_count: u32) -> u32 {
    row_count.max(1).next_power_of_two()
}

/// Bytes of a vertex texture of `rows` rows (`R32G32_FLOAT`).
pub fn zone_vertex_bytes(rows: u32) -> u64 {
    u64::from(rows) * u64::from(ZONE_VERTEX_WIDTH) * 8
}

/// Writes the vertex texture into `out`: zero, then the world vertices of every polygon zone
/// as `x, y` pairs on its row.
pub fn fill_zone_vertices(store: &AmbientZoneStore, rows: u32, out: &mut [u8]) {
    let total = (zone_vertex_bytes(rows) as usize).min(out.len());
    out[..total].fill(0);
    let row_bytes = ZONE_VERTEX_WIDTH as usize * 8;
    for (row, zone) in store.polygons() {
        let start = row as usize * row_bytes;
        let Some(dst) = out[..total].get_mut(start..start + row_bytes) else {
            continue;
        };
        for (d, v) in dst
            .as_chunks_mut::<8>()
            .0
            .iter_mut()
            .zip(zone.world_vertices())
        {
            d[..4].copy_from_slice(&v.x.to_le_bytes());
            d[4..].copy_from_slice(&v.y.to_le_bytes());
        }
    }
}

/// Writes the zone records ([`GPU_ZONE_BYTES`] each, 5 `float4`) of `list` into `out`; returns
/// the bytes written (fewer if `out` is too short).
pub fn write_zones(list: &AmbientZoneList, out: &mut [u8]) -> usize {
    let mut at = 0;
    for z in &list.zones {
        let Some(dst) = out.get_mut(at..at + GPU_ZONE_BYTES) else {
            break;
        };
        let rows = [&z.scene_rect, &z.color, &z.shape, &z.span, &z.geom];
        for (d, v) in dst
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(rows.iter().flat_map(|r| r.iter()))
        {
            d.copy_from_slice(&v.to_le_bytes());
        }
        at += GPU_ZONE_BYTES;
    }
    at
}

uniform_block! {
    /// `cbuffer ZonePassParams` of `ambient_zone.vert` (16 bytes).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct ZonePassParams[16] {
        /// Scene size in pixels.
        pub scene_size: [f32; 2],
        /// Index of this draw's first zone in the zone buffer.
        pub instance_offset: u32,
        /// Padding.
        pub pad: u32,
    }
}

uniform_block! {
    /// `cbuffer ZoneFragParams` of `ambient_zone.frag` (64 bytes).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct ZoneFragParams[64] {
        /// Rows of the inverse of `[x_axis y_axis]`.
        pub inv_axes: [f32; 4],
        /// xy projection origin, zw z axis (scene pixels per unit of height).
        pub origin_z_axis: [f32; 4],
        /// xy scene size in pixels, z surface height step, w 1 = surface buffer present.
        pub scene_step: [f32; 4],
        /// xy `1 / (width, rows)` of the vertex texture.
        pub vertex_size: [f32; 4],
    }
}

impl ZoneFragParams {
    /// The block of a frame; `vertex_rows` is 0 without a vertex texture. The inverse is the
    /// one of the light programs (`SurfaceParams`), so the world points agree.
    pub fn new(
        p: &Projection,
        scene_w: u32,
        scene_h: u32,
        height_step: f32,
        surface_buffer: bool,
        vertex_rows: u32,
    ) -> Self {
        let det = f64::from(p.x_axis.x) * f64::from(p.y_axis.y)
            - f64::from(p.x_axis.y) * f64::from(p.y_axis.x);
        let o = p.origin.raw();
        Self {
            inv_axes: [
                (f64::from(p.y_axis.y) / det) as f32,
                (-f64::from(p.y_axis.x) / det) as f32,
                (-f64::from(p.x_axis.y) / det) as f32,
                (f64::from(p.x_axis.x) / det) as f32,
            ],
            origin_z_axis: [o.x, o.y, p.z_axis.x, p.z_axis.y],
            scene_step: [
                scene_w as f32,
                scene_h as f32,
                height_step,
                if surface_buffer { 1.0 } else { 0.0 },
            ],
            vertex_size: [
                1.0 / ZONE_VERTEX_WIDTH as f32,
                if vertex_rows > 0 {
                    1.0 / vertex_rows as f32
                } else {
                    1.0
                },
                0.0,
                0.0,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_rows_are_a_power_of_two_and_at_least_one() {
        assert_eq!(zone_vertex_rows(0), 1);
        assert_eq!(zone_vertex_rows(1), 1);
        assert_eq!(zone_vertex_rows(3), 4);
        assert_eq!(zone_vertex_rows(4), 4);
        assert_eq!(zone_vertex_bytes(4), 4 * 64 * 8);
    }

    #[test]
    fn the_pass_block_is_sixteen_bytes_in_shader_order() {
        let b = ZonePassParams {
            scene_size: [96.0, 48.0],
            instance_offset: 7,
            pad: 0,
        }
        .to_bytes();
        assert_eq!(b.len(), 16);
        assert_eq!(&b[0..4], &96.0f32.to_le_bytes());
        assert_eq!(&b[4..8], &48.0f32.to_le_bytes());
        assert_eq!(&b[8..12], &7u32.to_le_bytes());
    }

    #[test]
    fn the_fragment_block_inverts_the_identity_and_sizes_the_vertex_texture() {
        let p = ZoneFragParams::new(&Projection::IDENTITY, 96, 48, 0.5, true, 4);
        assert_eq!(p.inv_axes, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(p.scene_step, [96.0, 48.0, 0.5, 1.0]);
        assert_eq!(p.vertex_size[..2], [1.0 / 64.0, 0.25]);
        assert_eq!(p.to_bytes().len(), 64);
        let none = ZoneFragParams::new(&Projection::IDENTITY, 96, 48, 0.5, false, 0);
        assert_eq!(none.vertex_size[1], 1.0);
        assert_eq!(none.scene_step[3], 0.0);
    }
}
