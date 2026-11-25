//! The ring light of shockwaves (P21, Chiaro `ShockwaveLightPass`, E143) end to end: waves culled
//! by `knorke_post::shockwave::collect_shockwaves`, converted by `Light2dRenderer`, drawn by the
//! ring pass of `knorke_light2d` and compared with C++ Chiaro reference images
//! (`tests/golden/chiaro/ring_light/`, rendered by `ring_ref.c` there, chiaro @ 9bc5b9b, zlib
//! license) under `ParityBudget::STRICT`:
//!
//! - `ring_light`: a point light and two waves (one also distorts): the ring pass loads the
//!   light buffer after the light pass;
//! - `ring_only`: no light, an isometric projection, one wave: the ring pass clears the light
//!   buffer itself.
//!
//! The test plays the host (ADR 0002). It skips without a Vulkan device or the shader pack of
//! `cargo xtask shaders` (a failure with `KNORKE_REQUIRE_GPU=1`). `KNORKE_SCENE_OUT=<dir>` writes
//! each image there.
#![cfg(feature = "gpu")]

use std::path::PathBuf;

use knorke_contract_tests::parity::{ParityBudget, compare_rgba8, load_png_rgba8, save_png_rgba8};
use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
};
use knorke_gpu::test_support::{sampled_rgba8, texture, vulkan_device_with_pack, workspace_root};
use knorke_gpu::{CommandBuffer, DownloadBuffer, Gpu, LiveCounter};
use knorke_integration::light2d::{
    Light2dFrame, Light2dRenderer, Light2dSettings, post_projection,
};
use knorke_light2d::light::{LightDesc, LightType};
use knorke_light2d::pass_plan::{PassStages, SceneStores};
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{LightStore, NoResources, ObstacleStore};
use knorke_math::{At, LinearRgba, Vec2, Vec3};
use knorke_post::settings::PostSettings;
use knorke_post::shockwave::{Shockwave, ShockwaveFrame, ShockwaveList, collect_shockwaves};

const W: u32 = 128;
const H: u32 = 96;

/// `Wave()` of ring_ref.c.
fn wave(
    x: f32,
    y: f32,
    radius: f32,
    width: f32,
    strength: f32,
    light: [f32; 3],
    i: f32,
) -> Shockwave {
    Shockwave {
        x,
        y,
        radius,
        width,
        strength,
        light: LinearRgba::new(light[0], light[1], light[2], 1.0),
        light_intensity: i,
    }
}

/// The checker of ring_ref.c (and of gate_ref.c).
fn checker() -> Vec<u8> {
    let mut color = vec![0u8; (W * H * 4) as usize];
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            let i = ((y * W as i32 + x) * 4) as usize;
            let c: i32 = if ((x / 8) + (y / 8)) & 1 == 1 {
                12
            } else {
                -12
            };
            color[i..i + 4].copy_from_slice(&[
                (110 + c) as u8,
                (120 + c) as u8,
                (130 + c) as u8,
                255,
            ]);
        }
    }
    color
}

struct Scene {
    name: &'static str,
    projection: Projection,
    light: Option<LightDesc>,
    waves: Vec<Shockwave>,
}

fn scenes() -> [Scene; 2] {
    let mut light = LightDesc::new(LightType::Point);
    light.position = At::new(Vec2::new(30.0, 48.0));
    light.radius = 80.0;
    light.intensity = 1.0;
    light.validate().unwrap();
    [
        Scene {
            name: "ring_light",
            projection: Projection::default(),
            light: Some(light),
            waves: vec![
                wave(64.0, 48.0, 20.0, 10.0, 0.0, [1.0, 0.5, 0.2], 2.0),
                wave(100.0, 70.0, 12.0, 6.0, 2.0, [0.3, 0.6, 1.0], 1.5),
            ],
        },
        Scene {
            name: "ring_only",
            projection: Projection {
                origin: At::new(Vec2::new(64.0, 20.0)),
                x_axis: Vec2::new(2.0, 1.0),
                y_axis: Vec2::new(-2.0, 1.0),
                z_axis: Vec2::new(0.0, -2.0),
            },
            light: None,
            waves: vec![wave(20.0, 18.0, 9.0, 5.0, 0.0, [1.0, 0.7, 0.3], 3.0)],
        },
    ]
}

#[test]
fn ring_lights_match_chiaro() {
    let Some((device, pack_bytes)) = vulkan_device_with_pack() else {
        return;
    };
    let counter = LiveCounter::new();
    let gpu = Gpu::new(&device, &counter);
    let mut renderer = Light2dRenderer::new(gpu, pack_bytes).unwrap();
    let out = std::env::var("KNORKE_SCENE_OUT").ok().map(PathBuf::from);
    let color = sampled_rgba8(gpu, W, H, &checker());
    let target = texture(
        gpu,
        (W, H),
        SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
        SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
        None,
    );
    let settings = Light2dSettings {
        light: LightSettings {
            ambient: Vec3::new(0.1, 0.1, 0.1),
            ..LightSettings::default()
        },
        post: PostSettings::default(),
    };
    let obstacles = ObstacleStore::new();
    let mut download = DownloadBuffer::new(gpu, W * H * 4).unwrap();
    let mut failures = Vec::new();

    for scene in scenes() {
        let mut list = ShockwaveList::default();
        list.set(&scene.waves).unwrap();
        let post = post_projection(&scene.projection);
        let mut waves = ShockwaveFrame::default();
        collect_shockwaves(&list, &post, W, H, &mut waves);
        let frame = Light2dFrame {
            shockwaves: Some((&post, &waves)),
            ..Light2dFrame::new(color.texture_ref(), target.texture_ref(), &scene.projection)
        };
        let mut lights = LightStore::new();
        if let Some(l) = &scene.light {
            lights.create(l, &NoResources).unwrap();
        }
        let stores = SceneStores::new(&lights, &obstacles);

        let prepared = renderer.prepare(&frame, &stores, &settings).unwrap();
        assert_eq!(
            prepared.light.stages.contains(PassStages::LIGHTS),
            scene.light.is_some(),
            "{}",
            scene.name
        );
        assert!(prepared.light.lighting.light_buffer, "{}", scene.name);
        let mut cmd = CommandBuffer::acquire(&device).unwrap();
        let record = renderer
            .record(&mut cmd, &frame, &settings, prepared)
            .unwrap();
        assert_eq!(
            record.lighting.rings_drawn as usize,
            scene
                .waves
                .iter()
                .filter(|w| w.light_intensity > 0.0)
                .count(),
            "{}: one instance per wave with light",
            scene.name
        );
        cmd.copy_pass()
            .unwrap()
            .download_texture(frame.target, &download)
            .unwrap();
        cmd.submit_and_wait().unwrap();
        let mut pixels = vec![0u8; (W * H * 4) as usize];
        download.read(&mut pixels).unwrap();

        let reference = load_png_rgba8(
            &workspace_root().join(format!("tests/golden/chiaro/ring_light/{}.png", scene.name)),
            W,
            H,
        );
        let strict = compare_rgba8(&pixels, &reference, W, H, &[], &ParityBudget::STRICT).unwrap();
        strict.print(scene.name, "STRICT");
        if !strict.within(&ParityBudget::STRICT) {
            failures.push(format!("{}: {strict:?}", scene.name));
        }
        if let Some(dir) = &out {
            std::fs::create_dir_all(dir).unwrap();
            save_png_rgba8(&dir.join(format!("{}.rust.png", scene.name)), &pixels, W, H);
        }
    }
    drop(renderer);
    drop((color, target, download));
    assert_eq!(counter.live(), 0, "GPU objects leaked");
    assert!(failures.is_empty(), "STRICT parity missed: {failures:#?}");
}
