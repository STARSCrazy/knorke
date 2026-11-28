//! ADR 0022 step 6: a recorded session of the stealth yard, played with a made-up input
//! stream through the loop of the example, without a window. The recording verifies, is
//! independent of the frame rate, survives a file round trip and is refused for another
//! scene.

// The loop is shared with the example; its `pub` items are its face there.
#[allow(dead_code, unreachable_pub)]
#[path = "../examples/stealth_yard/session.rs"]
mod session;

use knorke_input::{Button, InputEvent, Scancode};
use knorke_integration::stealth::{
    Command, Dir, REPLAY_FILE, REPLAY_SCENE, build, read_replay, replay_header,
};
use knorke_replay::{ReplayLog, verify};
use knorke_scene::Scene;
use knorke_types::Tick;
use session::{Session, TICKS_PER_SECOND};

const YARD: &str = include_str!("../../knorke-integration/scenes/guard_yard.json");
const TICK: u64 = 1_000_000_000 / TICKS_PER_SECOND;
/// Ticks each run lasts.
const TICKS: u64 = 20;

fn scene() -> Scene {
    Scene::from_text(YARD).unwrap()
}

/// Key changes at half ticks, so that they sit far from every tick boundary: a frame shorter
/// than half a tick always sees them on the same side of a boundary, at any frame rate.
fn script() -> Vec<(u64, InputEvent)> {
    let key = |at: f64, k: Scancode, down: bool| {
        (
            (at * TICK as f64) as u64,
            InputEvent::Button {
                button: Button::Key(k),
                down,
            },
        )
    };
    vec![
        key(0.5, Scancode::RIGHT, true),
        key(3.5, Scancode::RIGHT, false),
        key(5.5, Scancode::DOWN, true),
        key(8.5, Scancode::DOWN, false),
        key(10.5, Scancode::LEFT, true),
        key(12.5, Scancode::LEFT, false),
    ]
}

/// Plays the script for [`TICKS`] ticks; `frame(i)` is the length of frame `i` in nanos.
fn play(mut frame: impl FnMut(u64) -> u64) -> ReplayLog<Command> {
    let scene = scene();
    let mut game = Session::new(build(&scene).unwrap(), Some(replay_header(&scene)));
    let script = script();
    let (mut now, mut sent, mut i) = (0u64, 0usize, 0u64);
    while game.rt.next_tick() < Tick(TICKS) {
        let elapsed = frame(i);
        i += 1;
        now += elapsed;
        // The events that happened up to now, as the host collects them before a frame.
        let upto = sent
            + script[sent..]
                .iter()
                .take_while(|(at, _)| *at <= now)
                .count();
        let events: Vec<InputEvent> = script[sent..upto].iter().map(|(_, e)| *e).collect();
        sent = upto;
        assert!(
            game.frame(&events, elapsed).ticks <= 1,
            "a frame is shorter than a tick"
        );
    }
    game.into_log().unwrap()
}

/// Frame lengths between 4 and 40 ms, from a fixed pseudo-random sequence.
fn jitter() -> impl FnMut(u64) -> u64 {
    let mut x = 0x2545_f491_u64;
    move |_| {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        4_000_000 + (x >> 33) % 36_000_000
    }
}

#[test]
fn a_session_with_changing_frame_times_verifies() {
    let log = play(jitter());
    assert_eq!(log.ticks().len(), TICKS as usize);
    // One step per tick while an arrow is held: 3 right, 3 down, 2 left.
    let steps: Vec<Command> = log
        .ticks()
        .iter()
        .flat_map(|t| log.commands_of(t).iter().copied())
        .collect();
    let want: Vec<Command> = [(Dir::East, 3), (Dir::South, 3), (Dir::West, 2)]
        .into_iter()
        .flat_map(|(d, n)| std::iter::repeat_n(Command::Step(d), n))
        .collect();
    assert_eq!(steps, want);
    let report = verify(&mut build(&scene()).unwrap(), &log).unwrap();
    assert_eq!(report.checked, TICKS as usize);
}

#[test]
fn the_same_input_at_30_and_144_fps_records_the_same_run() {
    let at_30 = play(|_| 1_000_000_000 / 30);
    let at_144 = play(|_| 1_000_000_000 / 144);
    // Same commands per tick and the same checksums after every tick.
    assert_eq!(at_30, at_144);
    assert_eq!(at_30, play(jitter()));
}

#[test]
fn a_saved_session_verifies_after_reading_it_back() {
    let scene = scene();
    let mut bytes = Vec::new();
    play(jitter()).write(&mut bytes);
    // A fresh build stands for the restarted program.
    let read = read_replay(&bytes, &scene).unwrap();
    verify(&mut build(&scene).unwrap(), &read).unwrap();
}

#[test]
fn a_file_of_another_scene_or_no_replay_is_refused() {
    let mut bytes = Vec::new();
    play(jitter()).write(&mut bytes);
    let mut other = scene();
    // Any change of the scene content, here one entity less.
    let first = other.entities.keys().next().unwrap().clone();
    other.entities.remove(&first);
    assert_eq!(read_replay(&bytes, &other).unwrap_err().code, REPLAY_SCENE);
    assert_eq!(
        read_replay(b"KNRPxx", &scene()).unwrap_err().code,
        REPLAY_FILE
    );
}
