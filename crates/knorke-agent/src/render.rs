//! `render`: the scene after a number of ticks as a PNG. With the feature `gpu` this draws
//! on a Vulkan device through `knorke_integration::stealth_view`; without it the operation
//! answers [`NO_GPU`].

use knorke_scene::Scene;
use knorke_types::{Diagnostic, DiagnosticCode};
use serde_json::Value;

use crate::Order;

/// No GPU: the build has no GPU support, or no Vulkan device was found.
pub const NO_GPU: DiagnosticCode = DiagnosticCode::new("knorke.agent.no_gpu");
/// The shader pack was not found.
pub const NO_SHADERS: DiagnosticCode = DiagnosticCode::new("knorke.agent.no_shaders");

/// Where the shader pack is looked for when neither `--shaders` nor `KNORKE_SHADERS` names it.
pub const DEFAULT_SHADERS: &str = "target/shaders/knorke-chiaro.spirv.kpak";

/// Runs `scene` for `ticks` ticks with `orders`, draws it and writes the picture to `out`.
///
/// # Errors
/// What the scene's validation reports, [`NO_GPU`], [`NO_SHADERS`], or an I/O error.
#[cfg(not(feature = "gpu"))]
pub fn render(
    _scene: &Scene,
    _orders: &[Order],
    _ticks: u64,
    _shaders: Option<&str>,
    _out: &str,
) -> Result<Value, Vec<Diagnostic>> {
    let message = "this build has no GPU support (cargo feature `gpu`)";
    Err(vec![Diagnostic::error(NO_GPU, message)])
}

/// Runs `scene` for `ticks` ticks with `orders`, draws it and writes the picture to `out`.
///
/// # Errors
/// What the scene's validation reports, [`NO_GPU`], [`NO_SHADERS`], or an I/O error.
#[cfg(feature = "gpu")]
pub fn render(
    scene: &Scene,
    orders: &[Order],
    ticks: u64,
    shaders: Option<&str>,
    out: &str,
) -> Result<Value, Vec<Diagnostic>> {
    use knorke_gpu::sys::gpu::SDL_GPU_SHADERFORMAT_SPIRV;
    use knorke_gpu::{Device, Gpu, LiveCounter, init_video};
    use knorke_integration::stealth_view::{Pose, View};

    let one = |d: Diagnostic| vec![d];
    // The simulation first: an invalid scene is reported without touching the GPU.
    let mut rt = crate::start(scene)?;
    for _ in 0..ticks.min(crate::MAX_TICKS) {
        crate::tick(&mut rt, orders);
    }
    let from_env = std::env::var("KNORKE_SHADERS").ok();
    let path = shaders.or(from_env.as_deref()).unwrap_or(DEFAULT_SHADERS);
    let pack = std::fs::read(path).map_err(|e| {
        let message =
            format!("no shader pack at {path} ({e}); build it with `cargo xtask shaders`");
        one(Diagnostic::error(NO_SHADERS, message).at(path))
    })?;
    // The pack is SPIR-V, so the device has to be Vulkan.
    if !init_video(Some(c"vulkan")) {
        return Err(one(Diagnostic::error(NO_GPU, "SDL video did not start")));
    }
    let device = Device::create(SDL_GPU_SHADERFORMAT_SPIRV, false)
        .map_err(|d| one(Diagnostic::error(NO_GPU, d.message)))?;
    let counter = LiveCounter::new();
    let mut view = View::new(Gpu::new(&device, &counter), pack, scene).map_err(one)?;
    let pixels = view.draw(&Pose::of(&rt.sim.world.actors)).map_err(one)?;
    let (width, height) = view.size();

    let io = |e: &dyn core::fmt::Display| {
        one(Diagnostic::error(crate::IO, format!("{out}: {e}")).at(out))
    };
    let file = std::fs::File::create(out).map_err(|e| io(&e))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| io(&e))?;
    writer.write_image_data(&pixels).map_err(|e| io(&e))?;
    writer.finish().map_err(|e| io(&e))?;
    Ok(serde_json::json!({
        "picture": out, "width": width, "height": height, "tick": rt.next_tick().0,
    }))
}
