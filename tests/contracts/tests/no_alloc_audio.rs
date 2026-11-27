//! Contract: after `Mixer::new`, applying commands and rendering allocate nothing – the audio
//! callback has a hard deadline (ADR 0020, ADR 0006). Covers placed and unplaced voices, voice
//! replacement, fades, bus ducking, headphones, a room change, delayed starts, the cap per
//! sound, bus filters and sidechain ducking.
//! Own test binary with a single test, so the process-wide allocation counter is not disturbed
//! by other tests.

use knorke_audio::{Bus, Command, Config, Duck, Filter, Listener, Mixer, Pcm, Play, Room, VoiceId};
use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_math::{At, Vec2};

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn mixer_steady_state_allocates_nothing() {
    let data: Vec<f32> = (0..22_050)
        .map(|i| ((i % 97) as f32 / 48.0) - 1.0)
        .collect();
    let stereo: Vec<f32> = data.iter().flat_map(|&s| [s, -s]).collect();
    let sounds = [
        Pcm {
            samples: &data,
            rate: 22_050,
            stereo: false,
        },
        Pcm {
            samples: &stereo,
            rate: 44_100,
            stereo: true,
        },
    ];
    let mut m = Mixer::new(Config {
        voices: 8,
        sound_speed: 40.0,
        ..Config::default()
    });
    let mut out = vec![0.0; 2 * 480];

    let before = allocations();
    for tick in 0..200u32 {
        let id = VoiceId(tick % 12);
        let play = Play {
            sound: knorke_audio::SoundId(tick % 2),
            bus: Bus((tick % 3) as u8),
            at: (tick % 2 == 0).then(|| At::new(Vec2::new((tick % 7) as f32 - 3.0, 2.0))),
            walls: (tick % 3) as f32,
            looped: tick % 5 == 0,
            priority: (tick % 4) as u8,
            fade_in: 100,
            start: m.clock() + u64::from(tick % 4) * 300,
            limit: (tick % 3) as u8,
            ..Play::default()
        };
        m.apply(Command::Play { voice: id, play });
        m.apply(Command::Place {
            voice: id,
            at: At::new(Vec2::new(1.0, (tick % 9) as f32)),
        });
        m.apply(Command::Stop {
            voice: VoiceId((tick + 6) % 12),
            fade: 300,
        });
        m.apply(Command::BusGain {
            bus: Bus(1),
            gain: (tick % 2) as f32,
            fade: 960,
        });
        m.apply(Command::Headphones(tick % 3 == 0));
        let filter = [Filter::NONE, Filter::UNDERWATER, Filter::RADIO][(tick / 7 % 3) as usize];
        m.apply(Command::Filter {
            bus: Bus((tick % 3) as u8),
            filter,
            fade: 240,
        });
        m.apply(Command::Duck {
            bus: Bus(0),
            duck: (tick % 11 != 0).then_some(Duck {
                by: Bus(2),
                depth: 0.6,
                attack_ms: 10,
                release_ms: 200,
            }),
        });
        let room = [Room::NONE, Room::SMALL, Room::STONE, Room::CAVE][(tick / 40 % 4) as usize];
        m.apply(Command::Listener(Listener {
            room,
            ..Listener::default()
        }));
        m.render(&sounds[..], &mut out);
    }
    assert_eq!(allocations() - before, 0, "mixer allocated in apply/render");
    assert!(m.stats().peak < 1.0);
}
