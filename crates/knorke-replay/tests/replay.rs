//! Replay contract on a small two-module simulation (movers + scoring).

use knorke_replay::{
    RecordOptions, Recorder, ReplayHeader, ReplayLog, Simulation, VerifyError, minimize, seek,
    verify,
};
use knorke_types::{
    Reader, Seed, Slots, Snapshot, SnapshotBuf, SnapshotError, SnapshotValue, Tick, Writer, restore,
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmd {
    Spawn { x: i32, v: i32 },
    Push { dv: i32 },
    Noop,
}

impl SnapshotValue for Cmd {
    fn put(&self, w: &mut Writer<'_>) {
        match *self {
            Cmd::Spawn { x, v } => {
                w.put(&0u8);
                w.put(&x);
                w.put(&v);
            }
            Cmd::Push { dv } => {
                w.put(&1u8);
                w.put(&dv);
            }
            Cmd::Noop => w.put(&2u8),
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(match r.get::<u8>()? {
            0 => Cmd::Spawn {
                x: r.get()?,
                v: r.get()?,
            },
            1 => Cmd::Push { dv: r.get()? },
            2 => Cmd::Noop,
            _ => return Err(SnapshotError::Invalid("cmd tag")),
        })
    }
}

/// Module 1: moving particles with a seeded wobble.
#[derive(Default)]
struct Movers {
    seed: Seed,
    items: Slots<(i32, i32)>,
}
impl Snapshot for Movers {
    const ID: &'static str = "test.movers";
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

/// Module 2: a score derived from the movers.
#[derive(Default)]
struct Score {
    total: i64,
}
impl Snapshot for Score {
    const ID: &'static str = "test.score";
    const VERSION: u16 = 1;
    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.total);
    }
    fn restore(&mut self, _: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.total = r.get()?;
        Ok(())
    }
}

#[derive(Default)]
struct Game {
    movers: Movers,
    score: Score,
    /// Fault injection: from this tick on the score module misbehaves.
    corrupt_score_from: Option<u64>,
}

impl Simulation for Game {
    type Command = Cmd;
    fn save(&self, out: &mut SnapshotBuf) {
        out.save(&self.movers);
        out.save(&self.score);
    }
    fn restore(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
        let mut r = Reader::new(bytes);
        restore(&mut r, &mut self.movers)?;
        restore(&mut r, &mut self.score)?;
        r.finish("test.game")
    }
    fn step(&mut self, tick: Tick, commands: &[Cmd]) {
        for c in commands {
            match *c {
                Cmd::Spawn { x, v } => {
                    self.movers.items.insert((x, v));
                }
                Cmd::Push { dv } => {
                    for (_, m) in self.movers.items.iter_mut() {
                        m.1 += dv;
                    }
                }
                Cmd::Noop => {}
            }
        }
        let wobble = (self.movers.seed.derive(tick.0).0 % 3) as i32 - 1;
        for (_, m) in self.movers.items.iter_mut() {
            m.0 += m.1 + wobble;
        }
        self.score.total += self
            .movers
            .items
            .iter()
            .map(|(_, m)| m.0 as i64)
            .sum::<i64>();
        if self.corrupt_score_from.is_some_and(|t| tick.0 >= t) {
            self.score.total += 1;
        }
    }
}

fn commands_for(t: u64) -> Vec<Cmd> {
    match t % 7 {
        0 => vec![Cmd::Spawn { x: t as i32, v: 1 }],
        3 => vec![Cmd::Push { dv: -1 }, Cmd::Noop],
        _ => vec![],
    }
}

fn record(ticks: u64, opts: RecordOptions) -> (Game, ReplayLog<Cmd>) {
    let mut game = Game {
        movers: Movers {
            seed: Seed(99),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut rec = Recorder::start(
        &game,
        Tick(0),
        ReplayHeader {
            app: "test 1".into(),
            seed: Seed(99),
        },
        opts,
    );
    for t in 0..ticks {
        rec.step(&mut game, Tick(t), &commands_for(t));
    }
    (game, rec.into_log())
}

fn state_bytes(g: &Game) -> Vec<u8> {
    let mut b = SnapshotBuf::new();
    g.save(&mut b);
    b.bytes().to_vec()
}

#[test]
fn recorded_run_verifies() {
    let (_, log) = record(
        200,
        RecordOptions {
            checksum_every: 1,
            keyframe_every: 50,
            window: None,
        },
    );
    let report = verify(&mut Game::default(), &log).unwrap();
    assert_eq!((report.ticks, report.checked), (200, 200));
    assert_eq!(log.keyframes().len(), 3);
}

#[test]
fn divergence_names_tick_and_module() {
    let (_, log) = record(100, RecordOptions::default());
    let mut faulty = Game {
        corrupt_score_from: Some(37),
        ..Default::default()
    };
    match verify(&mut faulty, &log) {
        Err(VerifyError::Diverged(d)) => {
            assert_eq!(d.tick, Tick(37));
            assert_eq!(d.sections, ["test.score"]);
        }
        other => panic!("expected divergence, got {other:?}"),
    }
}

#[test]
fn sparse_checksums_bound_the_divergence() {
    let (_, log) = record(
        100,
        RecordOptions {
            checksum_every: 10,
            keyframe_every: 0,
            window: None,
        },
    );
    let mut faulty = Game {
        corrupt_score_from: Some(37),
        ..Default::default()
    };
    let Err(VerifyError::Diverged(d)) = verify(&mut faulty, &log) else {
        panic!()
    };
    assert_eq!(d.tick, Tick(39), "first checked tick at or after the fault");
}

#[test]
fn seek_matches_straight_run() {
    let (_, log) = record(
        300,
        RecordOptions {
            checksum_every: 1,
            keyframe_every: 64,
            window: None,
        },
    );
    for target in [0u64, 1, 63, 64, 65, 199, 300] {
        let mut direct = Game {
            movers: Movers {
                seed: Seed(99),
                ..Default::default()
            },
            ..Default::default()
        };
        for t in 0..target {
            direct.step(Tick(t), &commands_for(t));
        }
        let mut sought = Game::default();
        seek(&mut sought, &log, Tick(target)).unwrap();
        assert_eq!(
            state_bytes(&sought),
            state_bytes(&direct),
            "seek to {target}"
        );
    }
    assert!(seek(&mut Game::default(), &log, Tick(301)).is_err());
}

#[test]
fn serialized_log_round_trips_and_still_verifies() {
    let (_, log) = record(
        120,
        RecordOptions {
            checksum_every: 1,
            keyframe_every: 40,
            window: None,
        },
    );
    let mut bytes = Vec::new();
    log.write(&mut bytes);
    let back = ReplayLog::<Cmd>::read(&bytes).unwrap();
    assert_eq!(back, log);
    assert!(verify(&mut Game::default(), &back).is_ok());
    assert!(ReplayLog::<Cmd>::read(&bytes[..bytes.len() - 1]).is_err());
    let mut wrong = bytes.clone();
    wrong[0] = b'X';
    assert!(ReplayLog::<Cmd>::read(&wrong).is_err());
}

#[test]
fn flight_recorder_keeps_a_bounded_verifiable_window() {
    let opts = RecordOptions {
        checksum_every: 1,
        keyframe_every: 30,
        window: Some(100),
    };
    let (final_state, log) = record(1000, opts);
    let span = log.end_tick().0 - log.start().tick.0;
    assert!((100..130).contains(&span), "window kept {span} ticks");
    assert!(log.start().tick.0 > 800);
    verify(&mut Game::default(), &log).unwrap();
    let mut end = Game::default();
    seek(&mut end, &log, log.end_tick()).unwrap();
    assert_eq!(state_bytes(&end), state_bytes(&final_state));
}

#[test]
fn minimize_finds_the_commands_that_matter() {
    // Bug: a mover reaches velocity <= -30. Only the spawn with v = -5 at tick 40 followed by
    // the push of -30 at tick 90 causes it; 30 other spawns and 40 no-ops are noise.
    let background = |t: u64| match t % 7 {
        0 => vec![Cmd::Spawn { x: t as i32, v: 1 }],
        3 => vec![Cmd::Noop],
        _ => vec![],
    };
    let mut game = Game::default();
    let mut rec = Recorder::start(
        &game,
        Tick(0),
        ReplayHeader::default(),
        RecordOptions::default(),
    );
    for t in 0..200u64 {
        let mut cmds = background(t);
        if t == 40 {
            cmds.push(Cmd::Spawn { x: 0, v: -5 });
        }
        if t == 90 {
            cmds.push(Cmd::Push { dv: -30 });
        }
        rec.step(&mut game, Tick(t), &cmds);
    }
    let log = rec.into_log();
    let mut runs = 0;
    let mut fails = |candidate: &ReplayLog<Cmd>| {
        runs += 1;
        let mut g = Game::default();
        g.restore(&candidate.start().bytes).unwrap();
        candidate.ticks().iter().any(|t| {
            g.step(t.tick, candidate.commands_of(t));
            g.movers.items.iter().any(|(_, m)| m.1 <= -30)
        })
    };
    assert!(fails(&log), "scenario must fail");
    let min = minimize(&log, &mut fails);
    let kept: Vec<(u64, Cmd)> = min
        .ticks()
        .iter()
        .flat_map(|t| min.commands_of(t).iter().map(move |c| (t.tick.0, *c)))
        .collect();
    assert_eq!(
        kept,
        [
            (40, Cmd::Spawn { x: 0, v: -5 }),
            (90, Cmd::Push { dv: -30 })
        ],
        "from {} commands",
        log.command_count()
    );
    assert!(runs < 200, "{runs} replays needed");
}

/// Hand-encodes a log in format version 1 (see `ReplayLog::write`) with `Cmd::Noop`
/// commands and no checksums, so that single fields can be made inconsistent.
fn encode_log(
    start_tick: u64,
    ticks: &[(u64, u32, u32)],
    commands: u32,
    keyframe_ticks: &[u64],
) -> Vec<u8> {
    let mut state = SnapshotBuf::new();
    Game::default().save(&mut state);
    let mut out = Vec::new();
    let mut w = Writer::new(&mut out);
    w.put_bytes(b"KNRP");
    w.put(&knorke_replay::FORMAT_VERSION);
    w.put_len(0); // app
    w.put(&Seed(0));
    let keyframe = |w: &mut Writer<'_>, tick: u64| {
        w.put(&Tick(tick));
        w.put_len(state.bytes().len());
        w.put_bytes(state.bytes());
    };
    keyframe(&mut w, start_tick);
    w.put_len(0); // section ids
    w.put_len(ticks.len());
    for &(tick, from, to) in ticks {
        w.put(&Tick(tick));
        w.put(&from);
        w.put(&to);
        w.put(&0u32);
        w.put(&0u32);
    }
    w.put_len(commands as usize);
    for _ in 0..commands {
        w.put(&Cmd::Noop);
    }
    w.put_len(0); // sums
    w.put_len(keyframe_ticks.len());
    for &t in keyframe_ticks {
        keyframe(&mut w, t);
    }
    out
}

#[test]
fn read_rejects_inconsistent_logs() {
    let read = |b: &[u8]| ReplayLog::<Cmd>::read(b);
    let ok = encode_log(10, &[(10, 0, 1), (11, 1, 1), (12, 1, 3)], 3, &[11, 12]);
    let log = read(&ok).expect("the encoder produces valid logs");
    assert_eq!((log.end_tick(), log.command_count()), (Tick(13), 3));
    seek(&mut Game::default(), &log, Tick(13)).unwrap();

    let bad: [(&str, Vec<u8>); 10] = [
        (
            "command range past the end",
            encode_log(0, &[(0, 0, 4)], 3, &[]),
        ),
        (
            "reversed command range",
            encode_log(0, &[(0, 2, 1)], 3, &[]),
        ),
        (
            "ticks not consecutive",
            encode_log(0, &[(0, 0, 0), (2, 0, 0)], 0, &[]),
        ),
        (
            "ticks descending",
            encode_log(0, &[(1, 0, 0), (0, 0, 0)], 0, &[]),
        ),
        (
            "first tick is not the start tick",
            encode_log(0, &[(5, 0, 0)], 0, &[]),
        ),
        (
            "tick overflow",
            encode_log(u64::MAX, &[(u64::MAX, 0, 0)], 0, &[]),
        ),
        (
            "keyframe before the start",
            encode_log(5, &[(5, 0, 0), (6, 0, 0)], 0, &[4]),
        ),
        (
            "keyframe after the end",
            encode_log(0, &[(0, 0, 0)], 0, &[9]),
        ),
        (
            "keyframes descending",
            encode_log(0, &[(0, 0, 0), (1, 0, 0), (2, 0, 0)], 0, &[2, 1]),
        ),
        (
            "keyframe twice",
            encode_log(0, &[(0, 0, 0), (1, 0, 0)], 0, &[1, 1]),
        ),
    ];
    for (what, bytes) in bad {
        assert!(read(&bytes).is_err(), "accepted a log with {what}");
    }
    // Huge declared lengths must fail on the missing data, not allocate.
    let mut huge = ok.clone();
    let app_len_at = 6;
    huge[app_len_at..app_len_at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read(&huge).is_err());
    // Any truncation and any single flipped byte is either rejected or still a log that can
    // be handled without panicking.
    for cut in 0..ok.len() {
        assert!(read(&ok[..cut]).is_err(), "truncated at {cut}");
    }
    for i in 0..ok.len() {
        let mut flipped = ok.clone();
        flipped[i] ^= 0xFF;
        if let Ok(log) = read(&flipped) {
            let mut g = Game::default();
            let _ = seek(&mut g, &log, log.end_tick());
            let _ = verify(&mut g, &log);
        }
    }
}

#[test]
fn seek_uses_the_newest_keyframe_and_replays_only_the_rest() {
    struct Counting {
        game: Game,
        steps: u32,
    }
    impl Simulation for Counting {
        type Command = Cmd;
        fn save(&self, out: &mut SnapshotBuf) {
            self.game.save(out);
        }
        fn restore(&mut self, bytes: &[u8]) -> Result<(), SnapshotError> {
            self.game.restore(bytes)
        }
        fn step(&mut self, tick: Tick, commands: &[Cmd]) {
            self.steps += 1;
            self.game.step(tick, commands);
        }
    }
    let (_, log) = record(
        1000,
        RecordOptions {
            checksum_every: 0,
            keyframe_every: 100,
            window: None,
        },
    );
    let mut sim = Counting {
        game: Game::default(),
        steps: 0,
    };
    seek(&mut sim, &log, Tick(950)).unwrap();
    assert_eq!(sim.steps, 50);
    sim.steps = 0;
    seek(&mut sim, &log, Tick(900)).unwrap();
    assert_eq!(sim.steps, 0);
    seek(&mut sim, &log, Tick(99)).unwrap();
    assert_eq!(sim.steps, 99);
}

#[test]
fn flight_recorder_window_edges() {
    // Window shorter than, equal to and longer than the keyframe distance; window 0.
    for (window, keyframe_every) in [(0u64, 10u32), (1, 10), (10, 10), (11, 10), (35, 10), (5, 1)] {
        let opts = RecordOptions {
            checksum_every: 3,
            keyframe_every,
            window: Some(window),
        };
        let (final_state, log) = record(207, opts);
        let span = log.end_tick().0 - log.start().tick.0;
        assert!(
            span >= window && span < window + 2 * keyframe_every as u64,
            "window {window}, keyframes every {keyframe_every}: kept {span} ticks"
        );
        assert_eq!(log.ticks().len() as u64, span);
        assert!(log.keyframes().iter().all(|k| k.tick > log.start().tick));
        verify(&mut Game::default(), &log).unwrap();
        let mut bytes = Vec::new();
        log.write(&mut bytes);
        assert_eq!(ReplayLog::<Cmd>::read(&bytes).unwrap(), log);
        let mut end = Game::default();
        seek(&mut end, &log, log.end_tick()).unwrap();
        assert_eq!(state_bytes(&end), state_bytes(&final_state));
        let mut first = Game::default();
        seek(&mut first, &log, log.start().tick).unwrap();
        assert_eq!(state_bytes(&first), log.start().bytes);
    }
}

#[test]
fn minimize_result_is_one_minimal() {
    // Failure needs three specific commands out of many; check 1-minimality directly.
    let (_, log) = record(150, RecordOptions::default());
    let fails = |candidate: &ReplayLog<Cmd>| {
        let pushes = candidate
            .ticks()
            .iter()
            .flat_map(|t| candidate.commands_of(t))
            .filter(|c| matches!(c, Cmd::Push { .. }))
            .count();
        let spawns = candidate
            .ticks()
            .iter()
            .flat_map(|t| candidate.commands_of(t))
            .filter(|c| matches!(c, Cmd::Spawn { .. }))
            .count();
        pushes >= 2 && spawns >= 1
    };
    assert!(fails(&log));
    let min = minimize(&log, fails);
    assert!(fails(&min));
    assert_eq!(min.command_count(), 3);
    for drop in 0..min.command_count() {
        assert!(
            !fails(&min.filtered(|i| i != drop)),
            "command {drop} is redundant"
        );
    }
    // Nothing to minimise: an empty log and a log that fails without commands.
    let empty = minimize(&log.filtered(|_| false), |_| false);
    assert_eq!(empty.command_count(), 0);
    assert_eq!(minimize(&log, |_| true).command_count(), 0);
    // A log that does not fail at all is returned unchanged.
    assert_eq!(
        minimize(&log, |_| false).command_count(),
        log.command_count()
    );
}

#[cfg(target_pointer_width = "64")]
#[test]
#[should_panic(expected = "more than u32::MAX commands")]
fn command_index_overflow_is_reported_not_wrapped() {
    // Zero-sized commands make 2^32 of them cheap.
    struct Idle;
    impl Simulation for Idle {
        type Command = ();
        fn save(&self, _: &mut SnapshotBuf) {}
        fn restore(&mut self, _: &[u8]) -> Result<(), SnapshotError> {
            Ok(())
        }
        fn step(&mut self, _: Tick, _: &[()]) {}
    }
    let mut rec = Recorder::start(
        &Idle,
        Tick(0),
        ReplayHeader::default(),
        RecordOptions {
            checksum_every: 0,
            keyframe_every: 0,
            window: None,
        },
    );
    let many = vec![(); u32::MAX as usize];
    rec.step(&mut Idle, Tick(0), &many);
    rec.step(&mut Idle, Tick(1), &[(), ()]);
}
