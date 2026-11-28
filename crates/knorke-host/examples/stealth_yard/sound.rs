//! The yard's sound (ADR 0022 step 7, feature `audio`): the game reads what the simulation
//! did and gives it to the host's mixer as commands. Sound only listens; nothing flows back
//! into the simulation, so a recording replays with or without it.
//!
//! The listener is the intruder. Its own steps sound unplaced and soft; a guard's steps,
//! its alarm and the catch come from where the guard stands. All clips are made in code.

use std::f32::consts::TAU;

use knorke_host::audio::{Bus, Clip, Command, Config, Listener, Output, Play, SoundId, VoiceId};
use knorke_integration::prelude::Cell;
use knorke_integration::stealth::{CAUGHT, Event, Sim};
use knorke_math::{At, Vec2, World2, det};
use knorke_types::EventCursor;

const RATE: u32 = 48_000;
const WIND: SoundId = SoundId(0);
const STEP: SoundId = SoundId(1);
const ALARM: SoundId = SoundId(2);
const CATCH: SoundId = SoundId(3);
/// The looping wind keeps its voice; one-shots count up from the next one.
const WIND_VOICE: VoiceId = VoiceId(0);

/// What the sound side remembers between frames.
pub(crate) struct Sound {
    out: Output,
    events: EventCursor,
    intruder: Cell,
    guards: Vec<Cell>,
    next_voice: u32,
}

impl Sound {
    /// The mixer settings and clips the host opens the device with.
    pub(crate) fn device() -> (Config, Vec<Clip>) {
        let cfg = Config {
            rate: RATE,
            ..Config::default()
        };
        (cfg, vec![wind(), step(), alarm(), catch()])
    }

    /// Starts the wind; `out` is the device the host opened with [`Sound::device`].
    pub(crate) fn new(out: Output, sim: &Sim) -> Self {
        let actors = &sim.world.actors;
        let sound = Self {
            out,
            events: actors.events.cursor_at_end(),
            intruder: actors.intruder,
            guards: actors.guards.iter().map(|g| g.cell).collect(),
            next_voice: 1,
        };
        sound.send(Command::Play {
            voice: WIND_VOICE,
            play: Play {
                sound: WIND,
                gain: 0.25,
                looped: true,
                priority: 200,
                fade_in: RATE,
                ..Play::default()
            },
        });
        sound
    }

    /// Turns what happened since the last call into commands. Called after the ticks of a
    /// frame; a frame without a tick sends nothing.
    pub(crate) fn update(&mut self, sim: &Sim) {
        let actors = &sim.world.actors;
        if actors.intruder != self.intruder {
            self.intruder = actors.intruder;
            self.send(Command::Listener(Listener {
                at: at(self.intruder),
                // The yard is y-down: north is -y, so the right of north is +x.
                forward: Vec2::new(0.0, -1.0),
                right: Vec2::new(1.0, 0.0),
                ..Listener::default()
            }));
            self.one_shot(STEP, None, 0.35, 1.0);
        }
        for (i, g) in actors.guards.iter().enumerate() {
            if self.guards[i] != g.cell {
                self.guards[i] = g.cell;
                self.one_shot(STEP, Some(g.cell), 0.6, 0.8);
            }
        }
        // A cursor that fell behind retired events just skips them: missing a beep is fine.
        let events = actors.events.read(&mut self.events).since().unwrap_or(&[]);
        for e in events {
            match *e {
                Event::Spotted { guard, .. } => {
                    let cell = self.guards[guard as usize];
                    self.one_shot(ALARM, Some(cell), 0.7, 1.0);
                }
                Event::State { guard, to, .. } if to == CAUGHT => {
                    let cell = self.guards[guard as usize];
                    self.one_shot(CATCH, Some(cell), 0.9, 1.0);
                }
                Event::State { .. } => {}
            }
        }
    }

    fn one_shot(&mut self, sound: SoundId, from: Option<Cell>, gain: f32, pitch: f32) {
        let voice = VoiceId(self.next_voice);
        self.next_voice = self.next_voice.wrapping_add(1).max(1);
        self.send(Command::Play {
            voice,
            play: Play {
                sound,
                bus: Bus(0),
                gain,
                pitch,
                at: from.map(at),
                priority: 100,
                limit: 4,
                ..Play::default()
            },
        });
    }

    fn send(&self, cmd: Command) {
        // A full inbox means the device stalls; dropping a sound is better than waiting.
        let _ = self.out.send(cmd);
    }
}

fn at(c: Cell) -> At<World2, Vec2> {
    At::new(Vec2::new(c.x as f32, c.y as f32))
}

/// Deterministic white noise in `-1..=1`.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

fn mono(samples: Vec<f32>) -> Clip {
    Clip {
        samples,
        rate: RATE,
        stereo: false,
    }
}

/// Four seconds of night wind: heavily smoothed noise that swells twice, as a loop.
fn wind() -> Clip {
    let n = 4 * RATE as usize;
    let (mut noise, mut lp) = (Noise(3), 0.0f32);
    mono(
        (0..n)
            .map(|i| {
                lp += 0.02 * (noise.next() - lp);
                let swell = 0.6 + 0.4 * det::sin(TAU * 2.0 * i as f32 / n as f32);
                3.0 * lp * swell
            })
            .collect(),
    )
}

/// A soft step on gravel: a short burst of dull noise.
fn step() -> Clip {
    let n = RATE as usize / 12;
    let (mut noise, mut lp, mut env) = (Noise(11), 0.0f32, 1.0f32);
    mono(
        (0..n)
            .map(|_| {
                lp += 0.3 * (noise.next() - lp);
                env *= 0.9992;
                lp * env
            })
            .collect(),
    )
}

/// Two quick high beeps: the guard has seen you.
fn alarm() -> Clip {
    let beep = RATE as usize / 8;
    mono(
        (0..3 * beep)
            .map(|i| {
                let on = i < beep || i >= 2 * beep;
                let f = if i < beep { 880.0 } else { 1175.0 };
                if on {
                    0.5 * det::sin(TAU * f * i as f32 / RATE as f32)
                } else {
                    0.0
                }
            })
            .collect(),
    )
}

/// A low falling tone: caught.
fn catch() -> Clip {
    let n = RATE as usize;
    let (mut phase, mut env) = (0.0f32, 1.0f32);
    mono(
        (0..n)
            .map(|i| {
                let f = 330.0 - 200.0 * (i as f32 / n as f32);
                phase = (phase + TAU * f / RATE as f32) % TAU;
                env *= 0.99995;
                0.7 * env * (det::sin(phase) + 0.3 * det::sin(3.0 * phase))
            })
            .collect(),
    )
}
