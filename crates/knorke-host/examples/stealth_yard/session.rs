//! The loop of the stealth yard without the window (ADR 0022 steps 5 and 6): device events and
//! host time in, ticks out. `main.rs` feeds it from SDL; `tests/stealth_session.rs` feeds it
//! a made-up input stream, so the recorded session can be checked without a window.

use knorke_input::{Bindings, Button, InputEvent, InputState, Scancode, Trigger};
use knorke_integration::stealth::{Command, Dir, Sim};
use knorke_replay::{RecordOptions, Recorder, ReplayHeader, ReplayLog};
use knorke_runtime::{FixedStep, Frame, Runtime};
use knorke_types::Tick;

/// Steps per second; a held arrow key walks this fast.
pub const TICKS_PER_SECOND: u64 = 6;

/// A running game: the runtime, recorded or not, and what turns keys into commands.
pub struct Session {
    /// The runtime; its `sim` is what is drawn.
    pub rt: Runtime<Sim, Option<Recorder<Command>>>,
    bindings: Bindings<Command>,
    input: InputState,
}

impl Session {
    /// Starts on `sim`; with `record` the session is recorded under that header.
    pub fn new(sim: Sim, record: Option<ReplayHeader>) -> Self {
        // Stage 2 of ADR 0022: keys → actions, a table the game owns. Here the actions
        // already are the commands (stage 3 is the identity).
        let mut bindings = Bindings::new();
        for (key, dir) in [
            (Scancode::UP, Dir::North),
            (Scancode::RIGHT, Dir::East),
            (Scancode::DOWN, Dir::South),
            (Scancode::LEFT, Dir::West),
        ] {
            bindings.bind(Button::Key(key), Trigger::Held, Command::Step(dir));
        }
        let recorder = record
            .map(|header| Recorder::start(&sim, Tick::ZERO, header, RecordOptions::default()));
        let step = FixedStep::new(1_000_000_000 / TICKS_PER_SECOND, 4);
        Self {
            rt: Runtime::new(sim, recorder, step, Tick::ZERO),
            bindings,
            input: InputState::new(),
        }
    }

    /// One frame: the device events since the last frame, then `elapsed_nanos` of host time.
    pub fn frame(&mut self, events: &[InputEvent], elapsed_nanos: u64) -> Frame {
        for ev in events {
            self.input.apply(*ev);
        }
        // Sample the input only for a frame that runs a tick: one command per tick, and a
        // tap between two ticks keeps its edge until then. That is why the command stream
        // does not depend on the frame rate.
        if self.rt.ticks_due(elapsed_nanos) > 0 {
            for command in self.bindings.actions(&self.input) {
                self.rt.submit(command);
            }
        }
        let due = self.rt.frame(elapsed_nanos);
        if due.ticks > 0 {
            self.input.end_tick();
        }
        due
    }

    /// The recording, if the session was recorded.
    pub fn into_log(self) -> Option<ReplayLog<Command>> {
        self.rt.driver.map(Recorder::into_log)
    }
}
