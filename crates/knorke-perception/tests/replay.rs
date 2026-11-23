//! Replay test (CLAUDE.md, ADR 0010): a guard watches four allies who walk about and change
//! their deeds. Its per-pair `Awareness` and its `Contagion` memory are the perception
//! state; record → verify → seek must reproduce them byte for byte.

use knorke_grid::Cell;
use knorke_perception::{
    Awareness, Contagion, ContagionConfig, DeedRule, DisguiseRule, Eye, Grid, OpenAir, Percept,
    SenseProfile, Stats, Target, World, cell_centre, observe,
};
use knorke_replay::{RecordOptions, Recorder, ReplayHeader, ReplayLog, Simulation, seek, verify};
use knorke_types::{
    Reader, Seed, Snapshot, SnapshotBuf, SnapshotError, SnapshotValue, Tick, Writer, restore,
};

const S: i32 = 256;
const ALLIES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmd {
    Move { ally: u8, dx: i8, dy: i8 },
    Cues { ally: u8, bits: u32 },
}

impl SnapshotValue for Cmd {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Cmd::Move { ally, dx, dy } => {
                w.put(&0u8);
                w.put(&ally);
                w.put(&dx);
                w.put(&dy);
            }
            Cmd::Cues { ally, bits } => {
                w.put(&1u8);
                w.put(&ally);
                w.put(&bits);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Cmd::Move {
                ally: r.get()?,
                dx: r.get()?,
                dy: r.get()?,
            },
            1 => Cmd::Cues {
                ally: r.get()?,
                bits: r.get()?,
            },
            _ => return Err(SnapshotError::Invalid("cmd tag")),
        })
    }
}

/// Host state: where the allies stand and what they show.
#[derive(Default)]
struct Allies {
    cells: [(i32, i32); ALLIES],
    cues: [u32; ALLIES],
}

impl Snapshot for Allies {
    const ID: &'static str = "test.allies";
    const VERSION: u16 = 1;
    fn save(&self, w: &mut Writer<'_>) {
        for (c, b) in self.cells.iter().zip(&self.cues) {
            w.put(&c.0);
            w.put(&c.1);
            w.put(b);
        }
    }
    fn restore(&mut self, _: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        for i in 0..ALLIES {
            self.cells[i] = (r.get()?, r.get()?);
            self.cues[i] = r.get()?;
        }
        Ok(())
    }
}

/// The guard's perception state, saved by the host as one section.
struct Guard {
    aware: Vec<Awareness>,
    deeds: Contagion,
}

impl Snapshot for Guard {
    const ID: &'static str = "test.guard";
    const VERSION: u16 = 1;
    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.aware);
        w.put(&self.deeds);
    }
    fn restore(&mut self, _: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        r.get_into_vec(&mut self.aware)?;
        self.deeds.get_into(r)
    }
}

struct Scene {
    allies: Allies,
    guard: Guard,
    profile: SenseProfile,
    percepts: Vec<Percept>,
}

fn config() -> ContagionConfig {
    ContagionConfig::new(Grid::Square)
        .with_deed(DeedRule { bits: 1, window: 5 })
        .with_deed(DeedRule {
            bits: 2,
            window: 12,
        })
}

impl Scene {
    fn new() -> Self {
        Scene {
            allies: Allies {
                cells: [(3, 0), (4, 1), (5, -1), (6, 0)],
                cues: [0; ALLIES],
            },
            guard: Guard {
                aware: vec![Awareness::default(); ALLIES],
                deeds: Contagion::new(config(), ALLIES, 1),
            },
            profile: SenseProfile::human(S),
            percepts: Vec::new(),
        }
    }
}

impl Simulation for Scene {
    type Command = Cmd;

    fn save(&self, out: &mut SnapshotBuf) {
        out.save(&self.allies);
        out.save(&self.guard);
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let mut r = Reader::new(bytes);
        restore(&mut r, &mut self.allies)?;
        restore(&mut r, &mut self.guard)?;
        r.finish("test.scene")
    }

    fn step(&mut self, tick: Tick, commands: &[Cmd]) {
        for c in commands {
            match *c {
                Cmd::Move { ally, dx, dy } => {
                    let p = &mut self.allies.cells[usize::from(ally)];
                    p.0 = (p.0 + i32::from(dx)).clamp(1, 12);
                    p.1 = (p.1 + i32::from(dy)).clamp(-6, 6);
                }
                Cmd::Cues { ally, bits } => self.allies.cues[usize::from(ally)] = bits,
            }
        }
        // Clock starts at 1 (a zeroed Awareness was last checked at 0).
        let now = tick.0 as u32 + 1;
        let world = World::square(S);
        let eye = Eye {
            id: 1,
            point: cell_centre(&world, Cell::new(0, 0, 0)),
            facing: 0,
            profile: &self.profile,
            stats: Stats::NORMAL,
            disguise: DisguiseRule::default(),
        };
        self.percepts.clear();
        for i in 0..ALLIES {
            let (x, y) = self.allies.cells[i];
            let target = Target {
                id: 10 + i as u32,
                point: cell_centre(&world, Cell::new(x, y, 0)),
                size: 256,
                conspicuity: 256,
                cues: self.allies.cues[i],
                ..Target::default()
            };
            observe(
                &world,
                &mut OpenAir,
                now,
                &eye,
                &target,
                &mut self.guard.aware[i],
                &mut self.percepts,
            );
        }
        self.guard.deeds.learn(&self.percepts, now, |_, _| true);
    }
}

fn commands_for(t: u64) -> Vec<Cmd> {
    let h = Seed(11).derive(t).0;
    let ally = (h % ALLIES as u64) as u8;
    let mut out = vec![Cmd::Move {
        ally,
        dx: ((h >> 8) % 3) as i8 - 1,
        dy: ((h >> 16) % 3) as i8 - 1,
    }];
    if (h >> 24).is_multiple_of(6) {
        out.push(Cmd::Cues {
            ally: ((h >> 32) % ALLIES as u64) as u8,
            bits: ((h >> 40) % 4) as u32,
        });
    }
    out
}

fn record(ticks: u64) -> (Scene, ReplayLog<Cmd>) {
    let mut sim = Scene::new();
    let mut rec = Recorder::start(
        &sim,
        Tick(0),
        ReplayHeader {
            app: "knorke-perception replay".into(),
            seed: Seed(11),
        },
        RecordOptions {
            checksum_every: 1,
            keyframe_every: 9,
            window: None,
        },
    );
    for t in 0..ticks {
        rec.step(&mut sim, Tick(t), &commands_for(t));
    }
    (sim, rec.into_log())
}

fn bytes(sim: &Scene) -> Vec<u8> {
    let mut b = SnapshotBuf::new();
    sim.save(&mut b);
    b.bytes().to_vec()
}

#[test]
fn perception_replay_verifies_and_seeks() {
    let (end, log) = record(300);
    // The guard must have noticed someone and remembered a deed, or the test proves little.
    assert!(end.guard.aware.iter().any(|a| a.stage() > 0));
    assert!(!end.guard.deeds.is_empty());
    let report = verify(&mut Scene::new(), &log).unwrap();
    assert_eq!(report.ticks, 300);
    for target in [0u64, 1, 9, 10, 77, 299, 300] {
        let mut direct = Scene::new();
        for t in 0..target {
            direct.step(Tick(t), &commands_for(t));
        }
        let mut sought = Scene::new();
        seek(&mut sought, &log, Tick(target)).unwrap();
        assert_eq!(bytes(&sought), bytes(&direct), "tick {target}");
    }
}

#[test]
fn awareness_restore_rejects_bad_stage() {
    let mut bytes = Vec::new();
    let mut w = Writer::new(&mut bytes);
    w.put(&Awareness::default());
    assert_eq!(
        Reader::new(&bytes).get::<Awareness>(),
        Ok(Awareness::default())
    );
    bytes[4] = 3; // stage
    assert!(Reader::new(&bytes).get::<Awareness>().is_err());
}

#[test]
fn contagion_round_trip_keeps_config_and_capacity() {
    let (end, _) = record(120);
    let mut bytes = Vec::new();
    Writer::new(&mut bytes).put(&end.guard.deeds);
    let back: Contagion = Reader::new(&bytes).get().unwrap();
    assert_eq!(back, end.guard.deeds);
    let mut other = Contagion::new(ContagionConfig::new(Grid::Hex), 1, 99);
    other.get_into(&mut Reader::new(&bytes)).unwrap();
    assert_eq!(other, end.guard.deeds);
}
