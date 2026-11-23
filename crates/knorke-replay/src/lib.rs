//! Deterministic replays (ADR 0010).
//!
//! One artefact serves as bug report, regression test, fuzzing result, flight-recorder dump
//! and agent input: a [`ReplayLog`] = start state + commands per tick + checksums per module
//! section + keyframes for seeking.
//!
//! - [`Recorder`] records a running [`Simulation`]; with a window it is a **flight recorder**
//!   that keeps only the last N ticks and reuses its buffers.
//! - [`verify`] replays a log and reports the **first diverging tick and the diverging
//!   modules** (sections).
//! - [`seek`] jumps to any tick via the nearest keyframe (time-travel debugging).
//! - [`TickDriver`] lets the host switch recording off ([`NoRecord`]) at zero cost.
//! - [`minimize`] shrinks a failing replay to the commands that still trigger the failure
//!   (delta debugging on command level).
//!
//! Requirement on the simulation: every non-deterministic input (players, network, agents,
//! LLM answers, finished asset loads, wall-clock time) enters **only** as a command of a tick.

#![no_std]

extern crate alloc;

mod log;
mod minimize;
mod record;

pub use log::{FORMAT_VERSION, Keyframe, ReplayHeader, ReplayLog, TickRecord};
pub use minimize::minimize;
pub use record::{
    Divergence, NoRecord, RecordOptions, Recorder, TickDriver, VerifyError, VerifyReport, seek,
    verify,
};

use knorke_types::{SnapshotBuf, SnapshotError, SnapshotValue, Tick};

/// A deterministic simulation that can be recorded and replayed.
pub trait Simulation {
    /// External input of one tick (player input, agent transaction, network message, ...).
    type Command: SnapshotValue + Clone;

    /// Writes the complete simulation state, one section per module.
    fn save(&self, out: &mut SnapshotBuf);

    /// Replaces the complete state with one written by [`Simulation::save`].
    fn restore(&mut self, bytes: &[u8]) -> Result<(), SnapshotError>;

    /// Advances one tick, applying `commands` (in order) at the tick's sync point.
    fn step(&mut self, tick: Tick, commands: &[Self::Command]);
}
