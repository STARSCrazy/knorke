//! Geometry submission: quads, grids, sprites and triangle lists.

use super::*;
use crate::grid::PART_BACK;

impl Renderer {
    pub(super) fn proj(v: &View) -> Proj {
        Proj {
            near: v.near,
            tan_l: v.tan_l,
            tan_r: v.tan_r,
            tan_t: v.tan_t,
            tan_b: v.tan_b,
            focal: v.focal,
            cx: v.cx,
            cy: v.cy,
        }
    }

    /// Record a quad as two world-space shadow casters (only while lighting is on).
    pub(super) fn record_quad(&mut self, q: &Quad) {
        let q = self.resolve_quad(q);
        self.record_resolved(&q);
    }

    pub(super) fn record_resolved(&mut self, q: &Quad) {
        if !self.options.lighting || q.texture < 0 {
            return;
        }
        let texture = q.texture as usize;
        for idx in [[0, 1, 2], [0, 2, 3]] {
            self.casters.push(ShadowCaster {
                tri: idx.map(|i| q.corners[i]),
                uv: idx.map(|i| q.uv[i]),
                texture,
                wrap: q.wrap,
                alpha: 1.0,
                sun_only: false,
            });
        }
    }

    /// Blend settings for the next transparent draw call; None for BLEND_NONE or an unknown mode.
    pub fn next_blend(&mut self, mode: i32, alpha: f32) -> Option<Blend> {
        let b = Blend::new(mode, alpha, self.blend_order)?;
        self.blend_order += 1;
        Some(b)
    }

    pub(super) fn draw_quad(
        &mut self,
        q: &Quad,
        cull: bool,
        cast: bool,
        id: u32,
        blend: Option<Blend>,
    ) {
        let q = &self.resolve_quad(q);
        if cast {
            self.record_resolved(q);
        }
        let Some(view) = self.view else { return };
        let Some(tex) = self.textures.get(q.texture.max(0) as usize) else {
            return;
        };
        if q.texture < 0 {
            return;
        }
        let p = Self::proj(&view);
        let cv: [ClipVert; 4] = std::array::from_fn(|i| ClipVert {
            pos: view.to_view(q.corners[i]),
            u: q.uv[i][0],
            v: q.uv[i][1],
            light: q.light,
        });
        // one level for the whole face: texels per pixel at the face centre, along the texture's u axis
        let face_lod = {
            let centre = (q.corners[0] + q.corners[1] + q.corners[2] + q.corners[3]) * 0.25;
            let z = view.to_view(centre).z.max(view.near);
            let len = (q.corners[1] - q.corners[0]).length();
            let du = (q.uv[1][0] - q.uv[0][0]).abs();
            if len > 0.0 && du > 0.0 {
                (tex.width as f32 * du / len * z / view.focal)
                    .max(1e-6)
                    .log2()
            } else {
                f32::NAN
            }
        };
        let per_face = !q.wrap && self.options.face_lod;
        let m = Material {
            texture: tex,
            wrap: q.wrap,
            cull_back: cull,
            normal: q.normal,
            lod: self.options.texture_lod,
            lod_bias: self.options.lod_bias,
            lod_dither: self.options.lod_dither,
            face_lod: if per_face { face_lod } else { f32::NAN },
            id,
            blend,
        };
        draw_triangle(&mut self.gbuf, &p, &[cv[0], cv[1], cv[2]], &m);
        draw_triangle(&mut self.gbuf, &p, &[cv[0], cv[2], cv[3]], &m);
    }

    /// Draws the cells of a grid map (walls, floors, ceilings, doors) within `Options::draw_distance`.
    pub fn draw_grid(&mut self, grid: &GridMap) {
        if !self.frame_open {
            return;
        }
        let Some(view) = self.view else { return };
        let mut quads = std::mem::take(&mut self.quads);
        let s = grid.cell_size;
        let limit = self.options.draw_distance;
        let lighting = self.options.lighting;
        let shadow_range = self.shadow_half();
        for z in 0..grid.depth {
            for x in 0..grid.width {
                let cx = (x as f32 + 0.5) * s - view.eye.x;
                let cz = (z as f32 + 0.5) * s - view.eye.z;
                let dist = (cx * cx + cz * cz).sqrt();
                let visible = !(limit > 0.0 && dist > limit + s);
                let shadowing = lighting && dist <= shadow_range + s;
                if !visible && !shadowing {
                    continue;
                }
                for level in 0..grid.levels {
                    quads.clear();
                    grid.cell_quads(x, z, level, &mut quads);
                    for q in &quads {
                        if q.alpha < 1.0 {
                            // see-through wall: stippled shadow, not pickable
                            let b = Blend::new(BLEND_ALPHA, q.alpha, 0);
                            self.record_see_through(q, b);
                            if visible {
                                let b = self.next_blend(BLEND_ALPHA, q.alpha);
                                self.draw_quad(q, true, false, 0, b);
                            }
                        } else if visible {
                            let id = self.pick_id(PickEntry {
                                kind: PICK_GRID,
                                tag: 0,
                                cell_x: x as i32,
                                cell_z: z as i32,
                                level: level as i32,
                                side: q.side,
                                part: q.part,
                            });
                            // a back face repeats its front's geometry: one shadow caster is enough
                            self.draw_quad(q, true, lighting && q.part & PART_BACK == 0, id, None);
                        } else if q.part & PART_BACK == 0 {
                            self.record_quad(q);
                        }
                    }
                }
                if shadowing {
                    quads.clear();
                    grid.shadow_quads(x, z, &|t| self.texture_opaque(t), &mut quads);
                    quads.iter().for_each(|q| self.record_shadow_only(q));
                }
            }
        }
        self.quads = quads;
    }

    pub(super) fn draw_sprite_body(&mut self, s: &Sprite) {
        if !self.frame_open {
            return;
        }
        let Some(view) = self.view else { return };
        let (right, normal) = if s.flags & SPRITE_BILLBOARD != 0 {
            (
                view.flat_right,
                Vec3::new(-view.forward.x, 0.0, -view.forward.z).normalized(),
            )
        } else {
            let (sy, cy) = s.yaw.sin_cos();
            // Facing direction for yaw 0 is +z (south): the sprite looks at a viewer standing south of it.
            (Vec3::new(cy, 0.0, sy), Vec3::new(-sy, 0.0, cy))
        };
        let hw = right * (s.width * 0.5);
        let base = s.position;
        let top = Vec3::new(0.0, s.height, 0.0);
        let (u0, u1) = if s.flags & SPRITE_FLIP_X != 0 {
            (1.0, 0.0)
        } else {
            (0.0, 1.0)
        };
        let q = Quad {
            corners: [base - hw + top, base + hw + top, base + hw, base - hw],
            uv: [[u0, 0.0], [u1, 0.0], [u1, 1.0], [u0, 1.0]],
            texture: s.texture,
            light: s.light,
            normal,
            wrap: false,
            side: -1,
            alpha: 1.0,
            part: 0,
        };
        if let Some(b) = self.next_blend(s.blend, s.alpha) {
            self.record_see_through_sprite(s, &q, Some(b));
            self.draw_quad(&q, false, false, 0, Some(b));
            return;
        }
        let id = self.pick_id(PickEntry {
            kind: PICK_SPRITE,
            tag: s.tag,
            cell_x: -1,
            cell_z: -1,
            level: -1,
            side: -1,
            part: 0,
        });
        if self.options.lighting && s.flags & SPRITE_NO_SHADOW == 0 && s.texture >= 0 {
            if s.flags & SPRITE_BILLBOARD != 0 {
                let (texture, scroll) = self.resolve_texture(s.texture);
                self.billboards.push(ShadowBillboard {
                    base,
                    width: s.width,
                    height: s.height,
                    texture: texture as usize,
                    flip: s.flags & SPRITE_FLIP_X != 0,
                    scroll,
                    alpha: 1.0,
                });
                self.draw_quad(&q, false, false, id, None);
                return;
            }
            self.draw_quad(&q, false, true, id, None);
            return;
        }
        self.draw_quad(&q, false, false, id, None);
    }

    /// Draw a triangle list (three vertices per triangle), flat-shaded with face normals.
    pub fn draw_triangles(&mut self, verts: &[Vertex], texture: i32, light: f32, cull: bool) {
        self.draw_triangles_ex(
            verts,
            texture,
            light,
            cull,
            true,
            0,
            1.0,
            crate::blend::BLEND_NONE,
        );
    }

    /// Like `draw_triangles`; `cast` = false keeps the mesh out of the shadow casters, `tag` is reported by `pick`.
    /// `blend` BLEND_ALPHA / BLEND_ADD with `alpha` draws it see-through (then no shadow, not pickable).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_triangles_ex(
        &mut self,
        verts: &[Vertex],
        texture: i32,
        light: f32,
        cull: bool,
        cast: bool,
        tag: u32,
        alpha: f32,
        blend: i32,
    ) {
        if !self.frame_open {
            return;
        }
        let Some(view) = self.view else { return };
        if texture < 0 {
            return;
        }
        let blend = self.next_blend(blend, alpha);
        let stipple = self.stipple_alpha(blend);
        let cast = cast && (blend.is_none() || stipple.is_some());
        let id = if blend.is_some() {
            0
        } else {
            self.pick_id(PickEntry {
                kind: PICK_MESH,
                tag,
                cell_x: -1,
                cell_z: -1,
                level: -1,
                side: -1,
                part: 0,
            })
        };
        let (texture, off) = self.resolve_texture(texture);
        let [du, dv] = off.unwrap_or([0.0, 0.0]);
        let scrolled = off.is_some();
        let Some(tex) = self.textures.get(texture as usize) else {
            return;
        };
        let p = Self::proj(&view);
        let m_base = Material {
            texture: tex,
            wrap: true,
            cull_back: cull,
            normal: Vec3::default(),
            lod: self.options.texture_lod,
            lod_bias: self.options.lod_bias,
            lod_dither: self.options.lod_dither,
            face_lod: f32::NAN,
            id,
            blend,
        };
        for t in verts.as_chunks::<3>().0 {
            let w: [Vec3; 3] = std::array::from_fn(|i| Vec3::new(t[i].x, t[i].y, t[i].z));
            // Clockwise on screen when seen from the front, matching the grid quads.
            let normal = (w[2] - w[0]).cross(w[1] - w[0]).normalized();
            if cast && self.options.lighting {
                self.casters.push(ShadowCaster {
                    tri: w,
                    uv: std::array::from_fn(|i| {
                        if scrolled {
                            [t[i].u + du, t[i].v + dv]
                        } else {
                            [t[i].u, t[i].v]
                        }
                    }),
                    texture: texture as usize,
                    wrap: true,
                    alpha: stipple.unwrap_or(1.0),
                    sun_only: false,
                });
            }
            let cv: [ClipVert; 3] = std::array::from_fn(|i| ClipVert {
                pos: view.to_view(w[i]),
                u: if scrolled { t[i].u + du } else { t[i].u },
                v: if scrolled { t[i].v + dv } else { t[i].v },
                light,
            });
            let m = Material { normal, ..m_base };
            draw_triangle(&mut self.gbuf, &p, &cv, &m);
        }
    }
}
