//! Window host of the Knorke Engine (ADR 0022): the topmost, detachable layer.
//!
//! A toolbox, not a framework: the game owns `main` and the loop, there is no `run(app)`
//! and no callbacks (ADR 0008). One round:
//!
//! ```text
//! let mut frame = host.begin_frame()?;   // drain SDL events, measure time, acquire target
//! for ev in frame.input() { input.apply(*ev) }
//! if frame.quit() { break }
//! rt.frame(frame.elapsed_nanos());       // the game passes time on as a number
//! module.draw(frame.commands(), frame.target());
//! host.end_frame(frame)?;                // only the host submits and presents (ADR 0002)
//! ```
//!
//! Without features the crate holds the parts that need no SDL: the clock cap
//! ([`cap_elapsed`]) and shader pack loading with its diagnostics ([`load_shader_pack`]).
//! Feature `sdl` adds [`Host`] and [`Frame`].

use std::path::{Path, PathBuf};

use knorke_gpu::kpak::{ShaderFormat, ShaderPack};
use knorke_types::{Diagnostic, DiagnosticCode};

#[cfg(feature = "sdl")]
#[allow(unsafe_code)]
mod sdl;
#[cfg(feature = "sdl")]
pub use sdl::{Frame, Host, HostConfig};

#[cfg(feature = "audio")]
pub use knorke_audio_sdl as audio;

/// No shader pack file at the expected path.
pub const SHADER_PACK_MISSING: DiagnosticCode =
    DiagnosticCode::new("knorke.host.shader_pack.missing");
/// The pack file exists but was not built from the sources the game expects (or is not a
/// readable pack): rebuild it with `cargo xtask shaders`.
pub const SHADER_PACK_STALE: DiagnosticCode = DiagnosticCode::new("knorke.host.shader_pack.stale");
/// No GPU device, or the window cannot be presented to with a linear (sRGB) swapchain.
pub const DEVICE_UNAVAILABLE: DiagnosticCode =
    DiagnosticCode::new("knorke.host.device.unavailable");

/// No audio device could be opened (ADR 0022 step 7).
#[cfg(feature = "audio")]
pub const AUDIO_UNAVAILABLE: DiagnosticCode = DiagnosticCode::new("knorke.host.audio.unavailable");

#[cfg(feature = "audio")]
impl Host {
    /// Opens the default playback device with a mixer at `cfg.rate` over `clips`; the game
    /// sends the sounds as mixer commands. Dropping the output closes the device.
    pub fn open_audio(
        &self,
        cfg: audio::Config,
        clips: Vec<audio::Clip>,
    ) -> Result<audio::Output, Diagnostic> {
        audio::Output::open(cfg, clips)
            .map_err(|e| Diagnostic::error(AUDIO_UNAVAILABLE, format!("audio device: {e}")))
    }
}

/// Default cap for one frame's time step: a dragged window or a debugger halt must not turn
/// into seconds of catch-up; `FixedStep::max_ticks` handles the rest.
pub const DEFAULT_MAX_FRAME_NANOS: u64 = 250_000_000;

/// Time since the last frame, capped at `max` (both in nanoseconds). A clock that went
/// backwards counts as no time.
pub fn cap_elapsed(last: u64, now: u64, max: u64) -> u64 {
    now.saturating_sub(last).min(max)
}

/// The shader pack a game needs: file stem (`knorke-chiaro` →
/// `knorke-chiaro.spirv.kpak`) and the source checksum the game was built against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderPackSpec {
    /// File stem of the pack.
    pub name: &'static str,
    /// Expected [`ShaderPack::source_checksum`].
    pub source_checksum: u64,
}

/// Where packs are looked for: `KNORKE_SHADER_DIR`, else `shaders/` next to the executable.
pub fn shader_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("KNORKE_SHADER_DIR") {
        return dir.into();
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("shaders")))
        .unwrap_or_else(|| "shaders".into())
}

/// Reads `<dir>/<name>.<format>.kpak` and checks it against the spec. No fallback: a missing
/// or stale pack is an error the player sees, not a silently different picture.
pub fn load_shader_pack(
    dir: &Path,
    spec: &ShaderPackSpec,
    format: ShaderFormat,
) -> Result<Vec<u8>, Diagnostic> {
    let path = dir.join(format.pack_file_name(spec.name));
    let bytes = std::fs::read(&path).map_err(|e| {
        Diagnostic::error(
            SHADER_PACK_MISSING,
            format!(
                "shader pack {} not readable ({e}); set KNORKE_SHADER_DIR or run cargo xtask shaders",
                path.display()
            ),
        )
    })?;
    let stale = |d: Diagnostic| {
        Diagnostic::error(
            SHADER_PACK_STALE,
            format!("shader pack {}: {}", path.display(), d.message),
        )
    };
    let pack = ShaderPack::parse(&bytes).map_err(stale)?;
    if pack.format() != format {
        return Err(stale(Diagnostic::error(
            SHADER_PACK_STALE,
            format!("holds {} programs", pack.format().tag()),
        )));
    }
    pack.expect_source_checksum(spec.source_checksum)
        .map_err(stale)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use knorke_gpu::kpak::write_pack;

    #[test]
    fn elapsed_is_capped_and_never_negative() {
        assert_eq!(cap_elapsed(100, 350, DEFAULT_MAX_FRAME_NANOS), 250);
        assert_eq!(
            cap_elapsed(0, 2_000_000_000, DEFAULT_MAX_FRAME_NANOS),
            250_000_000
        );
        assert_eq!(cap_elapsed(500, 400, DEFAULT_MAX_FRAME_NANOS), 0);
    }

    #[test]
    fn missing_and_stale_packs_have_their_codes() {
        let dir = std::env::temp_dir().join(format!("knorke-host-pack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let spec = ShaderPackSpec {
            name: "probe",
            source_checksum: 7,
        };
        let err = load_shader_pack(&dir, &spec, ShaderFormat::Spirv).unwrap_err();
        assert_eq!(err.code, SHADER_PACK_MISSING);

        let path = dir.join(ShaderFormat::Spirv.pack_file_name("probe"));
        std::fs::write(&path, b"not a pack").unwrap();
        let err = load_shader_pack(&dir, &spec, ShaderFormat::Spirv).unwrap_err();
        assert_eq!(err.code, SHADER_PACK_STALE);

        let pack = |checksum| write_pack(ShaderFormat::Spirv, checksum, &[]).unwrap();
        std::fs::write(&path, pack(8)).unwrap();
        let err = load_shader_pack(&dir, &spec, ShaderFormat::Spirv).unwrap_err();
        assert_eq!(err.code, SHADER_PACK_STALE);

        std::fs::write(&path, pack(7)).unwrap();
        assert!(load_shader_pack(&dir, &spec, ShaderFormat::Spirv).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
