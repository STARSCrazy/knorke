//! Transparent and additive surfaces. The rasterizer records their fragments per pixel instead of
//! writing the G-buffer; `Renderer::end` blends them over the lit opaque scene, far to near.
//!
//! Storage: one flat fragment list plus one list head per pixel (fragments of a pixel are linked
//! through `Frag::next`). Both are kept between frames, so a frame allocates nothing once warm.

use crate::math::Vec3;

/// Opaque (no blending).
pub const BLEND_NONE: i32 = 0;
/// Mix over what is behind: colour * a + behind * (1 - a). Lit like opaque surfaces.
pub const BLEND_ALPHA: i32 = 1;
/// Add colour * a to what is behind; ignores light (glows), fades with fog.
pub const BLEND_ADD: i32 = 2;

/// End of a fragment list.
pub const NO_FRAG: u32 = u32::MAX;
/// Fragments stored per frame at most; later ones are dropped (in recording order, so the result
/// stays deterministic). Keeps memory bounded and every index below NO_FRAG.
pub const MAX_FRAGS: usize = 1 << 20;

pub use knorke_math::step_alpha;

/// How one draw call blends: mode (BLEND_ALPHA or BLEND_ADD), its alpha before stepping, and its
/// draw order this frame (ties at equal depth: the later draw lies on top).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blend {
    /// BLEND_ALPHA or BLEND_ADD.
    pub mode: i32,
    /// Draw alpha before stepping.
    pub alpha: f32,
    /// Draw order this frame.
    pub order: u32,
}

impl Blend {
    /// None for BLEND_NONE or an unknown mode (drawn opaque).
    pub fn new(mode: i32, alpha: f32, order: u32) -> Option<Blend> {
        (mode == BLEND_ALPHA || mode == BLEND_ADD).then_some(Blend { mode, alpha, order })
    }
}

/// One transparent fragment.
#[derive(Clone, Copy, Debug)]
pub struct Frag {
    /// View depth.
    pub depth: f32,
    /// Texel colour.
    pub color: [u8; 3],
    /// BLEND_ALPHA or BLEND_ADD.
    pub mode: i32,
    /// Effective alpha (draw alpha times texel alpha), stepped; > 0.
    pub alpha: f32,
    /// Light level of the surface (like GBuffer::light).
    pub light: f32,
    /// World normal of the surface.
    pub normal: Vec3,
    /// Draw order (see `Blend::order`).
    pub order: u32,
    /// Colour is final (particles): only fog applies, the light value is ignored.
    pub prelit: bool,
    /// `NO_OWNER`, or the particle (prelit) or billboard sprite this fragment belongs to
    /// (`GBuffer::owner`).
    pub owner: u32,
    /// Next fragment of the same pixel, or `NO_FRAG`.
    pub next: u32,
}

#[derive(Default)]
/// Transparent fragments of a frame, one linked list per pixel.
pub struct FragStore {
    /// All fragments of the frame.
    pub frags: Vec<Frag>,
    /// First fragment per pixel; empty until the first fragment of the renderer's life.
    head: Vec<u32>,
    pixels: usize,
    /// Fragments dropped over MAX_FRAGS since the last `clear`.
    pub dropped: u32,
}

impl FragStore {
    /// A store for frames of `pixels` pixels.
    pub fn new(pixels: usize) -> FragStore {
        FragStore {
            frags: Vec::new(),
            head: Vec::new(),
            pixels,
            dropped: 0,
        }
    }

    /// True if no fragment is stored.
    pub fn is_empty(&self) -> bool {
        self.frags.is_empty()
    }

    /// Drops all fragments; costs nothing when there were none.
    pub fn clear(&mut self) {
        if !self.frags.is_empty() {
            self.frags.clear();
            self.head.fill(NO_FRAG);
        }
        self.dropped = 0;
    }

    /// Calls `f(pixel, depth)` for every stored fragment.
    pub fn for_each(&self, mut f: impl FnMut(usize, f32)) {
        if self.frags.is_empty() {
            return;
        }
        for (pixel, &first) in self.head.iter().enumerate() {
            let mut k = first;
            while k != NO_FRAG {
                let fr = &self.frags[k as usize];
                f(pixel, fr.depth);
                k = fr.next;
            }
        }
    }

    /// Calls `f(pixel, index, fragment)` for every stored fragment (index into `frags`).
    pub fn for_each_frag(&self, mut f: impl FnMut(usize, usize, &Frag)) {
        if self.frags.is_empty() {
            return;
        }
        for (pixel, &first) in self.head.iter().enumerate() {
            let mut k = first;
            while k != NO_FRAG {
                let fr = &self.frags[k as usize];
                f(pixel, k as usize, fr);
                k = fr.next;
            }
        }
    }

    /// Records a fragment; dropped once MAX_FRAGS are stored or for a pixel outside the frame.
    pub fn push(&mut self, pixel: usize, mut f: Frag) {
        if pixel >= self.pixels {
            return;
        }
        if self.frags.len() >= MAX_FRAGS {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        if self.head.is_empty() {
            self.head = vec![NO_FRAG; self.pixels];
        }
        f.next = self.head[pixel];
        self.head[pixel] = self.frags.len() as u32;
        self.frags.push(f);
    }

    /// Indices of the fragments of `pixel` nearer than `limit`, in blending order: far to near,
    /// at equal depth by draw order, then by recording order. A fixed order, so the result does
    /// not depend on threads.
    pub fn sorted(&self, pixel: usize, limit: f32, out: &mut Vec<u32>) {
        out.clear();
        if self.frags.is_empty() {
            return;
        }
        let mut k = self.head[pixel];
        while k != NO_FRAG {
            let f = &self.frags[k as usize];
            if f.depth < limit {
                out.push(k);
            }
            k = f.next;
        }
        let fr = &self.frags;
        out.sort_unstable_by(|&a, &b| {
            let (fa, fb) = (&fr[a as usize], &fr[b as usize]);
            fb.depth
                .total_cmp(&fa.depth)
                .then(fa.order.cmp(&fb.order))
                .then(a.cmp(&b))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frag(depth: f32, order: u32) -> Frag {
        Frag {
            depth,
            color: [0; 3],
            mode: BLEND_ALPHA,
            alpha: 0.5,
            light: 1.0,
            normal: Vec3::default(),
            order,
            prelit: false,
            owner: crate::raster::NO_OWNER,
            next: NO_FRAG,
        }
    }

    #[test]
    fn push_stops_at_cap() {
        let mut s = FragStore::new(2);
        for k in 0..MAX_FRAGS + 10 {
            s.push(k & 1, frag(1.0, k as u32));
        }
        s.push(5, frag(1.0, 0)); // outside: ignored
        assert_eq!(s.frags.len(), MAX_FRAGS);
        let mut out = Vec::new();
        s.sorted(0, 5.0, &mut out);
        assert_eq!(out.len(), MAX_FRAGS / 2);
        assert!(out.iter().all(|&k| (k as usize) < MAX_FRAGS));
    }

    #[test]
    fn sorted_far_to_near_then_draw_order() {
        let mut s = FragStore::new(4);
        s.push(1, frag(2.0, 0));
        s.push(1, frag(3.0, 1));
        s.push(1, frag(2.0, 2));
        s.push(1, frag(9.0, 3)); // behind the opaque surface
        s.push(2, frag(1.0, 4));
        let mut out = Vec::new();
        s.sorted(1, 5.0, &mut out);
        assert_eq!(out, vec![1, 0, 2]);
        s.clear();
        s.sorted(1, 5.0, &mut out);
        assert!(out.is_empty() && s.is_empty());
    }
}
