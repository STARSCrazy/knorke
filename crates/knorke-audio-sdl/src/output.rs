//! FFI boundary to SDL3's audio API: the only module of this crate that uses `unsafe` (ADR 0005).

use core::ffi::{CStr, c_int, c_void};
use core::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use knorke_audio::{Command, Config, Mixer, Pcm, SoundId, Sounds};
use sdl3_sys::audio::{
    SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK, SDL_AUDIO_F32, SDL_AudioSpec, SDL_AudioStream,
    SDL_DestroyAudioStream, SDL_OpenAudioDeviceStream, SDL_PutAudioStreamData,
    SDL_ResumeAudioStreamDevice,
};
use sdl3_sys::error::SDL_GetError;
use sdl3_sys::init::{SDL_INIT_AUDIO, SDL_InitSubSystem, SDL_QuitSubSystem};

/// Commands the inbox holds between two callbacks; [`Output::send`] refuses more.
const INBOX: usize = 1024;
/// Frames per mixer call inside the callback (size of the render buffer).
const CHUNK: usize = 1024;
/// Bytes per output frame: two `f32` channels.
const FRAME_BYTES: usize = 8;

/// A decoded sound the [`Output`] owns for its lifetime.
#[derive(Clone, Debug, Default)]
pub struct Clip {
    /// Samples in `-1..=1`, interleaved when `stereo`.
    pub samples: Vec<f32>,
    /// Sample rate in Hz.
    pub rate: u32,
    /// Two interleaved channels instead of one.
    pub stereo: bool,
}

/// The clip table, indexed by [`SoundId`].
struct Clips(Vec<Clip>);

impl Sounds for Clips {
    fn pcm(&self, id: SoundId) -> Option<Pcm<'_>> {
        self.0.get(id.0 as usize).map(|c| Pcm {
            samples: &c.samples,
            rate: c.rate,
            stereo: c.stereo,
        })
    }
}

/// What only the audio callback touches. Behind a mutex so the callback can reach it through
/// a shared pointer; nobody else locks it, so the lock never waits.
struct Audio {
    mixer: Mixer,
    clips: Clips,
    buf: Vec<f32>,
}

struct Shared {
    inbox: Mutex<Vec<Command>>,
    audio: Mutex<Audio>,
    frames: AtomicU64,
}

/// [`Output::send`] found the inbox full: the audio callback has not run for a while (device
/// paused or stalled) or the game sends more than [`INBOX`] commands between two callbacks.
#[derive(Clone, Copy, Debug)]
pub struct SendError(pub Command);

/// The default playback device with a [`Mixer`] in its callback. Dropping it closes the device.
#[derive(Debug)]
pub struct Output {
    stream: NonNull<SDL_AudioStream>,
    // Boxed so its address stays fixed: the callback holds a pointer to it.
    shared: Box<Shared>,
}

impl core::fmt::Debug for Shared {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Shared")
            .field("frames", &self.frames)
            .finish_non_exhaustive()
    }
}

impl Output {
    /// Opens the default playback device at `cfg.rate`, stereo `f32` (SDL converts to what the
    /// device wants) and starts playing silence until commands arrive.
    pub fn open(cfg: Config, clips: Vec<Clip>) -> Result<Self, String> {
        Self::open_with_driver(cfg, clips, None)
    }

    /// Like [`Output::open`], but forces an SDL audio driver through the `SDL_AUDIO_DRIVER`
    /// hint, e.g. `c"dummy"` for tests on machines without speakers. Has no effect if SDL's
    /// audio subsystem is already running.
    pub fn open_with_driver(
        cfg: Config,
        clips: Vec<Clip>,
        driver: Option<&CStr>,
    ) -> Result<Self, String> {
        let shared = Box::new(Shared {
            inbox: Mutex::new(Vec::with_capacity(INBOX)),
            audio: Mutex::new(Audio {
                mixer: Mixer::new(cfg),
                clips: Clips(clips),
                buf: vec![0.0; 2 * CHUNK],
            }),
            frames: AtomicU64::new(0),
        });
        let spec = SDL_AudioSpec {
            format: SDL_AUDIO_F32,
            channels: 2,
            freq: cfg.rate as c_int,
        };
        // SAFETY: plain FFI calls. The hint strings are NUL-terminated and copied by SDL. The
        // userdata pointer stays valid until `Drop` has destroyed the stream, which stops the
        // callback, because `shared` is boxed and owned by the returned `Output`.
        unsafe {
            if let Some(driver) = driver {
                sdl3_sys::hints::SDL_SetHint(
                    sdl3_sys::hints::SDL_HINT_AUDIO_DRIVER,
                    driver.as_ptr(),
                );
            }
            if !SDL_InitSubSystem(SDL_INIT_AUDIO) {
                return Err(sdl_error());
            }
            let userdata = (&raw const *shared).cast_mut().cast::<c_void>();
            let stream = SDL_OpenAudioDeviceStream(
                SDL_AUDIO_DEVICE_DEFAULT_PLAYBACK,
                &spec,
                Some(feed),
                userdata,
            );
            let Some(stream) = NonNull::new(stream) else {
                let e = sdl_error();
                SDL_QuitSubSystem(SDL_INIT_AUDIO);
                return Err(e);
            };
            // Owned from here on, so a failure below still closes the device in `Drop`.
            let out = Self { stream, shared };
            if !SDL_ResumeAudioStreamDevice(out.stream.as_ptr()) {
                return Err(sdl_error());
            }
            Ok(out)
        }
    }

    /// Hands `cmd` to the audio thread; it takes effect at the next callback.
    pub fn send(&self, cmd: Command) -> Result<(), SendError> {
        let mut inbox = self
            .shared
            .inbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Pushing past the capacity would make the audio thread free a grown buffer later.
        if inbox.len() == inbox.capacity() {
            return Err(SendError(cmd));
        }
        inbox.push(cmd);
        Ok(())
    }

    /// Frames the device has asked for and got so far (a clock for the game, and proof in tests
    /// that the callback runs).
    pub fn frames(&self) -> u64 {
        self.shared.frames.load(Ordering::Relaxed)
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        // SAFETY: the stream came from SDL_OpenAudioDeviceStream and is destroyed once.
        // Destroying it closes the device and waits for a running callback, so `shared` is no
        // longer used once this returns; it is dropped after.
        unsafe {
            SDL_DestroyAudioStream(self.stream.as_ptr());
            SDL_QuitSubSystem(SDL_INIT_AUDIO);
        }
    }
}

/// SDL's audio callback: applies waiting commands, then renders what the device asks for.
unsafe extern "C" fn feed(
    userdata: *mut c_void,
    stream: *mut SDL_AudioStream,
    additional: c_int,
    _total: c_int,
) {
    // SAFETY: `userdata` is the boxed `Shared` of a live `Output` (see `open_with_driver`).
    let shared = unsafe { &*userdata.cast_const().cast::<Shared>() };
    let mut audio = shared.audio.lock().unwrap_or_else(PoisonError::into_inner);
    let Audio { mixer, clips, buf } = &mut *audio;
    // Never wait for the game thread: if it is just sending, its commands come next time.
    if let Ok(mut inbox) = shared.inbox.try_lock() {
        for cmd in inbox.drain(..) {
            mixer.apply(cmd);
        }
    }
    let mut frames = (additional.max(0) as usize).div_ceil(FRAME_BYTES);
    while frames > 0 {
        let n = frames.min(CHUNK);
        let out = &mut buf[..2 * n];
        mixer.render(clips, out);
        // SAFETY: `out` holds `n` frames of `f32` stereo, the format the stream was opened with;
        // SDL copies the bytes before returning.
        unsafe {
            SDL_PutAudioStreamData(stream, out.as_ptr().cast(), (n * FRAME_BYTES) as c_int);
        }
        shared.frames.fetch_add(n as u64, Ordering::Relaxed);
        frames -= n;
    }
}

/// The current SDL error text (empty if none).
fn sdl_error() -> String {
    let p = SDL_GetError();
    if p.is_null() {
        return String::new();
    }
    // SAFETY: SDL_GetError returns a valid NUL-terminated string owned by SDL that stays
    // valid until the next SDL call on this thread; we copy it out immediately.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use knorke_audio::{Play, VoiceId};
    use std::time::Duration;

    #[test]
    fn the_callback_runs_and_takes_commands() {
        let tone = Clip {
            samples: vec![0.25; 4800],
            rate: 48_000,
            stereo: false,
        };
        let out = Output::open_with_driver(Config::default(), vec![tone], Some(c"dummy"))
            .expect("dummy audio driver");
        let play = Play {
            looped: true,
            ..Play::default()
        };
        out.send(Command::Play {
            voice: VoiceId(1),
            play,
        })
        .unwrap();
        // Up to five seconds for the dummy device thread to start and pull 0.1 s of sound.
        for _ in 0..1000 {
            if out.frames() >= 4800 {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("callback did not run");
    }
}
