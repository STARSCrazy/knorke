//! Behaviour of the mixer through its public API.

use knorke_audio::{
    BLOCK, Bus, Command, Config, Listener, Mixer, Pcm, Play, Room, SoundId, VoiceId,
};
use knorke_math::{At, Vec2};

const RATE: u32 = 48_000;

/// A deterministic noise burst: rich in highs, so filters show.
fn noise(frames: usize) -> Vec<f32> {
    let mut x = 0x1234_5678u32;
    (0..frames)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 8) as f32 / 8_388_608.0 - 1.0
        })
        .collect()
}

fn mixer() -> Mixer {
    Mixer::new(Config {
        rate: RATE,
        ..Config::default()
    })
}

fn play(m: &mut Mixer, id: u32, play: Play) {
    m.apply(Command::Play {
        voice: VoiceId(id),
        play,
    });
}

fn render(m: &mut Mixer, sounds: &[Pcm<'_>], frames: usize) -> Vec<f32> {
    let mut out = vec![0.0; frames * 2];
    m.render(sounds, &mut out);
    out
}

/// Energy per channel and energy of the sample-to-sample difference (a proxy for highs).
fn energy(out: &[f32]) -> ([f32; 2], f32) {
    let mut e = [0.0; 2];
    let mut d = 0.0;
    for (i, f) in out.as_chunks::<2>().0.iter().enumerate() {
        e[0] += f[0] * f[0];
        e[1] += f[1] * f[1];
        if i > 0 {
            let p = out[2 * i - 2];
            d += (f[0] - p) * (f[0] - p);
        }
    }
    (e, d)
}

fn at(x: f32, y: f32) -> Option<At<knorke_math::World2, Vec2>> {
    Some(At::new(Vec2::new(x, y)))
}

#[test]
fn silence_without_voices_and_centre_is_balanced() {
    let data = noise(4800);
    let sounds = [Pcm {
        samples: &data,
        rate: RATE,
        stereo: false,
    }];
    let mut m = mixer();
    assert!(render(&mut m, &sounds, 300).iter().all(|&s| s == 0.0));
    play(&mut m, 1, Play::default());
    let out = render(&mut m, &sounds, 1000);
    assert!(out.as_chunks::<2>().0.iter().all(|f| f[0] == f[1]));
    assert!(energy(&out).0[0] > 1.0);
}

#[test]
fn output_does_not_depend_on_callback_size() {
    let data = noise(9000);
    let sounds = [Pcm {
        samples: &data,
        rate: 22_050,
        stereo: false,
    }];
    let setup = |m: &mut Mixer| {
        m.apply(Command::Headphones(true));
        m.apply(Command::Listener(Listener {
            room: Room::STONE,
            ..Listener::default()
        }));
        play(
            m,
            1,
            Play {
                at: at(2.0, 1.0),
                pitch: 1.3,
                looped: true,
                ..Play::default()
            },
        );
        play(
            m,
            2,
            Play {
                pan: -0.5,
                fade_in: 700,
                ..Play::default()
            },
        );
    };
    let mut a = mixer();
    setup(&mut a);
    let whole = render(&mut a, &sounds, 5000);
    let mut b = mixer();
    setup(&mut b);
    let mut pieces = Vec::new();
    for n in [1, 255, 256, 7, 1000, 3481] {
        pieces.extend(render(&mut b, &sounds, n));
    }
    assert_eq!(whole, pieces);
}

#[test]
fn right_is_right_behind_is_duller_walls_are_quieter() {
    let data = noise(48_000);
    let sounds = [Pcm {
        samples: &data,
        rate: RATE,
        stereo: false,
    }];
    let probe = |p: Play| {
        let mut m = mixer();
        play(&mut m, 1, p);
        energy(&render(&mut m, &sounds, 4 * BLOCK))
    };
    let right = probe(Play {
        at: at(3.0, 0.0),
        ..Play::default()
    });
    assert!(right.0[1] > 2.0 * right.0[0], "{right:?}");
    let front = probe(Play {
        at: at(0.0, 2.0),
        ..Play::default()
    });
    let back = probe(Play {
        at: at(0.0, -2.0),
        ..Play::default()
    });
    assert!(
        back.1 < front.1,
        "behind keeps fewer highs: {front:?} {back:?}"
    );
    let wall = probe(Play {
        at: at(0.0, 2.0),
        walls: 1.0,
        ..Play::default()
    });
    assert!(wall.0[0] < 0.5 * front.0[0], "{front:?} {wall:?}");
}

#[test]
fn fades_end_voices_and_buses_duck() {
    let data = noise(48_000);
    let sounds = [Pcm {
        samples: &data,
        rate: RATE,
        stereo: false,
    }];
    let mut m = mixer();
    play(
        &mut m,
        1,
        Play {
            bus: Bus(1),
            looped: true,
            ..Play::default()
        },
    );
    play(
        &mut m,
        2,
        Play {
            bus: Bus(0),
            looped: true,
            ..Play::default()
        },
    );
    m.apply(Command::BusGain {
        bus: Bus(1),
        gain: 0.0,
        fade: 480,
    });
    m.apply(Command::Stop {
        voice: VoiceId(2),
        fade: 480,
    });
    render(&mut m, &sounds, 600);
    assert_eq!(m.stats().voices, 1, "voice 2 ended after its fade");
    assert!(
        render(&mut m, &sounds, 300).iter().all(|&s| s == 0.0),
        "bus 1 is ducked to silence"
    );
    m.apply(Command::StopBus {
        bus: Bus(1),
        fade: 0,
    });
    assert_eq!(m.stats().voices, 0);
}

#[test]
fn pitch_shortens_and_full_mixer_replaces_by_priority() {
    let data = noise(4800);
    let sounds = [Pcm {
        samples: &data,
        rate: RATE,
        stereo: false,
    }];
    let mut m = Mixer::new(Config {
        rate: RATE,
        voices: 2,
        ..Config::default()
    });
    play(
        &mut m,
        1,
        Play {
            pitch: 2.0,
            ..Play::default()
        },
    );
    render(&mut m, &sounds, 2401);
    assert_eq!(
        m.stats().voices,
        0,
        "4800 frames at double speed end after 2400"
    );

    play(
        &mut m,
        1,
        Play {
            priority: 5,
            looped: true,
            ..Play::default()
        },
    );
    play(
        &mut m,
        2,
        Play {
            priority: 1,
            looped: true,
            ..Play::default()
        },
    );
    play(
        &mut m,
        3,
        Play {
            priority: 0,
            looped: true,
            ..Play::default()
        },
    );
    assert_eq!(m.stats().dropped, 1, "priority 0 cannot replace priority 1");
    play(
        &mut m,
        4,
        Play {
            priority: 3,
            looped: true,
            ..Play::default()
        },
    );
    assert_eq!(m.stats().voices, 2);
    m.apply(Command::Stop {
        voice: VoiceId(2),
        fade: 0,
    });
    assert_eq!(m.stats().voices, 2, "voice 2 was the one replaced");
}

#[test]
fn far_voices_run_virtual_and_keep_time() {
    let data = noise(4800);
    let sounds = [Pcm {
        samples: &data,
        rate: RATE,
        stereo: false,
    }];
    let mut m = mixer();
    play(
        &mut m,
        1,
        Play {
            at: at(0.0, 100.0),
            ..Play::default()
        },
    );
    let out = render(&mut m, &sounds, 2 * BLOCK);
    assert!(out.iter().all(|&s| s == 0.0));
    assert_eq!(m.stats().virtual_voices, 1);
    render(&mut m, &sounds, 4800);
    assert_eq!(m.stats().voices, 0, "a virtual voice still ends on time");
}

#[test]
fn loud_overlaps_bend_below_one_and_missing_sounds_end() {
    let data = vec![1.0; 4800];
    let sounds = [Pcm {
        samples: &data,
        rate: RATE,
        stereo: false,
    }];
    let mut m = mixer();
    for id in 0..8 {
        play(&mut m, id, Play::default());
    }
    render(&mut m, &sounds, 512);
    let peak = m.stats().peak;
    assert!(peak > 0.9 && peak < 1.0, "{peak}");
    play(
        &mut m,
        9,
        Play {
            sound: SoundId(7),
            ..Play::default()
        },
    );
    render(&mut m, &sounds, 1);
    assert_eq!(m.stats().voices, 8);
    assert_eq!(m.stats().dropped, 1);
}
