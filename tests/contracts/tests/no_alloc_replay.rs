//! Contract: an always-on flight recorder (checksums every tick, keyframes, bounded window)
//! allocates nothing in steady state, and invariant checks on a healthy world neither.
//!
//! Own test binary with a single test because the allocation counter is process-wide.

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_math::{At, Vec2, World2};
use knorke_observe::{Invariants, NoopSink};
use knorke_replay::{RecordOptions, Recorder, ReplayHeader, Simulation, verify};
use knorke_types::{
    DiagnosticCode, Reader, Seed, Slots, Snapshot, SnapshotBuf, SnapshotError, SnapshotValue, Tick,
    Writer, restore,
};

#[global_allocator]
static GLOBAL: Counting = Counting;

#[derive(Default)]
struct Bodies {
    seed: Seed,
    items: Slots<(At<World2, Vec2>, Vec2)>,
}

impl Snapshot for Bodies {
    const ID: &'static str = "test.bodies";
    const VERSION: u16 = 1;
    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.seed);
        w.put(&self.items);
    }
    fn restore(&mut self, _: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.seed = r.get()?;
        self.items.get_into(r)
    }
}

#[derive(Default)]
struct World {
    bodies: Bodies,
}

impl Simulation for World {
    type Command = (u8, i32);
    fn save(&self, out: &mut SnapshotBuf) {
        out.save(&self.bodies);
    }
    fn restore(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        restore(&mut Reader::new(bytes), &mut self.bodies)
    }
    fn step(&mut self, tick: Tick, commands: &[(u8, i32)]) {
        for &(kind, v) in commands {
            if kind == 1 {
                for (_, b) in self.bodies.items.iter_mut() {
                    b.1.y += v as f32 * 0.01;
                }
            }
        }
        let jitter = (self.bodies.seed.derive(tick.0).0 % 100) as f32 * 1e-4;
        for (_, (p, vel)) in self.bodies.items.iter_mut() {
            p.value = p.value + *vel + Vec2::splat(jitter);
            if p.value.y.abs() > 100.0 {
                vel.y = -vel.y;
            }
        }
    }
}

#[test]
fn flight_recorder_and_invariants_allocate_nothing_in_steady_state() {
    let mut world = World {
        bodies: Bodies {
            seed: Seed(5),
            ..Default::default()
        },
    };
    for i in 0..2000 {
        world
            .bodies
            .items
            .insert((At::new(Vec2::new(i as f32, 0.0)), Vec2::new(0.0, 1.0)));
    }
    let opts = RecordOptions {
        checksum_every: 1,
        keyframe_every: 60,
        window: Some(300),
    };
    let mut rec = Recorder::start(
        &world,
        Tick(0),
        ReplayHeader {
            app: "contract".into(),
            seed: Seed(5),
        },
        opts,
    );
    let mut inv: Invariants<World> = Invariants::new();
    inv.add(
        DiagnosticCode::new("knorke.test.finite"),
        1,
        |w: &World, _, v| {
            for (_, (p, _)) in w.bodies.items.iter() {
                if !(p.value.x.is_finite() && p.value.y.is_finite()) {
                    v.report("non-finite position", None);
                }
            }
        },
    );
    let commands = [(1u8, 3i32), (0, 0)];
    let mut found = 0;
    // Warm-up: fill the window once so buffers reach their working size.
    for t in 0..1000u64 {
        rec.step(&mut world, Tick(t), &commands);
        found += inv.run(&world, Tick(t), &mut NoopSink);
    }
    let before = allocations();
    for t in 1000..2000u64 {
        rec.step(&mut world, Tick(t), &commands);
        found += inv.run(&world, Tick(t), &mut NoopSink);
    }
    let allocs = allocations() - before;
    assert_eq!(found, 0);
    assert_eq!(allocs, 0, "1000 recorded ticks allocated {allocs} times");
    let log = rec.into_log();
    assert!(
        log.start().tick.0 >= 1600,
        "window moved: starts at {:?}",
        log.start().tick
    );
    verify(&mut World::default(), &log).expect("flight-recorder window must verify");
}
