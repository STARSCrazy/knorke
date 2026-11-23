use crate::Simulation;
use crate::log::{Keyframe, ReplayHeader, ReplayLog, TickRecord};
use alloc::string::String;
use alloc::vec::Vec;
use knorke_types::{Checksum, SnapshotBuf, SnapshotError, Tick};

/// How a [`Recorder`] records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordOptions {
    /// Record section checksums after every N-th tick (0 = never). 1 pinpoints a divergence
    /// to the exact tick; larger values trade precision for speed.
    pub checksum_every: u32,
    /// Store a keyframe before every N-th tick (0 = never). Bounds the cost of a seek.
    pub keyframe_every: u32,
    /// Flight-recorder mode: keep at least this many of the newest ticks and drop older
    /// history at keyframe boundaries. `None` keeps everything.
    pub window: Option<u64>,
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self {
            checksum_every: 1,
            keyframe_every: 600,
            window: None,
        }
    }
}

/// Records a running simulation into a [`ReplayLog`].
pub struct Recorder<C> {
    log: ReplayLog<C>,
    opts: RecordOptions,
    scratch: SnapshotBuf,
    spare_keyframes: Vec<Vec<u8>>,
}

impl<C: Clone> Recorder<C> {
    /// Starts recording; saves the current state of `sim` as the start keyframe.
    pub fn start<S: Simulation<Command = C>>(
        sim: &S,
        first_tick: Tick,
        header: ReplayHeader,
        opts: RecordOptions,
    ) -> Self {
        let mut scratch = SnapshotBuf::new();
        sim.save(&mut scratch);
        let start = Keyframe {
            tick: first_tick,
            bytes: scratch.bytes().to_vec(),
        };
        Self {
            log: ReplayLog::new(header, start),
            opts,
            scratch,
            spare_keyframes: Vec::new(),
        }
    }

    /// Records and executes one tick: keyframe (if due), `sim.step(tick, commands)`,
    /// checksums (if due), flight-recorder trimming (if configured).
    ///
    /// # Panics
    /// If the log would hold more than `u32::MAX` commands or checksums. A recorder with a
    /// `window` never gets there.
    pub fn step<S: Simulation<Command = C>>(&mut self, sim: &mut S, tick: Tick, commands: &[C]) {
        debug_assert_eq!(
            tick,
            self.log.end_tick(),
            "Recorder: ticks must be consecutive"
        );
        let n = tick.since(self.log.start.tick).unwrap_or(0);
        let every = |e: u32| e != 0 && n != 0 && n.is_multiple_of(e as u64);
        if every(self.opts.keyframe_every) {
            self.scratch.clear();
            sim.save(&mut self.scratch);
            let mut bytes = self.spare_keyframes.pop().unwrap_or_default();
            bytes.clear();
            bytes.extend_from_slice(self.scratch.bytes());
            self.log.keyframes.push(Keyframe { tick, bytes });
        }
        let from = self.log.command_index();
        self.log.commands.extend_from_slice(commands);
        let to = self.log.command_index();
        sim.step(tick, commands);
        let sums_from = self.log.sum_index();
        if self.opts.checksum_every != 0 && (n + 1).is_multiple_of(self.opts.checksum_every as u64)
        {
            self.scratch.clear();
            sim.save(&mut self.scratch);
            for (id, sum) in self.scratch.section_checksums() {
                let i = self.log.section_index(id);
                self.log.sums.push((i, sum));
            }
        }
        self.log.ticks.push(TickRecord {
            tick,
            commands: from..to,
            sums: sums_from..self.log.sum_index(),
        });
        if let Some(window) = self.opts.window {
            self.trim(tick, window);
        }
    }

    /// Drops history before the newest keyframe that still leaves `window` ticks.
    fn trim(&mut self, now: Tick, window: u64) {
        let Some(k) = self
            .log
            .keyframes
            .iter()
            .rposition(|k| now.0 + 1 - k.tick.0 >= window)
        else {
            return;
        };
        let new_start = self.log.keyframes.remove(k);
        for old in self.log.keyframes.drain(..k) {
            self.spare_keyframes.push(old.bytes);
        }
        let old_start = core::mem::replace(&mut self.log.start, new_start);
        self.spare_keyframes.push(old_start.bytes);
        let first = self
            .log
            .ticks
            .partition_point(|t| t.tick < self.log.start.tick);
        let (cmd_cut, sum_cut) = match self.log.ticks.get(first) {
            Some(t) => (t.commands.start, t.sums.start),
            None => (self.log.command_index(), self.log.sum_index()),
        };
        self.log.ticks.drain(..first);
        self.log.commands.drain(..cmd_cut as usize);
        self.log.sums.drain(..sum_cut as usize);
        for t in &mut self.log.ticks {
            t.commands = t.commands.start - cmd_cut..t.commands.end - cmd_cut;
            t.sums = t.sums.start - sum_cut..t.sums.end - sum_cut;
        }
    }

    /// The log so far.
    pub fn log(&self) -> &ReplayLog<C> {
        &self.log
    }

    /// Finishes recording.
    pub fn into_log(self) -> ReplayLog<C> {
        self.log
    }
}

/// How the host advances its simulation: recorded or not. Write the game loop once against
/// `impl TickDriver<S>` and choose at compile time ([`NoRecord`] / [`Recorder`]) or at run
/// time (`Option<Recorder<_>>`). [`NoRecord`] compiles to a plain `sim.step` call.
pub trait TickDriver<S: Simulation> {
    /// Advances `sim` by one tick with `commands`.
    fn step(&mut self, sim: &mut S, tick: Tick, commands: &[S::Command]);
}

/// Recording switched off: forwards straight to [`Simulation::step`], zero cost.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRecord;

impl<S: Simulation> TickDriver<S> for NoRecord {
    #[inline(always)]
    fn step(&mut self, sim: &mut S, tick: Tick, commands: &[S::Command]) {
        sim.step(tick, commands);
    }
}

impl<S: Simulation> TickDriver<S> for Recorder<S::Command> {
    #[inline]
    fn step(&mut self, sim: &mut S, tick: Tick, commands: &[S::Command]) {
        Recorder::step(self, sim, tick, commands);
    }
}

/// Run-time switch: `None` behaves like [`NoRecord`] (one predictable branch per tick).
impl<S: Simulation, D: TickDriver<S>> TickDriver<S> for Option<D> {
    #[inline]
    fn step(&mut self, sim: &mut S, tick: Tick, commands: &[S::Command]) {
        match self {
            Some(d) => d.step(sim, tick, commands),
            None => sim.step(tick, commands),
        }
    }
}

/// First point where a replay differs from its recording.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Divergence {
    /// The tick after which the state differs.
    pub tick: Tick,
    /// Sections (modules) whose checksum differs, appeared or disappeared.
    pub sections: Vec<String>,
}

/// Why [`verify`] failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerifyError {
    /// A keyframe could not be restored.
    Restore(SnapshotError),
    /// The replay diverged.
    Diverged(Divergence),
}

/// Result of a successful [`verify`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    /// Ticks replayed.
    pub ticks: usize,
    /// Ticks whose checksums were compared.
    pub checked: usize,
}

/// Replays `log` on `sim` from its start state and compares every recorded checksum.
/// Stops at the first divergence and names the diverging sections.
pub fn verify<S: Simulation>(
    sim: &mut S,
    log: &ReplayLog<S::Command>,
) -> Result<VerifyReport, VerifyError> {
    sim.restore(&log.start.bytes)
        .map_err(VerifyError::Restore)?;
    let mut scratch = SnapshotBuf::new();
    let mut checked = 0;
    for t in &log.ticks {
        sim.step(t.tick, log.commands_of(t));
        if t.sums.is_empty() {
            continue;
        }
        checked += 1;
        scratch.clear();
        sim.save(&mut scratch);
        let actual: Vec<(&str, Checksum)> = scratch.section_checksums().collect();
        let mut sections: Vec<String> = Vec::new();
        for (id, expected) in log.checksums_of(t) {
            if actual.iter().find(|(a, _)| *a == id).map(|(_, c)| *c) != Some(expected) {
                sections.push(String::from(id));
            }
        }
        for (id, _) in &actual {
            if !log.checksums_of(t).any(|(e, _)| e == *id) {
                sections.push(String::from(*id));
            }
        }
        if !sections.is_empty() {
            return Err(VerifyError::Diverged(Divergence {
                tick: t.tick,
                sections,
            }));
        }
    }
    Ok(VerifyReport {
        ticks: log.ticks.len(),
        checked,
    })
}

/// Puts `sim` into the state at the start of `target` (before `step(target)`): restores the
/// newest keyframe at or before `target`, then replays the remaining ticks.
pub fn seek<S: Simulation>(
    sim: &mut S,
    log: &ReplayLog<S::Command>,
    target: Tick,
) -> Result<(), SnapshotError> {
    if target < log.start.tick || target > log.end_tick() {
        return Err(SnapshotError::Invalid(
            "seek target outside the recorded range",
        ));
    }
    let key = log
        .keyframes
        .iter()
        .rev()
        .find(|k| k.tick <= target)
        .unwrap_or(&log.start);
    sim.restore(&key.bytes)?;
    // Ticks are ascending: find the stretch to replay without scanning the whole log.
    let from = log.ticks.partition_point(|t| t.tick < key.tick);
    let to = log.ticks.partition_point(|t| t.tick < target);
    for t in &log.ticks[from..to.max(from)] {
        sim.step(t.tick, log.commands_of(t));
    }
    Ok(())
}
