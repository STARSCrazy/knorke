//! Shadow casters recorded while drawing, for a light pass (`LightSamples::casters`).
//!
//! The renderer does not compute light; it only records what blocks light, in world space and
//! before clipping, with the texture ids for the alpha test. `knorke-integration` hands these
//! to `knorke-lighting` (whose `Caster` has the same fields).
//!
//! Origin: pixelraum `src/light.rs` (types) and `src/renderer/lights.rs` (recording) @ 81927d8.

use super::*;

/// A world-space triangle that blocks light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowCaster {
    /// Corners in world space.
    pub tri: [Vec3; 3],
    /// Texture coordinates of the corners.
    pub uv: [[f32; 2]; 3],
    /// Texture id (animation frame resolved), or [`SOLID_CASTER`] for an untextured blocker.
    pub texture: usize,
    /// Repeat the texture outside `[0, 1)` instead of clamping.
    pub wrap: bool,
    /// Draw alpha of a see-through caster; 1 = opaque. Below 1 the shadow is stippled.
    pub alpha: f32,
    /// Only the sun map draws it (grid roofs and ceiling caps, `grid::SIDE_SUN_ONLY`).
    pub sun_only: bool,
}

/// [`ShadowCaster::texture`] of untextured, fully opaque shadow-only blockers (grid volume closing).
pub const SOLID_CASTER: usize = usize::MAX;

/// A billboard sprite as a caster: an upright quad turned towards each light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowBillboard {
    /// Bottom centre in world space.
    pub base: Vec3,
    /// Width in world units.
    pub width: f32,
    /// Height in world units.
    pub height: f32,
    /// Texture id (animation frame resolved).
    pub texture: usize,
    /// Mirrored horizontally.
    pub flip: bool,
    /// Texture scroll offset (None = no scroll, clamped sampling as on screen).
    pub scroll: Option<[f32; 2]>,
    /// Draw alpha (see [`ShadowCaster::alpha`]).
    pub alpha: f32,
}

impl Renderer {
    /// The renderer's textures by id (for a light pass's alpha test).
    pub fn textures(&self) -> &[Texture] {
        &self.textures
    }

    /// Per texture id: no texel of the base level has alpha below 128.
    pub fn textures_opaque(&self) -> &[bool] {
        &self.tex_opaque
    }

    /// Changes on every texture change (add, replace, new level).
    pub fn texture_generation(&self) -> u64 {
        self.tex_gen
    }

    /// True if texture `id` (its current animation frame) has no transparent texel.
    pub(super) fn texture_opaque(&self, id: i32) -> bool {
        let (frame, _) = self.resolve_texture(id);
        frame >= 0
            && self
                .tex_opaque
                .get(frame as usize)
                .copied()
                .unwrap_or(false)
    }

    /// Records a shadow-only quad from `GridMap::shadow_quads` (closing the volume; never drawn).
    pub(super) fn record_shadow_only(&mut self, q: &Quad) {
        let sun_only = q.side == SIDE_SUN_ONLY;
        if q.texture != SHADOW_SOLID {
            let n = self.casters.len();
            self.record_quad(q);
            self.casters[n..]
                .iter_mut()
                .for_each(|c| c.sun_only = sun_only);
            return;
        }
        for idx in [[0, 1, 2], [0, 2, 3]] {
            self.casters.push(ShadowCaster {
                tri: idx.map(|i| q.corners[i]),
                uv: [[0.0; 2]; 3],
                texture: SOLID_CASTER,
                wrap: false,
                alpha: 1.0,
                sun_only,
            });
        }
    }

    /// Shadow alpha of a see-through draw: Some(alpha in (0, 1]) for BLEND_ALPHA while lighting and
    /// `shadow_stipple` are on; None for opaque draws, BLEND_ADD, alpha 0 or NaN.
    pub(super) fn stipple_alpha(&self, b: Option<Blend>) -> Option<f32> {
        let b = b?;
        let on = self.options.lighting && self.options.shadow_stipple && b.mode == BLEND_ALPHA;
        (on && b.alpha > 0.0).then(|| b.alpha.min(1.0))
    }

    /// Records a see-through quad as a stippled caster (`stipple_alpha`).
    pub(super) fn record_see_through(&mut self, q: &Quad, b: Option<Blend>) {
        let Some(alpha) = self.stipple_alpha(b) else {
            return;
        };
        let n = self.casters.len();
        self.record_quad(q);
        self.casters[n..].iter_mut().for_each(|c| c.alpha = alpha);
    }

    /// Records a see-through sprite as a stippled caster (unless SPRITE_NO_SHADOW); billboards turn
    /// towards each light as opaque ones do.
    pub(super) fn record_see_through_sprite(&mut self, s: &Sprite, q: &Quad, b: Option<Blend>) {
        let Some(alpha) = self.stipple_alpha(b) else {
            return;
        };
        if s.flags & SPRITE_NO_SHADOW != 0 || s.texture < 0 {
            return;
        }
        if s.flags & SPRITE_BILLBOARD != 0 {
            let (texture, scroll) = self.resolve_texture(s.texture);
            self.billboards.push(ShadowBillboard {
                base: s.position,
                width: s.width,
                height: s.height,
                texture: texture as usize,
                flip: s.flags & SPRITE_FLIP_X != 0,
                scroll,
                alpha,
            });
        } else {
            self.record_see_through(q, b);
        }
    }
}
