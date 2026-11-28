//! The guard yard in a window (ADR 0022 step 5): the arrow keys walk the intruder, the guard
//! looks with its lamp, chases and catches. Escape or closing the window ends it; an optional
//! argument ends it after that many seconds (for smoke runs).
//!
//! `KNORKE_SHADER_DIR=target/shaders cargo run -p knorke-host --features sdl-build-static --example stealth_yard [seconds]`
//!
//! The game owns the loop: SDL events → `InputState` → `Bindings` → `Command::Step` →
//! `Runtime::frame` with the host's real time → `View::draw_into` the swapchain texture →
//! the host presents. Real time never reaches the simulation, only commands per tick do.

use std::path::Path;

use knorke_gpu::kpak::source_checksum;
use knorke_host::{Host, HostConfig, ShaderPackSpec};
use knorke_input::{Bindings, Button, InputState, Scancode, Trigger};
use knorke_integration::stealth::{Command, Dir, build};
use knorke_integration::stealth_view::{PX, Pose, View};
use knorke_runtime::{FixedStep, Runtime};
use knorke_scene::Scene;
use knorke_types::Tick;

const YARD: &str = include_str!("../../knorke-integration/scenes/guard_yard.json");
/// Steps per second; a held arrow key walks this fast.
const TICKS_PER_SECOND: u64 = 6;
/// Window pixels per picture pixel.
const ZOOM: u32 = 4;

/// The full pack of `cargo xtask shaders`, checked against the shader sources of this
/// checkout, as the xtask sums them (every file of the source dirs, sorted by name). A
/// shipped game would bake the number in at build time instead.
fn pack_spec() -> Result<ShaderPackSpec, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../knorke-integration/shaders");
    let mut files = Vec::new();
    for dir in ["chiaro/src", "src", "shared"] {
        for entry in std::fs::read_dir(root.join(dir)).map_err(|e| format!("{dir}: {e}"))? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_file() {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into();
                files.push((name, std::fs::read(&path).map_err(|e| e.to_string())?));
            }
        }
    }
    files.sort();
    Ok(ShaderPackSpec {
        name: "knorke-chiaro",
        source_checksum: source_checksum(
            files
                .iter()
                .map(|(n, b): &(String, Vec<u8>)| (n.as_str(), b.as_slice())),
        ),
    })
}

fn main() -> Result<(), String> {
    let limit_nanos = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<f64>().ok())
        .map(|s| (s * 1e9) as u64);
    let shown = |d: knorke_types::Diagnostic| format!("{}: {}", d.code, d.message);

    let scene = Scene::from_text(YARD).map_err(|e| format!("{e:?}"))?;
    let sim = build(&scene).map_err(|found| format!("invalid scene: {found:#?}"))?;
    let (w, h) = sim.world.size();
    let host = Host::new(&HostConfig {
        title: "knorke stealth yard - arrow keys walk, Escape ends".into(),
        width: w as u32 * PX * ZOOM,
        height: h as u32 * PX * ZOOM,
        shader_pack: Some(pack_spec()?),
        ..HostConfig::default()
    })
    .map_err(shown)?;
    let pack = host.shader_pack().unwrap_or_default().to_vec();
    let mut view = View::new(host.gpu(), pack, &scene).map_err(shown)?;

    // Stage 2 of ADR 0022: keys → actions, a table the game owns. Here the actions already
    // are the commands (stage 3 is the identity).
    let mut bindings = Bindings::new();
    for (key, dir) in [
        (Scancode::UP, Dir::North),
        (Scancode::RIGHT, Dir::East),
        (Scancode::DOWN, Dir::South),
        (Scancode::LEFT, Dir::West),
    ] {
        bindings.bind(Button::Key(key), Trigger::Held, Command::Step(dir));
    }
    let mut input = InputState::new();
    let step = FixedStep::new(1_000_000_000 / TICKS_PER_SECOND, 4);
    let mut rt = Runtime::new(sim, knorke_replay::NoRecord, step, Tick::ZERO);
    let mut before = Pose::of(&rt.sim.world.actors);
    let mut total = 0u64;
    loop {
        let mut frame = host.begin_frame().map_err(shown)?;
        for ev in frame.input() {
            input.apply(*ev);
        }
        let quit = frame.quit() || input.pressed(Button::Key(Scancode::ESCAPE));
        let elapsed = frame.elapsed_nanos();
        total += elapsed;
        // Sample the input only for a frame that runs a tick: one command per tick, and a
        // tap between two ticks keeps its edge until then.
        let now = Pose::of(&rt.sim.world.actors);
        if rt.ticks_due(elapsed) > 0 {
            for command in bindings.actions(&input) {
                rt.submit(command);
            }
        }
        let due = rt.frame(elapsed);
        if due.ticks > 0 {
            input.end_tick();
            before = now;
        }
        let pose = Pose::blend(&before, &Pose::of(&rt.sim.world.actors), due.alpha);
        if let Some(target) = frame.target() {
            view.draw_into(frame.commands(), target, &pose)
                .map_err(shown)?;
        }
        host.end_frame(frame).map_err(shown)?;
        if quit || limit_nanos.is_some_and(|l| total >= l) {
            return Ok(());
        }
    }
}
