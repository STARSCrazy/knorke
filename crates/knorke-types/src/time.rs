use core::fmt;

/// Index of a fixed simulation tick.
///
/// Knorke modules never run their own clocks (plan section 4.1): the host advances
/// simulation time and hands the current tick to every system that needs it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(pub u64);

impl Tick {
    /// The first tick of a simulation.
    pub const ZERO: Tick = Tick(0);

    /// The following tick. Panics on overflow in debug builds, like integer arithmetic.
    #[inline]
    #[must_use]
    pub const fn next(self) -> Tick {
        Tick(self.0 + 1)
    }

    /// Number of ticks from `earlier` to `self`, or `None` if `earlier` is later.
    #[inline]
    pub const fn since(self, earlier: Tick) -> Option<u64> {
        self.0.checked_sub(earlier.0)
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t{}", self.0)
    }
}

/// Seed for deterministic pseudo-random decisions.
///
/// Systems that promise determinism take a `Seed` explicitly instead of reading a global
/// random source. Derive independent streams with [`Seed::derive`] rather than reusing one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Seed(pub u64);

impl Seed {
    /// Derives an independent seed for the sub-stream `stream` (e.g. one per entity or system).
    ///
    /// Two rounds of the SplitMix64 mixer: the seed is mixed before the stream number
    /// enters, so seed and stream do not cancel each other out (with a single round,
    /// `Seed(a).derive(s)` and `Seed(a ^ k).derive(s')` collide for a fixed, simple `k`).
    /// Neighbouring stream numbers give unrelated seeds.
    ///
    /// The mapping is part of the determinism contract: changing it invalidates every
    /// recorded replay.
    #[inline]
    #[must_use]
    pub const fn derive(self, stream: u64) -> Seed {
        Seed(mix(mix(self.0) ^ stream))
    }
}

/// SplitMix64 step: add the golden-ratio increment, then the finaliser. A bijection on
/// `u64`. Same function as `sagax_core::mix` (sagax fab00c0, crates/sagax-core/src/turn.rs:70).
#[inline]
const fn mix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_arithmetic() {
        assert_eq!(Tick::ZERO.next().next(), Tick(2));
        assert_eq!(Tick(10).since(Tick(4)), Some(6));
        assert_eq!(Tick(4).since(Tick(10)), None);
    }

    #[test]
    fn derive_is_stable() {
        // Frozen values: changing them breaks recorded replays.
        assert_eq!(Seed(0).derive(0), Seed(FROZEN_0_0));
        assert_eq!(Seed(42).derive(1), Seed(FROZEN_42_1));
        assert_eq!(Seed(42).derive(1), Seed(42).derive(1));
    }

    #[test]
    fn mixer_is_the_sagax_mixer() {
        // Reference values of SplitMix64 with state 0: first outputs of the generator.
        assert_eq!(mix(0), 0xE220_A839_7B1D_CDAF);
        assert_eq!(Seed(7).derive(3).0, mix(mix(7) ^ 3));
    }

    #[test]
    fn derive_separates_streams() {
        let s = Seed(1234);
        assert_ne!(s.derive(0), s.derive(1));
        assert_ne!(s.derive(1), Seed(1235).derive(1));
    }

    #[test]
    fn seed_and_stream_do_not_cancel() {
        const PHI: u64 = 0x9E37_79B9_7F4A_7C15;
        // The collisions of the former single-round mapping `mix(seed ^ stream * PHI)`.
        assert_ne!(Seed(PHI).derive(0), Seed(0).derive(1));
        assert_ne!(Seed(0).derive(2), Seed(PHI).derive(3));
        // Weyl-sequence seeds (n * PHI) against plain stream numbers, and small grids.
        let mut seen = alloc::vec::Vec::new();
        for seed in 0..64u64 {
            for stream in 0..64u64 {
                seen.push(Seed(seed).derive(stream).0);
                seen.push(Seed(seed.wrapping_mul(PHI)).derive(stream).0);
            }
        }
        seen.sort_unstable();
        let unique = seen.windows(2).filter(|w| w[0] != w[1]).count() + 1;
        // Seed 0 appears in both families: 64 duplicates expected, nothing else.
        assert_eq!(unique, seen.len() - 64);
    }

    const FROZEN_0_0: u64 = 0xA706_DD2F_4D19_7E6F;
    const FROZEN_42_1: u64 = 0xF34F_E924_8C93_42E5;
}
