//! Audio mixer of the Knorke Engine (ADR 0020).
//!
//! [`Mixer`] is a plain struct: no thread, no device, no queue. The host owns the audio device
//! and its callback, carries [`Command`]s from the game thread to the audio thread over a queue
//! of its choice (e.g. a lock-free ring) and there calls [`Mixer::apply`] for each one and then
//! [`Mixer::render`] with the sound data it lends through [`Sounds`]. Output is interleaved
//! stereo `f32`.
//!
//! Cost is fixed at [`Mixer::new`]: `apply` and `render` never allocate. Work happens in blocks
//! of [`BLOCK`] frames on a grid counted from frame 0, so the output does not depend on the
//! callback size, and all math goes through `knorke_math::det`, so a rendered buffer is the same
//! on every platform (usable as a test oracle). Voices too far away to hear only advance their
//! play position.
//!
//! Positional sound follows the model of the Wizardry 7 rewrite (`wizardry7/revised/platform/
//! sdl/audio_sdl.c`): level and brightness fall with distance, walls and sounds behind the
//! listener; with headphones the far ear hears the sound slightly later and duller.
//!
//! Beyond that (what middleware such as Wwise, FMOD or Unreal's Quartz offers, Plan 17.6):
//! sample-exact start on a given output frame for music on the beat ([`Play::start`]), sound
//! arriving late over distance ([`Config::sound_speed`]), a cap per sound against the
//! machine-gun effect ([`Play::limit`]), a filter per bus for underwater, radio or stunned
//! ([`Command::Filter`]) and a bus that ducks under another one by itself ([`Command::Duck`]).

mod bus;
mod echo;
mod mixer;
mod ramp;
mod spatial;
mod voice;

pub use mixer::{Mixer, Stats};

use knorke_math::{At, Vec2, World2};

/// Frames per processing block. Positional values are computed once per block and ramped
/// linearly across it, which keeps them click-free and cheap.
pub const BLOCK: usize = 256;

/// A sound the host lends through [`Sounds`] (index into the host's sound table).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SoundId(pub u32);

/// A playing voice. The game picks the id, so commands need no answer from the audio thread;
/// playing an id that is already playing restarts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoiceId(pub u32);

/// A mix bus (music, effects, interface, …): index below [`Config::buses`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Bus(pub u8);

impl Bus {
    pub(crate) fn index(self) -> usize {
        usize::from(self.0)
    }
}

/// Borrowed PCM: `f32` samples in `-1..=1`, interleaved when `stereo`.
#[derive(Clone, Copy, Debug)]
pub struct Pcm<'a> {
    /// The samples.
    pub samples: &'a [f32],
    /// Sample rate in Hz; 0 marks the sound as unusable.
    pub rate: u32,
    /// Two interleaved channels instead of one.
    pub stereo: bool,
}

impl Pcm<'_> {
    fn frames(&self) -> usize {
        self.samples.len() >> usize::from(self.stereo)
    }
}

/// Where the mixer finds sound data. Old formats (VOC, SND, …) are decoded by the game's loader;
/// the mixer only sees PCM.
pub trait Sounds {
    /// The PCM of `id`, or `None` if there is none (the voice then ends).
    fn pcm(&self, id: SoundId) -> Option<Pcm<'_>>;
}

impl Sounds for [Pcm<'_>] {
    fn pcm(&self, id: SoundId) -> Option<Pcm<'_>> {
        self.get(id.0 as usize).copied()
    }
}

/// How a voice starts.
#[derive(Clone, Copy, Debug)]
pub struct Play {
    /// The sound to play.
    pub sound: SoundId,
    /// The bus it is mixed into.
    pub bus: Bus,
    /// Linear gain.
    pub gain: f32,
    /// Playback speed: 2.0 is an octave up and half as long.
    pub pitch: f32,
    /// Stereo position of an unplaced voice, -1 (left) to 1 (right).
    pub pan: f32,
    /// World position; `None` for music, interface and other unplaced sounds.
    pub at: Option<At<World2, Vec2>>,
    /// Walls between source and listener (fractions allowed, e.g. an open door).
    pub walls: f32,
    /// Start again at the end instead of stopping.
    pub looped: bool,
    /// When all voices are busy, a voice of lower or equal priority is replaced.
    pub priority: u8,
    /// Fade-in length in frames (0 starts at full gain).
    pub fade_in: u32,
    /// Output frame (see [`Mixer::clock`]) the voice starts on; a past frame starts at once.
    /// The game reads the clock, works out the next beat and sends the play ahead of time.
    pub start: u64,
    /// At most this many voices of the same sound at once (0: no cap); a new one replaces the
    /// one furthest through the sound.
    pub limit: u8,
}

impl Default for Play {
    fn default() -> Self {
        Self {
            sound: SoundId(0),
            bus: Bus(0),
            gain: 1.0,
            pitch: 1.0,
            pan: 0.0,
            at: None,
            walls: 0.0,
            looped: false,
            priority: 0,
            fade_in: 0,
            start: 0,
            limit: 0,
        }
    }
}

/// Everything the game can tell the mixer. Fade lengths are in frames.
#[derive(Clone, Copy, Debug)]
pub enum Command {
    /// Starts (or restarts) `voice`.
    Play {
        /// The voice.
        voice: VoiceId,
        /// How it starts.
        play: Play,
    },
    /// Fades `voice` out and ends it.
    Stop {
        /// The voice.
        voice: VoiceId,
        /// Fade length.
        fade: u32,
    },
    /// Fades the gain of `voice` to `gain`.
    Gain {
        /// The voice.
        voice: VoiceId,
        /// Target gain.
        gain: f32,
        /// Fade length.
        fade: u32,
    },
    /// Sets the playback speed of `voice`.
    Pitch {
        /// The voice.
        voice: VoiceId,
        /// New speed.
        pitch: f32,
    },
    /// Moves a placed voice (or places an unplaced one).
    Place {
        /// The voice.
        voice: VoiceId,
        /// New world position.
        at: At<World2, Vec2>,
    },
    /// Sets the walls between `voice` and the listener (the game counts them, e.g. on its grid).
    Occlude {
        /// The voice.
        voice: VoiceId,
        /// Wall count.
        walls: f32,
    },
    /// Fades a bus to `gain` (mix states, ducking music under speech).
    BusGain {
        /// The bus.
        bus: Bus,
        /// Target gain.
        gain: f32,
        /// Fade length.
        fade: u32,
    },
    /// Fades out and ends every voice on `bus`.
    StopBus {
        /// The bus.
        bus: Bus,
        /// Fade length.
        fade: u32,
    },
    /// Moves or turns the listener, or changes its room.
    Listener(Listener),
    /// Headphones on: ear delay and head shadow for placed voices.
    Headphones(bool),
    /// Fades the filter of a bus to `filter` (underwater, radio, stunned).
    Filter {
        /// The bus.
        bus: Bus,
        /// Target filter.
        filter: Filter,
        /// Fade length.
        fade: u32,
    },
    /// Lets `bus` duck under another bus while that one sounds (sidechain), or stops it (`None`).
    Duck {
        /// The bus that gets quieter.
        bus: Bus,
        /// How, or `None` for no ducking.
        duck: Option<Duck>,
    },
}

/// Filter on a bus: two cutoffs, each 12 dB per octave. 0 Hz switches a side off.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Filter {
    /// Above this the sound gets dull (low-pass).
    pub lowpass_hz: f32,
    /// Below this the sound gets thin (high-pass).
    pub highpass_hz: f32,
}

impl Filter {
    /// No filter.
    pub const NONE: Self = Self {
        lowpass_hz: 0.0,
        highpass_hz: 0.0,
    };
    /// Under water, behind a thick door, stunned: only the lows remain.
    pub const UNDERWATER: Self = Self {
        lowpass_hz: 600.0,
        highpass_hz: 0.0,
    };
    /// Radio or telephone: a narrow band in the middle.
    pub const RADIO: Self = Self {
        lowpass_hz: 3_000.0,
        highpass_hz: 400.0,
    };
}

/// Sidechain ducking: a bus gets quieter while the bus `by` carries sound (speech over music,
/// an explosion pressing everything else down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Duck {
    /// The bus that is listened to.
    pub by: Bus,
    /// Share of the level taken away at full duck, 0..=1.
    pub depth: f32,
    /// How fast the duck goes down, in milliseconds.
    pub attack_ms: u16,
    /// How fast the level comes back, in milliseconds.
    pub release_ms: u16,
}

/// The ears: position, orientation and the room they are in.
#[derive(Clone, Copy, Debug)]
pub struct Listener {
    /// Position.
    pub at: At<World2, Vec2>,
    /// Unit vector the listener faces.
    pub forward: Vec2,
    /// Unit vector to the listener's right. Given explicitly, so y-up and y-down worlds both work.
    pub right: Vec2,
    /// Room echo.
    pub room: Room,
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            at: At::new(Vec2::new(0.0, 0.0)),
            forward: Vec2::new(0.0, 1.0),
            right: Vec2::new(1.0, 0.0),
            room: Room::NONE,
        }
    }
}

/// Short room echo: three taps on a shared delay line (values from the Wizardry 7 rewrite).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    /// Echo level.
    pub level: f32,
    /// Left, right and late tap in milliseconds (each below 150).
    pub taps_ms: [u16; 3],
}

impl Room {
    /// Open air: no echo.
    pub const NONE: Self = Self {
        level: 0.0,
        taps_ms: [0; 3],
    };
    /// Small wooden room.
    pub const SMALL: Self = Self {
        level: 0.10,
        taps_ms: [13, 22, 34],
    };
    /// Stone hall, masonry.
    pub const STONE: Self = Self {
        level: 0.17,
        taps_ms: [19, 32, 56],
    };
    /// Cave.
    pub const CAVE: Self = Self {
        level: 0.22,
        taps_ms: [31, 54, 88],
    };
}

/// Fixed setup of a [`Mixer`].
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Output rate in Hz.
    pub rate: u32,
    /// Voices that can sound at once; more replace by priority.
    pub voices: usize,
    /// Number of buses.
    pub buses: usize,
    /// World units per distance step (one grid cell in a dungeon).
    pub unit: f32,
    /// Level loss per step.
    pub rolloff: f32,
    /// Brightness loss per step (air absorbs highs).
    pub air: f32,
    /// Steps beyond which a voice is inaudible and runs virtual.
    pub range: f32,
    /// Headphones at start (see [`Command::Headphones`]).
    pub headphones: bool,
    /// World units sound travels per second; a placed voice then starts late by its distance
    /// (lightning before thunder). 0 switches it off. Air: 343 m/s.
    pub sound_speed: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            rate: 48_000,
            voices: 32,
            buses: 3,
            unit: 1.0,
            rolloff: 0.39,
            air: 0.9,
            range: 12.0,
            headphones: false,
            sound_speed: 0.0,
        }
    }
}

impl Config {
    /// Frames in `ms` milliseconds at the output rate, for fade lengths.
    pub fn frames(&self, ms: u32) -> u32 {
        (u64::from(self.rate) * u64::from(ms) / 1000) as u32
    }
}
