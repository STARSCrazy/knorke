//! Contract: a frame of the runtime allocates nothing in steady state, with the guard
//! scene (navigation, perception, state machine, events) as the simulation and the flight
//! recorder switched on.
//!
//! Own test binary with a single test because the allocation counter is process-wide.

// The example is shared with this test; its `pub` items are its public face as an example.
#[allow(dead_code, unreachable_pub)]
#[path = "../../../crates/knorke-runtime/examples/guard_scene.rs"]
mod guard_scene;

use guard_scene::{Command, HIDEOUT, Scene};
use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_integration::Cell;
use knorke_observe::NoProbe;
use knorke_replay::{RecordOptions, Recorder, ReplayHeader};
use knorke_runtime::{FixedStep, Runtime};
use knorke_types::{Seed, Tick};

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn runtime_frames_allocate_nothing_in_steady_state() {
    let scene = Scene::new(NoProbe);
    let header = ReplayHeader {
        app: "contract".into(),
        seed: Seed(0),
    };
    let opts = RecordOptions {
        checksum_every: 1,
        keyframe_every: 16,
        window: Some(64),
    };
    let recorder = Some(Recorder::start(&scene, Tick::ZERO, header, opts));
    let mut rt = Runtime::new(scene, recorder, FixedStep::new(10, 4), Tick::ZERO);
    // The intruder walks back and forth through the gap, in and out of the guard's view,
    // so every system has work in every phase of the scene.
    let goals = [Cell::new(6, 5, 0), HIDEOUT];
    let frame = |rt: &mut Runtime<_, _>, i: u64| {
        if i.is_multiple_of(40) {
            rt.submit(Command::Goto(goals[(i / 40 % 2) as usize]));
        }
        // Uneven host time: 0 to 2 ticks per frame.
        rt.frame(5 + i % 3 * 7);
    };
    for i in 0..2000 {
        frame(&mut rt, i);
    }
    let before = allocations();
    for i in 2000..4000 {
        frame(&mut rt, i);
    }
    let allocs = allocations() - before;
    assert_eq!(allocs, 0, "2000 frames allocated {allocs} times");
    assert!(rt.next_tick().0 > 3000, "ticks ran: {:?}", rt.next_tick());
}
