//! Animated and scrolling textures: resolved at draw time from the renderer time.
//! An animation has a normal texture id; drawing with it draws its current frame.

use crate::renderer::anim_frame;

#[derive(Clone, Debug)]
struct Anim {
    frames: Vec<usize>,
    fps: f32,
}

/// Per texture id: animation (if the id is one) and scroll speed in texture coordinates per second.
#[derive(Default)]
pub(crate) struct TexEffects {
    anims: Vec<Option<Anim>>,
    scroll: Vec<[f32; 2]>,
}

impl TexEffects {
    /// Makes room for texture id `id` (no effects).
    pub(crate) fn grow(&mut self, len: usize) {
        if self.anims.len() < len {
            self.anims.resize(len, None);
            self.scroll.resize(len, [0.0, 0.0]);
        }
    }

    pub(crate) fn is_anim(&self, id: usize) -> bool {
        matches!(self.anims.get(id), Some(Some(_)))
    }

    pub(crate) fn set_anim(&mut self, id: usize, frames: &[usize], fps: f32) {
        self.grow(id + 1);
        self.anims[id] = Some(Anim {
            frames: frames.to_vec(),
            fps,
        });
    }

    /// The id becomes a plain texture again (it was replaced).
    pub(crate) fn clear_anim(&mut self, id: usize) {
        if let Some(a) = self.anims.get_mut(id) {
            *a = None;
        }
    }

    pub(crate) fn set_scroll(&mut self, id: usize, du: f32, dv: f32) {
        self.grow(id + 1);
        self.scroll[id] = [du, dv];
    }

    /// Texture to sample for id `id` at `time`: the current frame for an animation, else `id` itself.
    pub(crate) fn frame(&self, id: usize, time: f64) -> usize {
        match self.anims.get(id) {
            Some(Some(a)) => a.frames[anim_frame(time, a.fps, a.frames.len() as u32) as usize],
            _ => id,
        }
    }

    /// Texture coordinate offset of id `id` at `time` (the scroll set on that id), each in [0, 1).
    /// None when the id does not scroll.
    pub(crate) fn offset(&self, id: usize, time: f64) -> Option<[f32; 2]> {
        let s = *self.scroll.get(id)?;
        if s == [0.0, 0.0] {
            return None;
        }
        let f = |speed: f32| {
            let x = speed as f64 * time;
            if x.is_finite() {
                (x - x.floor()) as f32 % 1.0
            } else {
                0.0
            }
        };
        Some([f(s[0]), f(s[1])])
    }
}
