//! Persistent backdrops (background layers) and the legacy sky.

use super::*;
use crate::backdrop::{
    BACKDROP_CLAMP_BOTTOM, BACKDROP_CLAMP_TOP, BACKDROP_IGNORE_ALPHA, BACKDROP_TINT, Backdrop,
    Layer,
};

impl Renderer {
    fn backdrop_ok(&self, b: &Backdrop) -> bool {
        b.is_valid() && b.texture >= 0 && (b.texture as usize) < self.textures.len()
    }

    /// New backdrop; its id (never reused), or None for an invalid backdrop (unknown kind or
    /// texture, band height <= 0, numbers not finite).
    pub fn create_backdrop(&mut self, b: Backdrop) -> Option<u32> {
        if !self.backdrop_ok(&b) {
            return None;
        }
        let id = self.next_backdrop;
        self.next_backdrop = id.wrapping_add(1);
        self.backdrops.insert(id, b.public());
        Some(id)
    }

    /// Changes a backdrop; false for an unknown id or an invalid backdrop (then nothing changes).
    pub fn set_backdrop(&mut self, id: u32, b: Backdrop) -> bool {
        let ok = self.backdrop_ok(&b);
        match self.backdrops.get_mut(&id) {
            Some(slot) if ok => {
                *slot = b.public();
                true
            }
            _ => false,
        }
    }

    /// Removes a backdrop; false for an unknown or destroyed id.
    pub fn destroy_backdrop(&mut self, id: u32) -> bool {
        self.backdrops.remove(&id).is_some()
    }

    /// Backdrop `id`, if it exists.
    pub fn backdrop(&self, id: u32) -> Option<Backdrop> {
        self.backdrops.get(&id).copied()
    }

    /// Layers of this frame, back to front: the legacy sky, then backdrops by (order, id).
    /// Empty (no allocation) when nothing is set.
    pub(super) fn backdrop_layers(
        &self,
        view: &View,
        out: &mut Vec<Layer>,
        order: &mut Vec<(i32, u32)>,
    ) {
        out.clear();
        if self.sky.is_none() && self.backdrops.is_empty() {
            return;
        }
        let sky = self.sky.map(|(id, rep)| Backdrop {
            texture: id as i32,
            repeats: rep,
            flags: BACKDROP_CLAMP_TOP
                | BACKDROP_CLAMP_BOTTOM
                | BACKDROP_TINT
                | BACKDROP_IGNORE_ALPHA,
            ..Backdrop::default()
        });
        // (order, id) is unique, so the unstable sort (no allocation) is deterministic.
        order.clear();
        order.extend(self.backdrops.iter().map(|(&id, b)| (b.order, id)));
        order.sort_unstable();
        for b in sky
            .iter()
            .chain(order.iter().map(|(_, id)| &self.backdrops[id]))
        {
            let (frame, off) = self.resolve_texture(b.texture);
            let Some(t) = self.textures.get(frame as usize) else {
                continue;
            };
            if let Some(l) = Layer::new(b, frame as usize, t, off, view) {
                out.push(l);
            }
        }
    }
}
