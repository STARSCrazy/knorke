//! The 2D GPU light of the Chiaro port end to end (P31): the scenes of the former gate spike
//! rendered through `knorke_light2d::pass_plan::PassPlan` and `knorke_post::pass::PostRenderer`,
//! joined by `knorke_integration::light2d::Light2dRenderer`, on Vulkan, and compared with C++
//! Chiaro reference images (`tests/golden/chiaro`, chiaro @ 9bc5b9b, zlib license) under
//! `ParityBudget::STRICT`:
//!
//! - `minimal_normal_map`: the P3 golden scene (one point light on a normal-mapped dome);
//! - `gate_sprite_shadow`: one soft point light, shadows from an occlusion sprite buffer (the
//!   application sprite mask of `OcclusionMask`), rendered by `tests/golden/chiaro/gate/gate_ref.c`.
//!
//! The test plays the host (ADR 0002): it owns device, textures and command buffers, submits
//! and reads back, all through the safe API of `knorke-gpu`. It skips (passes with a note)
//! without a Vulkan device or without the shader pack from `cargo xtask shaders` in
//! `target/shaders/`; with `KNORKE_REQUIRE_GPU=1` a skip is a failure.
//!
//! `KNORKE_SCENE_OUT=<dir>` writes each image and an amplified diff image there;
//! `KNORKE_SCENE_BENCH=<frames>` also times that many frames (wall clock, record + submit +
//! wait, as `gate_ref.c` does with `CHI_Render`).
#![cfg(feature = "gpu")]

use std::path::PathBuf;
use std::time::Instant;

use knorke_contract_tests::parity::{ParityBudget, compare_rgba8, load_png_rgba8, save_png_rgba8};
use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
};
use knorke_gpu::test_support::{sampled_rgba8, texture, vulkan_device_with_pack, workspace_root};
use knorke_gpu::{CommandBuffer, Device, DownloadBuffer, Gpu, LiveCounter, Texture};
use knorke_integration::light2d::{Light2dFrame, Light2dRenderer, Light2dSettings};
use knorke_light2d::codec::{AlphaSource, Material, NormalMapOptions, convert_normal_map};
use knorke_light2d::light::{LightDesc, LightType};
use knorke_light2d::pass_plan::{PassStages, SceneStores};
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::LightSettings;
use knorke_light2d::shadow::MaskUpdate;
use knorke_light2d::{LightStore, NoResources, ObstacleStore};
use knorke_math::{At, Vec2, Vec3};
use knorke_post::settings::{Filter as OutputFilter, PostSettings};

/// A scene as RGBA8 inputs plus light and settings.
struct Scene {
    name: &'static str,
    w: u32,
    h: u32,
    color: Vec<u8>,
    material: Option<Vec<u8>>,
    occlusion: Option<Vec<u8>>,
    light: LightDesc,
    ambient: f32,
    filter: OutputFilter,
}

/// examples/minimal_normal_map/main.c: a grey square, a dome normal map (satin), one white
/// point light at (4, 32), radius 96, intensity 1.5, automatic height; ambient 0.1; integer
/// scaling with nearest filter.
fn minimal_normal_map() -> Scene {
    let (w, h) = (64u32, 64u32);
    let color = [160u8, 160, 160, 255].repeat((w * h) as usize);
    let mut normal = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f32 + 0.5 - (w / 2) as f32) / 24.0;
            let dy = (y as f32 + 0.5 - (h / 2) as f32) / 24.0;
            let d2 = dx * dx + dy * dy;
            let n = if d2 < 1.0 {
                [dx, -dy, (1.0 - d2).sqrt()]
            } else {
                [0.0, 0.0, 1.0]
            };
            let p = ((y * w + x) * 4) as usize;
            for i in 0..3 {
                // SDL_lroundf: round half away from zero, like f32::round.
                normal[p + i] = ((n[i] * 0.5 + 0.5) * 255.0).round() as u8;
            }
            normal[p + 3] = 255;
        }
    }
    let options = NormalMapOptions {
        material: Material::SATIN,
        alpha: Some(AlphaSource {
            rgba: &color,
            pitch: (w * 4) as usize,
        }),
        ..Default::default()
    };
    let mut material = vec![0u8; (w * h * 4) as usize];
    convert_normal_map(&normal, w, h, (w * 4) as usize, &options, &mut material).unwrap();
    let mut d = LightDesc::new(LightType::Point);
    d.position = At::new(Vec2::new(4.0, 32.0));
    d.radius = 96.0;
    d.intensity = 1.5;
    d.validate().unwrap();
    Scene {
        name: "minimal_normal_map",
        w,
        h,
        color,
        material: Some(material),
        occlusion: None,
        light: d,
        ambient: 0.1,
        filter: OutputFilter::Nearest,
    }
}

/// tests/golden/chiaro/gate/gate_ref.c `shadow`: 192x128 checker, occlusion sprite with two
/// boxes and a disc, one warm point light at (40, 64), radius 170, intensity 1.3, source size
/// 4 (soft shadows); ambient 0.08; default output settings (linear filter).
fn gate_sprite_shadow() -> Scene {
    let (w, h) = (192u32, 128u32);
    let mut color = vec![0u8; (w * h * 4) as usize];
    let mut occlusion = vec![0u8; (w * h * 4) as usize];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let i = ((y * w as i32 + x) * 4) as usize;
            let checker: i32 = if ((x / 8) + (y / 8)) & 1 == 1 {
                12
            } else {
                -12
            };
            color[i..i + 4].copy_from_slice(&[
                (110 + checker) as u8,
                (120 + checker) as u8,
                (130 + checker) as u8,
                255,
            ]);
            let (cx, cy) = (x - 100, y - 100);
            let solid = ((90..110).contains(&x) && (30..60).contains(&y))
                || ((130..170).contains(&x) && (85..95).contains(&y))
                || cx * cx + cy * cy < 100;
            occlusion[i + 3] = if solid { 255 } else { 0 };
        }
    }
    let mut d = LightDesc::new(LightType::Point);
    d.position = At::new(Vec2::new(40.0, 64.0));
    d.radius = 170.0;
    d.color = Vec3::new(1.0, 0.85, 0.6);
    d.intensity = 1.3;
    d.source_radius = 4.0;
    d.validate().unwrap();
    Scene {
        name: "gate_sprite_shadow",
        w,
        h,
        color,
        material: None,
        occlusion: Some(occlusion),
        light: d,
        ambient: 0.08,
        filter: OutputFilter::Linear,
    }
}

/// Records one frame into a fresh command buffer, then the readback into `readback` if
/// given, submits and waits.
fn frame_once(
    device: &Device,
    renderer: &mut Light2dRenderer<'_>,
    frame: &Light2dFrame<'_>,
    stores: &SceneStores<'_>,
    settings: &Light2dSettings,
    readback: Option<&DownloadBuffer<'_>>,
) -> knorke_light2d::pass_plan::PassPrepared {
    let prepared = renderer.prepare(frame, stores, settings).unwrap();
    let mut cmd = CommandBuffer::acquire(device).unwrap();
    renderer
        .record(&mut cmd, frame, settings, prepared)
        .unwrap();
    if let Some(download) = readback {
        cmd.copy_pass()
            .unwrap()
            .download_texture(frame.target, download)
            .unwrap();
    }
    cmd.submit_and_wait().unwrap();
    prepared.light
}

#[test]
fn scenes_match_chiaro_through_pass_plan_and_post() {
    let Some((device, pack_bytes)) = vulkan_device_with_pack() else {
        return;
    };
    let counter = LiveCounter::new();
    let gpu = Gpu::new(&device, &counter);
    let mut renderer = Light2dRenderer::new(gpu, pack_bytes).unwrap();
    let bench = std::env::var("KNORKE_SCENE_BENCH")
        .ok()
        .and_then(|s| s.parse::<u32>().ok());
    let out = std::env::var("KNORKE_SCENE_OUT").ok().map(PathBuf::from);
    let mut failures = Vec::new();

    for scene in [minimal_normal_map(), gate_sprite_shadow()] {
        let (w, h) = (scene.w, scene.h);
        let color = sampled_rgba8(gpu, w, h, &scene.color);
        let material = scene
            .material
            .as_deref()
            .map(|m| sampled_rgba8(gpu, w, h, m));
        let occlusion = scene
            .occlusion
            .as_deref()
            .map(|o| sampled_rgba8(gpu, w, h, o));
        let target = texture(
            gpu,
            (w, h),
            SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
            SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
            None,
        );
        let projection = Projection::default();
        let frame = Light2dFrame {
            material: material.as_ref().map(Texture::texture_ref),
            occlusion: occlusion.as_ref().map(Texture::texture_ref),
            ..Light2dFrame::new(color.texture_ref(), target.texture_ref(), &projection)
        };
        // Chiaro's defaults with the scene's ambient and output filter.
        let mut settings = Light2dSettings {
            light: LightSettings {
                ambient: Vec3::new(scene.ambient, scene.ambient, scene.ambient),
                ..LightSettings::default()
            },
            post: PostSettings::default(),
        };
        settings.post.output.filter = scene.filter;
        let mut lights = LightStore::new();
        lights.create(&scene.light, &NoResources).unwrap();
        let obstacles = ObstacleStore::new();
        let stores = SceneStores::new(&lights, &obstacles);
        let mut download = DownloadBuffer::new(gpu, w * h * 4).unwrap();
        let prepared = frame_once(
            &device,
            &mut renderer,
            &frame,
            &stores,
            &settings,
            Some(&download),
        );
        let shadows = scene.occlusion.is_some();
        assert_eq!(
            prepared.stages.contains(PassStages::OCCLUSION),
            shadows,
            "{}: the sprite mask is the only shadow source",
            scene.name
        );
        if shadows {
            // The sprite may change anywhere every frame: never reused.
            assert_ne!(prepared.mask_update, Some(MaskUpdate::Reused));
        }
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        download.read(&mut pixels).unwrap();

        let reference = load_png_rgba8(
            &workspace_root().join(format!("tests/golden/chiaro/{}.png", scene.name)),
            w,
            h,
        );
        let strict = compare_rgba8(&pixels, &reference, w, h, &[], &ParityBudget::STRICT).unwrap();
        strict.print(scene.name, "STRICT");
        if !strict.within(&ParityBudget::STRICT) {
            let loose =
                compare_rgba8(&pixels, &reference, w, h, &[], &ParityBudget::LOOSE).unwrap();
            loose.print(scene.name, "LOOSE");
            failures.push(format!("{}: {strict:?}", scene.name));
        }
        if let Some(dir) = &out {
            std::fs::create_dir_all(dir).unwrap();
            save_png_rgba8(&dir.join(format!("{}.rust.png", scene.name)), &pixels, w, h);
            // Diff x32, red where it exceeds the STRICT tolerance.
            let diff: Vec<u8> = pixels
                .as_chunks::<4>()
                .0
                .iter()
                .zip(reference.as_chunks::<4>().0)
                .flat_map(|(a, b)| {
                    let m = (0..3).map(|i| a[i].abs_diff(b[i])).max().unwrap();
                    let v = m.saturating_mul(32);
                    let g = if m > 2 { 0 } else { v };
                    [v, g, g, 255]
                })
                .collect();
            save_png_rgba8(&dir.join(format!("{}.diff.png", scene.name)), &diff, w, h);
        }
        // Host-side wall clock for the benchmark only; the renderers read no clock.
        #[allow(clippy::disallowed_methods)]
        if let Some(frames) = bench {
            let mut times = Vec::with_capacity(frames as usize);
            for _ in 0..frames {
                let t = Instant::now();
                frame_once(&device, &mut renderer, &frame, &stores, &settings, None);
                times.push(t.elapsed().as_secs_f64() * 1e6);
            }
            times.sort_by(f64::total_cmp);
            let mean = times.iter().sum::<f64>() / times.len() as f64;
            eprintln!(
                "bench {} {w}x{h} vulkan (Rust), {frames} frames: mean {mean:.1} us, median {:.1} us, p95 {:.1} us",
                scene.name,
                times[times.len() / 2],
                times[times.len() * 95 / 100]
            );
        }
    }
    drop(renderer);
    assert_eq!(counter.live(), 0, "the renderers leaked GPU objects");
    assert!(failures.is_empty(), "STRICT parity missed: {failures:#?}");
}
