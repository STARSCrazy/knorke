//! Dense bit set indexed by slot index: per-entity flags such as "dirty", "visible",
//! "selected", "in combat". 1 bit per entity, set iteration skips empty 64-bit words.

use alloc::vec::Vec;

/// A growable set of `usize` indices stored as bits.
///
/// Equality compares the indices, not the reserved capacity: a set that once held index
/// 500 equals a fresh one after that index is removed.
#[derive(Clone, Debug, Default)]
pub struct BitSet {
    words: Vec<u64>,
}

impl PartialEq for BitSet {
    fn eq(&self, other: &Self) -> bool {
        let n = self.words.len().min(other.words.len());
        self.words[..n] == other.words[..n]
            && self.words[n..].iter().all(|&w| w == 0)
            && other.words[n..].iter().all(|&w| w == 0)
    }
}
impl Eq for BitSet {}

impl BitSet {
    /// Empty set.
    pub const fn new() -> Self {
        Self { words: Vec::new() }
    }

    /// Empty set that holds indices below `bits` without reallocating.
    pub fn with_capacity(bits: usize) -> Self {
        Self {
            words: alloc::vec![0; bits.div_ceil(64)],
        }
    }

    #[inline]
    fn split(i: usize) -> (usize, u64) {
        (i / 64, 1u64 << (i % 64))
    }

    /// Adds `i`; returns `true` if it was not present.
    #[inline]
    pub fn insert(&mut self, i: usize) -> bool {
        let (w, m) = Self::split(i);
        if w >= self.words.len() {
            self.words.resize(w + 1, 0);
        }
        let was = self.words[w] & m != 0;
        self.words[w] |= m;
        !was
    }

    /// Removes `i`; returns `true` if it was present.
    #[inline]
    pub fn remove(&mut self, i: usize) -> bool {
        let (w, m) = Self::split(i);
        match self.words.get_mut(w) {
            Some(word) => {
                let was = *word & m != 0;
                *word &= !m;
                was
            }
            None => false,
        }
    }

    /// `true` if `i` is in the set.
    #[inline]
    pub fn contains(&self, i: usize) -> bool {
        let (w, m) = Self::split(i);
        self.words.get(w).is_some_and(|word| word & m != 0)
    }

    /// Number of indices in the set.
    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    /// `true` if the set is empty.
    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|&w| w == 0)
    }

    /// Removes all indices, keeping capacity.
    pub fn clear(&mut self) {
        self.words.fill(0);
    }

    /// Indices in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(wi, &word)| {
            let mut bits = word;
            core::iter::from_fn(move || {
                if bits == 0 {
                    return None;
                }
                let tz = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                Some(wi * 64 + tz)
            })
        })
    }

    /// Adds every index of `other`.
    pub fn union_with(&mut self, other: &BitSet) {
        if other.words.len() > self.words.len() {
            self.words.resize(other.words.len(), 0);
        }
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            *a |= b;
        }
    }

    /// Keeps only indices also in `other`.
    pub fn intersect_with(&mut self, other: &BitSet) {
        for (i, a) in self.words.iter_mut().enumerate() {
            *a &= other.words.get(i).copied().unwrap_or(0);
        }
    }

    /// The raw words (bit `i` is bit `i % 64` of word `i / 64`), e.g. for a GPU upload.
    pub fn words(&self) -> &[u64] {
        &self.words
    }

    pub(crate) fn words_vec_mut(&mut self) -> &mut Vec<u64> {
        &mut self.words
    }

    /// Mutable raw words, e.g. to let parallel chunks of 64·k indices write disjoint words.
    pub fn words_mut(&mut self) -> &mut [u64] {
        &mut self.words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_ops() {
        let mut s = BitSet::new();
        assert!(s.insert(3));
        assert!(!s.insert(3));
        assert!(s.insert(200));
        assert!(s.contains(200) && !s.contains(199) && !s.contains(10_000));
        assert_eq!(s.count(), 2);
        assert_eq!(s.iter().collect::<Vec<_>>(), [3, 200]);
        assert!(s.remove(3) && !s.remove(3) && !s.remove(9999));
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn set_algebra() {
        let mut a = BitSet::with_capacity(128);
        let mut b = BitSet::new();
        for i in [1, 64, 65, 127] {
            a.insert(i);
        }
        for i in [64, 127, 300] {
            b.insert(i);
        }
        let mut u = a.clone();
        u.union_with(&b);
        assert_eq!(u.iter().collect::<Vec<_>>(), [1, 64, 65, 127, 300]);
        a.intersect_with(&b);
        assert_eq!(a.iter().collect::<Vec<_>>(), [64, 127]);
        assert_eq!(a.words().len(), 2);
    }
}
