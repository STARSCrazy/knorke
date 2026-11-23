//! Contract: modules exchange data with the host without copying, and the steady-state
//! frame allocates nothing (ADR 0006, plan section 11 "keine Heap-Allocations pro Tick").
//!
//! The "modules" here are stand-ins shaped like the real ones will be: a navigation-like
//! system that reads host positions and keeps its own per-entity state, and a render-like
//! system that uploads the same positions as bytes.

#![allow(unsafe_code)] // only for the counting allocator below

use knorke_math::{At, Vec2, World2, bytemuck};
use knorke_observe::{NoProbe, Probe, Recorder, Sink, counter, scope};
use knorke_types::{
    Column, ColumnMut, Diagnostic, DiagnosticCode, EventCursor, EventQueue, Events, Handle,
    HandleMap, Revision, Slots, Tick, project,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

// ---- allocation counter (per thread, so parallel tests do not interfere) -------------

struct Counting;
thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}
// SAFETY: forwards to the system allocator unchanged; only counts calls.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|c| c.set(c.get() + 1));
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = ALLOCATIONS.try_with(|c| c.set(c.get() + 1));
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations_during(f: impl FnOnce()) -> u64 {
    let before = ALLOCATIONS.with(Cell::get);
    f();
    ALLOCATIONS.with(Cell::get) - before
}

// ---- host side ------------------------------------------------------------------------

type WorldPos = At<World2, Vec2>;

/// The game's own entity struct: positions live *inside* it, next to unrelated data.
struct Npc {
    id: Handle<Npc>,
    pos: WorldPos,
    _name: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum WorldChange {
    BlockerMoved { cell: (i32, i32) },
}

// ---- navigation-like module ---------------------------------------------------------

#[derive(Default)]
struct Steering {
    goal: Vec2,
    replans: u32,
}

#[derive(Default)]
struct Navigation {
    state: HandleMap<Npc, Steering>,
    cursor: EventCursor,
    seen_revision: Revision,
}

const NAV_REPLANNED: DiagnosticCode = DiagnosticCode::new("knorke.navigation.replanned");

impl Navigation {
    /// Reads positions through a borrowed column, writes velocities back through another,
    /// consumes only unseen world changes. Generic over all observation channels.
    #[allow(clippy::too_many_arguments)]
    fn tick(
        &mut self,
        ids: impl Column<Handle<Npc>>,
        positions: impl Column<WorldPos>,
        mut velocities: impl ColumnMut<Vec2>,
        world: &EventQueue<WorldChange>,
        world_revision: Revision,
        probe: &mut impl Probe,
        diagnostics: &mut impl Sink<Diagnostic>,
    ) {
        let mut probe = probe.scope(scope!("knorke.navigation", "tick"));
        let world_changed = world_revision != self.seen_revision;
        self.seen_revision = world_revision;
        let blockers_moved = match world.read(&mut self.cursor) {
            Events::Since { events, .. } => events.len(),
            Events::Resync => usize::MAX, // first tick or fell behind: replan everyone
        };
        probe.count(
            counter!("knorke.navigation", "world_changes", "events"),
            blockers_moved as f64,
        );
        for i in 0..positions.len() {
            let (Some(&id), Some(pos)) = (ids.get(i), positions.get(i)) else {
                continue;
            };
            let Some(s) = self.state.get_mut(id) else {
                continue;
            };
            if world_changed && blockers_moved > 0 {
                s.replans += 1;
                if diagnostics.enabled() {
                    diagnostics.emit(Diagnostic::new(
                        knorke_types::Severity::Info,
                        NAV_REPLANNED,
                        "blocker moved",
                    ));
                }
            }
            if let Some(v) = velocities.get_mut(i) {
                *v = (s.goal - pos.raw()).normalize_or_zero();
            }
        }
    }
}

// ---- render-like module -------------------------------------------------------------

/// Stand-in for a GPU upload: takes bytes, returns a checksum. A real renderer would
/// `memcpy` these bytes into a mapped transfer buffer – the only copy, and an unavoidable one.
fn upload(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0u64, |h, &b| h.wrapping_mul(31).wrapping_add(b as u64))
}

// ---- the contract ---------------------------------------------------------------------

fn setup() -> (Slots<()>, Vec<Npc>, Navigation, EventQueue<WorldChange>) {
    let mut ids: Slots<()> = Slots::new();
    let mut npcs = Vec::new();
    let mut nav = Navigation::default();
    for i in 0..1000 {
        let id: Handle<Npc> = ids.insert(()).cast();
        npcs.push(Npc {
            id,
            pos: At::new(Vec2::new(i as f32, 0.0)),
            _name: "guard",
        });
        nav.state.insert(
            id,
            Steering {
                goal: Vec2::new(0.0, 100.0),
                replans: 0,
            },
        );
    }
    let mut world = EventQueue::new();
    world.push(Tick(0), WorldChange::BlockerMoved { cell: (0, 0) });
    (ids, npcs, nav, world)
}

/// One simulation frame. Navigation reads ids and positions straight out of the host's
/// structs and writes velocities into a buffer the host allocated once.
fn frame(
    npcs: &[Npc],
    velocities: &mut [Vec2],
    nav: &mut Navigation,
    world: &EventQueue<WorldChange>,
    rev: Revision,
    probe: &mut impl Probe,
    diags: &mut impl Sink<Diagnostic>,
) -> u64 {
    let ids = project(npcs, |n| &n.id);
    let positions = project(npcs, |n| &n.pos);
    nav.tick(&ids, &positions, &mut *velocities, world, rev, probe, diags);
    // Renderer: the same positions, packed by the host for upload. Here the host keeps a
    // separate packed array (as physics engines usually do); we checksum one field view.
    upload(bytemuck::cast_slice(velocities))
}

#[test]
fn positions_cross_module_boundaries_without_copies() {
    let (_ids, npcs, _, _) = setup();
    // Navigation view: points at the host's memory.
    let positions = project(&npcs, |n| &n.pos);
    assert!(std::ptr::eq(positions.get(500).unwrap(), &npcs[500].pos));
    // A packed position array (e.g. a physics engine's) is the same memory as raw vectors,
    // floats and bytes for the GPU.
    let packed: Vec<WorldPos> = npcs.iter().map(|n| n.pos).collect();
    let raw: &[Vec2] = At::slice_raw(&packed);
    let floats: &[f32] = bytemuck::cast_slice(raw);
    let bytes: &[u8] = bytemuck::cast_slice(raw);
    assert_eq!(raw.as_ptr().cast::<u8>(), packed.as_ptr().cast::<u8>());
    assert_eq!(floats.as_ptr().cast::<u8>(), bytes.as_ptr());
    assert_eq!(bytes.len(), packed.len() * 8);
    assert_ne!(upload(bytes), 0);
}

#[test]
fn steady_state_frame_allocates_nothing() {
    let (_ids, npcs, mut nav, mut world) = setup();
    let mut rev = Revision::ZERO;
    let mut recorder = Recorder::new("sim", 1, 4096);
    let mut diags: Vec<Diagnostic> = Vec::with_capacity(16_000);
    let mut velocities = vec![Vec2::ZERO; npcs.len()];
    // Warm-up frame: first read of the change log is a resync.
    frame(
        &npcs,
        &mut velocities,
        &mut nav,
        &world,
        rev.bump(),
        &mut recorder,
        &mut diags,
    );
    world.retire_before(Tick(1));

    let allocs = allocations_during(|| {
        for t in 1..10u64 {
            recorder.set_tick(Tick(t));
            frame(
                &npcs,
                &mut velocities,
                &mut nav,
                &world,
                rev,
                &mut recorder,
                &mut diags,
            );
        }
        frame(
            &npcs,
            &mut velocities,
            &mut nav,
            &world,
            rev,
            &mut NoProbe,
            &mut knorke_observe::NoopSink,
        );
    });
    assert_eq!(allocs, 0, "hot path allocated");
    assert!(recorder.spans().count() >= 9);
}

#[test]
fn incremental_changes_reach_each_consumer_once() {
    let (_ids, npcs, mut nav, mut world) = setup();
    let ids = project(&npcs, |n| &n.id);
    let pos = project(&npcs, |n| &n.pos);
    let mut out = vec![Vec2::ZERO; npcs.len()];
    let mut rev = Revision::ZERO;
    let mut diags = Vec::new();

    nav.tick(
        &ids,
        &pos,
        &mut out[..],
        &world,
        rev.bump(),
        &mut NoProbe,
        &mut diags,
    ); // resync
    let after_first = diags.len();
    assert_eq!(after_first, npcs.len());

    nav.tick(
        &ids,
        &pos,
        &mut out[..],
        &world,
        rev,
        &mut NoProbe,
        &mut diags,
    ); // nothing new
    assert_eq!(diags.len(), after_first);

    world.push(Tick(5), WorldChange::BlockerMoved { cell: (3, 4) });
    nav.tick(
        &ids,
        &pos,
        &mut out[..],
        &world,
        rev.bump(),
        &mut NoProbe,
        &mut diags,
    );
    assert_eq!(diags.len(), after_first + npcs.len());
    assert!(out[1].abs_diff_eq(Vec2::new(-1.0, 100.0).normalize_or_zero(), 1e-6));
    assert_eq!(nav.state.get(npcs[0].id).unwrap().replans, 2);
}

#[test]
fn per_module_state_survives_host_reuse_safely() {
    let (mut ids, mut npcs, mut nav, _) = setup();
    let removed = npcs.remove(0);
    ids.remove(removed.id.cast());
    let reused: Handle<Npc> = ids.insert(()).cast();
    assert_eq!(reused.index(), removed.id.index());
    assert!(
        nav.state.get(reused).is_none(),
        "new entity must not inherit old nav state"
    );
    nav.state.retain_live(|h| ids.contains(h.cast()));
    assert_eq!(nav.state.len(), npcs.len());
}

#[test]
fn allocation_counter_actually_counts() {
    assert!(allocations_during(|| drop(std::hint::black_box(vec![1u8; 10]))) > 0);
}
