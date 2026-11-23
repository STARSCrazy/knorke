//! Things lit as a whole rather than per surface pixel: particles (one light value per particle,
//! handed in by the light pass) and billboard sprites (their pixels carry the billboard's index,
//! so a light pass can treat them as facing each light).

use super::*;

/// A particle drawn this frame, waiting for its light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleLight {
    /// World position.
    pub pos: Vec3,
    /// Particle colour (0..1 per channel).
    pub color: [f32; 3],
    /// false = emissive or lighting off: the colour as it is.
    pub lit: bool,
}

impl Renderer {
    /// Draws a sprite. With lighting, billboards record their axis so that point lights can light
    /// them as facing the light (their pixels and fragments get the billboard's index as owner).
    pub fn draw_sprite(&mut self, s: &Sprite) {
        if !(self.frame_open && self.options.lighting && s.flags & SPRITE_BILLBOARD != 0) {
            self.draw_sprite_body(s);
            return;
        }
        self.gbuf.cur_owner = self.billboard_axes.len() as u32;
        self.gbuf.owner_any = true;
        self.billboard_axes.push(s.position);
        self.draw_sprite_body(s);
        self.gbuf.cur_owner = NO_OWNER;
    }

    /// Linear tint of the pixels and fragments of every particle drawn this frame: lit ones take
    /// the light of the light pass (`light`, one value per particle) stepped by `light_levels`;
    /// emissive ones, and all without a light pass (`lit` false), keep their colour.
    pub(super) fn light_particles<P: Column<[f32; 3]>>(&mut self, light: &P, lit: bool) {
        let levels = self.options.light_levels;
        self.particle_tints.clear();
        for (k, p) in self.particle_lights.iter().enumerate() {
            let t = match light.get(k) {
                Some(&l) if p.lit && lit => {
                    let l = step_light(l, levels);
                    std::array::from_fn(|k| p.color[k] * l[k].clamp(0.0, 1.0))
                }
                _ => p.color,
            };
            self.particle_tints.push(t);
        }
        // The tints are applied in linear space while shading (`Shade::base`).
    }
}
