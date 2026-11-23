//! Contract: a steady-state parallel frame allocates nothing – on **any** thread.
//!
//! Own test binary with a single test, so the process-wide allocation counter is not
//! disturbed by other tests running concurrently.

#![allow(unsafe_code)] // counting allocator

use knorke_contract_tests::steering::{Agent, Arrived, CHUNK, ChunkOut, agents, steer};
use knorke_exec::Rayon;
use knorke_math::Vec2;
use knorke_observe::{Probe, Recorder, scope};
use knorke_types::{EventQueue, Parallel, Serial, Tick, chunk_count};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

struct Counting;
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
// SAFETY: forwards to the system allocator unchanged; only counts calls.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
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
    let before = ALLOCATIONS.load(Ordering::SeqCst);
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
    let serial_allocs = ALLOCATIONS.load(Ordering::SeqCst) - before;
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
    let before = ALLOCATIONS.load(Ordering::SeqCst);
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
    let rayon_allocs = ALLOCATIONS.load(Ordering::SeqCst) - before;
    let injector_bound = frames.div_ceil(63) + 1;
    assert!(spawned > 0);
    assert!(
        rayon_allocs <= injector_bound,
        "{frames} rayon frames allocated {rayon_allocs} times (allowed: {injector_bound} injector blocks)"
    );
}
