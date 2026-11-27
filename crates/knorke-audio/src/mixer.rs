//! The mixer: voices into buses, buses and room echo into the output.

use crate::bus::BusState;
use crate::echo::Echo;
use crate::voice::{Step, Voice};
use crate::{BLOCK, Bus, Command, Config, Listener, Play, Sounds, VoiceId};
use knorke_math::det;

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

/// The audio mixer (see the crate docs). Build once, then [`apply`](Self::apply) commands and
/// [`render`](Self::render) on the audio thread.
#[derive(Debug)]
pub struct Mixer {
    cfg: Config,
    headphones: bool,
    listener: Listener,
    voices: Vec<Voice>,
    buses: Vec<BusState>,
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
            buses: vec![BusState::new(); cfg.buses],
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

    /// Output frames rendered so far: the time base of [`Play::start`]. A device host counts the
    /// same frames (e.g. `knorke-audio-sdl`'s `Output::frames`).
    pub fn clock(&self) -> u64 {
        self.clock
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

    fn bus(&mut self, bus: Bus) -> Option<&mut BusState> {
        self.buses.get_mut(bus.index())
    }

    fn stop(&mut self, i: usize, fade: u32) {
        // A voice that has not started yet has nothing to fade.
        if fade == 0 || self.voices[i].wait > 0 {
            self.voices.swap_remove(i);
        } else {
            self.voices[i].gain.to(0.0, fade);
            self.voices[i].stopping = true;
        }
    }

    /// Removes voice `i` to make room for a new one.
    fn evict(&mut self, i: usize) {
        self.voices.swap_remove(i);
        self.stats.dropped += 1;
    }

    /// Applies one command. Commands about unknown voices are ignored: the voice has ended.
    pub fn apply(&mut self, cmd: Command) {
        let rate = self.cfg.rate;
        let buses = self.buses.len();
        match cmd {
            Command::Play { voice, play } => self.play(voice, &play),
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
                if let Some(b) = self.bus(bus) {
                    b.gain.to(det::max(gain, 0.0), fade);
                }
            }
            Command::StopBus { bus, fade } => {
                for i in (0..self.voices.len()).rev() {
                    if self.voices[i].bus == bus.index() {
                        self.stop(i, fade);
                    }
                }
            }
            Command::Listener(l) => {
                if l.room != self.listener.room {
                    self.echo.switch(l.room);
                }
                self.listener = l;
            }
            Command::Headphones(on) => self.headphones = on,
            Command::Filter { bus, filter, fade } => {
                if let Some(b) = self.bus(bus) {
                    b.set_filter(filter, rate, fade);
                }
            }
            Command::Duck { bus, duck } => {
                if let Some(b) = self.bus(bus) {
                    b.set_duck(duck, buses, rate);
                }
            }
        }
    }

    fn play(&mut self, id: VoiceId, play: &Play) {
        if play.bus.index() >= self.buses.len() || self.cfg.voices == 0 {
            self.stats.dropped += 1;
            return;
        }
        if let Some(i) = self.find(id) {
            self.voices.swap_remove(i);
        }
        if play.limit > 0 {
            let same = || {
                self.voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.sound == play.sound)
            };
            if same().count() >= usize::from(play.limit) {
                // Replace the one furthest through the sound.
                if let Some((i, _)) = same().max_by(|a, b| a.1.pos.total_cmp(&b.1.pos)) {
                    self.evict(i);
                }
            }
        }
        if self.voices.len() == self.cfg.voices {
            // Replace the least important voice, the quietest among equals.
            let mut victim = 0;
            for (i, v) in self.voices.iter().enumerate() {
                let w = &self.voices[victim];
                if (v.priority, v.gain.value()) < (w.priority, w.gain.value()) {
                    victim = i;
                }
            }
            if self.voices[victim].priority > play.priority {
                self.stats.dropped += 1;
                return;
            }
            self.evict(victim);
        }
        let mut wait = play.start.saturating_sub(self.clock);
        if let Some(at) = play.at.filter(|_| self.cfg.sound_speed > 0.0) {
            let d = (at.raw() - self.listener.at.raw()).length();
            wait += (d / self.cfg.sound_speed * self.cfg.rate as f32) as u64;
        }
        let (listener, cfg, headphones) = (&self.listener, &self.cfg, self.headphones);
        let v = Voice::new(id, play, wait, |v| v.ears(listener, cfg, headphones));
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
        let (listener, cfg, headphones) = (&self.listener, &self.cfg, self.headphones);
        for v in &mut self.voices {
            v.retarget(v.ears(listener, cfg, headphones));
        }
        self.echo.settle();
    }

    /// Mixes `n` frames from block offset `phase` into the bus mixes and the echo send.
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
            // A voice waiting for its start sits out (part of) this stretch.
            let skip = v.wait.min(n as u64) as usize;
            v.wait -= skip as u64;
            if skip == n {
                i += 1;
                continue;
            }
            let step = match sounds.pcm(v.sound).filter(|p| p.rate > 0 && p.frames() > 0) {
                None => {
                    stats.dropped += 1;
                    Step::Ended
                }
                Some(pcm) => {
                    let out = &mut mix[2 * (v.bus * BLOCK + skip)..2 * (v.bus * BLOCK + n)];
                    v.render(&pcm, cfg.rate, phase + skip, out, &mut send[skip..n])
                }
            };
            stats.virtual_voices += usize::from(step == Step::Virtual);
            if step == Step::Ended || v.stopping && v.gain.silent() {
                voices.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    fn output(&mut self, out: &mut [f32]) {
        let echo = self.echo.active();
        let mut peak = self.stats.peak;
        let mix = &self.mix;
        for (i, o) in out.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let frame = |b: usize| [mix[2 * (b * BLOCK + i)], mix[2 * (b * BLOCK + i) + 1]];
            let mut sum = [0.0; 2];
            for (b, bus) in self.buses.iter_mut().enumerate() {
                let x = bus.next(b, frame);
                sum = [sum[0] + x[0], sum[1] + x[1]];
            }
            if echo {
                let e = self.echo.frame(self.send[i]);
                sum = [sum[0] + e[0], sum[1] + e[1]];
            }
            *o = sum.map(soft_clip);
            peak = det::max(peak, det::max(o[0].abs(), o[1].abs()));
        }
        if !echo {
            self.echo.idle();
        }
        self.stats.peak = peak;
    }
}
