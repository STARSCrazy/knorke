//! The guard yard in a window (ADR 0022 steps 5 and 6): the arrow keys walk the intruder, the
//! guard looks with its lamp, chases and catches. Escape or closing the window ends it.
//!
//! `KNORKE_SHADER_DIR=target/shaders cargo run -p knorke-host --features sdl-build-static --example stealth_yard -- [seconds] [--record <file>] [--replay <file>]`
//!
//! - `--features audio` adds the yard's sound (wind, steps, alarm, catch; see `sound.rs`).
//!   Without it, neither `knorke-audio-sdl` is built nor an audio device opened.
//! - `seconds` ends the run after that long (for smoke runs).
//! - `--record <file>` records the session; at the end it is replayed in memory with
//!   `verify` (result on stderr) and written to the file.
//! - `--replay <file>` opens no window: it reads the file, rejects one of another scene and
//!   checks it with `verify`.
//!
//! The game owns the loop: SDL events → [`session::Session`] (`InputState` → `Bindings` →
//! `Command::Step` → `Runtime::frame` with the host's real time) → `View::draw_into` the
//! swapchain texture → the host presents. Real time never reaches the simulation, only
//! commands per tick do.

// Shared with `tests/stealth_session.rs`; its `pub` items are its face there.
#[allow(unreachable_pub)]
mod session;
#[cfg(feature = "audio")]
mod sound;

use std::path::{Path, PathBuf};

use knorke_gpu::kpak::source_checksum;
use knorke_host::{Host, HostConfig, ShaderPackSpec};
use knorke_input::{Button, InputEvent, Scancode};
use knorke_integration::stealth::{Command, build, read_replay, replay_header};
use knorke_integration::stealth_view::{PX, Pose, View};
use knorke_replay::{ReplayLog, verify};
use knorke_scene::Scene;
use knorke_types::Diagnostic;
use session::Session;

const YARD: &str = include_str!("../../../knorke-integration/scenes/guard_yard.json");
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

/// What the command line asked for.
#[derive(Default)]
struct Args {
    limit_nanos: Option<u64>,
    record: Option<PathBuf>,
    replay: Option<PathBuf>,
}

fn args() -> Result<Args, String> {
    let mut out = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut file = || {
            it.next()
                .map(PathBuf::from)
                .ok_or(format!("{arg} needs a file"))
        };
        match arg.as_str() {
            "--record" => out.record = Some(file()?),
            "--replay" => out.replay = Some(file()?),
            s => {
                let secs: f64 = s.parse().map_err(|_| format!("unknown argument {s}"))?;
                out.limit_nanos = Some((secs * 1e9) as u64);
            }
        }
    }
    Ok(out)
}

fn shown(d: Diagnostic) -> String {
    format!("{}: {}", d.code, d.message)
}

/// Replays `log` on a fresh build of `scene`; the result goes to stderr.
fn check(scene: &Scene, log: &ReplayLog<Command>) -> Result<(), String> {
    let mut sim = build(scene).map_err(|found| format!("invalid scene: {found:#?}"))?;
    match verify(&mut sim, log) {
        Ok(report) => {
            eprintln!(
                "verify: ok, {} ticks replayed, {} checked, {} commands",
                report.ticks,
                report.checked,
                log.command_count()
            );
            Ok(())
        }
        Err(e) => Err(format!("verify failed: {e:?}")),
    }
}

fn main() -> Result<(), String> {
    let args = args()?;
    let scene = Scene::from_text(YARD).map_err(shown)?;
    if let Some(path) = &args.replay {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        return check(&scene, &read_replay(&bytes, &scene).map_err(shown)?);
    }

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

    let mut game = Session::new(sim, args.record.is_some().then(|| replay_header(&scene)));
    #[cfg(feature = "audio")]
    let mut sound = {
        let (cfg, clips) = sound::Sound::device();
        sound::Sound::new(host.open_audio(cfg, clips).map_err(shown)?, &game.rt.sim)
    };
    let mut before = Pose::of(&game.rt.sim.world.actors);
    let mut total = 0u64;
    loop {
        let mut frame = host.begin_frame().map_err(shown)?;
        let quit = frame.quit()
            || frame.input().iter().any(|e| {
                *e == InputEvent::Button {
                    button: Button::Key(Scancode::ESCAPE),
                    down: true,
                }
            });
        let elapsed = frame.elapsed_nanos();
        total += elapsed;
        let now = Pose::of(&game.rt.sim.world.actors);
        let due = game.frame(frame.input(), elapsed);
        if due.ticks > 0 {
            before = now;
            #[cfg(feature = "audio")]
            sound.update(&game.rt.sim);
        }
        let pose = Pose::blend(&before, &Pose::of(&game.rt.sim.world.actors), due.alpha);
        if let Some(target) = frame.target() {
            view.draw_into(frame.commands(), target, &pose)
                .map_err(shown)?;
        }
        host.end_frame(frame).map_err(shown)?;
        if quit || args.limit_nanos.is_some_and(|l| total >= l) {
            break;
        }
    }
    #[cfg(feature = "audio")]
    drop(sound);
    drop(view);
    drop(host);

    if let (Some(path), Some(log)) = (&args.record, game.into_log()) {
        check(&scene, &log)?;
        let mut bytes = Vec::new();
        log.write(&mut bytes);
        std::fs::write(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        eprintln!("recorded {} bytes to {}", bytes.len(), path.display());
    }
    Ok(())
}
