// Origin: chiaro @ 9bc5b9b, src/rendering/OccluderGeometry.cpp (CollectTriangles, WidenCircle,
// TriangleBuffer::Fill).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The world triangles of the obstacles that are not tile grids, with what the shaders need per
//! triangle: its opacity (and, in height mode, its height interval).

use super::params::interval_channels;
use crate::ObstacleStore;
use crate::obstacle::{ObstacleKind, PatternSource, StoredObstacle};
use knorke_math::{At, Vec2, World2, bytemuck, det};

/// Smallest circle radius ever reported when there is no circle.
const NO_CIRCLE: f32 = 1e30;

/// A [`PatternSource`] for a host without patterns: every pattern is gone, so an obstacle writes
/// its scalar opacity.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoPatterns;

impl PatternSource for NoPatterns {
    fn mean_opacity(&self, _id: u32) -> Option<f32> {
        None
    }
}

/// Moves the world triangles `first..` of a circle away from its centre so that its radius is
/// `min_radius` (thinner than one mask texel: it would vanish between the texel centres).
pub fn widen_circle(
    o: &StoredObstacle,
    min_radius: f32,
    first: usize,
    out: &mut [At<World2, Vec2>],
) {
    let k = min_radius / o.radius;
    let c = o.common.position.raw();
    for v in &mut out[first..] {
        let p = v.raw();
        *v = At::new(Vec2::new(c.x + (p.x - c.x) * k, c.y + (p.y - c.y) * k));
    }
}

/// Floats a triangle buffer holds: the vertices (two each) and the opacities, padded to an even
/// count so that the shader can read them as `float2`.
pub fn triangle_buffer_floats(vertices: usize, opacities: usize) -> usize {
    vertices * 2 + opacities.div_ceil(2) * 2
}

/// Writes the triangle buffer: the vertices, then the opacities (one per triangle, the pad
/// float 0). `out` must hold [`triangle_buffer_floats`]` * 4` bytes; the rest is not touched.
pub fn write_triangle_buffer(vertices: &[At<World2, Vec2>], opacities: &[f32], out: &mut [u8]) {
    let split = vertices.len() * 8;
    out[..split].copy_from_slice(bytemuck::cast_slice(vertices));
    let tail = opacities.len() * 4;
    out[split..split + tail].copy_from_slice(bytemuck::cast_slice(opacities));
    if opacities.len() % 2 == 1 {
        out[split + tail..split + tail + 4].copy_from_slice(&0.0f32.to_le_bytes());
    }
}

/// The triangles of the static or the dynamic obstacles, collected into reused buffers.
#[derive(Clone, Debug)]
pub struct TriangleSet {
    vertices: Vec<At<World2, Vec2>>,
    opacities: Vec<f32>,
    intervals: Vec<[f32; 4]>,
    thin: u32,
    smallest: f32,
}

impl Default for TriangleSet {
    fn default() -> Self {
        Self {
            vertices: Vec::new(),
            opacities: Vec::new(),
            intervals: Vec::new(),
            thin: 0,
            smallest: NO_CIRCLE,
        }
    }
}

impl TriangleSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Collects the world triangles of every active obstacle that is no tile grid and whose
    /// `dynamic` flag is `dynamic`, in store order, with one opacity per triangle: the value
    /// the obstacle writes to the mask ([`ObstacleStore::mask_opacity`]), negated for an
    /// obstacle that casts a footprint shadow. With `with_intervals` also the four height
    /// channels per triangle. Circles with a radius below `min_radius` are widened to it
    /// ([`thin`](Self::thin) counts them). Allocates only while the buffers grow.
    pub fn collect(
        &mut self,
        obstacles: &ObstacleStore,
        patterns: &impl PatternSource,
        dynamic: bool,
        min_radius: f32,
        with_intervals: bool,
    ) {
        self.vertices.clear();
        self.opacities.clear();
        self.intervals.clear();
        self.thin = 0;
        self.smallest = NO_CIRCLE;
        let wanted = |o: &StoredObstacle| {
            o.is_active() && o.common.dynamic == dynamic && o.kind != ObstacleKind::TileGrid
        };
        let total: usize = obstacles
            .iter()
            .filter(|(_, o)| wanted(o))
            .map(|(id, _)| obstacles.triangles(id).len())
            .sum();
        self.vertices.reserve(total);
        self.opacities.reserve(total / 3);
        if with_intervals {
            self.intervals.reserve(total / 3);
        }
        for (id, o) in obstacles.iter() {
            if !wanted(o) {
                continue;
            }
            let first = self.vertices.len();
            obstacles.append_world_triangles(id, &mut self.vertices);
            if o.kind == ObstacleKind::Circle {
                self.smallest = det::min(self.smallest, o.radius);
                if o.radius < min_radius {
                    self.thin += 1;
                    widen_circle(o, min_radius, first, &mut self.vertices);
                }
            }
            let triangles = (self.vertices.len() - first) / 3;
            let opacity = obstacles
                .mask_opacity(id, 0, patterns)
                .unwrap_or(o.common.opacity);
            let opacity = if o.common.shadow_footprint {
                -opacity
            } else {
                opacity
            };
            self.opacities
                .resize(self.opacities.len() + triangles, opacity);
            if with_intervals {
                let channels = interval_channels(o);
                self.intervals
                    .resize(self.intervals.len() + triangles, channels);
            }
        }
    }

    /// The vertices, three per triangle.
    pub fn vertices(&self) -> &[At<World2, Vec2>] {
        &self.vertices
    }

    /// One opacity per triangle (negative: a footprint obstacle).
    pub fn opacities(&self) -> &[f32] {
        &self.opacities
    }

    /// One `[-base, top, finite -base, finite top]` per triangle; empty unless collected with
    /// intervals.
    pub fn intervals(&self) -> &[[f32; 4]] {
        &self.intervals
    }

    /// Circles that were widened.
    pub fn thin(&self) -> u32 {
        self.thin
    }

    /// The smallest circle radius among the collected circles (`1e30` without a circle).
    pub fn smallest_circle(&self) -> f32 {
        self.smallest
    }

    /// `true` without triangles.
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }
}
