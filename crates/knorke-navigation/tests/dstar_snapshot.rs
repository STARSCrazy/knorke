//! Snapshot and replay of D* Lite (CLAUDE.md, ADR 0010, ADR 0012): a walker repairs its
//! plan with a small expansion budget while cells open and close, so most keyframes fall in
//! the middle of a repair. Record → verify → seek must give the same bytes as the straight
//! run, which only holds if the `DStarLite` snapshot is complete.

use knorke_navigation::dstar::{DStarLite, DStarStatus, HexHeuristic};
use knorke_navigation::grid::{Cell, DenseGrid, GridGraph, GridRules, Preset};
use knorke_navigation::hex::HexLayout;
use knorke_replay::{RecordOptions, Recorder, ReplayHeader, ReplayLog, Simulation, seek, verify};
use knorke_types::{
    Reader, Seed, Snapshot, SnapshotBuf, SnapshotError, SnapshotValue, Tick, Writer, restore,
};

const SIZE: i32 = 20;
const BUDGET: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmd {
    Toggle(i32, i32),
}

impl SnapshotValue for Cmd {
    fn put(&self, w: &mut Writer<'_>) {
        let Cmd::Toggle(x, y) = *self;
        w.put(&x);
        w.put(&y);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Cmd::Toggle(r.get()?, r.get()?))
    }
}

type Planner = DStarLite<Cell, GridRules>;

struct Walk {
    graph: GridGraph<DenseGrid>,
    at: Cell,
    goal: Cell,
    planner: Planner,
    path: Vec<Cell>,
    changed: Vec<Cell>,
}

impl Snapshot for Walk {
    const ID: &'static str = "test.dstar";
    const VERSION: u16 = 1;
    fn save(&self, w: &mut Writer<'_>) {
        for y in 0..SIZE {
            for x in 0..SIZE {
                w.put(&self.graph.map.get(x, y).unwrap_or(0));
            }
        }
        w.put(&self.at);
        w.put(&self.goal);
        w.put(&self.planner);
    }
    fn restore(&mut self, _: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        for y in 0..SIZE {
            for x in 0..SIZE {
                self.graph.map.set(x, y, r.get()?);
            }
        }
        self.at = r.get()?;
        self.goal = r.get()?;
        self.planner.get_into(r)
    }
}

fn rules() -> GridRules {
    GridRules::preset(Preset::Octile)
}

impl Walk {
    fn new() -> Self {
        let at = Cell::new(0, 0, 0);
        let goal = Cell::new(SIZE - 1, SIZE - 1, 0);
        let mut planner = Planner::with_capacity(512);
        planner.begin(at, goal, rules());
        Walk {
            graph: GridGraph::new(DenseGrid::new(SIZE, SIZE), rules(), None),
            at,
            goal,
            planner,
            path: Vec::new(),
            changed: Vec::new(),
        }
    }
}

impl Simulation for Walk {
    type Command = Cmd;

    fn save(&self, out: &mut SnapshotBuf) {
        out.save(self);
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let mut r = Reader::new(bytes);
        restore(&mut r, self)?;
        r.finish("test.walk")
    }

    fn step(&mut self, _tick: Tick, commands: &[Cmd]) {
        for &Cmd::Toggle(x, y) in commands {
            if (x, y) == (self.at.x, self.at.y) || (x, y) == (self.goal.x, self.goal.y) {
                continue;
            }
            let new = if self.graph.map.open(x, y) {
                DenseGrid::BLOCKED
            } else {
                0
            };
            self.graph.map.set(x, y, new);
            self.changed.clear();
            for dy in -1..=1 {
                for dx in -1..=1 {
                    self.changed.push(Cell::new(x + dx, y + dy, 0));
                }
            }
            self.planner
                .nodes_changed(&mut self.graph, self.changed.iter().copied());
        }
        if self.planner.compute(&mut self.graph, BUDGET) == DStarStatus::Found
            && self.planner.path(&mut self.graph, &mut self.path)
            && self.path.len() > 1
        {
            self.at = self.path[1];
            self.planner.move_start(self.at);
        }
        if self.at == self.goal {
            // Walk back and forth, so the run keeps planning.
            std::mem::swap(&mut self.at, &mut self.goal);
            self.planner.begin(self.at, self.goal, rules());
        }
    }
}

fn commands_for(t: u64) -> Vec<Cmd> {
    let h = Seed(11).derive(t).0;
    let mut out = Vec::new();
    for i in 0..(h % 6).saturating_sub(3) {
        let v = h >> (8 + 12 * i);
        out.push(Cmd::Toggle(
            (v % SIZE as u64) as i32,
            ((v >> 6) % SIZE as u64) as i32,
        ));
    }
    out
}

fn record(ticks: u64) -> (Walk, ReplayLog<Cmd>) {
    let mut sim = Walk::new();
    let mut rec = Recorder::start(
        &sim,
        Tick(0),
        ReplayHeader {
            app: "knorke-navigation dstar replay".into(),
            seed: Seed(11),
        },
        RecordOptions {
            checksum_every: 1,
            keyframe_every: 5,
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
    Simulation::save(sim, &mut b);
    b.bytes().to_vec()
}

#[test]
fn dstar_replay_verifies_and_seeks() {
    // The planner must be stopped by its budget on many ticks, or the test proves little.
    let mut probe = Walk::new();
    let (mut running, mut moved) = (0, 0);
    for t in 0..300 {
        let before = probe.at;
        probe.step(Tick(t), &commands_for(t));
        running += u32::from(probe.planner.status() == DStarStatus::Running);
        moved += u32::from(probe.at != before);
    }
    assert!(
        running > 20,
        "only {running} ticks with a repair in progress"
    );
    assert!(moved > 20, "the walker moved only {moved} times");
    let (_, log) = record(300);
    let report = verify(&mut Walk::new(), &log).unwrap();
    assert_eq!(report.ticks, 300);
    for target in [0u64, 2, 5, 6, 77, 151, 299, 300] {
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
fn restored_planner_continues_identically() {
    let mut a = Walk::new();
    for t in 0..40 {
        a.step(Tick(t), &commands_for(t));
    }
    let mut saved = Vec::new();
    Writer::new(&mut saved).put(&a.planner);
    // Restore into a planner that has worked on a different map.
    let mut b = Walk::new();
    for t in 0..40 {
        b.step(Tick(t + 1000), &commands_for(t + 1000));
    }
    b.graph.map = a.graph.map.clone();
    b.at = a.at;
    b.goal = a.goal;
    b.planner.get_into(&mut Reader::new(&saved)).unwrap();
    let mut again = Vec::new();
    Writer::new(&mut again).put(&b.planner);
    assert_eq!(again, saved);
    assert_eq!(b.planner.cost(), a.planner.cost());
    for t in 40..120 {
        a.step(Tick(t), &commands_for(t));
        b.step(Tick(t), &commands_for(t));
        assert_eq!(bytes(&a), bytes(&b), "tick {t}");
    }
}

#[test]
fn hex_heuristic_round_trips() {
    let h = HexHeuristic {
        layout: HexLayout::EvenQ,
        step_cost: 7,
    };
    let mut d: DStarLite<Cell, HexHeuristic> = DStarLite::new();
    d.begin(Cell::new(0, 0, 0), Cell::new(3, 2, 0), h);
    let mut saved = Vec::new();
    Writer::new(&mut saved).put(&d);
    let back: DStarLite<Cell, HexHeuristic> = Reader::new(&saved).get().unwrap();
    assert_eq!(back.goal(), d.goal());
    assert_eq!(back.start(), d.start());
    assert_eq!(back.status(), DStarStatus::Running);
}

#[test]
fn restore_rejects_inconsistent_planner() {
    let mut a = Walk::new();
    for t in 0..10 {
        a.step(Tick(t), &commands_for(t));
    }
    let mut good = Vec::new();
    Writer::new(&mut good).put(&a.planner);
    let mut p = Planner::new();
    p.get_into(&mut Reader::new(&good)).unwrap();

    // Truncated.
    assert!(
        Reader::new(&good[..good.len() - 1])
            .get::<Planner>()
            .is_err()
    );

    // Layout: records (u32 len, each: cell 12, g 8, rhs 8, queued 1 or 17), then the open
    // list (u32 len, each: key 16, index 4). Build small planners by hand instead.
    let record = |w: &mut Writer<'_>, x: i32| {
        w.put(&Cell::new(x, 0, 0));
        w.put(&u64::MAX);
        w.put(&0u64);
        w.put(&None::<(u64, u64)>);
    };
    let tail = |w: &mut Writer<'_>, status: u8| {
        w.put(&Some(rules()));
        w.put(&Some(Cell::new(0, 0, 0)));
        w.put(&Some(Cell::new(1, 0, 0)));
        w.put(&0u64); // km
        w.put(&0u64); // computes
        w.put(&status);
        w.put(&0u64); // expanded
        w.put(&u64::MAX); // max nodes
        w.put(&false);
    };
    let build = |records: &[i32], open: &[u32], status: u8| {
        let mut b = Vec::new();
        let mut w = Writer::new(&mut b);
        w.put_len(records.len());
        for &x in records {
            record(&mut w, x);
        }
        w.put_len(open.len());
        for &i in open {
            w.put(&(0u64, 0u64));
            w.put(&i);
        }
        tail(&mut w, status);
        b
    };
    let ok = build(&[1, 0], &[0, 1], 1);
    let back: Planner = Reader::new(&ok).get().unwrap();
    assert_eq!(back.known(), 2);
    // An open entry past the records.
    assert!(
        Reader::new(&build(&[1, 0], &[2], 1))
            .get::<Planner>()
            .is_err()
    );
    // A node known twice.
    assert!(
        Reader::new(&build(&[1, 1], &[0], 1))
            .get::<Planner>()
            .is_err()
    );
    // An unknown status.
    assert!(Reader::new(&build(&[1], &[0], 9)).get::<Planner>().is_err());
    // A failed restore leaves a usable, empty planner.
    let bad = build(&[1, 1], &[0], 1);
    assert!(p.get_into(&mut Reader::new(&bad)).is_err());
    assert_eq!(p.known(), 0);
    assert_eq!(p.status(), DStarStatus::Idle);
    let mut g = GridGraph::new(DenseGrid::new(4, 4), rules(), None);
    assert_eq!(p.compute(&mut g, 10), DStarStatus::Idle);
}
