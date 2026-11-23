//! Particle emitters and particle drawing.

use super::*;

impl Renderer {
    /// Creates a persistent emitter; ids are never reused. Drawn by `draw_emitters`.
    pub fn create_emitter(&mut self, e: Emitter) -> u32 {
        let id = self.next_emitter;
        self.next_emitter = id.wrapping_add(1);
        self.emitters.insert(id, e);
        id
    }

    /// Changes an emitter; false for an unknown or destroyed id.
    pub fn set_emitter(&mut self, id: u32, e: Emitter) -> bool {
        match self.emitters.get_mut(&id) {
            Some(slot) => {
                *slot = e;
                true
            }
            None => false,
        }
    }

    /// Removes an emitter; false for an unknown or destroyed id.
    pub fn destroy_emitter(&mut self, id: u32) -> bool {
        self.emitters.remove(&id).is_some()
    }

    /// Draws all emitters at the current time (`set_time`), in id order. With a grid, rain and snow
    /// are left out under roofs, below floors and inside solid cells.
    pub fn draw_emitters(&mut self, grid: Option<&GridMap>) {
        if !self.frame_open || self.emitters.is_empty() {
            return;
        }
        let time = self.time;
        // Walk the ids in order without collecting them (no allocation per frame).
        let mut next = self.emitters.keys().next().copied();
        while let Some(id) = next {
            next = self.emitters.range(id + 1..).next().map(|(&k, _)| k);
            let e = self.emitters[&id];
            let Some((lo, hi)) = emit_range(&e, time) else {
                continue;
            };
            let weather = e.kind == EMIT_RAIN || e.kind == EMIT_SNOW;
            // At most EMIT_MAX living particles per emitter, oldest first; generation stops as soon
            // as the frame budget is full (same result as generating all and dropping the rest).
            let (mut i, mut n) = (lo, 0usize);
            while i <= hi && n < EMIT_MAX {
                if let Some(p) = emit_one(&e, time, i) {
                    n += 1;
                    if !(weather && grid.is_some_and(|g| indoors(g, p.particle.position))) {
                        // past the budget only counted (Stats::dropped_particles)
                        self.draw_particle(&p.particle, p.stretch);
                    }
                }
                i += 1;
            }
        }
    }

    /// Camera-facing squares, depth-tested; blended with alpha < 1 or PARTICLE_ADDITIVE. No shadows;
    /// `pick` sees through them. At most PARTICLE_BUDGET per frame; the rest is dropped in drawing order.
    pub fn draw_particles(&mut self, list: &[Particle]) {
        if !self.frame_open {
            return;
        }
        for (k, p) in list.iter().enumerate() {
            if self.particle_count >= PARTICLE_BUDGET {
                self.drop_particles(list.len() - k);
                return;
            }
            self.draw_particle(p, 1.0);
        }
    }

    /// Counts `n` particles dropped over the budget (Stats::dropped_particles).
    pub(crate) fn drop_particles(&mut self, n: usize) {
        if self.frame_open {
            self.dropped_particles = self
                .dropped_particles
                .saturating_add(n.min(u32::MAX as usize) as u32);
        }
    }

    /// One particle, `stretch` times taller than wide (rain streaks).
    pub(super) fn draw_particle(&mut self, p: &Particle, stretch: f32) {
        let Some(view) = self.view else { return };
        if self.particle_count >= PARTICLE_BUDGET {
            self.drop_particles(1);
            return;
        }
        self.particle_count += 1;
        let vp = view.to_view(p.position);
        let z = vp.z;
        if z.is_nan()
            || z < view.near
            || p.size.is_nan()
            || p.size <= 0.0
            || p.alpha.is_nan()
            || p.alpha <= 0.0
        {
            return;
        }
        let hw = p.size * 0.5 * view.focal / z;
        let hh = hw * stretch;
        let sx = view.cx + vp.x * view.focal / z;
        let sy = view.cy - vp.y * view.focal / z;
        if !(hw.is_finite() && hh.is_finite() && sx.is_finite() && sy.is_finite()) {
            return;
        }
        // pixel centres inside the rectangle; at least one pixel
        let span = |c: f32, h: f32| {
            let (a, b) = ((c - h - 0.5).ceil(), (c + h - 0.5).ceil());
            if b > a {
                (a, b)
            } else {
                ((c - 0.5).round(), (c - 0.5).round() + 1.0)
            }
        };
        let (x0, x1) = span(sx, hw);
        let (y0, y1) = span(sy, hh);
        let (w, h) = (self.width as f32, self.height as f32);
        if x1 <= 0.0 || y1 <= 0.0 || x0 >= w || y0 >= h {
            return;
        }
        let tex_id = if p.texture >= 0 {
            Some(self.resolve_texture(p.texture).0 as usize)
        } else {
            None
        };
        if tex_id.is_some_and(|t| t >= self.textures.len()) {
            return;
        }
        let blend = if p.flags & PARTICLE_ADDITIVE != 0 {
            self.next_blend(BLEND_ADD, p.alpha)
        } else if p.alpha < 1.0 {
            self.next_blend(BLEND_ALPHA, p.alpha)
        } else {
            None
        };
        // pre-lit colour: emissive, or ambient + sun + point lights without shadows, worked out in
        // `end` once the frame's lights are known (`light_particles`); until then the pixels and
        // fragments hold the plain texel and the particle's index as their owner
        let pid = self.particle_lights.len() as u32;
        self.particle_lights.push(ParticleLight {
            pos: p.position,
            color: color_scaled(p.color, 1.0),
            lit: self.options.lighting && p.flags & PARTICLE_EMISSIVE == 0,
        });
        let tex = tex_id.map(|t| &self.textures[t]);
        let normal = -view.forward;
        let g = &mut self.gbuf;
        let (px0, px1) = (x0.max(0.0) as usize, x1.min(w) as usize);
        let (py0, py1) = (y0.max(0.0) as usize, y1.min(h) as usize);
        for py in py0..py1 {
            for px in px0..px1 {
                let i = py * self.width + px;
                if z >= g.depth[i] {
                    continue;
                }
                let texel = match tex {
                    Some(t) => t.sample_clamp(
                        (px as f32 + 0.5 - (sx - hw)) / (2.0 * hw),
                        (py as f32 + 0.5 - (sy - hh)) / (2.0 * hh),
                    ),
                    None => [255; 4],
                };
                let color = [texel[0], texel[1], texel[2]];
                if let Some(b) = blend {
                    let a = step_alpha(b.alpha * texel[3] as f32 / 255.0);
                    if a > 0.0 {
                        g.frags.push(
                            i,
                            Frag {
                                depth: z,
                                color,
                                mode: b.mode,
                                alpha: a,
                                light: 1.0,
                                normal,
                                order: b.order,
                                prelit: true,
                                owner: pid,
                                next: NO_FRAG,
                            },
                        );
                    }
                    continue;
                }
                if texel[3] < 128 {
                    continue;
                }
                if g.pick && !g.prelit[i] {
                    // first particle over this surface: keep it for picking (the id stays as it is)
                    g.pick_under.insert(i, (g.depth[i], g.normal[i]));
                }
                g.depth[i] = z;
                g.color[i] = color;
                g.light[i] = 1.0;
                g.prelit_any = true;
                g.prelit[i] = true;
                g.owner_any = true;
                g.owner[i] = pid;
                g.normal[i] = normal;
            }
        }
    }
}
