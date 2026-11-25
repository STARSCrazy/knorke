//! The guard scene in `examples/guard_scene.rs`, run by the runtime: both endings, the
//! schedule, frames from host time, and a replay (record, verify, seek).

// The example is shared with this test; its `pub` items are its public face as an example.
#[allow(dead_code, unreachable_pub)]
#[path = "../examples/guard_scene.rs"]
mod guard_scene;

use guard_scene::*;
use knorke_integration::Cell;
use knorke_observe::{NoProbe, Probe, Scope};
use knorke_replay::{
    NoRecord, RecordOptions, Recorder, ReplayHeader, Simulation, TickDriver, seek, verify,
};
use knorke_runtime::{FixedStep, Frame, Runtime};
use knorke_types::{EventCursor, Seed, SnapshotBuf, SnapshotError, Tick};

/// Shows itself at the gap and is behind the wall again before the guard gets there.
const SHY: Script = Script {
    visit: Cell::new(7, 3, 0),
    leave_at: 17,
    refuge: Cell::new(9, 6, 0),
};

fn state(s: &impl Simulation) -> Vec<u8> {
    let mut buf = SnapshotBuf::new();
    s.save(&mut buf);
    buf.bytes().to_vec()
}

fn one_tick_per_call() -> FixedStep {
    FixedStep::new(1, 1)
}

/// Runs `script` for ticks `rt.next_tick() .. end` and returns the events with their tick.
fn play<D: TickDriver<Scene>>(
    rt: &mut Runtime<Scene, D>,
    script: Script,
    end: u64,
) -> Vec<(u64, Event)> {
    let mut cursor = rt.sim.yard.actors.events.cursor_at_end();
    let mut seen = Vec::new();
    for t in rt.next_tick().0..end {
        if let Some(command) = script.command(t) {
            rt.submit(command);
        }
        rt.tick();
        let events = rt.sim.yard.actors.events.read(&mut cursor);
        seen.extend(events.since().unwrap().iter().map(|&e| (t, e)));
    }
    seen
}

fn fresh() -> Runtime<Scene> {
    Runtime::new(
        Scene::new(NoProbe),
        NoRecord,
        one_tick_per_call(),
        Tick::ZERO,
    )
}

#[test]
fn a_bold_intruder_is_caught_and_stays() {
    let mut rt = fresh();
    let events = play(&mut rt, Script::BOLD, END);
    let change = |from, to| Event::State { from, to };
    assert_eq!(
        events,
        [
            (18, Event::Spotted(Cell::new(7, 4, 0))),
            (18, change(POST, CHASE)),
            (21, change(CHASE, CAUGHT)),
        ]
    );
    let a = &rt.sim.yard.actors;
    assert_eq!(
        (a.intruder, a.guard),
        (Cell::new(7, 1, 0), Cell::new(6, 2, 0))
    );
    assert_ne!(a.intruder, Script::BOLD.refuge);
}

#[test]
fn a_shy_intruder_gets_away_and_the_guard_goes_back() {
    let mut rt = fresh();
    let events = play(&mut rt, SHY, END);
    let change = |from, to| Event::State { from, to };
    assert_eq!(
        events,
        [
            (19, Event::Spotted(Cell::new(8, 1, 0))),
            (19, change(POST, CHASE)),
            (23, change(CHASE, RETURN)),
            (27, change(RETURN, POST)),
        ]
    );
    let a = &rt.sim.yard.actors;
    assert_eq!((a.intruder, a.guard), (SHY.refuge, Cell::new(3, 5, 0)));
}

#[test]
fn a_goal_inside_the_wall_is_ignored() {
    let mut rt = fresh();
    rt.submit(Command::Goto(Cell::new(8, 5, 0)));
    rt.submit(Command::Goto(Cell::new(99, 0, 0)));
    for _ in 0..5 {
        rt.tick();
    }
    assert_eq!(rt.sim.yard.actors.intruder, HIDEOUT);
}

/// Remembers the scopes it was told to enter and checks that each one is left again.
#[derive(Default)]
struct Names {
    entered: Vec<&'static str>,
    open: Vec<&'static str>,
}

impl Probe for Names {
    fn enter(&mut self, scope: &'static Scope) {
        self.entered.push(scope.name);
        self.open.push(scope.name);
    }
    fn exit(&mut self, scope: &'static Scope) {
        assert_eq!(self.open.pop(), Some(scope.name));
    }
}

#[test]
fn every_tick_runs_the_systems_in_table_order() {
    let scene = Scene::new(Names::default());
    let mut rt = Runtime::new(scene, NoRecord, one_tick_per_call(), Tick::ZERO);
    rt.tick();
    rt.tick();
    let order = [
        "walk_intruder",
        "guard_looks",
        "guard_decides",
        "walk_guard",
    ];
    assert_eq!(rt.sim.probe.entered, [order, order].concat());
    assert!(rt.sim.probe.open.is_empty());
}

#[test]
fn frames_from_host_time_match_single_ticks() {
    // 16 ms per tick, at most 4 ticks per frame.
    const TICK: u64 = 16_000_000;
    let mut rt = Runtime::new(
        Scene::new(NoProbe),
        NoRecord,
        FixedStep::new(TICK, 4),
        Tick::ZERO,
    );
    // A command handed in while no tick is due waits for the next one.
    rt.submit(Script::BOLD.command(2).unwrap());
    let idle = rt.frame(TICK / 2);
    assert_eq!(
        idle,
        Frame {
            ticks: 0,
            alpha: 0.5
        }
    );
    assert_eq!(rt.next_tick(), Tick(0));
    // Uneven frames: half a tick is still pending, so 2.6 ticks of time give 3 ticks.
    let frame = rt.frame(TICK * 26 / 10);
    assert_eq!(frame.ticks, 3);
    assert!((frame.alpha - 0.1).abs() < 1e-3, "alpha {}", frame.alpha);
    // A stall of a second is capped instead of being caught up.
    assert_eq!(rt.frame(1_000_000_000).ticks, 4);
    assert_eq!(rt.next_tick(), Tick(7));

    let mut single = fresh();
    single.submit(Script::BOLD.command(2).unwrap());
    for _ in 0..7 {
        single.tick();
    }
    assert_eq!(state(&rt.sim), state(&single.sim));
    assert_ne!(rt.sim.yard.actors.intruder, HIDEOUT);
}

#[test]
fn a_recorded_run_verifies_and_seeks() {
    let scene = Scene::new(NoProbe);
    let header = ReplayHeader {
        app: "guard_scene".into(),
        seed: Seed(0),
    };
    let opts = RecordOptions {
        checksum_every: 1,
        keyframe_every: 8,
        window: None,
    };
    let recorder = Recorder::start(&scene, Tick::ZERO, header, opts);
    let mut rt = Runtime::new(scene, recorder, one_tick_per_call(), Tick::ZERO);
    let events = play(&mut rt, SHY, END);
    let end = state(&rt.sim);
    let log = rt.driver.log();
    assert_eq!(log.command_count(), 2);

    // The log alone reproduces the run, tick by tick and section by section.
    let report = verify(&mut Scene::new(NoProbe), log).unwrap();
    assert_eq!((report.ticks, report.checked), (END as usize, END as usize));

    // Seek into the chase (between two keyframes), compare with a straight run to there,
    // then play on to the same end and the same events.
    const MID: u64 = 21;
    let mut straight = fresh();
    let early = play(&mut straight, SHY, MID);
    let mut sought = fresh();
    seek(&mut sought.sim, log, Tick(MID)).unwrap();
    sought.jump(Tick(MID));
    assert_eq!(state(&sought.sim), state(&straight.sim));
    let late = play(&mut sought, SHY, END);
    assert_eq!(state(&sought.sim), end);
    assert_eq!([early, late].concat(), events);
}

#[test]
fn restore_rejects_an_actor_off_the_grid() {
    let mut broken = Scene::new(NoProbe);
    broken.yard.actors.guard = Cell::new(16, 0, 0);
    let result = Scene::new(NoProbe).restore(&state(&broken));
    assert_eq!(
        result,
        Err(SnapshotError::Invalid("guard scene: actor off the grid"))
    );
}

#[test]
fn a_reader_that_starts_late_resyncs_instead_of_missing_events() {
    let mut rt = fresh();
    play(&mut rt, SHY, END);
    // Events are kept for a few ticks only; a cursor from before that has to resync.
    let mut stale = EventCursor::default();
    assert!(rt.sim.yard.actors.events.read(&mut stale).since().is_none());
    assert!(rt.sim.yard.actors.events.is_empty());
}
