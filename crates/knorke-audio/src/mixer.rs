//! The mixer: voices into buses, buses and room echo into the output.

use crate::spatial::{self, Ears};
use crate::{BLOCK, Command, Config, Listener, Pcm, Play, Room, SoundId, Sounds, VoiceId};
use knorke_math::{Vec2, det};
use ramp::Ramp;

/// Linear fades, advanced one frame at a time.
mod ramp {
    #[derive(Clone, Copy, Debug)]
    pub(super) struct Ramp {
        pub(super) value: f32,
        target: f32,
        step: f32,
        pub(super) left: u32,
    }

    impl Ramp {
        pub(super) fn at(value: f32) -> Self {
            Self {
                value,
                target: value,
                step: 0.0,
                left: 0,
            }
        }

        pub(super) fn to(&mut self, target: f32, frames: u32) {
            self.target = target;
            self.left = frames;
            if frames == 0 {
                self.value = target;
            } else {
                self.step = (target - self.value) / frames as f32;
            }
        }

        #[inline]
        pub(super) fn next(&mut self) -> f32 {
            if self.left > 0 {
                self.left -= 1;
                // Land exactly on the target, whatever the rounding on the way.
                self.value = if self.left == 0 {
                    self.target
                } else {
                    self.value + self.step
                };
            }
            self.value
        }

        pub(super) fn skip(&mut self, frames: u32) {
            if frames >= self.left {
                self.to(self.target, 0);
            } else {
                self.left -= frames;
                self.value += self.step * frames as f32;
            }
        }

        pub(super) fn silent(&self) -> bool {
            self.left == 0 && self.value == 0.0
        }
    }
}

#[derive(Debug)]
struct Voice {
    id: VoiceId,
    sound: SoundId,
    bus: usize,
    priority: u8,
    looped: bool,
    stopping: bool,
    /// Play position in source frames.
    pos: f64,
    pitch: f32,
    gain: Ramp,
    at: Option<Vec2>,
    walls: f32,
    pan: f32,
    /// Ear values at the start and end of the current block.
    from: Ears,
    to: Ears,
    /// Low-pass state per ear.
    lp: [f32; 2],
}

/// What the mixer did, for debug overlays and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    /// Voices playing.
    pub voices: usize,
    /// Of these, voices out of earshot in the last block (position advanced, nothing mixed).
    pub virtual_voices: usize,
    /// Plays refused or voices replaced because all voices were busy, or with a bad bus or sound.
    pub dropped: u64,
    /// Largest output magnitude of the last [`Mixer::render`] call.
    pub peak: f32,
}

/// Short stereo echo on one delay line (Wizardry 7 rewrite, `mix_reflections`).
#[derive(Debug)]
struct Echo {
    line: Vec<f32>,
    pos: usize,
    taps: [usize; 3],
    feedback: usize,
    input: f32,
    level: Ramp,
    room: Room,
    next: Option<Room>,
    stale: bool,
}

/// Feedback tap of the echo line in milliseconds.
const FEEDBACK_MS: u32 = 67;
/// Longest tap a [`Room`] may use in milliseconds.
const MAX_TAP_MS: u32 = 150;

impl Echo {
    fn new(rate: u32) -> Self {
        let len = (rate * (MAX_TAP_MS + FEEDBACK_MS) / 1000 + 1).next_power_of_two() as usize;
        Self {
            line: vec![0.0; len],
            pos: 0,
            taps: [0; 3],
            feedback: (rate * FEEDBACK_MS / 1000) as usize,
            input: 0.0,
            level: Ramp::at(0.0),
            room: Room::NONE,
            next: None,
            stale: false,
        }
    }

    /// Fades the current room out; [`Echo::settle`] then takes over `room`.
    fn switch(&mut self, room: Room, fade: u32) {
        self.next = Some(room);
        self.level.to(0.0, fade);
    }

    /// At a block start: adopt a pending room once the old one is silent.
    fn settle(&mut self, rate: u32, fade: u32) {
        if let Some(room) = self.next.filter(|_| self.level.silent()) {
            if self.stale {
                // The line was not fed while silent; old content must not ring out.
                self.line.fill(0.0);
                self.input = 0.0;
                self.stale = false;
            }
            let ms = |t: u16| (rate * u32::from(t).min(MAX_TAP_MS) / 1000) as usize;
            self.taps = room.taps_ms.map(ms);
            self.room = room;
            self.level.to(room.level, fade);
            self.next = None;
        }
    }

    fn active(&self) -> bool {
        !self.level.silent()
    }

    #[inline]
    fn frame(&mut self, send: f32) -> (f32, f32) {
        let (pos, mask) = (self.pos, self.line.len() - 1);
        let tap = |line: &[f32], d: usize| line[pos.wrapping_sub(d) & mask];
        self.input += (send - self.input) * 0.18;
        self.line[pos] = self.input + 0.16 * tap(&self.line, self.feedback);
        let level = self.level.next();
        let late = 0.40 * tap(&self.line, self.taps[2]);
        self.pos = (pos + 1) & mask;
        (
            level * (tap(&self.line, self.taps[0]) + late),
            level * (0.86 * tap(&self.line, self.taps[1]) + late),
        )
    }
}

/// Output headroom and soft knee (Wizardry 7 rewrite, `output_sample`): loud overlaps bend
/// instead of clipping, and stay below 1.0.
fn soft_clip(x: f32) -> f32 {
    const HEADROOM: f32 = 0.8;
    const KNEE: f32 = 0.72;
    const SPAN: f32 = 0.25;
    let a = x.abs() * HEADROOM;
    let y = if a > KNEE {
        KNEE + SPAN * (a - KNEE) / (SPAN + a - KNEE)
    } else {
        a
    };
    y.copysign(x)
}

/// Frame `p` of `pcm`, linearly interpolated; silence outside unless `looped`.
#[inline]
fn fetch(pcm: &Pcm<'_>, p: f64, looped: bool) -> (f32, f32) {
    let frames = pcm.frames();
    // Only an ear delay reaches before the start; the play position itself stays in range.
    let p = if p >= 0.0 {
        p
    } else if looped {
        p + frames as f64
    } else {
        return (0.0, 0.0);
    };
    let get = |k: usize| {
        let k = match k {
            k if k < frames => k,
            k if looped => k % frames,
            _ => return (0.0, 0.0),
        };
        let s = pcm.samples;
        if pcm.stereo {
            (s[2 * k], s[2 * k + 1])
        } else {
            (s[k], s[k])
        }
    };
    let i = p as usize;
    let f = (p - i as f64) as f32;
    let (a, b) = (get(i), get(i + 1));
    (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
}

fn target(listener: &Listener, cfg: &Config, headphones: bool, v: &Voice) -> Ears {
    match v.at {
        Some(at) => spatial::placed(listener, at, v.walls, cfg, headphones),
        None => spatial::unplaced(v.pan),
    }
}

/// Mixes `n` frames of `v` from block offset `phase` into `out` (stereo) and `send` (mono echo
/// send). Returns `true` when the voice reached its end.
fn mix_voice(
    v: &mut Voice,
    pcm: &Pcm<'_>,
    step: f64,
    phase: usize,
    out: &mut [f32],
    send: &mut [f32],
) -> bool {
    let frames = pcm.frames() as f64;
    let placed = v.at.is_some();
    for (i, (o, s)) in out
        .as_chunks_mut::<2>()
        .0
        .iter_mut()
        .zip(send.iter_mut())
        .enumerate()
    {
        let e = v.from.lerp(&v.to, (phase + i) as f32 / BLOCK as f32);
        let g = v.gain.next();
        let a = fetch(pcm, v.pos - f64::from(e.delay[0]) * step, v.looped);
        let b = if e.delay[1] == e.delay[0] {
            a
        } else {
            fetch(pcm, v.pos - f64::from(e.delay[1]) * step, v.looped)
        };
        // A placed source is a point: mono. An unplaced stereo source keeps its channels.
        let x = if placed {
            [(a.0 + a.1) * 0.5, (b.0 + b.1) * 0.5]
        } else {
            [a.0, b.1]
        };
        for ear in 0..2 {
            v.lp[ear] += (x[ear] - v.lp[ear]) * e.bright[ear];
            o[ear] += v.lp[ear] * e.gain[ear] * g;
        }
        if placed {
            *s += (o[0] + o[1]) * 0.5;
        }
        v.pos += step;
        if v.pos >= frames {
            if !v.looped {
                return true;
            }
            v.pos -= frames;
        }
    }
    false
}

/// The audio mixer (see the crate docs). Build once, then [`apply`](Self::apply) commands and
/// [`render`](Self::render) on the audio thread.
#[derive(Debug)]
pub struct Mixer {
    cfg: Config,
    headphones: bool,
    listener: Listener,
    voices: Vec<Voice>,
    buses: Vec<Ramp>,
    /// Per bus `BLOCK` stereo frames.
    mix: Vec<f32>,
    send: Vec<f32>,
    echo: Echo,
    /// Frames rendered since start: the block grid.
    clock: u64,
    stats: Stats,
}

impl Mixer {
    /// Allocates every buffer the mixer will ever use.
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            headphones: cfg.headphones,
            listener: Listener::default(),
            voices: Vec::with_capacity(cfg.voices),
            buses: vec![Ramp::at(1.0); cfg.buses],
            mix: vec![0.0; cfg.buses * BLOCK * 2],
            send: vec![0.0; BLOCK],
            echo: Echo::new(cfg.rate),
            clock: 0,
            stats: Stats::default(),
        }
    }

    /// The setup.
    pub fn config(&self) -> &Config {
        &self.cfg
    }

    /// What happened so far.
    pub fn stats(&self) -> Stats {
        Stats {
            voices: self.voices.len(),
            ..self.stats
        }
    }

    fn find(&self, id: VoiceId) -> Option<usize> {
        self.voices.iter().position(|v| v.id == id)
    }

    fn voice(&mut self, id: VoiceId) -> Option<&mut Voice> {
        self.voices.iter_mut().find(|v| v.id == id)
    }

    fn stop(&mut self, i: usize, fade: u32) {
        if fade == 0 {
            self.voices.swap_remove(i);
        } else {
            self.voices[i].gain.to(0.0, fade);
            self.voices[i].stopping = true;
        }
    }

    /// Applies one command. Commands about unknown voices are ignored: the voice has ended.
    pub fn apply(&mut self, cmd: Command) {
        match cmd {
            Command::Play { voice, play } => self.play(voice, play),
            Command::Stop { voice, fade } => {
                if let Some(i) = self.find(voice) {
                    self.stop(i, fade);
                }
            }
            Command::Gain { voice, gain, fade } => {
                // A stopping voice stays on its way out.
                if let Some(v) = self.voice(voice).filter(|v| !v.stopping) {
                    v.gain.to(det::max(gain, 0.0), fade);
                }
            }
            Command::Pitch { voice, pitch } => {
                if let Some(v) = self.voice(voice) {
                    v.pitch = det::max(pitch, 0.0);
                }
            }
            Command::Place { voice, at } => {
                if let Some(v) = self.voice(voice) {
                    v.at = Some(at.raw());
                }
            }
            Command::Occlude { voice, walls } => {
                if let Some(v) = self.voice(voice) {
                    v.walls = walls;
                }
            }
            Command::BusGain { bus, gain, fade } => {
                if let Some(b) = self.buses.get_mut(usize::from(bus.0)) {
                    b.to(det::max(gain, 0.0), fade);
                }
            }
            Command::StopBus { bus, fade } => {
                let bus = usize::from(bus.0);
                let mut i = self.voices.len();
                while i > 0 {
                    i -= 1;
                    if self.voices[i].bus == bus {
                        self.stop(i, fade);
                    }
                }
            }
            Command::Listener(l) => {
                if l.room != self.listener.room {
                    self.echo.switch(l.room, self.cfg.frames(50));
                }
                self.listener = l;
            }
            Command::Headphones(on) => self.headphones = on,
        }
    }

    fn play(&mut self, id: VoiceId, play: Play) {
        let bus = usize::from(play.bus.0);
        if bus >= self.buses.len() || self.cfg.voices == 0 {
            self.stats.dropped += 1;
            return;
        }
        if let Some(i) = self.find(id) {
            self.voices.swap_remove(i);
        }
        if self.voices.len() == self.cfg.voices {
            // Replace the least important voice, the quietest among equals.
            let mut victim = 0;
            for (i, v) in self.voices.iter().enumerate() {
                let w = &self.voices[victim];
                if (v.priority, v.gain.value) < (w.priority, w.gain.value) {
                    victim = i;
                }
            }
            self.stats.dropped += 1;
            if self.voices[victim].priority > play.priority {
                return;
            }
            self.voices.swap_remove(victim);
        }
        let gain = det::max(play.gain, 0.0);
        let mut v = Voice {
            id,
            sound: play.sound,
            bus,
            priority: play.priority,
            looped: play.looped,
            stopping: false,
            pos: 0.0,
            pitch: det::max(play.pitch, 0.0),
            gain: Ramp::at(if play.fade_in > 0 { 0.0 } else { gain }),
            at: play.at.map(|a| a.raw()),
            walls: play.walls,
            pan: play.pan,
            from: spatial::unplaced(0.0),
            to: spatial::unplaced(0.0),
            lp: [0.0; 2],
        };
        v.gain.to(gain, play.fade_in);
        v.to = target(&self.listener, &self.cfg, self.headphones, &v);
        v.from = v.to;
        self.voices.push(v);
    }

    /// Fills `out` (interleaved stereo; a trailing odd sample stays 0) with the next frames.
    pub fn render<S: Sounds + ?Sized>(&mut self, sounds: &S, out: &mut [f32]) {
        out.fill(0.0);
        self.stats.peak = 0.0;
        let mut done = 0;
        let frames = out.len() / 2;
        while done < frames {
            let phase = (self.clock % BLOCK as u64) as usize;
            let n = (BLOCK - phase).min(frames - done);
            if phase == 0 {
                self.begin_block();
            }
            self.mix_block(sounds, phase, n);
            self.output(&mut out[2 * done..2 * (done + n)]);
            self.clock += n as u64;
            done += n;
        }
    }

    fn begin_block(&mut self) {
        let Self {
            voices,
            listener,
            cfg,
            headphones,
            ..
        } = self;
        for v in voices.iter_mut() {
            v.from = v.to;
            v.to = target(listener, cfg, *headphones, v);
        }
        self.echo.settle(self.cfg.rate, self.cfg.frames(50));
    }

    fn mix_block<S: Sounds + ?Sized>(&mut self, sounds: &S, phase: usize, n: usize) {
        let Self {
            voices,
            mix,
            send,
            cfg,
            stats,
            ..
        } = self;
        for bus in mix.as_chunks_mut::<{ BLOCK * 2 }>().0 {
            bus[..2 * n].fill(0.0);
        }
        send[..n].fill(0.0);
        stats.virtual_voices = 0;
        let mut i = 0;
        while i < voices.len() {
            let v = &mut voices[i];
            let ended = match sounds.pcm(v.sound).filter(|p| p.rate > 0 && p.frames() > 0) {
                None => {
                    stats.dropped += 1;
                    true
                }
                Some(pcm) => {
                    let step = f64::from(pcm.rate) / f64::from(cfg.rate) * f64::from(v.pitch);
                    if v.from.audible() || v.to.audible() {
                        let out = &mut mix[v.bus * BLOCK * 2..][..2 * n];
                        mix_voice(v, &pcm, step, phase, out, &mut send[..n])
                    } else {
                        // Out of earshot: keep time, skip the work.
                        stats.virtual_voices += 1;
                        v.gain.skip(n as u32);
                        v.pos += step * n as f64;
                        let frames = pcm.frames() as f64;
                        if v.looped {
                            v.pos %= frames;
                        }
                        v.pos >= frames
                    }
                }
            };
            if ended || v.stopping && v.gain.silent() {
                voices.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    fn output(&mut self, out: &mut [f32]) {
        let echo = self.echo.active();
        let mut peak = self.stats.peak;
        for (i, o) in out.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let (mut l, mut r) = (0.0, 0.0);
            for (b, gain) in self.buses.iter_mut().enumerate() {
                let g = gain.next();
                l += self.mix[b * BLOCK * 2 + 2 * i] * g;
                r += self.mix[b * BLOCK * 2 + 2 * i + 1] * g;
            }
            if echo {
                let (el, er) = self.echo.frame(self.send[i]);
                l += el;
                r += er;
            }
            o[0] = soft_clip(l);
            o[1] = soft_clip(r);
            peak = det::max(peak, det::max(o[0].abs(), o[1].abs()));
        }
        if !echo {
            self.echo.stale = true;
        }
        self.stats.peak = peak;
    }
}
