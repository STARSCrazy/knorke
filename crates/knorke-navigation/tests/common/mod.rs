//! Helpers shared by the integration tests; each test binary uses a subset.
#![allow(dead_code)] // every test binary compiles this module and uses only part of it

use knorke_navigation::grid::{Cell, DenseGrid};
use knorke_navigation::{Cost, Graph};

/// Small deterministic random numbers for test maps.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    pub(crate) fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

pub(crate) fn random_grid(
    rng: &mut Rng,
    w: i32,
    h: i32,
    wall_percent: u32,
    weighted: bool,
) -> DenseGrid {
    let mut g = DenseGrid::new(w, h);
    for y in 0..h {
        for x in 0..w {
            if rng.below(100) < wall_percent {
                g.set(x, y, DenseGrid::BLOCKED);
            } else if weighted {
                g.set(x, y, rng.below(4) * 7);
            }
        }
    }
    g
}

/// Checks that `path` starts at `start`, each step is a move the graph offers,
/// and the step costs add up to `cost`.
pub(crate) fn assert_valid_path<G: Graph<Node = Cell>>(
    graph: &mut G,
    path: &[Cell],
    start: Cell,
    cost: u64,
) {
    assert_eq!(path.first(), Some(&start));
    let mut sum = 0u64;
    let mut buf = Vec::new();
    for pair in path.windows(2) {
        buf.clear();
        graph.neighbors(pair[0], &mut buf);
        let step: Cost = buf
            .iter()
            .filter(|(n, _)| *n == pair[1])
            .map(|(_, c)| *c)
            .min()
            .unwrap_or_else(|| panic!("illegal step {:?} -> {:?}", pair[0], pair[1]));
        sum += u64::from(step);
    }
    assert_eq!(sum, cost, "step costs do not add up");
}
