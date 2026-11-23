//! Texture access for the alpha test of shadow casters.
//!
//! Lighting does not own textures: the host (or `knorke-integration` for the renderer's
//! textures) implements [`CasterTextures`] over whatever it stores. Only the alpha channel of
//! the base level is read, nearest texel, exactly like the renderer samples on screen, so a
//! caster's shadow has the same holes as its picture.

/// The textures shadow casters refer to by id.
pub trait CasterTextures: Sync {
    /// Size `(width, height)` of texture `id` (both > 0); `None` for an unknown id (casters
    /// with an unknown texture are skipped).
    fn size(&self, id: usize) -> Option<(usize, usize)>;
    /// Alpha of the texel of texture `id` at `(u, v)`: nearest texel, `floor(u * width)`,
    /// repeating outside `[0, 1)` when `wrap`, clamped to the edge texel otherwise. Only called
    /// for ids that [`size`](Self::size) knows.
    fn alpha(&self, id: usize, u: f32, v: f32, wrap: bool) -> u8;
    /// `true` if no texel of `id` has alpha below 128: its casters skip the alpha test and
    /// their texture coordinates do not count for the shadow cache.
    fn opaque(&self, id: usize) -> bool;
    /// Changes whenever any texture changes; part of the shadow cache keys.
    fn generation(&self) -> u64;
}

/// One texture of a caster: the set and the id in it.
pub(crate) struct Tex<'a, T: ?Sized> {
    set: &'a T,
    id: usize,
    width: usize,
    height: usize,
}

impl<T: ?Sized> Clone for Tex<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: ?Sized> Copy for Tex<'_, T> {}

impl<'a, T: CasterTextures + ?Sized> Tex<'a, T> {
    /// Texture `id` of `set`, `None` for an unknown id.
    #[inline]
    pub(crate) fn get(set: &'a T, id: usize) -> Option<Self> {
        let (width, height) = set.size(id)?;
        Some(Tex {
            set,
            id,
            width,
            height,
        })
    }

    #[inline]
    pub(crate) fn width(&self) -> usize {
        self.width
    }

    #[inline]
    pub(crate) fn height(&self) -> usize {
        self.height
    }

    #[inline]
    pub(crate) fn alpha(&self, u: f32, v: f32, wrap: bool) -> u8 {
        self.set.alpha(self.id, u, v, wrap)
    }
}

/// A plain alpha texture for hosts that use lighting without the renderer (and for tests).
#[derive(Clone, Debug, PartialEq)]
pub struct AlphaTexture {
    /// Width in texels (> 0).
    pub width: usize,
    /// Height in texels (> 0).
    pub height: usize,
    /// One alpha value per texel, row by row.
    pub alpha: Vec<u8>,
}

impl AlphaTexture {
    /// A texture of `width * height` texels; `None` for a zero size or a wrong `alpha` length.
    pub fn new(width: usize, height: usize, alpha: Vec<u8>) -> Option<AlphaTexture> {
        (width > 0 && height > 0 && alpha.len() == width * height).then_some(AlphaTexture {
            width,
            height,
            alpha,
        })
    }

    /// Alpha at `(u, v)`, see [`CasterTextures::alpha`].
    pub fn sample(&self, u: f32, v: f32, wrap: bool) -> u8 {
        let (w, h) = (self.width as i64, self.height as i64);
        let x = (u * self.width as f32).floor() as i64;
        let y = (v * self.height as f32).floor() as i64;
        let (x, y) = if wrap {
            (x.rem_euclid(w), y.rem_euclid(h))
        } else {
            (x.clamp(0, w - 1), y.clamp(0, h - 1))
        };
        self.alpha[y as usize * self.width + x as usize]
    }
}

/// A slice of [`AlphaTexture`]s by index. The generation is always 0: a host that changes
/// textures between frames implements [`CasterTextures`] itself.
impl CasterTextures for [AlphaTexture] {
    fn size(&self, id: usize) -> Option<(usize, usize)> {
        self.get(id).map(|t| (t.width, t.height))
    }
    fn alpha(&self, id: usize, u: f32, v: f32, wrap: bool) -> u8 {
        self[id].sample(u, v, wrap)
    }
    fn opaque(&self, id: usize) -> bool {
        self.get(id)
            .is_some_and(|t| t.alpha.iter().all(|&a| a >= 128))
    }
    fn generation(&self) -> u64 {
        0
    }
}
