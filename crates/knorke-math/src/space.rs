use crate::{Affine2, Vec2};
use core::fmt;
use core::marker::PhantomData;

/// A coordinate space. Implemented by zero-sized marker types; modules may define their
/// own (e.g. a hex-axial space in navigation).
pub trait Space: Copy + Default + Send + Sync + 'static {
    /// Human-readable name for diagnostics.
    const NAME: &'static str;
}

macro_rules! space {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name;
        impl Space for $name {
            const NAME: &'static str = stringify!($name);
        }
    };
}

space!(
    /// 2D world units of the game (gameplay, physics, navigation in continuous space).
    World2
);
space!(
    /// Render-target pixels, origin top-left, y down.
    Screen
);
space!(
    /// Integer tile/grid cells of a square map.
    Tile
);

/// A value `V` (usually a vector) expressed in space `S`.
///
/// `#[repr(transparent)]`: same layout as `V`. Slices convert in bulk without copying via
/// [`At::slice_raw`] / [`At::slice_from_raw`], so typing positions by space costs nothing
/// at module boundaries.
#[repr(transparent)]
pub struct At<S, V> {
    /// The raw value. Public so code that knows what it does can reach it; prefer
    /// [`At::raw`] to make the space crossing visible.
    pub value: V,
    _space: PhantomData<S>,
}

impl<S: Space, V> At<S, V> {
    /// Tags `value` as being in space `S`.
    #[inline]
    pub const fn new(value: V) -> Self {
        Self {
            value,
            _space: PhantomData,
        }
    }

    /// The raw value, dropping the space tag.
    #[inline]
    pub fn raw(self) -> V {
        self.value
    }

    /// Views a slice of tagged values as raw values (no copy).
    #[inline]
    pub fn slice_raw(s: &[Self]) -> &[V]
    where
        V: bytemuck::Pod,
    {
        bytemuck::cast_slice(s)
    }

    /// Views a slice of raw values as values in space `S` (no copy).
    #[inline]
    pub fn slice_from_raw(s: &[V]) -> &[Self]
    where
        V: bytemuck::Pod,
    {
        bytemuck::cast_slice(s)
    }

    /// Mutable variant of [`At::slice_from_raw`].
    #[inline]
    pub fn slice_from_raw_mut(s: &mut [V]) -> &mut [Self]
    where
        V: bytemuck::Pod,
    {
        bytemuck::cast_slice_mut(s)
    }
}

impl<S, V: Clone> Clone for At<S, V> {
    fn clone(&self) -> Self {
        Self {
            value: self.value.clone(),
            _space: PhantomData,
        }
    }
}
impl<S, V: Copy> Copy for At<S, V> {}
impl<S, V: PartialEq> PartialEq for At<S, V> {
    fn eq(&self, o: &Self) -> bool {
        self.value == o.value
    }
}
impl<S, V: Eq> Eq for At<S, V> {}
impl<S, V: core::hash::Hash> core::hash::Hash for At<S, V> {
    fn hash<H: core::hash::Hasher>(&self, h: &mut H) {
        self.value.hash(h);
    }
}
impl<S, V: Default> Default for At<S, V> {
    fn default() -> Self {
        Self {
            value: V::default(),
            _space: PhantomData,
        }
    }
}
impl<S: Space, V: fmt::Debug> fmt::Debug for At<S, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}@{}", self.value, S::NAME)
    }
}

/// A 2D transform from space `A` to space `B`, e.g. `Transform2<World2, Screen>` for a
/// camera. Composition checks the spaces at compile time.
#[repr(transparent)]
pub struct Transform2<A, B> {
    /// The underlying affine transform.
    pub affine: Affine2,
    _spaces: PhantomData<(A, B)>,
}

impl<A: Space, B: Space> Transform2<A, B> {
    /// Declares that `affine` maps `A` coordinates to `B` coordinates.
    #[inline]
    pub const fn new(affine: Affine2) -> Self {
        Self {
            affine,
            _spaces: PhantomData,
        }
    }

    /// Maps a point.
    #[inline]
    pub fn point(&self, p: At<A, Vec2>) -> At<B, Vec2> {
        At::new(self.affine.transform_point(p.value))
    }

    /// Maps a direction.
    #[inline]
    pub fn vector(&self, v: At<A, Vec2>) -> At<B, Vec2> {
        At::new(self.affine.transform_vector(v.value))
    }

    /// `B → A`, or `None` if singular.
    pub fn inverse(&self) -> Option<Transform2<B, A>> {
        self.affine.inverse().map(Transform2::new)
    }

    /// First `self` (`A → B`), then `next` (`B → C`).
    #[inline]
    pub fn then<C: Space>(&self, next: &Transform2<B, C>) -> Transform2<A, C> {
        Transform2::new(next.affine * self.affine)
    }
}

impl<A, B> Clone for Transform2<A, B> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<A, B> Copy for Transform2<A, B> {}
impl<A: Space, B: Space> fmt::Debug for Transform2<A, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Transform2<{}→{}>({:?})", A::NAME, B::NAME, self.affine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_are_free_and_castable() {
        assert_eq!(size_of::<At<World2, Vec2>>(), size_of::<Vec2>());
        let raw = [Vec2::new(1.0, 2.0), Vec2::new(3.0, 4.0)];
        let typed: &[At<World2, Vec2>] = At::slice_from_raw(&raw);
        assert!(core::ptr::eq(typed.as_ptr().cast::<Vec2>(), raw.as_ptr()));
        assert_eq!(At::slice_raw(typed), &raw);
        assert_eq!(format!("{:?}", typed[0]), "Vec2 { x: 1.0, y: 2.0 }@World2");
    }

    #[test]
    fn typed_transforms_compose() {
        let camera: Transform2<World2, Screen> =
            Transform2::new(Affine2::from_translation(Vec2::new(100.0, 0.0)));
        let zoom: Transform2<Screen, Screen> =
            Transform2::new(Affine2::from_scale(Vec2::splat(2.0)));
        let both = camera.then(&zoom);
        let s = both.point(At::new(Vec2::new(1.0, 1.0)));
        assert_eq!(s.raw(), Vec2::new(202.0, 2.0));
        let w = both.inverse().unwrap().point(s);
        assert!(w.raw().abs_diff_eq(Vec2::new(1.0, 1.0), 1e-5));
    }
}
