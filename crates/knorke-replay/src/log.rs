use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;
use knorke_types::{Checksum, Reader, Seed, SnapshotError, SnapshotValue, Tick, Writer};

/// Version of the serialized replay format.
pub const FORMAT_VERSION: u16 = 1;
const MAGIC: &[u8; 4] = b"KNRP";

/// Identifies what produced a replay.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplayHeader {
    /// Engine/content version or build id. A replay is only guaranteed to verify against
    /// the same version.
    pub app: String,
    /// Seed of the run (informational; the start state already contains all seeds).
    pub seed: Seed,
}

/// Full simulation state at the start of `tick` (before `step(tick)`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keyframe {
    /// Tick this state precedes.
    pub tick: Tick,
    /// Bytes from [`Simulation::save`](crate::Simulation::save).
    pub bytes: Vec<u8>,
}

/// What happened in one tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickRecord {
    /// The tick.
    pub tick: Tick,
    pub(crate) commands: Range<u32>,
    /// Checksums of the state *after* this tick, per section; empty if not recorded.
    pub(crate) sums: Range<u32>,
}

/// A recorded run: start state, commands per tick, checksums, keyframes.
#[derive(Clone, Debug, PartialEq)]
pub struct ReplayLog<C> {
    /// Producer information.
    pub header: ReplayHeader,
    pub(crate) start: Keyframe,
    pub(crate) ticks: Vec<TickRecord>,
    pub(crate) commands: Vec<C>,
    pub(crate) section_ids: Vec<String>,
    pub(crate) sums: Vec<(u16, Checksum)>,
    pub(crate) keyframes: Vec<Keyframe>,
}

impl<C> ReplayLog<C> {
    pub(crate) fn new(header: ReplayHeader, start: Keyframe) -> Self {
        Self {
            header,
            start,
            ticks: Vec::new(),
            commands: Vec::new(),
            section_ids: Vec::new(),
            sums: Vec::new(),
            keyframes: Vec::new(),
        }
    }

    /// State before the first recorded tick.
    pub fn start(&self) -> &Keyframe {
        &self.start
    }

    /// Recorded ticks, oldest first.
    pub fn ticks(&self) -> &[TickRecord] {
        &self.ticks
    }

    /// Commands of a recorded tick.
    pub fn commands_of(&self, t: &TickRecord) -> &[C] {
        &self.commands[t.commands.start as usize..t.commands.end as usize]
    }

    /// Checksums after a recorded tick as `(section id, checksum)`.
    pub fn checksums_of<'a>(
        &'a self,
        t: &TickRecord,
    ) -> impl Iterator<Item = (&'a str, Checksum)> + 'a {
        self.sums[t.sums.start as usize..t.sums.end as usize]
            .iter()
            .map(|(i, c)| (self.section_ids[*i as usize].as_str(), *c))
    }

    /// Additional keyframes (besides the start), ascending by tick.
    pub fn keyframes(&self) -> &[Keyframe] {
        &self.keyframes
    }

    /// Total number of recorded commands.
    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    /// The tick after the last recorded one (where a seek to "the end" lands).
    pub fn end_tick(&self) -> Tick {
        self.ticks.last().map_or(self.start.tick, |t| t.tick.next())
    }

    /// Current length of the command list as a tick-record index.
    ///
    /// # Panics
    /// If the log holds more than `u32::MAX` commands (use a flight-recorder window for
    /// endless recordings).
    pub(crate) fn command_index(&self) -> u32 {
        u32::try_from(self.commands.len()).expect("replay: more than u32::MAX commands")
    }

    /// Current length of the checksum list as a tick-record index. Panics like
    /// [`Self::command_index`].
    pub(crate) fn sum_index(&self) -> u32 {
        u32::try_from(self.sums.len()).expect("replay: more than u32::MAX checksums")
    }

    pub(crate) fn section_index(&mut self, id: &str) -> u16 {
        match self.section_ids.iter().position(|s| s == id) {
            Some(i) => i as u16,
            None => {
                self.section_ids.push(String::from(id));
                u16::try_from(self.section_ids.len() - 1).expect("replay: more than 65535 sections")
            }
        }
    }

    /// Copy containing only the commands whose global index `keep` returns `true` for,
    /// without checksums or keyframes (they no longer match). Used by [`minimize`](crate::minimize).
    pub fn filtered(&self, mut keep: impl FnMut(usize) -> bool) -> Self
    where
        C: Clone,
    {
        let mut out = ReplayLog::new(self.header.clone(), self.start.clone());
        out.ticks.reserve(self.ticks.len());
        for t in &self.ticks {
            let from = out.command_index();
            let start = t.commands.start as usize;
            let end = t.commands.end as usize;
            for (offset, cmd) in self.commands[start..end].iter().enumerate() {
                if keep(start + offset) {
                    out.commands.push(cmd.clone());
                }
            }
            out.ticks.push(TickRecord {
                tick: t.tick,
                commands: from..out.command_index(),
                sums: 0..0,
            });
        }
        out
    }
}

impl<C: SnapshotValue> ReplayLog<C> {
    /// Serializes the log (format [`FORMAT_VERSION`]).
    pub fn write(&self, out: &mut Vec<u8>) {
        let mut w = Writer::new(out);
        w.put_bytes(MAGIC);
        w.put(&FORMAT_VERSION);
        put_str(&mut w, &self.header.app);
        w.put(&self.header.seed);
        put_keyframe(&mut w, &self.start);
        w.put_len(self.section_ids.len());
        for id in &self.section_ids {
            put_str(&mut w, id);
        }
        w.put_len(self.ticks.len());
        for t in &self.ticks {
            w.put(&t.tick);
            w.put(&t.commands.start);
            w.put(&t.commands.end);
            w.put(&t.sums.start);
            w.put(&t.sums.end);
        }
        w.put_slice(&self.commands);
        w.put_slice(&self.sums);
        w.put_len(self.keyframes.len());
        for k in &self.keyframes {
            put_keyframe(&mut w, k);
        }
    }

    /// Reads a log written by [`ReplayLog::write`].
    pub fn read(bytes: &[u8]) -> Result<Self, SnapshotError> {
        let mut r = Reader::new(bytes);
        if r.take(4)? != MAGIC {
            return Err(SnapshotError::Invalid("not a Knorke replay"));
        }
        let version: u16 = r.get()?;
        if version != FORMAT_VERSION {
            return Err(SnapshotError::UnsupportedVersion {
                id: "knorke.replay",
                version,
            });
        }
        let header = ReplayHeader {
            app: get_str(&mut r)?,
            seed: r.get()?,
        };
        let mut log = ReplayLog::new(header, get_keyframe(&mut r)?);
        for _ in 0..r.get_len()? {
            log.section_ids.push(get_str(&mut r)?);
        }
        for _ in 0..r.get_len()? {
            let tick = r.get()?;
            let commands = r.get()?..r.get()?;
            let sums = r.get()?..r.get()?;
            log.ticks.push(TickRecord {
                tick,
                commands,
                sums,
            });
        }
        r.get_into_vec(&mut log.commands)?;
        r.get_into_vec(&mut log.sums)?;
        for _ in 0..r.get_len()? {
            log.keyframes.push(get_keyframe(&mut r)?);
        }
        r.finish("knorke.replay")?;
        let ranges_ok = log.ticks.iter().all(|t| {
            t.commands.start <= t.commands.end
                && t.commands.end as usize <= log.commands.len()
                && t.sums.start <= t.sums.end
                && t.sums.end as usize <= log.sums.len()
        }) && log
            .sums
            .iter()
            .all(|(i, _)| (*i as usize) < log.section_ids.len());
        if !ranges_ok {
            return Err(SnapshotError::Invalid("replay: index out of range"));
        }
        // `seek`, `end_tick` and the recorder rely on these: ticks are consecutive from the
        // start tick, and keyframes are strictly ascending inside the recorded range.
        let mut expected = Some(log.start.tick.0);
        for t in &log.ticks {
            if expected != Some(t.tick.0) {
                return Err(SnapshotError::Invalid(
                    "replay: ticks are not consecutive from the start tick",
                ));
            }
            expected = t.tick.0.checked_add(1);
        }
        let Some(end) = expected else {
            return Err(SnapshotError::Invalid("replay: tick overflows"));
        };
        let mut after = log.start.tick.0;
        for k in &log.keyframes {
            if k.tick.0 <= after || k.tick.0 >= end {
                return Err(SnapshotError::Invalid(
                    "replay: keyframes are not ascending inside the recorded range",
                ));
            }
            after = k.tick.0;
        }
        Ok(log)
    }
}

fn put_str(w: &mut Writer<'_>, s: &str) {
    w.put_len(s.len());
    w.put_bytes(s.as_bytes());
}

fn get_str(r: &mut Reader<'_>) -> Result<String, SnapshotError> {
    let n = r.get_len()?;
    let b = r.take(n)?;
    core::str::from_utf8(b)
        .map(String::from)
        .map_err(|_| SnapshotError::Invalid("replay: string is not UTF-8"))
}

fn put_keyframe(w: &mut Writer<'_>, k: &Keyframe) {
    w.put(&k.tick);
    w.put_len(k.bytes.len());
    w.put_bytes(&k.bytes);
}

fn get_keyframe(r: &mut Reader<'_>) -> Result<Keyframe, SnapshotError> {
    let tick = r.get()?;
    let n = r.get_len()?;
    Ok(Keyframe {
        tick,
        bytes: r.take(n)?.to_vec(),
    })
}
