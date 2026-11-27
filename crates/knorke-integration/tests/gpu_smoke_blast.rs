//! The Chiaro demo scene `smoke_blast` (examples/smoke_demo of chiaro @ 9bc5b9b, zlib license)
//! at `--time 2.5`, end to end through [`Layers`](knorke_integration::layers::Layers) and
//! [`Light2dRenderer`](knorke_integration::light2d::Light2dRenderer), compared with the P3 golden
//! `tests/golden/chiaro/smoke_blast.png` under `ParityBudget::STRICT` (no HUD).
//!
//! The scene: haze 0.015 with shafts (the lamp behind the broken court wall throws its shaft
//! through the gap), the blast's flash and ember lights with envelope animations, the dust cloud
//! (entry 0), the ground fog patch (entry 4) and the grenade column (entry 1, not yet started at
//! 2.5 s) as 340 puffs of `SmokePuffs.hpp` drawn by `MediumBatch`, the glow layer's embers, the
//! surface boxes of walls and barrel, and the blast's `CHI_ExplosionShockwave`. At 2.5 s the wave
//! (1.5 s old) is past its life: no ring light and no distortion, the same in Chiaro (checked
//! against its desc); the ring light itself is compared in `gpu_ring_light.rs`.
//!
//! `tests/golden/chiaro/smoke_blast/smoke_blast_ref.cpp` repeats the demo's scene code, renders
//! frames 1 and 3 byte-identical to the golden, and writes the resolved scene as
//! `smoke_blast.txt` (format: `common/demo_scene.rs`) with the colour and glow layers.
//!
//! The test plays the host (ADR 0002). It skips without a Vulkan device or the shader pack of
//! `cargo xtask shaders` (a failure with `KNORKE_REQUIRE_GPU=1`). `KNORKE_SCENE_OUT=<dir>` writes
//! the frames and amplified diff images there.
#![cfg(feature = "gpu")]

#[path = "common/gpu.rs"]
mod scene_common;

#[path = "common/demo_scene.rs"]
mod demo_scene;

use demo_scene::{Demo, H, HEIGHT_STEP, Scene, W};
use knorke_integration::layers::{Layer, LayerSet};
use knorke_integration::light2d::Light2dSettings;
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::LightSettings;
use knorke_math::{At, Vec2, Vec3};
use knorke_post::output_rect::ScaleMode;
use knorke_post::settings::{Filter, PostSettings, Tonemap};

/// The demo's setup: `SetAmbient`, `SetScaling(INTEGER, NEAREST)`, `SetTonemap(SOFT)` (default
/// knee 0.8), `SetHeightStep(0.1)`, `SetBloom(0.5)` (enables bloom), `SetGlowScale(2)`, quality
/// exact with the linear medium filter, haze 0.015 with shafts.
fn settings(_: &Scene) -> Light2dSettings {
    let mut s = Light2dSettings {
        light: LightSettings {
            ambient: Vec3::new(0.07, 0.08, 0.12),
            surface_height_step: HEIGHT_STEP,
            // Chiaro's light: the scene is compared with Chiaro's picture.
            light_rows: false,
            ..LightSettings::default()
        },
        post: PostSettings::default(),
    };
    s.light.glow.scale = 2.0;
    s.light.haze.density = 0.015;
    s.light.haze.color = Vec3::new(0.85, 0.88, 1.0);
    s.post.output.scale_mode = ScaleMode::Integer;
    s.post.output.filter = Filter::Nearest;
    s.post.output.tonemap = Tonemap::Soft { knee: 0.8 };
    s.post.bloom.enabled = true;
    s.post.bloom.intensity = 0.5;
    s
}

#[test]
fn smoke_blast_matches_chiaro() {
    Demo {
        dir: "smoke_blast",
        variant: "smoke_blast",
        golden: "smoke_blast",
        kinds: LayerSet::of(Layer::Surface).with(Layer::Glow),
        layer_suffix: "",
        projection: Projection::iso(At::new(Vec2::new(11.0, 7.0)), 17.0, 1.0, W, H),
        time: 2.5,
        overlay: None,
        ignore: &[],
        // Chiaro's two layer kinds differ in 3 densities by 1 (the reference dumps both): the
        // batch equals the GPU layers, the light starts from the demo's renderer layer.
        medium_gpu: "medium_gpu",
        medium_renderer: Some("medium"),
    }
    .check(settings);
}
