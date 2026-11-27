//! Cost of the audio mixer (ADR 0020). Run: `cargo bench -p knorke-contract-tests --bench audio`.
//! Renders 10 s at 48 kHz in 10 ms callbacks, best of several runs; prints the share of one core.

#![allow(clippy::disallowed_methods)]

use knorke_audio::{Command, Config, Listener, Mixer, Pcm, Play, Room, VoiceId};
use knorke_math::{At, Vec2};
use std::hint::black_box;
use std::time::{Duration, Instant};

const RATE: u32 = 48_000;
const SECONDS: usize = 10;

fn best(runs: u32, mut f: impl FnMut()) -> Duration {
    f();
    (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .min()
        .unwrap()
}

fn case(name: &str, voices: u32, headphones: bool, room: Room, far: bool, sounds: &[Pcm<'_>]) {
    let d = best(5, || {
        let mut m = Mixer::new(Config {
            rate: RATE,
            voices: voices as usize,
            ..Config::default()
        });
        m.apply(Command::Headphones(headphones));
        m.apply(Command::Listener(Listener {
            room,
            ..Listener::default()
        }));
        for i in 0..voices {
            let x = (i % 9) as f32 - 4.0 + if far { 100.0 } else { 0.0 };
            let play = Play {
                at: Some(At::new(Vec2::new(x, (i % 5) as f32 - 2.0))),
                pitch: 0.9 + (i % 7) as f32 * 0.03,
                looped: true,
                ..Play::default()
            };
            m.apply(Command::Play {
                voice: VoiceId(i),
                play,
            });
        }
        let mut out = vec![0.0; 2 * 480];
        for _ in 0..SECONDS * 100 {
            m.render(sounds, &mut out);
            black_box(&out);
        }
    });
    let share = d.as_secs_f64() / SECONDS as f64 * 100.0;
    println!(
        "{name:<48} {:>8.2} ms per s audio  {share:>6.3} % of one core",
        d.as_secs_f64() * 1e3 / SECONDS as f64
    );
}

fn main() {
    let data: Vec<f32> = (0..22_050)
        .map(|i| ((i * 7919 % 255) as f32 / 127.5) - 1.0)
        .collect();
    let sounds = [Pcm {
        samples: &data,
        rate: 22_050,
        stereo: false,
    }];
    case(
        "32 placed voices, speakers, no room",
        32,
        false,
        Room::NONE,
        false,
        &sounds,
    );
    case(
        "32 placed voices, headphones, stone room",
        32,
        true,
        Room::STONE,
        false,
        &sounds,
    );
    case(
        "64 placed voices, headphones, stone room",
        64,
        true,
        Room::STONE,
        false,
        &sounds,
    );
    case(
        "32 voices out of earshot (virtual)",
        32,
        false,
        Room::NONE,
        true,
        &sounds,
    );
}
