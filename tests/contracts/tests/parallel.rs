//! Contract: parallel module work is deterministic and executor-independent (ADR 0008).
//!
//! A navigation-like system steers many agents in parallel chunks. Per chunk it writes
//! velocities into a host buffer, emits events and records spawn commands into its own
//! buffers. Serial, scoped threads and rayon must produce bit-identical results.

use knorke_contract_tests::steering::{Agent, Arrived, CHUNK, ChunkOut, Spawn, agents, steer};
use knorke_exec::{Rayon, ScopedThreads};
use knorke_math::Vec2;
use knorke_types::{BitSet, EventQueue, Parallel, Serial, StableState, Tick, chunk_count};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;

type Frame = (Vec<Vec2>, Vec<Spawn>, Vec<Arrived>);

fn run_frame(par: &impl Parallel, agents: &[Agent]) -> Frame {
    let mut velocities = vec![Vec2::ZERO; agents.len()];
    let mut outs: Vec<ChunkOut> = (0..chunk_count(agents.len(), CHUNK))
        .map(|_| ChunkOut::default())
        .collect();
    steer(par, agents, &mut velocities, &mut outs);
    let mut spawns = Vec::new();
    let mut queue = EventQueue::new();
    let mut cursor = queue.cursor_at_end();
    for o in &mut outs {
        o.spawns.apply(|s| spawns.push(s));
        queue.append(Tick(1), &mut o.arrived);
    }
    let arrived = queue.read(&mut cursor).since().unwrap().to_vec();
    (velocities, spawns, arrived)
}

#[test]
fn executors_are_bit_identical() {
    let agents = agents(50_000);
    let reference = run_frame(&Serial, &agents);
    assert!(!reference.1.is_empty(), "scenario should produce commands");
    let executors: [(&str, &dyn Fn() -> Frame); 4] = [
        ("scoped-1", &|| {
            run_frame(&ScopedThreads::new(NonZeroUsize::MIN), &agents)
        }),
        ("scoped-7", &|| {
            run_frame(&ScopedThreads::new(NonZeroUsize::new(7).unwrap()), &agents)
        }),
        ("rayon-global", &|| run_frame(&Rayon::global(), &agents)),
        ("rayon-3", &|| {
            run_frame(&Rayon::with_threads(3).unwrap(), &agents)
        }),
    ];
    for (name, run) in executors {
        let got = run();
        let same_bits = got
            .0
            .iter()
            .zip(&reference.0)
            .all(|(a, b)| a.x.to_bits() == b.x.to_bits() && a.y.to_bits() == b.y.to_bits());
        assert!(same_bits, "{name}: velocities differ");
        assert_eq!(got.1, reference.1, "{name}: commands differ");
        assert_eq!(got.2, reference.2, "{name}: events differ");
    }
}

#[test]
fn join_runs_both_sides_on_every_executor() {
    fn check(p: &impl Parallel) {
        let data: Vec<u64> = (0..10_000).collect();
        let (a, b) = p.join(
            || data[..5000].iter().sum::<u64>(),
            || data[5000..].iter().sum::<u64>(),
        );
        assert_eq!(a + b, data.iter().sum());
    }
    check(&Serial);
    check(&ScopedThreads::available());
    check(&Rayon::global());
}

#[test]
fn edge_cases() {
    fn check(p: &impl Parallel) {
        let mut empty: [u8; 0] = [];
        p.for_each_chunk_mut(&mut empty, 4, |_, _| panic!("no chunks expected"));
        let mut one = [1u8; 3];
        p.for_each_chunk_mut(&mut one, 100, |i, c| {
            assert_eq!((i, c.len()), (0, 3));
            c[0] = 9;
        });
        assert_eq!(one, [9, 1, 1]);
        let mut seen = [0u8; 5];
        let items = [1u8, 2, 3, 4, 5];
        p.for_each_chunk_with(&items, 1, &mut seen, |i, c, s| *s = c[0] + i as u8);
        assert_eq!(seen, [1, 3, 5, 7, 9]);
        // Index ranges for inputs that are not slices (ADR 0006 columns).
        let mut sums = [0usize; 3];
        p.for_each_range_with(10, 4, &mut sums, |_, r, s| *s = r.sum());
        assert_eq!(sums, [6, 22, 17]);
        p.for_each_range(0, 4, |_, _| panic!("no chunks expected"));
    }
    check(&Serial);
    check(&ScopedThreads::new(NonZeroUsize::new(4).unwrap()));
    check(&Rayon::with_threads(2).unwrap());
}

/// Simulation code keeps keyed data in ordered containers: same content, same order,
/// whatever the insertion order, platform or Rust version (ADR 0003).
#[test]
fn ordered_maps_iterate_identically_for_any_insertion_order() {
    let keys: Vec<u32> = (0..1000u32)
        .map(|i| i.wrapping_mul(2_654_435_761))
        .collect();
    let forward: BTreeMap<u32, u32> = keys.iter().map(|&k| (k, k ^ 1)).collect();
    let backward: BTreeMap<u32, u32> = keys.iter().rev().map(|&k| (k, k ^ 1)).collect();
    assert!(forward.iter().eq(backward.iter()));
    assert!(forward.keys().is_sorted());
}

/// Host tools may use a hash map with the fixed hasher: reproducible from run to run of
/// the same build, which is all `StableState` promises.
#[test]
#[allow(clippy::disallowed_types)] // host-side use, not simulation state
fn stable_hash_map_iteration_is_reproducible_within_a_build() {
    use std::collections::HashMap;
    let build = || {
        let mut m: HashMap<u32, u32, StableState> = HashMap::default();
        for i in 0..1000u32 {
            m.insert(i.wrapping_mul(2_654_435_761), i);
        }
        m.into_values().take(20).collect::<Vec<_>>()
    };
    assert_eq!(build(), build());
}

#[test]
fn bitset_words_split_across_chunks() {
    // Parallel chunks of 64·k indices write disjoint words: no locks, no atomics.
    let agents = agents(10_000);
    let mut near = BitSet::with_capacity(agents.len());
    Rayon::global().for_each_chunk_mut(near.words_mut(), 4, |ci, words| {
        for (wi, w) in words.iter_mut().enumerate() {
            for bit in 0..64 {
                let i = (ci * 4 + wi) * 64 + bit;
                if agents
                    .get(i)
                    .is_some_and(|a| a.pos.raw().distance(a.goal) < 100.0)
                {
                    *w |= 1 << bit;
                }
            }
        }
    });
    let expected: Vec<usize> = agents
        .iter()
        .enumerate()
        .filter(|(_, a)| a.pos.raw().distance(a.goal) < 100.0)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(near.iter().collect::<Vec<_>>(), expected);
}
