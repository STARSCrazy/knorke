//! Fast, deterministic hashing (ADR 0003).
//!
//! [`StableHasher`] is the FxHash function used inside rustc and Firefox: very fast for
//! small keys (integers, handles), identical hash values on every run and platform. It is
//! the basis of [`Checksum`](crate::Checksum). **Not** resistant to adversarial keys – do
//! not use it for data from the network.
//!
//! **Simulation code does not use hash maps at all** (`clippy.toml` forbids `HashMap` and
//! `HashSet` in the workspace): `std`'s default hasher is randomly seeded per process, and
//! even with a fixed hasher the iteration order depends on the table layout, which differs
//! between targets (SIMD width) and `std` versions. Use [`HandleMap`](crate::HandleMap) for
//! per-entity data, `BTreeMap`/`BTreeSet` or a sorted `Vec` otherwise.
//!
//! [`StableState`] remains for host tools and presentation code that want a fast map whose
//! order is at least reproducible from run to run of the same build:
//!
//! ```
//! use std::collections::HashMap;
//! use knorke_types::StableState;
//! let mut m: HashMap<u32, &str, StableState> = HashMap::default();
//! m.insert(7, "seven");
//! ```

use core::hash::{BuildHasherDefault, Hasher};

const K: u64 = 0x517c_c1b7_2722_0a95;

/// FxHash, deterministic across runs and platforms (`usize` is hashed as `u64`).
#[derive(Clone, Copy, Debug, Default)]
pub struct StableHasher {
    hash: u64,
}

impl StableHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(K);
    }
}

impl Hasher for StableHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for c in chunks {
            self.add(u64::from_le_bytes(*c));
        }
        if !rest.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(buf) ^ ((rest.len() as u64) << 56));
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

/// `BuildHasher` for [`StableHasher`]: `HashMap<K, V, StableState>` in host tools and
/// presentation code. Not for simulation state (see the module documentation).
pub type StableState = BuildHasherDefault<StableHasher>;

#[cfg(test)]
mod tests {
    use super::*;
    use core::hash::{BuildHasher, Hash};

    fn h(v: impl Hash) -> u64 {
        StableState::default().hash_one(v)
    }

    #[test]
    fn frozen_values() {
        // Changing these changes every `Checksum` and therefore every recorded replay.
        assert_eq!(h(0u64), 0);
        assert_eq!(h(1u32), K);
        assert_eq!(h(1usize), h(1u64));
    }

    #[test]
    fn distinguishes_inputs() {
        assert_ne!(h(1u32), h(2u32));
        assert_ne!(h("ab"), h("ba"));
        assert_ne!(h([0u8; 3].as_slice()), h([0u8; 4].as_slice()));
    }
}
