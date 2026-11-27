//! Short stereo room echo on one delay line (Wizardry 7 rewrite, `mix_reflections`).

use crate::Room;
use crate::ramp::Ramp;

/// Feedback tap of the echo line in milliseconds.
const FEEDBACK_MS: u32 = 67;
/// Longest tap a [`Room`] may use in milliseconds.
const MAX_TAP_MS: u32 = 150;
/// Fade between rooms in milliseconds.
const FADE_MS: u32 = 50;

#[derive(Debug)]
pub(crate) struct Echo {
    rate: u32,
    line: Vec<f32>,
    pos: usize,
    taps: [usize; 3],
    feedback: usize,
    input: f32,
    level: Ramp,
    next: Option<Room>,
    /// The line went unfed while silent and holds old sound.
    stale: bool,
}

impl Echo {
    pub(crate) fn new(rate: u32) -> Self {
        let len = (rate * (MAX_TAP_MS + FEEDBACK_MS) / 1000 + 1).next_power_of_two() as usize;
        Self {
            rate,
            line: vec![0.0; len],
            pos: 0,
            taps: [0; 3],
            feedback: (rate * FEEDBACK_MS / 1000) as usize,
            input: 0.0,
            level: Ramp::at(0.0),
            next: None,
            stale: false,
        }
    }

    fn frames(&self, ms: u32) -> usize {
        (self.rate * ms / 1000) as usize
    }

    /// Fades the current room out; [`Echo::settle`] then takes over `room`.
    pub(crate) fn switch(&mut self, room: Room) {
        self.next = Some(room);
        self.level.to(0.0, self.frames(FADE_MS) as u32);
    }

    /// At a block start: adopt a pending room once the old one is silent.
    pub(crate) fn settle(&mut self) {
        if let Some(room) = self.next.filter(|_| self.level.silent()) {
            if self.stale {
                self.line.fill(0.0);
                self.input = 0.0;
                self.stale = false;
            }
            self.taps = room
                .taps_ms
                .map(|t| self.frames(u32::from(t).min(MAX_TAP_MS)));
            self.level.to(room.level, self.frames(FADE_MS) as u32);
            self.next = None;
        }
    }

    pub(crate) fn active(&self) -> bool {
        !self.level.silent()
    }

    /// Called for output stretches without echo: the line was not fed.
    pub(crate) fn idle(&mut self) {
        self.stale = true;
    }

    #[inline]
    pub(crate) fn frame(&mut self, send: f32) -> [f32; 2] {
        let (pos, mask) = (self.pos, self.line.len() - 1);
        let tap = |line: &[f32], d: usize| line[pos.wrapping_sub(d) & mask];
        self.input += (send - self.input) * 0.18;
        self.line[pos] = self.input + 0.16 * tap(&self.line, self.feedback);
        let level = self.level.next();
        let late = 0.40 * tap(&self.line, self.taps[2]);
        self.pos = (pos + 1) & mask;
        [
            level * (tap(&self.line, self.taps[0]) + late),
            level * (0.86 * tap(&self.line, self.taps[1]) + late),
        ]
    }
}
