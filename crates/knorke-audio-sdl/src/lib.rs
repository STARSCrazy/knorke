//! SDL3 playback for `knorke-audio` (ADR 0020: the host owns device and thread).
//!
//! `Output::open` opens the default playback device and runs a `Mixer` in SDL's audio
//! callback. The game thread hands over `Command`s with `Output::send`; the callback applies
//! them before it renders the next frames.
//!
//! The audio thread never waits for the game: it only `try_lock`s the inbox, and if the game
//! holds it at that moment the commands wait for the next callback (a few milliseconds). Inbox
//! and render buffer are sized at open, so the callback does not allocate.
//!
//! Without the feature `sdl` the crate is empty.

#[cfg(feature = "sdl")]
#[allow(unsafe_code)]
mod output;

#[cfg(feature = "sdl")]
pub use output::{Clip, Output, SendError};

#[cfg(feature = "sdl")]
pub use knorke_audio::{Bus, Command, Config, Listener, Mixer, Play, SoundId, VoiceId};
