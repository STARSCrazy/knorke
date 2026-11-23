//! Texture, palette and sky management.

use super::*;

impl Renderer {
    /// Adds a texture and returns its id.
    pub fn add_texture(&mut self, t: Texture) -> i32 {
        self.tex_opaque.push(t.texels.iter().all(|t| t[3] >= 128));
        self.textures.push(t);
        self.tex_gen += 1;
        self.textures.len() as i32 - 1
    }

    /// Replaces texture `id`; false for an unknown id.
    pub fn replace_texture(&mut self, id: i32, t: Texture) -> bool {
        match self.textures.get_mut(id.max(0) as usize) {
            Some(slot) if id >= 0 => {
                self.tex_opaque[id as usize] = t.texels.iter().all(|t| t[3] >= 128);
                // a replaced animation becomes a plain texture
                self.effects.clear_anim(id as usize);
                *slot = t;
                self.tex_gen += 1;
                true
            }
            _ => false,
        }
    }

    /// Animated texture: a new texture id that draws `frames[anim_frame(time, fps, n)]` at the
    /// renderer time. Frames must be existing textures that are not animations themselves.
    /// Returns -1 for no frames, an invalid frame or fps <= 0 (or NaN).
    pub fn add_texture_anim(&mut self, frames: &[i32], fps: f32) -> i32 {
        let n = self.textures.len();
        let valid = |&f: &i32| f >= 0 && (f as usize) < n && !self.effects.is_anim(f as usize);
        if frames.is_empty() || !(fps > 0.0) || !fps.is_finite() || !frames.iter().all(valid) {
            return -1;
        }
        let frames: Vec<usize> = frames.iter().map(|&f| f as usize).collect();
        // the slot holds a copy of the first frame, so code that reads the texture itself (sizes) works
        let id = self.add_texture(self.textures[frames[0]].clone()) as usize;
        self.effects.set_anim(id, &frames, fps);
        id as i32
    }

    /// Scrolls texture `id` by (du, dv) texture coordinates per second of renderer time. Applies
    /// wherever the id is drawn (also clamped faces, which then repeat). False for an unknown id.
    pub fn set_texture_scroll(&mut self, id: i32, du: f32, dv: f32) -> bool {
        if id < 0 || id as usize >= self.textures.len() || !du.is_finite() || !dv.is_finite() {
            return false;
        }
        self.effects.set_scroll(id as usize, du, dv);
        true
    }

    /// Texture id to sample and texture coordinate offset for id `id` at the current time.
    pub(super) fn resolve_texture(&self, id: i32) -> (i32, Option<[f32; 2]>) {
        if id < 0 {
            return (id, None);
        }
        let t = self.time;
        (
            self.effects.frame(id as usize, t) as i32,
            self.effects.offset(id as usize, t),
        )
    }

    /// Quad with its animation frame and scroll offset applied.
    pub(super) fn resolve_quad(&self, q: &Quad) -> Quad {
        let (texture, off) = self.resolve_texture(q.texture);
        let mut r = Quad { texture, ..*q };
        if let Some([du, dv]) = off {
            for uv in r.uv.iter_mut() {
                uv[0] += du;
                uv[1] += dv;
            }
            r.wrap = true;
        }
        r
    }

    /// Gives texture `id` a far level of its own (see Texture::add_level).
    pub fn add_texture_level(&mut self, id: i32, level: Texture) -> bool {
        match self.textures.get_mut(id.max(0) as usize) {
            Some(t) if id >= 0 => {
                t.add_level(level);
                self.tex_gen += 1;
                true
            }
            _ => false,
        }
    }

    /// Contour mask of texture `id` (one byte per texel, 0 = normal, 1..255 = importance; None
    /// removes it); rebuilds its levels. False for an unknown id or a wrong size.
    pub fn set_texture_contour(&mut self, id: i32, mask: Option<&[u8]>) -> bool {
        self.set_texture_level_contour(id, 0, mask)
    }

    /// Mask for level `level` of texture `id` (0 = the texture, 1.. = caller-made far levels,
    /// widest first; see Texture::set_level_contour).
    pub fn set_texture_level_contour(
        &mut self,
        id: i32,
        level: usize,
        mask: Option<&[u8]>,
    ) -> bool {
        match self.textures.get_mut(id.max(0) as usize) {
            Some(t) if id >= 0 => t.set_level_contour(level, mask.map(|m| m.to_vec())),
            _ => false,
        }
    }

    /// Automatic contour mask from the picture of texture `id` (see Texture::contour_auto);
    /// strength 0 removes the mask.
    pub fn texture_contour_auto(&mut self, id: i32, strength: f32) -> bool {
        match self.textures.get_mut(id.max(0) as usize) {
            Some(t) if id >= 0 => {
                let m = t.contour_auto(strength);
                t.set_contour(m)
            }
            _ => false,
        }
    }

    /// Texture `id`, if any.
    pub fn texture(&self, id: i32) -> Option<&Texture> {
        self.textures.get(id.max(0) as usize).filter(|_| id >= 0)
    }

    /// Output palette (RGB triples, at most 256); empty removes it. False for a bad length.
    pub fn set_palette(&mut self, rgb: &[u8]) -> bool {
        if rgb.is_empty() {
            self.palette = None;
            return true;
        }
        match Palette::new(rgb) {
            Some(p) => {
                self.palette = Some(p);
                true
            }
            None => false,
        }
    }

    /// Legacy sky: texture `texture` repeated `repeats` times around the horizon; -1 removes it.
    pub fn set_sky(&mut self, texture: i32, repeats: f32) {
        self.sky = if texture >= 0 && (texture as usize) < self.textures.len() {
            Some((texture as usize, if repeats > 0.0 { repeats } else { 1.0 }))
        } else {
            None
        };
    }
}
