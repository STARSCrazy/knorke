//! Handle-indexed storage.
//!
//! [`Slots`] owns values and issues [`Handle`]s. [`HandleMap`] lets *another* module keep
//! its own per-object data keyed by those handles, in a dense array indexed by the slot
//! index instead of a hash map: a lookup is one bounds check plus one generation compare.

use crate::snapshot::{Reader, SnapshotError, SnapshotValue, Writer};
use crate::{Generation, Handle};
use alloc::vec::Vec;

const FIRST: Generation = Generation::MIN;

#[derive(Clone, Debug, PartialEq)]
enum Entry<T> {
    Occupied {
        generation: Generation,
        value: T,
    },
    Free {
        next_generation: Option<Generation>,
        next_free: Option<u32>,
    },
}

/// Owning storage that issues generation-checked handles.
///
/// Freed slots are reused with a new generation, so a stale handle never reaches a new
/// value. A slot whose generation would overflow is retired instead of wrapping.
#[derive(Clone, Debug, PartialEq)]
pub struct Slots<T> {
    entries: Vec<Entry<T>>,
    free_head: Option<u32>,
    len: usize,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Slots<T> {
    /// Empty storage.
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            free_head: None,
            len: 0,
        }
    }

    /// Empty storage with room for `capacity` values before reallocating.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            free_head: None,
            len: 0,
        }
    }

    /// Number of live values.
    pub fn len(&self) -> usize {
        self.len
    }

    /// `true` if there are no live values.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// One past the highest slot index ever used. A [`HandleMap`] keyed by these handles
    /// never needs more than this many entries.
    pub fn index_bound(&self) -> u32 {
        self.entries.len() as u32
    }

    /// Stores `value` and returns its handle.
    ///
    /// # Panics
    /// If more than `u32::MAX` slots would be needed.
    pub fn insert(&mut self, value: T) -> Handle<T> {
        self.len += 1;
        if let Some(index) = self.free_head {
            let entry = &mut self.entries[index as usize];
            let Entry::Free {
                next_generation: Some(generation),
                next_free,
            } = *entry
            else {
                unreachable!("free list points at an occupied or retired slot")
            };
            self.free_head = next_free;
            *entry = Entry::Occupied { generation, value };
            return Handle::new(index, generation);
        }
        let index = u32::try_from(self.entries.len()).expect("Slots: more than u32::MAX slots");
        self.entries.push(Entry::Occupied {
            generation: FIRST,
            value,
        });
        Handle::new(index, FIRST)
    }

    /// Removes and returns the value, or `None` if the handle is stale.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        let entry = self.entries.get_mut(handle.index() as usize)?;
        match entry {
            Entry::Occupied { generation, .. } if *generation == handle.generation() => {}
            _ => return None,
        }
        let next_generation = handle.generation().checked_add(1);
        let next_free = if next_generation.is_some() {
            self.free_head
        } else {
            None
        };
        let old = core::mem::replace(
            entry,
            Entry::Free {
                next_generation,
                next_free,
            },
        );
        if next_generation.is_some() {
            self.free_head = Some(handle.index());
        }
        self.len -= 1;
        match old {
            Entry::Occupied { value, .. } => Some(value),
            Entry::Free { .. } => unreachable!(),
        }
    }

    /// `true` if the handle refers to a live value.
    pub fn contains(&self, handle: Handle<T>) -> bool {
        self.get(handle).is_some()
    }

    /// The value, or `None` if the handle is stale.
    #[inline]
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        match self.entries.get(handle.index() as usize)? {
            Entry::Occupied { generation, value } if *generation == handle.generation() => {
                Some(value)
            }
            _ => None,
        }
    }

    /// The value, or `None` if the handle is stale.
    #[inline]
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        match self.entries.get_mut(handle.index() as usize)? {
            Entry::Occupied { generation, value } if *generation == handle.generation() => {
                Some(value)
            }
            _ => None,
        }
    }

    /// Live values with their handles, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e {
                Entry::Occupied { generation, value } => {
                    Some((Handle::new(i as u32, *generation), value))
                }
                Entry::Free { .. } => None,
            })
    }

    /// Live values with their handles, in slot order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle<T>, &mut T)> {
        self.entries
            .iter_mut()
            .enumerate()
            .filter_map(|(i, e)| match e {
                Entry::Occupied { generation, value } => {
                    Some((Handle::new(i as u32, *generation), value))
                }
                Entry::Free { .. } => None,
            })
    }
}

/// Per-object data of type `V`, keyed by handles that someone else issued for `K`.
///
/// Typical use: the host owns entities in a `Slots<Npc>`; the navigation module keeps its
/// path state in a `HandleMap<Npc, PathState>` without the host knowing about it.
pub struct HandleMap<K: ?Sized, V> {
    entries: Vec<Option<(Generation, V)>>,
    len: usize,
    _key: core::marker::PhantomData<fn() -> K>,
}

impl<K: ?Sized, V: Clone> Clone for HandleMap<K, V> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            len: self.len,
            _key: core::marker::PhantomData,
        }
    }
}

impl<K: ?Sized, V> Default for HandleMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: ?Sized, V> HandleMap<K, V> {
    /// Empty map.
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            len: 0,
            _key: core::marker::PhantomData,
        }
    }

    /// Empty map that can hold indices below `index_bound` without reallocating.
    pub fn with_index_bound(index_bound: u32) -> Self {
        let mut entries = Vec::with_capacity(index_bound as usize);
        entries.resize_with(index_bound as usize, || None);
        Self {
            entries,
            len: 0,
            _key: core::marker::PhantomData,
        }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.len
    }

    /// `true` if there are no entries.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Sets the value for `handle`. Replaces a value stored for an older generation of the
    /// same slot (that object is gone). Returns the value previously stored for exactly
    /// this handle.
    ///
    /// If the slot already holds a value for a **newer** generation, `handle` is stale
    /// (generations of a slot only grow): the map is left untouched and `value` is dropped,
    /// so a late write through a dead object's handle cannot destroy its successor's data.
    pub fn insert(&mut self, handle: Handle<K>, value: V) -> Option<V> {
        let i = handle.index() as usize;
        if i >= self.entries.len() {
            self.entries.resize_with(i + 1, || None);
        }
        let slot = &mut self.entries[i];
        match slot {
            Some((g, _)) if *g > handle.generation() => None,
            Some((g, v)) if *g == handle.generation() => Some(core::mem::replace(v, value)),
            Some(_) => {
                *slot = Some((handle.generation(), value));
                None
            }
            None => {
                *slot = Some((handle.generation(), value));
                self.len += 1;
                None
            }
        }
    }

    /// Removes the value for `handle`.
    pub fn remove(&mut self, handle: Handle<K>) -> Option<V> {
        let slot = self.entries.get_mut(handle.index() as usize)?;
        if slot
            .as_ref()
            .is_some_and(|(g, _)| *g == handle.generation())
        {
            self.len -= 1;
            slot.take().map(|(_, v)| v)
        } else {
            None
        }
    }

    /// The value for `handle`, or `None` if absent or stored for another generation.
    #[inline]
    pub fn get(&self, handle: Handle<K>) -> Option<&V> {
        match self.entries.get(handle.index() as usize)? {
            Some((g, v)) if *g == handle.generation() => Some(v),
            _ => None,
        }
    }

    /// The value for `handle`, or `None` if absent or stored for another generation.
    #[inline]
    pub fn get_mut(&mut self, handle: Handle<K>) -> Option<&mut V> {
        match self.entries.get_mut(handle.index() as usize)? {
            Some((g, v)) if *g == handle.generation() => Some(v),
            _ => None,
        }
    }

    /// Entries with their handles, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = (Handle<K>, &V)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.as_ref().map(|(g, v)| (Handle::new(i as u32, *g), v)))
    }

    /// Drops every entry whose handle `is_live` rejects, e.g. `|h| host.contains(h)`.
    pub fn retain_live(&mut self, mut is_live: impl FnMut(Handle<K>) -> bool) {
        for (i, slot) in self.entries.iter_mut().enumerate() {
            if let Some((g, _)) = slot
                && !is_live(Handle::new(i as u32, *g))
            {
                *slot = None;
                self.len -= 1;
            }
        }
    }
}

// ---- snapshot support (ADR 0010): exact internal state, so handles stay identical ----

impl<T: SnapshotValue> Slots<T> {
    pub(crate) fn snapshot_put(&self, w: &mut Writer<'_>) {
        w.put_len(self.entries.len());
        for e in &self.entries {
            match e {
                Entry::Occupied { generation, value } => {
                    w.put(&0u8);
                    w.put(generation);
                    w.put(value);
                }
                Entry::Free {
                    next_generation,
                    next_free,
                } => {
                    w.put(&1u8);
                    w.put(&next_generation.map_or(0, Generation::get));
                    w.put(next_free);
                }
            }
        }
        w.put(&self.free_head);
    }

    pub(crate) fn snapshot_get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        let n = r.get_len()?;
        self.entries.clear();
        self.entries.reserve(n.min(r.remaining()));
        let mut len = 0;
        for _ in 0..n {
            let e = match r.get::<u8>()? {
                0 => {
                    len += 1;
                    Entry::Occupied {
                        generation: r.get()?,
                        value: r.get()?,
                    }
                }
                1 => Entry::Free {
                    next_generation: Generation::new(r.get()?),
                    next_free: r.get()?,
                },
                _ => return Err(SnapshotError::Invalid("slot entry tag")),
            };
            self.entries.push(e);
        }
        self.free_head = r.get()?;
        self.len = len;
        // The free list must be a simple chain through exactly the reusable free slots:
        // otherwise a later `insert` would hit an occupied slot (cycle, wrong link) or a
        // freed slot would never be reused (not exact). Walking it costs no memory.
        let reusable = self
            .entries
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Entry::Free {
                        next_generation: Some(_),
                        ..
                    }
                )
            })
            .count();
        let retired_unlinked = self.entries.iter().all(|e| {
            !matches!(
                e,
                Entry::Free {
                    next_generation: None,
                    next_free: Some(_)
                }
            )
        });
        let mut walked = 0;
        let mut next = self.free_head;
        while let Some(i) = next {
            match self.entries.get(i as usize) {
                Some(Entry::Free {
                    next_generation: Some(_),
                    next_free,
                }) if walked < reusable => {
                    walked += 1;
                    next = *next_free;
                }
                _ => {
                    walked = usize::MAX;
                    break;
                }
            }
        }
        if walked == reusable && retired_unlinked {
            Ok(())
        } else {
            Err(SnapshotError::Invalid(
                "slot free list is not a chain through all reusable free slots",
            ))
        }
    }
}

impl<K: ?Sized, V: PartialEq> PartialEq for HandleMap<K, V> {
    fn eq(&self, o: &Self) -> bool {
        self.len == o.len && self.iter().eq(o.iter())
    }
}

impl<K: ?Sized, V: core::fmt::Debug> core::fmt::Debug for HandleMap<K, V> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<K: ?Sized, V: SnapshotValue> HandleMap<K, V> {
    pub(crate) fn snapshot_put(&self, w: &mut Writer<'_>) {
        w.put_len(self.entries.len());
        for e in &self.entries {
            w.put(e);
        }
    }

    pub(crate) fn snapshot_get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        r.get_into_vec(&mut self.entries)?;
        self.len = self.entries.iter().filter(|e| e.is_some()).count();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_remove() {
        let mut s = Slots::new();
        let a = s.insert("a");
        let b = s.insert("b");
        assert_eq!((s.get(a), s.get(b), s.len()), (Some(&"a"), Some(&"b"), 2));
        assert_eq!(s.remove(a), Some("a"));
        assert_eq!(s.get(a), None);
        assert_eq!(s.remove(a), None);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn reuse_bumps_generation() {
        let mut s = Slots::new();
        let a = s.insert(1);
        s.remove(a);
        let c = s.insert(2);
        assert_eq!(c.index(), a.index());
        assert_ne!(c.generation(), a.generation());
        assert_eq!((s.get(a), s.get(c)), (None, Some(&2)));
        assert_eq!(s.index_bound(), 1);
    }

    #[test]
    fn exhausted_generation_retires_slot() {
        let mut s: Slots<u8> = Slots::new();
        s.entries.push(Entry::Occupied {
            generation: Generation::MAX,
            value: 1,
        });
        s.len = 1;
        let h = Handle::new(0, Generation::MAX);
        assert_eq!(s.remove(h), Some(1));
        let n = s.insert(2);
        assert_eq!(n.index(), 1, "retired slot must not be reused");
    }

    #[test]
    fn iteration_skips_free_slots() {
        let mut s = Slots::new();
        let hs: Vec<_> = (0..4).map(|i| s.insert(i)).collect();
        s.remove(hs[1]);
        let got: Vec<_> = s.iter().map(|(h, v)| (h, *v)).collect();
        assert_eq!(got, [(hs[0], 0), (hs[2], 2), (hs[3], 3)]);
        for (_, v) in s.iter_mut() {
            *v *= 10;
        }
        assert_eq!(s.get(hs[3]), Some(&30));
    }

    fn slots_bytes(entries: &[(Option<u32>, Option<u32>)], free_head: Option<u32>) -> Vec<u8> {
        // Free entries only: (next_generation, next_free).
        let mut b = Vec::new();
        let mut w = Writer::new(&mut b);
        w.put_len(entries.len());
        for (next_generation, next_free) in entries {
            w.put(&1u8);
            w.put(&next_generation.unwrap_or(0));
            w.put(next_free);
        }
        w.put(&free_head);
        b
    }

    #[test]
    fn restore_rejects_corrupt_free_lists() {
        let get = |b: &[u8]| <Slots<u8> as SnapshotValue>::get(&mut Reader::new(b));
        // Valid: 1 -> 0 -> end.
        let mut ok = get(&slots_bytes(
            &[(Some(2), None), (Some(5), Some(0))],
            Some(1),
        ))
        .unwrap();
        assert_eq!(ok.insert(1).index(), 1);
        assert_eq!(ok.insert(2).index(), 0);
        assert_eq!(ok.insert(3).index(), 2);
        // Cycle: slot 0 points at itself. Used to restore fine and panic on the second insert.
        assert!(get(&slots_bytes(&[(Some(2), Some(0))], Some(0))).is_err());
        // Cycle of two.
        assert!(
            get(&slots_bytes(
                &[(Some(2), Some(1)), (Some(2), Some(0))],
                Some(0)
            ))
            .is_err()
        );
        // A reusable slot that is not on the list would be lost forever.
        assert!(get(&slots_bytes(&[(Some(2), None), (Some(2), None)], Some(0))).is_err());
        // A retired slot must not be linked.
        assert!(get(&slots_bytes(&[(None, None)], Some(0))).is_err());
        assert!(get(&slots_bytes(&[(None, Some(1)), (Some(2), None)], Some(1))).is_err());
        assert!(get(&slots_bytes(&[(None, None), (Some(2), None)], Some(1))).is_ok());
    }

    /// Deterministic pseudo-random operations against a simple reference model.
    #[test]
    fn slots_and_handle_map_match_a_reference_model() {
        use crate::Seed;
        for seed in 0..20u64 {
            let mut slots: Slots<u64> = Slots::new();
            let mut side: HandleMap<u64, u64> = HandleMap::new();
            let mut model: Vec<(Handle<u64>, u64, Option<u64>)> = Vec::new();
            let mut dead: Vec<Handle<u64>> = Vec::new();
            for step in 0..400u64 {
                let r = Seed(seed).derive(step).0;
                match r % 8 {
                    0..=2 => {
                        let h = slots.insert(r);
                        assert!(dead.iter().all(|d| *d != h), "handle reissued");
                        assert!(model.iter().all(|m| m.0 != h), "handle issued twice");
                        model.push((h, r, None));
                    }
                    3 | 4 if !model.is_empty() => {
                        let (h, v, _) = model.swap_remove((r >> 8) as usize % model.len());
                        assert_eq!(slots.remove(h), Some(v));
                        dead.push(h);
                    }
                    5 if !model.is_empty() => {
                        let i = (r >> 8) as usize % model.len();
                        let old = side.insert(model[i].0, r);
                        assert_eq!(old, model[i].2);
                        model[i].2 = Some(r);
                    }
                    6 if !dead.is_empty() => {
                        // Stale handles never reach or disturb anything.
                        let d = dead[(r >> 8) as usize % dead.len()];
                        assert_eq!(slots.get(d), None);
                        assert_eq!(slots.remove(d), None);
                        side.retain_live(|h| slots.contains(h));
                        assert_eq!(side.insert(d, 0), None);
                        side.retain_live(|h| slots.contains(h));
                    }
                    _ => {
                        // Snapshot round trip in the middle of the run.
                        let mut b = Vec::new();
                        slots.put(&mut Writer::new(&mut b));
                        side.put(&mut Writer::new(&mut b));
                        let mut r = Reader::new(&b);
                        let s2 = <Slots<u64> as SnapshotValue>::get(&mut r).unwrap();
                        let m2 = <HandleMap<u64, u64> as SnapshotValue>::get(&mut r).unwrap();
                        assert_eq!(r.remaining(), 0);
                        assert_eq!(s2, slots);
                        assert_eq!(m2, side);
                        let mut b2 = Vec::new();
                        s2.put(&mut Writer::new(&mut b2));
                        m2.put(&mut Writer::new(&mut b2));
                        assert_eq!(b2, b, "snapshot bytes are canonical");
                        slots = s2;
                        side = m2;
                    }
                }
                assert_eq!(slots.len(), model.len());
                for (h, v, s) in &model {
                    assert_eq!(slots.get(*h), Some(v));
                    assert_eq!(side.get(*h), s.as_ref());
                }
                assert_eq!(slots.iter().count(), model.len());
                assert_eq!(side.len(), side.iter().count(), "len matches the entries");
            }
        }
    }

    #[test]
    fn stale_handle_does_not_clobber_the_successor() {
        struct Npc;
        let mut host: Slots<Npc> = Slots::new();
        let mut nav: HandleMap<Npc, u32> = HandleMap::new();
        let old = host.insert(Npc);
        host.remove(old);
        let new = host.insert(Npc);
        nav.insert(new, 7);
        // A late write through the dead object's handle must not destroy the live one's data.
        assert_eq!(nav.insert(old, 99), None);
        assert_eq!((nav.get(new), nav.get(old), nav.len()), (Some(&7), None, 1));
        let copy = nav.clone();
        assert_eq!(copy, nav);
    }

    #[test]
    fn handle_map_follows_generations() {
        struct Npc;
        let mut host: Slots<Npc> = Slots::new();
        let mut nav: HandleMap<Npc, u32> = HandleMap::new();
        let a = host.insert(Npc);
        assert_eq!(nav.insert(a, 7), None);
        assert_eq!(nav.insert(a, 8), Some(7));
        host.remove(a);
        let b = host.insert(Npc); // same slot, new generation
        assert_eq!(nav.get(b), None);
        assert_eq!(nav.get(a), Some(&8));
        nav.retain_live(|h| host.contains(h));
        assert_eq!((nav.get(a), nav.len()), (None, 0));
        nav.insert(b, 1);
        assert_eq!(nav.iter().collect::<Vec<_>>(), [(b, &1)]);
        assert_eq!(nav.remove(b), Some(1));
    }
}
