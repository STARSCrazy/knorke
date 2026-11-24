// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/lighting/AmbientZoneCulling.hpp, src/lighting/AmbientZoneCulling.cpp), zlib licence:
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Culling of the ambient zones against one scene (P11): the zones that can touch the scene, in
//! fold order, as [`GpuZone`] records for the zone pass.
//!
//! Redesign against the C++ version: the scene arguments are a [`CullScene`] (projection, height
//! limit, surface flag and size) instead of seven loose parameters, colours are `[f32; 3]`, and
//! the output buffer is a host-owned [`AmbientZoneList`] that keeps its capacity from frame to
//! frame.

use crate::ambient_zone::{GpuZone, ZoneShape, zone_color, zone_scene_rect};
use crate::ambient_zone_store::AmbientZoneStore;
use crate::projection::Projection;

/// The scene a frame is culled against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CullScene {
    /// World to scene-pixel projection.
    pub projection: Projection,
    /// Highest height any pixel can show.
    pub max_height: f32,
    /// A surface buffer exists: zones are tested at their heights, not only on the ground.
    pub has_surface: bool,
    /// Scene width in pixels.
    pub width: u32,
    /// Scene height in pixels.
    pub height: u32,
}

/// The visible zones of one frame, in fold order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AmbientZoneList {
    /// The records, in fold order; keeps its capacity between frames.
    pub zones: Vec<GpuZone>,
    /// A polygon zone is among them: the pass reads the vertex texture.
    pub any_polygon: bool,
    /// Visible zones with `base_height > 0` or a finite height (for the height-span warning).
    pub height_spans: u32,
}

/// Culls the store's zones against the scene and writes each visible one as [`GpuZone`]: colour
/// `C` (relative zones times the global ambient) plus `directional` (the flat directional sum of
/// the lights). Disabled zones and zones whose rectangle misses the scene fall away.
pub fn collect_ambient_zones(
    store: &AmbientZoneStore,
    global: [f32; 3],
    scene: &CullScene,
    directional: [f32; 3],
    out: &mut AmbientZoneList,
) {
    out.zones.clear();
    out.any_polygon = false;
    out.height_spans = 0;
    for (id, z) in store.iter_ordered() {
        let p = z.params();
        if p.disabled {
            continue;
        }
        let Some(rect) = zone_scene_rect(
            z,
            &scene.projection,
            scene.max_height,
            scene.has_surface,
            scene.width,
            scene.height,
        ) else {
            continue;
        };
        let c = zone_color(z, global);
        let (cos_r, sin_r) = z.cos_sin();
        let mut g = GpuZone {
            scene_rect: rect,
            color: [
                c[0] + directional[0],
                c[1] + directional[1],
                c[2] + directional[2],
                p.shape.code(),
            ],
            shape: [p.x, p.y, cos_r, sin_r],
            span: [
                p.soft_edge,
                p.base_height,
                p.height.unwrap_or(f32::MAX),
                if scene.has_surface { 1.0 } else { 0.0 },
            ],
            geom: [0.0; 4],
        };
        match p.shape {
            ZoneShape::Circle => g.geom[0] = p.radius,
            ZoneShape::Polygon => {
                g.geom[0] = store.polygon_row(id).map_or(-1.0, |r| r as f32);
                g.geom[1] = z.world_vertices().len() as f32;
                out.any_polygon = true;
            }
            ZoneShape::Box => {
                let (hw, hd) = z.half_extents();
                g.geom[0] = hw;
                g.geom[1] = hd;
            }
        }
        if p.base_height > 0.0 || p.height.is_some() {
            out.height_spans += 1;
        }
        out.zones.push(g);
    }
}
