// Origin: chiaro @ 9bc5b9b, src/rendering/OcclusionCommon.hpp (DomainParams, TileParams,
// MakeDomainParams, ObstacleInterval, kInfiniteTop) and src/rendering/OccluderGeometry.cpp
// (ThinCircleRadius, ObstacleIntervals).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The uniform blocks of the occluder and tile shaders and the height values of an obstacle.

use crate::Projection;
use crate::obstacle::{ObstacleCommon, StoredObstacle};
use crate::shadow::{MaskAnchor, OcclusionDomain};
use knorke_math::{Vec2, det};

/// Top of an infinitely tall obstacle in the height texture: the largest half float.
pub const INFINITE_TOP: f32 = 65504.0;

fn put(out: &mut [u8], at: usize, values: &[f32]) {
    for (i, v) in values.iter().enumerate() {
        out[at + 4 * i..at + 4 * i + 4].copy_from_slice(&v.to_le_bytes());
    }
}

/// `cbuffer DomainParams` of `occluder.vert` and `tile.vert`: how a world position becomes a
/// mask texel (the projection, the margin band, the texel scale and the anchor remainder).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DomainParams {
    /// xy projection origin, zw x axis (scene pixels).
    pub origin_x_axis: [f32; 4],
    /// xy y axis, zw margin in scene pixels.
    pub y_axis_margin: [f32; 4],
    /// xy mask texels per scene pixel, zw mask size in texels.
    pub texel_scale_domain_size: [f32; 4],
    /// xy [`MaskAnchor::remainder`]; `z` is, for the occluder shaders, where the opacities
    /// start in the vertex buffer (a `u32` stored as its bits, see
    /// [`with_opacity_start`](Self::with_opacity_start)).
    pub remainder_pad: [f32; 4],
}

impl DomainParams {
    /// Size of the block in bytes.
    pub const SIZE: usize = 64;

    /// The parameters of a frame.
    pub fn new(p: &Projection, d: &OcclusionDomain, a: &MaskAnchor) -> Self {
        let origin = p.origin.raw();
        let margin = d.margin as f32;
        Self {
            origin_x_axis: [origin.x, origin.y, p.x_axis.x, p.x_axis.y],
            y_axis_margin: [p.y_axis.x, p.y_axis.y, margin, margin],
            texel_scale_domain_size: [
                d.texels_per_scene[0],
                d.texels_per_scene[1],
                d.width as f32,
                d.height as f32,
            ],
            remainder_pad: [a.remainder[0], a.remainder[1], 0.0, 0.0],
        }
    }

    /// The same, with the index where the opacities start in the vertex buffer: the number of
    /// vertices (the opacities follow them, two per entry).
    #[must_use]
    pub fn with_opacity_start(mut self, vertices: u32) -> Self {
        self.remainder_pad[2] = f32::from_bits(vertices);
        self
    }

    /// The block as the shader reads it (little endian).
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut out = [0; Self::SIZE];
        put(&mut out, 0, &self.origin_x_axis);
        put(&mut out, 16, &self.y_axis_margin);
        put(&mut out, 32, &self.texel_scale_domain_size);
        put(&mut out, 48, &self.remainder_pad);
        out
    }
}

/// `cbuffer DomainParams` of `tile.vert`: the domain and where one grid stands.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TileParams {
    /// The domain.
    pub domain: DomainParams,
    /// xy world position, z rotation in radians.
    pub placement: [f32; 4],
    /// xy cell size, zw columns and rows.
    pub grid_size: [f32; 4],
}

impl TileParams {
    /// Size of the block in bytes.
    pub const SIZE: usize = 96;

    /// The block as the shader reads it (little endian).
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut out = [0; Self::SIZE];
        out[..DomainParams::SIZE].copy_from_slice(&self.domain.to_bytes());
        put(&mut out, 64, &self.placement);
        put(&mut out, 80, &self.grid_size);
        out
    }
}

/// `(-base, top)` of an obstacle in the height texture; infinitely tall: `(0, INFINITE_TOP)`.
pub fn obstacle_interval(c: &ObstacleCommon) -> [f32; 2] {
    if c.height > 0.0 {
        [-c.base_height, c.height]
    } else {
        [0.0, INFINITE_TOP]
    }
}

/// The four height channels of an obstacle's triangles: its interval, and the same again if the
/// obstacle is finite, else "none" (`-INFINITE_TOP` twice).
pub fn interval_channels(o: &StoredObstacle) -> [f32; 4] {
    let [neg_base, top] = obstacle_interval(&o.common);
    if o.has_finite_height() {
        [neg_base, top, neg_base, top]
    } else {
        [neg_base, top, -INFINITE_TOP, -INFINITE_TOP]
    }
}

/// Circle radius below which the mask widens circles: one texel across along the shorter
/// world axis.
pub fn thin_circle_radius(p: &Projection, d: &OcclusionDomain) -> f32 {
    let [tx, ty] = d.texels_per_scene;
    let along = |axis: Vec2| {
        let (x, y) = (axis.x * tx, axis.y * ty);
        (x * x + y * y).sqrt()
    };
    0.5 / det::max(1e-6, det::min(along(p.x_axis), along(p.y_axis)))
}
