//! Replay test (CLAUDE.md, ADR 0010): a walker re-plans with a small node budget, so most
//! keyframes fall in the middle of a search. Record → verify → seek must give the same bytes
//! as the straight run, which only holds if the `Navigator` snapshot is complete.

use std::num::NonZeroUsize;

use knorke_navigation::Budget;
use knorke_navigation::grid::{Cell, DenseGrid, GridRules, Preset};
use knorke_navigation::nav::{Navigator, Status};
use knorke_navigation::topology::Topology;
use knorke_replay::{RecordOptions, Recorder, ReplayHeader, ReplayLog, Simulation, seek, verify};
use knorke_types::{
    Reader, Seed, Snapshot, SnapshotBuf, SnapshotError, SnapshotValue, Tick, Writer, restore,
};

const SIZE: i32 = 24;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmd {
    Goal(Cell),
    Block(i32, i32),
    Open(i32, i32),
}

impl SnapshotValue for Cmd {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Cmd::Goal(c) => {
                w.put(&0u8);
                w.put(&c);
            }
            Cmd::Block(x, y) => {
                w.put(&1u8);
                w.put(&x);
                w.put(&y);
            }
            Cmd::Open(x, y) => {
                w.put(&2u8);
                w.put(&x);
                w.put(&y);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Cmd::Goal(r.get()?),
            1 => Cmd::Block(r.get()?, r.get()?),
            2 => Cmd::Open(r.get()?, r.get()?),
            _ => return Err(SnapshotError::Invalid("cmd tag")),
        })
    }
}

/// Host state: the map, the walker and its goal.
struct World {
    map: DenseGrid,
    at: Cell,
    goal: Cell,
    status: u8,
}

impl Snapshot for World {
    const ID: &'static str = "test.world";
    const VERSION: u16 = 1;
    fn save(&self, w: &mut Writer<'_>) {
        for y in 0..SIZE {
            for x in 0..SIZE {
                w.put(&self.map.get(x, y).unwrap_or(0));
            }
        }
        w.put(&self.at);
        w.put(&self.goal);
        w.put(&self.status);
    }
    fn restore(&mut self, _: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        for y in 0..SIZE {
            for x in 0..SIZE {
                self.map.set(x, y, r.get()?);
            }
        }
        self.at = r.get()?;
        self.goal = r.get()?;
        self.status = r.get()?;
        Ok(())
    }
}

struct Walk {
    world: World,
    nav: Navigator,
}

impl Walk {
    fn new() -> Self {
        Walk {
            world: World {
                map: DenseGrid::new(SIZE, SIZE),
                at: Cell::new(0, 0, 0),
                goal: Cell::new(0, 0, 0),
                status: 0,
            },
            nav: Navigator::new(Topology::Square(GridRules::preset(Preset::Octile)), 1024),
        }
    }
}

fn budget() -> Budget {
    Budget::from(NonZeroUsize::new(16).unwrap())
}

fn code(s: Status) -> u8 {
    match s {
        Status::Idle => 0,
        Status::Running => 1,
        Status::Found => 2,
        Status::Partial => 3,
        Status::Unreachable => 4,
        Status::Done => 5,
    }
}

impl Simulation for Walk {
    type Command = Cmd;

    fn save(&self, out: &mut SnapshotBuf) {
        out.save(&self.world);
        out.save(&self.nav);
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let mut r = Reader::new(bytes);
        restore(&mut r, &mut self.world)?;
        restore(&mut r, &mut self.nav)?;
        r.finish("test.walk")
    }

    fn step(&mut self, _: Tick, commands: &[Cmd]) {
        let w = &mut self.world;
        let mut replan = false;
        for c in commands {
            match *c {
                Cmd::Goal(g) => {
                    w.goal = g;
                    replan = true;
                }
                Cmd::Block(x, y) if Cell::new(x, y, 0) != w.at => {
                    w.map.set(x, y, DenseGrid::BLOCKED);
                    replan = true;
                }
                Cmd::Block(..) => {}
                Cmd::Open(x, y) => w.map.set(x, y, 0),
            }
        }
        let report = if replan || (w.status != 1 && w.at != w.goal) {
            self.nav.find_path(&mut w.map, w.at, &[w.goal], budget())
        } else if w.status == 1 {
            self.nav.continue_search(&mut w.map, budget())
        } else {
            return;
        };
        w.status = code(report.status);
        if report.status == Status::Found {
            if let Some(&next) = self.nav.path().iter().find(|&&c| c != w.at) {
                w.at = next;
            }
        }
    }
}

/// Seeded commands: a new goal now and then, walls appearing and disappearing.
fn commands_for(t: u64) -> Vec<Cmd> {
    let h = Seed(7).derive(t).0;
    let cell = |h: u64| ((h % SIZE as u64) as i32, ((h >> 8) % SIZE as u64) as i32);
    let mut out = Vec::new();
    if t.is_multiple_of(40) {
        let (x, y) = cell(h >> 16);
        out.push(Cmd::Goal(Cell::new(x, y, 0)));
    }
    match h % 5 {
        0 => {
            let (x, y) = cell(h >> 24);
            out.push(Cmd::Block(x, y));
        }
        1 => {
            let (x, y) = cell(h >> 32);
            out.push(Cmd::Open(x, y));
        }
        _ => {}
    }
    out
}

fn record(ticks: u64) -> (Walk, ReplayLog<Cmd>) {
    let mut sim = Walk::new();
    let mut rec = Recorder::start(
        &sim,
        Tick(0),
        ReplayHeader {
            app: "knorke-navigation replay".into(),
            seed: Seed(7),
        },
        RecordOptions {
            checksum_every: 1,
            keyframe_every: 7,
            window: None,
        },
    );
    for t in 0..ticks {
        rec.step(&mut sim, Tick(t), &commands_for(t));
    }
    (sim, rec.into_log())
}

fn bytes(sim: &Walk) -> Vec<u8> {
    let mut b = SnapshotBuf::new();
    sim.save(&mut b);
    b.bytes().to_vec()
}

#[test]
fn navigator_replay_verifies_and_seeks() {
    let (end, log) = record(400);
    // The walker must actually have walked and searched over several ticks.
    assert_ne!(end.world.at, Cell::new(0, 0, 0));
    let mut probe = Walk::new();
    let mut running = 0;
    for t in 0..400 {
        probe.step(Tick(t), &commands_for(t));
        running += u32::from(probe.world.status == 1);
    }
    assert!(
        running > 20,
        "only {running} ticks with a search in progress"
    );
    let report = verify(&mut Walk::new(), &log).unwrap();
    assert_eq!(report.ticks, 400);
    for target in [0u64, 3, 7, 8, 50, 133, 399, 400] {
        let mut direct = Walk::new();
        for t in 0..target {
            direct.step(Tick(t), &commands_for(t));
        }
        let mut sought = Walk::new();
        seek(&mut sought, &log, Tick(target)).unwrap();
        assert_eq!(bytes(&sought), bytes(&direct), "tick {target}");
    }
}

#[test]
fn restore_rejects_corrupt_search() {
    let (sim, _) = record(30);
    let mut buf = SnapshotBuf::new();
    buf.save(&sim.nav);
    let good = buf.bytes().to_vec();
    let mut nav = Navigator::new(Topology::Square(GridRules::preset(Preset::Octile)), 1024);
    restore(&mut Reader::new(&good), &mut nav).unwrap();
    // Truncated data must fail cleanly instead of panicking later.
    assert!(restore(&mut Reader::new(&good[..good.len() - 3]), &mut nav).is_err());
}
