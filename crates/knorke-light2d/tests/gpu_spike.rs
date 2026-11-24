//! Gate spike of the Chiaro port (plan §5.1, ADR 0014): renders two scenes through
//! `knorke_light2d::gpu_spike` (whose light pass is the real `lighting::LightingStage` since
//! P25, fed by `collect_lights`, and whose composite is the real `composite::CompositeStage`
//! since P27) on Vulkan and compares them with C++ Chiaro reference images
//! (`tests/golden/chiaro`) under `ParityBudget::STRICT`:
//!
//! - `minimal_normal_map`: the P3 golden scene (one point light on a normal-mapped dome);
//! - `gate_sprite_shadow`: one soft point light, shadows from an occlusion sprite buffer,
//!   rendered by `tests/golden/chiaro/gate/gate_ref.c` (C++ Chiaro 9bc5b9b, Vulkan).
//!
//! The test plays the host (ADR 0002): it owns device, textures and command buffers, submits
//! and reads back, all through the safe API of `knorke-gpu` (no `unsafe`). It skips (passes
//! with a note) without a Vulkan device or without the shader pack from `cargo xtask shaders`
//! in `target/shaders/`.
//!
//! `KNORKE_SPIKE_OUT=<dir>` writes each image and an amplified diff image there;
//! `KNORKE_SPIKE_BENCH=<frames>` also times that many frames (wall clock, record + submit +
//! wait, as `gate_ref.c` does with `CHI_Render`).
#![cfg(feature = "gpu")]

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use knorke_contract_tests::parity::{ParityBudget, ParityReport, compare_rgba8};
use knorke_gpu::sys::gpu::{
    SDL_GPU_SHADERFORMAT_SPIRV, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
    SDL_GPU_TEXTUREUSAGE_COLOR_TARGET, SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUBufferUsageFlags,
    SDL_GPUTextureUsageFlags,
};
use knorke_gpu::{
    CommandBuffer, Device, DownloadBuffer, Filter, Gpu, LiveCounter, Texture, TextureDesc,
    UploadBuffer, init_video,
};
use knorke_light2d::codec::{AlphaSource, Material, NormalMapOptions, convert_normal_map};
use knorke_light2d::gpu_spike::{SpikeFrame, SpikeRenderer};
use knorke_light2d::light::{LightDesc, LightType};
use knorke_light2d::light_culling::{LightList, LightModes, collect_lights};
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{LightStore, NoResources};
use knorke_math::{At, Vec2, Vec3};

/// One device at a time: SDL's video init and the GPU tests of this crate must not overlap.
static SERIAL: Mutex<()> = Mutex::new(());

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

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
    filter: Filter,
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
        filter: Filter::Nearest,
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
        filter: Filter::Linear,
    }
}

/// A RGBA8 UNORM texture, filled with `pixels` if given (host work: in Chiaro the demo's
/// renderer draws these layers).
fn texture<'d>(
    gpu: Gpu<'d>,
    w: u32,
    h: u32,
    usage: SDL_GPUTextureUsageFlags,
    pixels: Option<&[u8]>,
) -> Texture<'d> {
    let desc = TextureDesc::new(w, h, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, usage);
    let texture = Texture::new(gpu, desc).expect("texture");
    if let Some(px) = pixels {
        let mut up = UploadBuffer::new(gpu, SDL_GPUBufferUsageFlags(0), px.len() as u32, "pixels");
        up.reserve(px.len() as u64).unwrap();
        up.map().unwrap().bytes()[..px.len()].copy_from_slice(px);
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        cmd.copy_pass()
            .unwrap()
            .upload_to_texture(&up, texture.texture_ref(), false)
            .unwrap();
        cmd.submit_and_wait().unwrap();
    }
    texture
}

fn save_png(path: &Path, rgba: &[u8], w: u32, h: u32) {
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}

fn golden(name: &str, w: u32, h: u32) -> Vec<u8> {
    let path = workspace().join(format!("tests/golden/chiaro/{name}.png"));
    let decoder = png::Decoder::new(std::fs::File::open(&path).unwrap());
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (w, h), "{}", path.display());
    let px = &buf[..info.buffer_size()];
    match info.color_type {
        png::ColorType::Rgb => px
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::Rgba => px.to_vec(),
        other => panic!("unexpected golden colour type {other:?}"),
    }
}

fn report(name: &str, budget: &str, r: &ParityReport) {
    eprintln!(
        "{name} vs C++ [{budget}]: max diff {}, differing {} / {} px ({:.4} %), outliers {} ({} ppm), mean {:.4}",
        r.max_diff,
        r.differing,
        r.compared,
        r.differing as f64 * 100.0 / r.compared as f64,
        r.outliers,
        r.outlier_ppm(),
        r.sum_diff as f64 / r.compared as f64
    );
}

/// Records one frame into a fresh command buffer, then the readback into `readback` if
/// given, submits and waits.
fn frame_once(
    device: &Device,
    renderer: &mut SpikeRenderer<'_>,
    frame: &SpikeFrame<'_>,
    lights: &LightList,
    readback: Option<&DownloadBuffer<'_>>,
) {
    let prepared = renderer.prepare(frame, lights).unwrap();
    let mut cmd = CommandBuffer::acquire(device).unwrap();
    renderer.record(&mut cmd, frame, lights, prepared).unwrap();
    if let Some(download) = readback {
        cmd.copy_pass()
            .unwrap()
            .download_texture(frame.target, download)
            .unwrap();
    }
    cmd.submit_and_wait().unwrap();
}

#[test]
fn spike_matches_chiaro() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let pack = workspace().join("target/shaders/knorke-chiaro.spirv.kpak");
    let Ok(pack_bytes) = std::fs::read(&pack) else {
        eprintln!(
            "skipped: no shader pack at {} (cargo xtask shaders)",
            pack.display()
        );
        return;
    };
    init_video(Some(c"vulkan"));
    let device = match Device::create(SDL_GPU_SHADERFORMAT_SPIRV, false) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skipped: no Vulkan device ({})", e.message);
            return;
        }
    };
    let counter = LiveCounter::new();
    let gpu = Gpu::new(&device, &counter);
    let mut renderer = SpikeRenderer::new(gpu, pack_bytes).unwrap();
    let bench = std::env::var("KNORKE_SPIKE_BENCH")
        .ok()
        .and_then(|s| s.parse::<u32>().ok());
    let out = std::env::var("KNORKE_SPIKE_OUT").ok().map(PathBuf::from);
    let mut failures = Vec::new();

    for scene in [minimal_normal_map(), gate_sprite_shadow()] {
        let (w, h) = (scene.w, scene.h);
        let sampled = SDL_GPU_TEXTUREUSAGE_SAMPLER;
        let color = texture(gpu, w, h, sampled, Some(&scene.color));
        let material = scene
            .material
            .as_deref()
            .map(|m| texture(gpu, w, h, sampled, Some(m)));
        let occlusion = scene
            .occlusion
            .as_deref()
            .map(|o| texture(gpu, w, h, sampled, Some(o)));
        let target = texture(gpu, w, h, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET, None);
        let frame = SpikeFrame {
            scene: color.texture_ref(),
            material: material.as_ref().map(Texture::texture_ref),
            occlusion: occlusion.as_ref().map(Texture::texture_ref),
            target: target.texture_ref(),
            projection: Projection::default(),
            ambient: Vec3::new(scene.ambient, scene.ambient, scene.ambient),
            output_filter: scene.filter,
        };
        // The culling of Chiaro's context: one light, default settings with the ambient.
        let mut store = LightStore::new();
        store.create(&scene.light, &NoResources).unwrap();
        let settings = LightSettings {
            ambient: frame.ambient,
            ..LightSettings::default()
        };
        let modes = LightModes {
            normal_mode: scene.material.is_some(),
            ..Default::default()
        };
        let mut lights = LightList::default();
        collect_lights(
            &store,
            &settings,
            &frame.projection,
            0.0,
            w,
            h,
            &modes,
            &mut lights,
        );
        let mut download = DownloadBuffer::new(gpu, w * h * 4).unwrap();
        frame_once(&device, &mut renderer, &frame, &lights, Some(&download));
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        download.read(&mut pixels).unwrap();

        let reference = golden(scene.name, w, h);
        let strict = compare_rgba8(&pixels, &reference, w, h, &[], &ParityBudget::STRICT).unwrap();
        report(scene.name, "STRICT", &strict);
        if !strict.within(&ParityBudget::STRICT) {
            let loose =
                compare_rgba8(&pixels, &reference, w, h, &[], &ParityBudget::LOOSE).unwrap();
            report(scene.name, "LOOSE", &loose);
            failures.push(format!("{}: {strict:?}", scene.name));
        }
        if let Some(dir) = &out {
            std::fs::create_dir_all(dir).unwrap();
            save_png(&dir.join(format!("{}.rust.png", scene.name)), &pixels, w, h);
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
            save_png(&dir.join(format!("{}.diff.png", scene.name)), &diff, w, h);
        }
        // Host-side wall clock for the benchmark only; the spike itself reads no clock.
        #[allow(clippy::disallowed_methods)]
        if let Some(frames) = bench {
            let mut times = Vec::with_capacity(frames as usize);
            for _ in 0..frames {
                let t = Instant::now();
                frame_once(&device, &mut renderer, &frame, &lights, None);
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
    assert_eq!(counter.live(), 0, "spike leaked GPU objects");
    assert!(failures.is_empty(), "STRICT parity missed: {failures:#?}");
}
