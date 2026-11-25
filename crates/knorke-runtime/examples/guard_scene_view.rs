//! The guard scene as a picture: the host's side of a frame (ADR 0009, phases 3 and 4).
//! `cargo run -p knorke-runtime --features gpu --example guard_scene_view -- <out dir>`
//! writes one PNG per frame.
//!
//! The drawing is `knorke_integration::stealth_view`; this file is the host's frame loop
//! ([`play`]): input, ticks, blending the pose of the last two ticks with `Frame::alpha`,
//! draw.
//!
//! Needs a Vulkan device and the shader pack from `cargo xtask shaders` (as the GPU tests).

// The scene is shared with its own example and tests; here its `main` is unused.
#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "guard_scene.rs"]
mod guard_scene;

pub use guard_scene::*;

pub use knorke_integration::stealth_view::{PX, Pose, View};

/// Simulation ticks per second of the played scene.
pub const TICKS_PER_SECOND: u64 = 4;
/// Drawn frames per second: three frames per tick, so two of three show a blended pose.
pub const FRAMES_PER_SECOND: u64 = 12;

/// Plays `script` at [`FRAMES_PER_SECOND`] for `frames` frames and calls `show` with every
/// drawn frame. The host's whole frame loop: input, ticks, blend, draw.
pub fn play(
    view: &mut View<'_>,
    sim: Sim,
    script: Script,
    frames: u64,
    mut show: impl FnMut(u64, &[u8]),
) {
    use knorke_runtime::{FixedStep, Runtime};
    const SECOND: u64 = 1_000_000_000;
    let step = FixedStep::new(SECOND / TICKS_PER_SECOND, 4);
    let mut rt = Runtime::new(sim, knorke_replay::NoRecord, step, knorke_types::Tick::ZERO);
    // Input: the script speaks in ticks; a command goes in once, just before its tick.
    let feed = |rt: &mut Runtime<Sim>| {
        if let Some(command) = script.command(rt.next_tick().0) {
            rt.submit(command);
        }
    };
    feed(&mut rt);
    let mut before = Pose::of(&rt.sim.world.actors);
    for f in 0..frames {
        let mut now = Pose::of(&rt.sim.world.actors);
        // At most one tick per frame, so the pose before the last tick is at hand to blend.
        let due = rt.frame(SECOND / FRAMES_PER_SECOND);
        debug_assert!(due.ticks <= 1, "three frames per tick");
        if due.ticks > 0 {
            before = now;
            now = Pose::of(&rt.sim.world.actors);
            feed(&mut rt);
        }
        let pixels = view
            .draw(&Pose::blend(&before, &now, due.alpha))
            .expect("the GPU draws");
        show(f, &pixels);
    }
}

#[allow(dead_code)]
fn main() {
    use knorke_contract_tests::parity::save_png_rgba8;
    use knorke_gpu::test_support::vulkan_device_with_pack;
    use knorke_gpu::{Gpu, LiveCounter};

    // <out dir> [scene file]; without a scene file the built-in yard.
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().expect("usage: <out dir> [scene.json]"));
    let scene = match args.next() {
        Some(path) => Scene::from_text(&std::fs::read_to_string(path).unwrap()).unwrap(),
        None => yard(),
    };
    let sim = build(&scene).unwrap_or_else(|found| panic!("invalid scene: {found:#?}"));
    std::fs::create_dir_all(&out).unwrap();
    let Some((device, pack)) = vulkan_device_with_pack() else {
        return;
    };
    let counter = LiveCounter::new();
    let mut view = View::new(Gpu::new(&device, &counter), pack, &scene).unwrap();
    let (w, h) = view.size();
    let frames = END * FRAMES_PER_SECOND / TICKS_PER_SECOND;
    play(&mut view, sim, Script::BOLD, frames, |f, pixels| {
        save_png_rgba8(&out.join(format!("guard_{f:03}.png")), pixels, w, h);
    });
    println!("{frames} frames in {}", out.display());
}
