//! Replay test (CLAUDE.md, ADR 0010): a guard hears noises and sees intruders, reports
//! them by radio to a partner who passes them back with a delay. Its `Alertness`,
//! `Knowledge`, `StateMachine`, `Areas`, `Timers`, `Outbox`/`Inbox`, `Progress`,
//! `Interval` and `GoalGuess` are the AI state; record → verify → seek must reproduce them
//! byte for byte.

use knorke_ai::memory::{
    AlertConfig, Alertness, Area, Areas, Evict, Fact, GoalGuess, GoalGuessConfig, Inbox, Interval,
    Knowledge, Message, Outbox, Progress, Timers,
};
use knorke_ai::state::{ANY, StateMachine, Transition};
use knorke_replay::{RecordOptions, Recorder, ReplayHeader, ReplayLog, Simulation, seek, verify};
use knorke_types::{
    Reader, Seed, Snapshot, SnapshotBuf, SnapshotError, SnapshotValue, Tick, Writer, restore,
};

const PATROL: u32 = 0;
const INVESTIGATE: u32 = 1;
const ALERT: u32 = 2;
/// Flag bits handed to the state machine.
const ALERTED: u64 = 1;
const KNOWS_SOMEONE: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmd {
    Noise(u32),
    Seen { subject: u32, x: i32, y: i32 },
}

impl SnapshotValue for Cmd {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Cmd::Noise(a) => {
                w.put(&0u8);
                w.put(&a);
            }
            Cmd::Seen { subject, x, y } => {
                w.put(&1u8);
                w.put(&subject);
                w.put(&x);
                w.put(&y);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Cmd::Noise(r.get()?),
            1 => Cmd::Seen {
                subject: r.get()?,
                x: r.get()?,
                y: r.get()?,
            },
            _ => return Err(SnapshotError::Invalid("cmd tag")),
        })
    }
}

/// The guard's AI state, saved by the host as one section.
struct Guard {
    alert: Alertness,
    memory: Knowledge,
    mind: StateMachine,
    /// Noisy places to avoid.
    noisy: Areas,
    /// Intruders not to report again for a while.
    reported: Timers,
    radio: Outbox,
    inbox: Inbox,
    /// Whether the chase of subject 0 still gets closer.
    chase: Progress,
    sweep: Interval,
    /// Is subject 0 heading for the gate (0, 0) or the store (19, 19)?
    guess: GoalGuess,
    /// Where subject 0 was last seen.
    last_seen: Option<(i32, i32)>,
}

impl Snapshot for Guard {
    const ID: &'static str = "test.guard";
    const VERSION: u16 = 1;
    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.alert);
        w.put(&self.memory);
        w.put(&self.mind);
        w.put(&self.noisy);
        w.put(&self.reported);
        w.put(&self.radio);
        w.put(&self.inbox);
        w.put(&self.chase);
        w.put(&self.sweep);
        w.put(&self.guess);
        w.put(&self.last_seen);
    }
    fn restore(&mut self, _: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.alert = r.get()?;
        self.memory.get_into(r)?;
        self.mind.get_into(r)?;
        self.noisy.get_into(r)?;
        self.reported.get_into(r)?;
        self.radio.get_into(r)?;
        self.inbox.get_into(r)?;
        self.chase = r.get()?;
        self.sweep = r.get()?;
        self.guess = r.get()?;
        self.last_seen = r.get()?;
        Ok(())
    }
}

/// Chebyshev distance.
fn far(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

const GOALS: [(i32, i32); 2] = [(0, 0), (19, 19)];

struct Post {
    guard: Guard,
    cfg: AlertConfig,
}

impl Post {
    fn new() -> Self {
        let mut mind = StateMachine::new(3, PATROL).unwrap();
        mind.add_transition(Transition::new(ANY, ALERT, ALERTED | KNOWS_SOMEONE))
            .unwrap();
        mind.add_transition(Transition::new(PATROL, INVESTIGATE, ALERTED))
            .unwrap();
        mind.set_timeout(INVESTIGATE, 20, PATROL).unwrap();
        mind.set_timeout(ALERT, 35, INVESTIGATE).unwrap();
        let mut thresholds = [0; 8];
        thresholds[0] = 300;
        thresholds[1] = 700;
        Post {
            guard: Guard {
                alert: Alertness::default(),
                memory: Knowledge::new(4, Evict::Weakest).unwrap(),
                mind,
                noisy: Areas::new(3).unwrap(),
                reported: Timers::new(2).unwrap(),
                radio: Outbox::new(3).unwrap(),
                inbox: Inbox::new(2).unwrap(),
                chase: Progress::default(),
                sweep: Interval::default(),
                guess: GoalGuess::new(GoalGuessConfig::default(), GOALS.len()),
                last_seen: None,
            },
            cfg: AlertConfig {
                max: 1000,
                decay_per_tick: 6,
                hold: 5,
                hysteresis: 50,
                count: 2,
                thresholds,
            },
        }
    }
}

impl Simulation for Post {
    type Command = Cmd;

    fn save(&self, out: &mut SnapshotBuf) {
        out.save(&self.guard);
    }

    fn restore(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let mut r = Reader::new(bytes);
        restore(&mut r, &mut self.guard)?;
        r.finish("test.post")
    }

    fn step(&mut self, tick: Tick, commands: &[Cmd]) {
        let now = tick.0 as u32 + 1;
        let g = &mut self.guard;
        for c in commands {
            match *c {
                Cmd::Noise(amount) => {
                    g.alert.stimulus(amount, now, &self.cfg);
                    let x = (amount % 20) as i32;
                    g.noisy.add(
                        now,
                        Area {
                            kind: 1,
                            pos: [x, 3, 0],
                            radius: 2,
                            strength: amount,
                            lifetime: 25,
                            ..Area::default()
                        },
                    );
                }
                Cmd::Seen { subject, x, y } => {
                    let fact = Fact {
                        subject,
                        kind: 1,
                        pos: [x, y, 0],
                        seen_at: now,
                        strength: 100 + subject,
                        data: 0,
                        source: 0,
                    };
                    g.memory.learn(now, fact);
                    if !g.reported.active(subject, now) {
                        let msg = Message {
                            kind: 1,
                            subject,
                            pos: fact.pos,
                            seen_at: now,
                            strength: fact.strength,
                            data: 0,
                            sender: 1,
                            keep_if_sender_gone: 0,
                        };
                        if g.radio.post(now, msg, 2 + subject, 3).is_ok() {
                            g.reported.set(subject, now, 15);
                        }
                    }
                    if subject == 0 {
                        let dist = far((x, y), GOALS[0]);
                        if g.chase.started == 0 || g.chase.stalled(now, 40) {
                            g.chase.start(dist, now);
                        } else {
                            g.chase.update(dist, now, 2);
                        }
                        // What a step from the last place to here tells about the goal.
                        if let Some(prev) = g.last_seen
                            && prev != (x, y)
                        {
                            let mut moved = [0; 2];
                            let mut options = [0; 2 * 8];
                            for (k, &goal) in GOALS.iter().enumerate() {
                                let before = far(prev, goal);
                                moved[k] = far((x, y), goal) - before;
                                let mut j = 0;
                                for dy in -1..=1 {
                                    for dx in -1..=1 {
                                        if (dx, dy) != (0, 0) {
                                            options[k * 8 + j] =
                                                far((prev.0 + dx, prev.1 + dy), goal) - before;
                                            j += 1;
                                        }
                                    }
                                }
                            }
                            g.guess.observe(&moved, &options, 8).unwrap();
                        }
                        g.last_seen = Some((x, y));
                    }
                }
            }
        }
        // The partner hears the radio and tells the guard back, later.
        let mut due = Vec::new();
        g.radio.due(now, &mut due);
        for m in due {
            let _ = g.inbox.push(m, now + 4);
        }
        let mut told = Vec::new();
        g.inbox.take_due(now, &mut told);
        for m in told {
            g.memory.learn(now, m.to_fact(2));
        }
        g.noisy.expire(now);
        if g.sweep.due(now, 9) {
            g.sweep.mark(now);
            g.reported.remove(5);
        }
        g.memory.expire(now, 30, |_| {});
        g.alert.update(now, &self.cfg);
        let mut flags = 0;
        if g.alert.level() > 0 {
            flags |= ALERTED;
        }
        if !g.memory.is_empty() {
            flags |= KNOWS_SOMEONE;
        }
        g.mind.update(now, flags);
    }
}

fn commands_for(t: u64) -> Vec<Cmd> {
    let h = Seed(5).derive(t).0;
    let mut out = Vec::new();
    if h.is_multiple_of(4) {
        out.push(Cmd::Noise(((h >> 8) % 400) as u32));
    }
    if (h >> 16).is_multiple_of(9) {
        out.push(Cmd::Seen {
            subject: ((h >> 24) % 6) as u32,
            x: ((h >> 32) % 20) as i32,
            y: ((h >> 40) % 20) as i32,
        });
    }
    out
}

fn record(ticks: u64) -> (Post, ReplayLog<Cmd>) {
    let mut sim = Post::new();
    let mut rec = Recorder::start(
        &sim,
        Tick(0),
        ReplayHeader {
            app: "knorke-ai replay".into(),
            seed: Seed(5),
        },
        RecordOptions {
            checksum_every: 1,
            keyframe_every: 11,
            window: None,
        },
    );
    for t in 0..ticks {
        rec.step(&mut sim, Tick(t), &commands_for(t));
    }
    (sim, rec.into_log())
}

fn bytes(sim: &Post) -> Vec<u8> {
    let mut b = SnapshotBuf::new();
    sim.save(&mut b);
    b.bytes().to_vec()
}

#[test]
fn ai_replay_verifies_and_seeks() {
    // The guard must visit every state, or the test proves little.
    let mut probe = Post::new();
    let mut seen = [false; 3];
    let (mut told, mut guessed, mut noisy, mut pending) = (false, false, false, false);
    for t in 0..400 {
        probe.step(Tick(t), &commands_for(t));
        let g = &probe.guard;
        seen[g.mind.state() as usize] = true;
        told |= g.memory.iter().any(|f| f.source == 2);
        guessed |= g.guess.chance(0) != 128;
        noisy |= !g.noisy.is_empty();
        pending |= g.radio.pending() > 0 && !g.inbox.is_empty();
    }
    assert_eq!(seen, [true; 3]);
    assert!(
        told && guessed && noisy && pending,
        "{told} {guessed} {noisy} {pending}"
    );
    let (_, log) = record(400);
    let report = verify(&mut Post::new(), &log).unwrap();
    assert_eq!(report.ticks, 400);
    for target in [0u64, 1, 11, 12, 150, 399, 400] {
        let mut direct = Post::new();
        for t in 0..target {
            direct.step(Tick(t), &commands_for(t));
        }
        let mut sought = Post::new();
        seek(&mut sought, &log, Tick(target)).unwrap();
        assert_eq!(bytes(&sought), bytes(&direct), "tick {target}");
    }
}

#[test]
fn restore_rejects_inconsistent_state() {
    let post = Post::new();
    let mut bytes = Vec::new();
    Writer::new(&mut bytes).put(&post.guard.mind);
    let back: StateMachine = Reader::new(&bytes).get().unwrap();
    assert_eq!(back.state(), PATROL);
    // `current` is the second-to-last u32: point it past the three states.
    let at = bytes.len() - 8;
    bytes[at..at + 4].copy_from_slice(&7u32.to_le_bytes());
    assert!(Reader::new(&bytes).get::<StateMachine>().is_err());

    let mut k = Vec::new();
    let mut w = Writer::new(&mut k);
    w.put(&1u64); // capacity 1
    w.put(&0u8);
    w.put(&vec![Fact::default(), Fact::default()]);
    assert!(Reader::new(&k).get::<Knowledge>().is_err());
}
