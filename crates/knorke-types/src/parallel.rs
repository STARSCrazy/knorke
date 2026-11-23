//! Parallel execution as a contract (ADR 0008).
//!
//! Modules never create threads or thread pools. A module that has parallel work takes
//! `&impl Parallel` and splits its data into **chunks of a fixed length** it chooses. The
//! host passes an executor: [`Serial`] here, or a thread/rayon executor from `knorke-exec`.
//!
//! Determinism: chunk boundaries depend only on the data length and `chunk_len`, never on
//! the number of threads. Per-chunk results (command buffers, event lists, partial sums)
//! merged in chunk index order therefore give the same result on 1 or 32 cores.

use alloc::vec::Vec;
use core::ops::Range;

/// Number of chunks `len` items split into with chunks of `chunk_len` (last one shorter).
#[inline]
pub fn chunk_count(len: usize, chunk_len: usize) -> usize {
    assert!(chunk_len > 0, "chunk_len must be > 0");
    len.div_ceil(chunk_len)
}

/// Runs closures over chunks of data, possibly concurrently.
///
/// Implementors provide [`for_each_chunk_mut`](Self::for_each_chunk_mut) and
/// [`join`](Self::join); everything else is derived. All methods return only after all
/// work is done (fork-join), so borrowed data can be used freely.
pub trait Parallel: Sync {
    /// Upper bound of useful concurrency (1 for [`Serial`]). Use it to size per-chunk state,
    /// never to choose chunk boundaries (that would break determinism).
    fn threads(&self) -> usize;

    /// Calls `f(chunk_index, chunk)` for each chunk of `items`, possibly concurrently.
    fn for_each_chunk_mut<T: Send>(
        &self,
        items: &mut [T],
        chunk_len: usize,
        f: impl Fn(usize, &mut [T]) + Sync,
    );

    /// Runs `a` and `b`, possibly concurrently, and returns both results.
    fn join<RA: Send, RB: Send>(
        &self,
        a: impl FnOnce() -> RA + Send,
        b: impl FnOnce() -> RB + Send,
    ) -> (RA, RB);

    /// Calls `f(chunk_index, index_range)` for each chunk of `0..len`, possibly concurrently.
    ///
    /// The variant for inputs that are not slices: a module that reads `impl Column<T>`
    /// (e.g. a [`project`](crate::project)ion into host structs) chunks the index range
    /// and calls `column.get(i)` inside.
    fn for_each_range(&self, len: usize, chunk_len: usize, f: impl Fn(usize, Range<usize>) + Sync) {
        let n = chunk_count(len, chunk_len);
        // A `Vec` of zero-sized values never allocates; it only provides `n` disjoint slots.
        let mut slots: Vec<()> = alloc::vec![(); n];
        self.for_each_chunk_mut(&mut slots, 1, |i, _| f(i, range(len, chunk_len, i)));
    }

    /// Like [`for_each_range`](Self::for_each_range) with one state per chunk:
    /// `f(chunk_index, index_range, &mut states[chunk_index])`.
    ///
    /// # Panics
    /// If `states` has fewer elements than there are chunks.
    fn for_each_range_with<S: Send>(
        &self,
        len: usize,
        chunk_len: usize,
        states: &mut [S],
        f: impl Fn(usize, Range<usize>, &mut S) + Sync,
    ) {
        let n = chunk_count(len, chunk_len);
        assert!(
            states.len() >= n,
            "Parallel: {} states for {n} chunks",
            states.len()
        );
        self.for_each_chunk_mut(&mut states[..n], 1, |i, s| {
            f(i, range(len, chunk_len, i), &mut s[0])
        });
    }

    /// Calls `f(chunk_index, chunk)` for each chunk of `items`, possibly concurrently.
    fn for_each_chunk<T: Sync>(
        &self,
        items: &[T],
        chunk_len: usize,
        f: impl Fn(usize, &[T]) + Sync,
    ) {
        self.for_each_range(items.len(), chunk_len, |i, r| f(i, &items[r]));
    }

    /// Calls `f(chunk_index, chunk, &mut states[chunk_index])`, possibly concurrently: every
    /// chunk gets its own state (command buffer, output list, accumulator). Merge `states`
    /// in index order afterwards for a deterministic result.
    ///
    /// # Panics
    /// If `states` has fewer elements than there are chunks.
    fn for_each_chunk_with<T: Sync, S: Send>(
        &self,
        items: &[T],
        chunk_len: usize,
        states: &mut [S],
        f: impl Fn(usize, &[T], &mut S) + Sync,
    ) {
        self.for_each_range_with(items.len(), chunk_len, states, |i, r, s| f(i, &items[r], s));
    }
}

/// Index range of chunk `i`. `i * chunk_len < len` for every chunk index, so no overflow.
#[inline]
fn range(len: usize, chunk_len: usize, i: usize) -> Range<usize> {
    let start = i * chunk_len;
    start..len.min(start.saturating_add(chunk_len))
}

impl<P: Parallel + ?Sized> Parallel for &P {
    fn threads(&self) -> usize {
        (**self).threads()
    }
    fn for_each_chunk_mut<T: Send>(
        &self,
        items: &mut [T],
        chunk_len: usize,
        f: impl Fn(usize, &mut [T]) + Sync,
    ) {
        (**self).for_each_chunk_mut(items, chunk_len, f);
    }
    fn join<RA: Send, RB: Send>(
        &self,
        a: impl FnOnce() -> RA + Send,
        b: impl FnOnce() -> RB + Send,
    ) -> (RA, RB) {
        (**self).join(a, b)
    }
}

/// Runs everything on the calling thread, in chunk order. Zero overhead; the reference
/// for determinism tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct Serial;

impl Parallel for Serial {
    fn threads(&self) -> usize {
        1
    }

    #[inline]
    fn for_each_chunk_mut<T: Send>(
        &self,
        items: &mut [T],
        chunk_len: usize,
        f: impl Fn(usize, &mut [T]) + Sync,
    ) {
        assert!(chunk_len > 0, "chunk_len must be > 0");
        for (i, c) in items.chunks_mut(chunk_len).enumerate() {
            f(i, c);
        }
    }

    #[inline]
    fn join<RA: Send, RB: Send>(
        &self,
        a: impl FnOnce() -> RA + Send,
        b: impl FnOnce() -> RB + Send,
    ) -> (RA, RB) {
        (a(), b())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommandBuffer;

    #[test]
    fn serial_chunks() {
        let mut v: Vec<u32> = (0..10).collect();
        Serial.for_each_chunk_mut(&mut v, 4, |i, c| {
            c.iter_mut().for_each(|x| *x += 100 * i as u32)
        });
        assert_eq!(v, [0, 1, 2, 3, 104, 105, 106, 107, 208, 209]);
        let mut sums = [0u32; 3];
        Serial.for_each_chunk_with(&v, 4, &mut sums, |_, c, s| *s = c.iter().sum());
        assert_eq!(sums, [6, 422, 417]);
        assert_eq!(chunk_count(0, 4), 0);
        assert_eq!(chunk_count(8, 4), 2);
        let (a, b) = Serial.join(|| 1, || "b");
        assert_eq!((a, b), (1, "b"));
    }

    #[test]
    fn per_chunk_command_buffers() {
        let items: Vec<u32> = (0..10).collect();
        let mut buffers: Vec<CommandBuffer<u32>> = (0..chunk_count(items.len(), 3))
            .map(|_| CommandBuffer::new())
            .collect();
        Serial.for_each_chunk_with(&items, 3, &mut buffers, |_, c, b| {
            c.iter().filter(|x| *x % 2 == 0).for_each(|&x| b.push(x));
        });
        let mut out = Vec::new();
        CommandBuffer::apply_all(&mut buffers, |c| out.push(c));
        assert_eq!(out, [0, 2, 4, 6, 8]);
    }

    #[test]
    fn ranges_cover_columns_that_are_not_slices() {
        use crate::{Column, project};
        struct Npc {
            hp: u32,
        }
        let npcs: Vec<Npc> = (0..10).map(|hp| Npc { hp }).collect();
        let hp = project(&npcs, |n| &n.hp);
        let mut sums = [0u32; 4];
        Serial.for_each_range_with(hp.len(), 3, &mut sums, |ci, r, s| {
            assert_eq!(r.start, ci * 3);
            *s = r.map(|i| *hp.get(i).unwrap()).sum();
        });
        assert_eq!(sums, [3, 12, 21, 9]);
        let seen = core::sync::atomic::AtomicU32::new(0);
        Serial.for_each_range(10, 4, |_, r| {
            seen.fetch_add(r.len() as u32, core::sync::atomic::Ordering::Relaxed);
        });
        assert_eq!(seen.into_inner(), 10);
        Serial.for_each_range(0, 4, |_, _| panic!("no chunks expected"));
        // Chunk length near `usize::MAX` must not overflow the range end.
        Serial.for_each_range(5, usize::MAX, |i, r| assert_eq!((i, r), (0, 0..5)));
    }

    #[test]
    #[should_panic(expected = "states for")]
    fn too_few_states_panics() {
        Serial.for_each_chunk_with(&[1, 2, 3], 1, &mut [0u8; 2], |_, _, _| {});
    }
}
