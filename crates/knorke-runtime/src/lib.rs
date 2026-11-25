//! Runs a deterministic simulation (ADR 0003, ADR 0009).
//!
//! - [`FixedStep`] turns the time the host measured into whole ticks plus the fraction left
//!   over for interpolating what is drawn. It owns no clock.
//! - [`Runtime`] is the frame loop around a [`Simulation`]: commands submitted by the host
//!   enter at the next tick, ticks run through a [`TickDriver`] (recorded or not).
//! - [`System`] and [`run_systems`] are the schedule of one tick: named steps in a fixed
//!   order, each inside its [`Probe`] scope.
//!
//! There is no scheduler that orders or parallelises systems on its own: the order is the
//! table the game writes (ADR 0009). A system parallelises inside through `Parallel`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use knorke_observe::{Probe, Scope};
use knorke_replay::{NoRecord, Simulation, TickDriver};
use knorke_types::Tick;

/// Turns elapsed host time into whole ticks. The host measures the time (ADR 0003).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedStep {
    tick_nanos: u64,
    max_ticks: u32,
    /// Time not yet spent on a tick, always below `tick_nanos`.
    pending: u64,
}

impl FixedStep {
    /// A tick lasts `tick_nanos`; one [`advance`](Self::advance) yields at most `max_ticks`.
    ///
    /// # Panics
    /// If `tick_nanos` is 0.
    pub const fn new(tick_nanos: u64, max_ticks: u32) -> Self {
        assert!(tick_nanos > 0, "a tick needs a length");
        Self {
            tick_nanos,
            max_ticks,
            pending: 0,
        }
    }

    /// Adds `elapsed_nanos` and returns the ticks due now. Time beyond `max_ticks` is
    /// dropped: a host that falls behind runs the game slower instead of piling up ticks
    /// it can never catch up on.
    pub fn advance(&mut self, elapsed_nanos: u64) -> u32 {
        let total = self.pending.saturating_add(elapsed_nanos);
        let due = total / self.tick_nanos;
        if due > u64::from(self.max_ticks) {
            self.pending = 0;
            self.max_ticks
        } else {
            self.pending = total % self.tick_nanos;
            due as u32
        }
    }

    /// How far the host is into the next tick, `0.0 ..< 1.0`: the weight for blending the
    /// last two simulation states when drawing. Never feed it back into the simulation.
    pub fn alpha(&self) -> f32 {
        self.pending as f32 / self.tick_nanos as f32
    }
}

/// What one [`Runtime::frame`] did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Ticks simulated in this frame.
    pub ticks: u32,
    /// See [`FixedStep::alpha`].
    pub alpha: f32,
}

/// The frame loop around a simulation `S`, stepped through the driver `D`
/// ([`NoRecord`], a `Recorder`, or an `Option` of one as a run-time switch).
pub struct Runtime<S: Simulation, D = NoRecord> {
    /// The simulation. Read it freely; change it only through [`submit`](Self::submit),
    /// or a recording no longer replays.
    pub sim: S,
    /// The driver, e.g. to take the log out of a recorder.
    pub driver: D,
    step: FixedStep,
    next: Tick,
    /// Commands of the next tick; the buffer is reused.
    inbox: Vec<S::Command>,
}

impl<S: Simulation, D: TickDriver<S>> Runtime<S, D> {
    /// A runtime whose first tick is `first`. A recorder has to be started on `sim` with the
    /// same tick.
    pub fn new(sim: S, driver: D, step: FixedStep, first: Tick) -> Self {
        Self {
            sim,
            driver,
            step,
            next: first,
            inbox: Vec::new(),
        }
    }

    /// Hands in a command from outside the simulation (player, network, agent, finished
    /// load). It is applied at the next tick, in the order submitted.
    pub fn submit(&mut self, command: S::Command) {
        self.inbox.push(command);
    }

    /// The tick the next step runs.
    pub fn next_tick(&self) -> Tick {
        self.next
    }

    /// Continues at `tick`, after `sim` was put into the state before it (`seek`, a loaded
    /// save). Commands submitted but not yet run stay queued.
    pub fn jump(&mut self, tick: Tick) {
        self.next = tick;
    }

    /// Runs exactly one tick with all submitted commands: for turn-based games, headless
    /// runs and tests.
    pub fn tick(&mut self) {
        self.driver.step(&mut self.sim, self.next, &self.inbox);
        self.inbox.clear();
        self.next = self.next.next();
    }

    /// Runs the ticks that are due after `elapsed_nanos` of host time. Submitted commands
    /// go into the first of them and wait if none is due.
    pub fn frame(&mut self, elapsed_nanos: u64) -> Frame {
        let ticks = self.step.advance(elapsed_nanos);
        for _ in 0..ticks {
            self.tick();
        }
        Frame {
            ticks,
            alpha: self.step.alpha(),
        }
    }
}

/// One step of a tick over the world `W`.
pub struct System<W> {
    /// Its name for profiling and inspection.
    pub scope: &'static Scope,
    /// Runs it. Called once per tick, not per entity.
    pub run: fn(&mut W, Tick),
}

/// Runs `systems` in the order given, each inside its probe scope. The table is the
/// schedule: a system sees everything the ones before it wrote in the same tick.
pub fn run_systems<W>(world: &mut W, tick: Tick, systems: &[System<W>], probe: &mut impl Probe) {
    for system in systems {
        probe.enter(system.scope);
        (system.run)(world, tick);
        probe.exit(system.scope);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_step_carries_the_rest_and_caps_a_stall() {
        let mut step = FixedStep::new(10, 3);
        assert_eq!(step.advance(4), 0);
        assert_eq!(step.alpha(), 0.4);
        assert_eq!(step.advance(17), 2);
        assert_eq!(step.alpha(), 0.1);
        // 100 would be ten ticks: capped, and the backlog is gone.
        assert_eq!(step.advance(100), 3);
        assert_eq!(step.alpha(), 0.0);
        assert_eq!(step.advance(u64::MAX), 3);
    }
}
