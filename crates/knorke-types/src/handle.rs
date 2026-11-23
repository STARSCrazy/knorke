use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::num::NonZeroU32;

/// Generation counter of a slot. Never zero, so `Option<Handle<T>>` is as large as `Handle<T>`.
pub type Generation = NonZeroU32;

/// A generation-checked runtime handle to a `T` living in some slot storage.
///
/// A handle is a *runtime* identity: it is only meaningful for the storage that issued it
/// and must not be persisted as an authoring identity (ADR 0001). A slot that is freed and
/// reused gets a new generation, so stale handles can be detected instead of aliasing.
///
/// The type parameter only prevents mixing handles of different kinds; it imposes no
/// bounds on `T` and does not affect `Send`/`Sync`.
pub struct Handle<T: ?Sized> {
    index: u32,
    generation: Generation,
    _kind: PhantomData<fn() -> T>,
}

impl<T: ?Sized> Handle<T> {
    /// Creates a handle for slot `index` at `generation`.
    #[inline]
    pub const fn new(index: u32, generation: Generation) -> Self {
        Self {
            index,
            generation,
            _kind: PhantomData,
        }
    }

    /// Slot index inside the issuing storage.
    #[inline]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Generation of the slot when this handle was issued.
    #[inline]
    pub const fn generation(self) -> Generation {
        self.generation
    }

    /// Packs the handle into 64 bits (generation high, index low), e.g. for C or WASM bindings.
    #[inline]
    pub const fn to_bits(self) -> u64 {
        ((self.generation.get() as u64) << 32) | self.index as u64
    }

    /// Inverse of [`Handle::to_bits`]. Returns `None` if the generation part is zero.
    #[inline]
    pub const fn from_bits(bits: u64) -> Option<Self> {
        match NonZeroU32::new((bits >> 32) as u32) {
            Some(generation) => Some(Self::new(bits as u32, generation)),
            None => None,
        }
    }

    /// Reinterprets the handle as a handle of another kind. Use only at explicit conversion
    /// boundaries (e.g. an integration adapter that knows both storages are the same).
    #[inline]
    pub const fn cast<U: ?Sized>(self) -> Handle<U> {
        Handle::new(self.index, self.generation)
    }
}

// Manual impls: derives would add `T: Trait` bounds that a marker type does not need.
impl<T: ?Sized> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: ?Sized> Copy for Handle<T> {}
impl<T: ?Sized> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}
impl<T: ?Sized> Eq for Handle<T> {}
impl<T: ?Sized> PartialOrd for Handle<T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T: ?Sized> Ord for Handle<T> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        (self.index, self.generation).cmp(&(other.index, other.generation))
    }
}
impl<T: ?Sized> Hash for Handle<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.to_bits().hash(state);
    }
}
impl<T: ?Sized> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle({}v{})", self.index, self.generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Npc;
    struct Light;

    fn gen_(n: u32) -> Generation {
        Generation::new(n).unwrap()
    }

    #[test]
    fn bits_round_trip() {
        let h = Handle::<Npc>::new(u32::MAX, gen_(u32::MAX));
        assert_eq!(Handle::<Npc>::from_bits(h.to_bits()), Some(h));
        let h = Handle::<Npc>::new(7, gen_(3));
        assert_eq!(h.to_bits(), (3u64 << 32) | 7);
        assert_eq!(Handle::<Npc>::from_bits(h.to_bits()), Some(h));
    }

    #[test]
    fn zero_generation_is_rejected() {
        assert_eq!(Handle::<Npc>::from_bits(42), None);
    }

    #[test]
    fn stale_generation_differs() {
        assert_ne!(
            Handle::<Npc>::new(1, gen_(1)),
            Handle::<Npc>::new(1, gen_(2))
        );
    }

    #[test]
    fn niche_keeps_option_small() {
        assert_eq!(size_of::<Option<Handle<Npc>>>(), size_of::<Handle<Npc>>());
        assert_eq!(size_of::<Handle<Npc>>(), 8);
    }

    #[test]
    fn cast_keeps_identity() {
        let h = Handle::<Npc>::new(5, gen_(9));
        let l: Handle<Light> = h.cast();
        assert_eq!((l.index(), l.generation()), (5, gen_(9)));
    }

    #[test]
    fn handle_is_send_sync_without_bounds_on_kind() {
        fn assert_send_sync<X: Send + Sync>() {}
        assert_send_sync::<Handle<*const u8>>();
    }
}
