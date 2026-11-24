//! Contract: a steady-state parallel frame allocates nothing – on **any** thread.
//!
//! Own test binary with a single test, so the process-wide allocation counter is not
//! disturbed by other tests running concurrently.

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_contract_tests::steering::{Agent, Arrived, CHUNK, ChunkOut, agents, steer};
use knorke_exec::Rayon;
use knorke_math::Vec2;
use knorke_observe::{Probe, Recorder, scope};
use knorke_types::{EventQueue, Parallel, Serial, Tick, chunk_count};

#[global_allocator]
static GLOBAL: Counting = Counting;

fn frame(
    par: &impl Parallel,
    agents: &[Agent],
    velocities: &mut [Vec2],
    outs: &mut [ChunkOut],
    arrived: &mut EventQueue<Arrived>,
    tick: Tick,
    probe: &mut impl Probe,
) -> usize {
    let mut probe = probe.scope(scope!("knorke.test", "frame"));
    {
        let _s = probe.scope(scope!("knorke.test", "steer"));
        steer(par, agents, velocities, outs);
    }
    let mut spawned = 0;
    for o in outs.iter_mut() {
        o.spawns.apply(|_| spawned += 1);
        arrived.append(tick, &mut o.arrived);
    }
    arrived.keep_ticks(tick, 2);
    spawned
}

/// Measures serial frames first (no pool exists yet, so no background thread can interfere),
/// then rayon frames. Our code must not allocate at all. Rayon's scheduler may: jobs
/// injected from outside the pool go through crossbeam's injector queue, which allocates one
/// block per 63 injected jobs (measured: 15 allocations per 1000 calls). Running the whole
/// frame inside the pool (`Rayon::install`) injects one job per frame instead of one per
/// parallel call.
#[test]
fn steady_state_parallel_frame_allocates_nothing_on_any_thread() {
    let agents = agents(100_000);
    let mut velocities = vec![Vec2::ZERO; agents.len()];
    let mut outs: Vec<ChunkOut> = (0..chunk_count(agents.len(), CHUNK))
        .map(|_| ChunkOut::default())
        .collect();
    let mut arrived = EventQueue::with_capacity(4096);
    let mut recorder = Recorder::new("sim", 1, 1024);
    let mut spawned = 0;

    // 1. Serial: strictly zero.
    for t in 0..3 {
        frame(
            &Serial,
            &agents,
            &mut velocities,
            &mut outs,
            &mut arrived,
            Tick(t),
            &mut recorder,
        );
    }
    let before = allocations();
    for t in 3..23 {
        spawned += frame(
            &Serial,
            &agents,
            &mut velocities,
            &mut outs,
            &mut arrived,
            Tick(t),
            &mut recorder,
        );
    }
    let serial_allocs = allocations() - before;
    assert_eq!(
        serial_allocs, 0,
        "serial frames allocated {serial_allocs} times"
    );

    // 2. Rayon, whole frame inside the pool: only the injector's block allocation is allowed.
    let rayon = Rayon::with_threads(4).unwrap();
    for t in 23..26 {
        rayon.install(|| {
            frame(
                &rayon,
                &agents,
                &mut velocities,
                &mut outs,
                &mut arrived,
                Tick(t),
                &mut recorder,
            )
        });
    }
    let frames = 200u64;
    let before = allocations();
    for t in 26..26 + frames {
        spawned += rayon.install(|| {
            frame(
                &rayon,
                &agents,
                &mut velocities,
                &mut outs,
                &mut arrived,
                Tick(t),
                &mut recorder,
            )
        });
    }
    let rayon_allocs = allocations() - before;
    let injector_bound = frames.div_ceil(63) + 1;
    assert!(spawned > 0);
    assert!(
        rayon_allocs <= injector_bound,
        "{frames} rayon frames allocated {rayon_allocs} times (allowed: {injector_bound} injector blocks)"
    );
}
