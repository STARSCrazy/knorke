//! A one-minute listening tour of the mixer: music, a fountain that circles you, walks away and
//! hides behind walls, dripping water in three rooms, headphone mode, music ducking and
//! bus filters (underwater, radio).
//!
//! All sounds are made in code, so no files are needed:
//! `cargo run -p knorke-audio-sdl --example dungeon_sound --features sdl-build-static`
//! (needs CMake; `--features sdl` with an installed SDL3 works too).

use std::f32::consts::TAU;
use std::thread::sleep;
use std::time::Duration;

use knorke_audio::{Bus, Command, Config, Filter, Listener, Play, Room, SoundId, VoiceId};
use knorke_audio_sdl::{Clip, Output};
use knorke_math::det;
use knorke_math::{At, Vec2};

const RATE: u32 = 48_000;
const MUSIC: Bus = Bus(0);
const EFFECTS: Bus = Bus(1);
const SONG: VoiceId = VoiceId(1);
const FOUNTAIN: VoiceId = VoiceId(2);
/// Game loop step: commands are sent 20 times per second, like a slow game tick.
const STEP: Duration = Duration::from_millis(50);

fn main() {
    let cfg = Config {
        rate: RATE,
        ..Config::default()
    };
    let clips = vec![song(), fountain(), drip()];
    let out = match Output::open(cfg, clips) {
        Ok(out) => out,
        Err(e) => {
            eprintln!("Kein Audiogerät: {e}");
            std::process::exit(1);
        }
    };
    let send = |cmd| {
        if out.send(cmd).is_err() {
            eprintln!("Befehl verworfen (Eingang voll)");
        }
    };
    let ms = |ms| cfg.frames(ms);

    say("1/10 Musik allein");
    send(Command::Play {
        voice: SONG,
        play: Play {
            sound: SoundId(0),
            bus: MUSIC,
            gain: 0.35,
            looped: true,
            priority: 200,
            fade_in: ms(1500),
            ..Play::default()
        },
    });
    wait(3.0);

    say("2/10 Ein Brunnen rechts neben dir");
    send(Command::Play {
        voice: FOUNTAIN,
        play: Play {
            sound: SoundId(1),
            bus: EFFECTS,
            gain: 0.8,
            at: Some(at(2.0, 0.0)),
            looped: true,
            priority: 100,
            fade_in: ms(500),
            ..Play::default()
        },
    });
    wait(3.0);

    say("3/10 Der Brunnen kreist um dich (hinter dir klingt er dumpfer)");
    circle(&send, 8.0);

    say("4/10 Der Brunnen entfernt sich vor dir (leiser und dumpfer)");
    ticks(6.0, |t| {
        send(Command::Place {
            voice: FOUNTAIN,
            at: at(0.0, 1.0 + 10.0 * t),
        })
    });

    say("5/10 Der Brunnen kommt zurück: offen, eine Wand, zwei Wände, wieder offen");
    send(Command::Place {
        voice: FOUNTAIN,
        at: at(0.0, 3.0),
    });
    for walls in [0.0, 1.0, 2.0, 0.0] {
        send(Command::Occlude {
            voice: FOUNTAIN,
            walls,
        });
        wait(2.0);
    }

    say("6/10 Tropfen in drei Räumen: kleiner Raum, Steinhalle, Höhle");
    send(Command::Gain {
        voice: FOUNTAIN,
        gain: 0.25,
        fade: ms(500),
    });
    let mut drop_voice = 10;
    for (name, room) in [
        ("kleiner Raum", Room::SMALL),
        ("Steinhalle", Room::STONE),
        ("Höhle", Room::CAVE),
    ] {
        say(&format!("     {name}"));
        send(Command::Listener(Listener {
            room,
            ..Listener::default()
        }));
        for i in 0..5 {
            send(Command::Play {
                voice: VoiceId(drop_voice),
                play: Play {
                    sound: SoundId(2),
                    bus: EFFECTS,
                    gain: 0.7,
                    pitch: [1.0, 1.12, 0.94, 1.06, 0.89][i],
                    at: Some(at([-1.5, 1.0, 0.5, -0.5, 1.5][i], 1.5)),
                    priority: 50,
                    ..Play::default()
                },
            });
            drop_voice += 1;
            wait(0.7);
        }
    }
    send(Command::Listener(Listener::default()));
    send(Command::Gain {
        voice: FOUNTAIN,
        gain: 0.8,
        fade: ms(500),
    });

    say("7/10 Kopfhörer-Modus an, der Brunnen kreist wieder (am besten mit Kopfhörern)");
    send(Command::Headphones(true));
    circle(&send, 8.0);
    send(Command::Headphones(false));

    say("8/10 Musik wird leiser gedreht, als würde jemand sprechen, dann wieder lauter");
    send(Command::BusGain {
        bus: MUSIC,
        gain: 0.2,
        fade: ms(400),
    });
    wait(3.0);
    send(Command::BusGain {
        bus: MUSIC,
        gain: 1.0,
        fade: ms(800),
    });
    wait(2.0);

    say("9/10 Unter Wasser (alles dumpf), dann wie aus dem Funkgerät, dann wieder klar");
    for filter in [Filter::UNDERWATER, Filter::RADIO, Filter::NONE] {
        for bus in [MUSIC, EFFECTS] {
            send(Command::Filter {
                bus,
                filter,
                fade: ms(600),
            });
        }
        wait(3.0);
    }

    say("10/10 Alles blendet aus");
    send(Command::StopBus {
        bus: MUSIC,
        fade: ms(2000),
    });
    send(Command::StopBus {
        bus: EFFECTS,
        fade: ms(2000),
    });
    wait(2.5);
    say(&format!(
        "Fertig: {:.1} s Ton ausgegeben",
        out.frames() as f64 / f64::from(RATE)
    ));
}

fn say(text: &str) {
    println!("{text}");
}

fn wait(seconds: f32) {
    ticks(seconds, |_| {});
}

/// Calls `f` once per game step with the progress `0..1` through `seconds`.
fn ticks(seconds: f32, mut f: impl FnMut(f32)) {
    let steps = (seconds / STEP.as_secs_f32()) as u32;
    for i in 0..steps {
        f(i as f32 / steps as f32);
        sleep(STEP);
    }
}

/// One turn around the listener at two cells, starting on the right, going clockwise through
/// the front (the listener faces +y).
fn circle(send: &impl Fn(Command), seconds: f32) {
    ticks(seconds, |t| {
        let (s, c) = det::sin_cos(TAU * t);
        send(Command::Place {
            voice: FOUNTAIN,
            at: at(2.0 * c, 2.0 * s),
        });
    });
}

fn at(x: f32, y: f32) -> At<knorke_math::World2, Vec2> {
    At::new(Vec2::new(x, y))
}

/// Deterministic white noise in `-1..=1` (no random crate needed).
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

/// Four seconds of an A-minor arpeggio, plucked notes with a little overtone, as a loop.
fn song() -> Clip {
    const NOTES: [f32; 16] = [
        220.0, 261.6, 329.6, 440.0, 329.6, 261.6, 196.0, 246.9, 293.7, 392.0, 293.7, 246.9, 174.6,
        220.0, 261.6, 349.2,
    ];
    let note_len = RATE as usize / 4;
    let mut samples = Vec::with_capacity(NOTES.len() * note_len);
    for f in NOTES {
        let (mut phase, step) = (0.0f32, TAU * f / RATE as f32);
        let mut env = 1.0f32;
        for _ in 0..note_len {
            samples.push(env * (0.7 * det::sin(phase) + 0.2 * det::sin(2.0 * phase)));
            phase = (phase + step) % TAU;
            env *= 0.99985;
        }
    }
    Clip {
        samples,
        rate: RATE,
        stereo: false,
    }
}

/// Two seconds of splashing water: filtered noise with a slow wobble, as a loop.
fn fountain() -> Clip {
    let n = 2 * RATE as usize;
    let mut noise = Noise(7);
    let (mut lp, mut samples) = (0.0f32, Vec::with_capacity(n));
    for i in 0..n {
        lp += 0.25 * (noise.next() - lp);
        let wobble = 0.75 + 0.25 * det::sin(TAU * 3.0 * i as f32 / n as f32);
        samples.push(0.9 * lp * wobble);
    }
    Clip {
        samples,
        rate: RATE,
        stereo: false,
    }
}

/// A water drop: a short falling "plink".
fn drip() -> Clip {
    let n = RATE as usize / 5;
    let (mut phase, mut env, mut samples) = (0.0f32, 1.0f32, Vec::with_capacity(n));
    for i in 0..n {
        let f = 1400.0 - 700.0 * (i as f32 / n as f32);
        phase = (phase + TAU * f / RATE as f32) % TAU;
        samples.push(0.8 * env * det::sin(phase));
        env *= 0.9994;
    }
    Clip {
        samples,
        rate: RATE,
        stereo: false,
    }
}
