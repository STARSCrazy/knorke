//! The medium stage (P30, Chiaro `MediumStage`) end to end against C++ Chiaro
//! (`tests/golden/chiaro/medium/medium_stage_ref.cpp` writes the references, chiaro @ 9bc5b9b,
//! zlib license): for every scene the medium stage's buffers bit for bit (scatter, filtered,
//! guides, the polar atlas of the light shafts, the sun height atlas) and the final image through
//! `Light2dRenderer` under `ParityBudget::STRICT`. The scenes are mirrored there; change both
//! together.
//!
//! - `shafts`: haze, a medium layer, a box and two shadowed lights (point, spot): two polar rows;
//! - `sun`: haze, two boxes, a shadowed and an unshadowed directional light: one sun block;
//! - `ring`: haze, an ambient zone, a point light and a ring light: the dual path of the light pass;
//! - `smoke`: no haze, a medium layer drawn with `MediumBatch` (a flat slab, two shaped puffs), a
//!   box and a shadowed point light.
//!
//! The test plays the host (ADR 0002). It skips without a Vulkan device or the shader pack (a
//! failure with `KNORKE_REQUIRE_GPU=1`). `KNORKE_SCENE_OUT=<dir>` writes each final image there;
//! `KNORKE_SCENE_BENCH=<frames>` also times that many frames per scene (wall clock, prepare +
//! record + submit + wait; the C++ side is `medium_stage_ref <outdir> <frames>`).
#![cfg(feature = "gpu")]

use knorke_contract_tests::parity::{ParityBudget, compare_rgba8, save_png_rgba8, scene_out};
use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
};
use knorke_gpu::test_support::{
    expected_bytes, golden_bytes, queue_download, read_download, rgba16f_diff, sampled_rgba8,
    texture, with_chiaro_pack,
};
use knorke_gpu::{CommandBuffer, Gpu, ShaderLibrary, TextureRef};
use knorke_integration::layers::{Layer, LayerSet, Layers};
use knorke_integration::light2d::{
    Light2dFrame, Light2dRenderer, Light2dSettings, post_projection,
};
use knorke_integration::medium::{MediumBatch, MediumItem};
use knorke_light2d::ambient_zone::{AmbientZoneParams, ZoneShape, make_stored_zone};
use knorke_light2d::ambient_zone_store::AmbientZoneStore;
use knorke_light2d::light::{Height, LightDesc, LightType};
use knorke_light2d::medium_draw::MediumDraw;
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::pass_plan::{PassStages, SceneStores};
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::{LightSettings, Quality, Resolutions};
use knorke_light2d::{LightStore, NoResources, ObstacleStore};
use knorke_math::{At, LinearRgba, Vec2, Vec3};
use knorke_post::settings::PostSettings;
use knorke_post::shockwave::{Shockwave, ShockwaveFrame, ShockwaveList, collect_shockwaves};

const W: u32 = 96;
const H: u32 = 64;
const MOON: Vec3 = Vec3::new(-0.7, 0.15, -0.7);

fn iso() -> Projection {
    Projection {
        origin: At::new(Vec2::new(48.3, 10.6)),
        x_axis: Vec2::new(1.6, 0.8),
        y_axis: Vec2::new(-1.6, 0.8),
        z_axis: Vec2::new(0.0, -1.2),
    }
}

/// The checker of the reference.
fn checker() -> Vec<u8> {
    let mut color = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
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

/// `MediumLayer` of the reference: kind 0 around (30, 20), kind 3 around (66, 26).
fn medium_layer() -> Vec<u8> {
    let mut px = vec![0u8; (W * H * 4) as usize];
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            let i = ((y as u32 * W + x as u32) * 4) as usize;
            let a = (x - 30) * (x - 30) + (y - 20) * (y - 20);
            let b = (x - 66) * (x - 66) + (y - 26) * (y - 26);
            if a < 196 {
                px[i..i + 4].copy_from_slice(&[12, 255 - 2, 0, (255 - a * 255 / 196) as u8]);
            } else if b < 100 {
                px[i..i + 4].copy_from_slice(&[30, 255 - 10, 3 * 16, (200 - b * 2) as u8]);
            }
        }
    }
    px
}

/// `Light()` of the reference.
#[allow(clippy::too_many_arguments)]
fn light(
    kind: LightType,
    (x, y): (f32, f32),
    height: Height,
    radius: f32,
    direction: Vec3,
    intensity: f32,
    shadows: bool,
) -> LightDesc {
    let mut d = LightDesc::new(kind);
    d.position = At::new(Vec2::new(x, y));
    d.height = height;
    d.radius = radius;
    d.direction = direction;
    d.intensity = intensity;
    d.inner_angle = 25.0;
    d.outer_angle = 45.0;
    d.cast_shadows = shadows;
    d
}

fn boxed(obstacles: &mut ObstacleStore, (x, y): (f32, f32), (w, d): (f32, f32), height: f32) {
    obstacles
        .create_shape(&ShapeDesc {
            common: ObstacleCommon {
                position: At::new(Vec2::new(x, y)),
                height,
                ..Default::default()
            },
            shape: Shape::Box { width: w, depth: d },
        })
        .unwrap();
}

/// `Settings()` of the reference.
fn settings(haze: f32, half: bool) -> Light2dSettings {
    let mut light = LightSettings {
        ambient: Vec3::new(0.1, 0.1, 0.1),
        ..LightSettings::default()
    };
    light.haze.density = haze;
    if half {
        light.quality.quality = Quality::Custom(Resolutions {
            light: 1.0,
            shadow: 0.5,
            medium: 0.5,
        });
    }
    Light2dSettings {
        light,
        post: PostSettings::default(),
    }
}

struct Scene {
    name: &'static str,
    projection: Projection,
    settings: Light2dSettings,
    lights: LightStore,
    obstacles: ObstacleStore,
    zones: Option<AmbientZoneStore>,
    waves: Vec<Shockwave>,
    layer: Option<Vec<u8>>,
    /// The buffers the reference wrote besides scatter, filtered, guides and final.
    polar: bool,
    sun: bool,
    /// The medium layer is drawn with [`MediumBatch`] (the reference's `Smoke`).
    smoke: bool,
}

impl Scene {
    fn new(name: &'static str, projection: Projection, settings: Light2dSettings) -> Self {
        Self {
            name,
            projection,
            settings,
            lights: LightStore::new(),
            obstacles: ObstacleStore::new(),
            zones: None,
            waves: Vec::new(),
            layer: None,
            polar: false,
            sun: false,
            smoke: false,
        }
    }

    fn light(&mut self, d: LightDesc) {
        self.lights.create(&d, &NoResources).unwrap();
    }
}

fn shafts() -> Scene {
    let mut s = Scene::new("shafts", iso(), settings(0.15, true));
    boxed(&mut s.obstacles, (14.0, 8.0), (4.0, 4.0), 8.0);
    let x = Vec3::new(1.0, 0.0, 0.0);
    s.light(light(
        LightType::Point,
        (10.0, 12.0),
        Height::At(4.0),
        30.0,
        x,
        2.0,
        true,
    ));
    let spot = Vec3::new(1.0, 0.4, -0.5);
    s.light(light(
        LightType::Spot,
        (6.0, 2.0),
        Height::At(10.0),
        35.0,
        spot,
        2.0,
        true,
    ));
    s.layer = Some(medium_layer());
    s.polar = true;
    s
}

fn sun() -> Scene {
    let mut s = Scene::new("sun", iso(), settings(0.2, true));
    boxed(&mut s.obstacles, (14.0, 8.0), (4.0, 4.0), 8.0);
    boxed(&mut s.obstacles, (24.0, 4.0), (6.0, 3.0), 4.0);
    s.light(light(
        LightType::Directional,
        (0.0, 0.0),
        Height::Auto,
        30.0,
        MOON,
        1.0,
        true,
    ));
    let other = Vec3::new(0.3, -0.5, -0.8);
    s.light(light(
        LightType::Directional,
        (0.0, 0.0),
        Height::Auto,
        30.0,
        other,
        0.4,
        false,
    ));
    s.sun = true;
    s
}

fn ring() -> Scene {
    let mut s = Scene::new("ring", Projection::default(), settings(0.12, false));
    let mut zones = AmbientZoneStore::new();
    zones
        .create(
            make_stored_zone(&AmbientZoneParams {
                shape: ZoneShape::Box,
                x: 60.0,
                y: 30.0,
                width: 40.0,
                depth: 30.0,
                ambient: [0.4, 0.3, 0.2],
                soft_edge: 4.0,
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    s.zones = Some(zones);
    let x = Vec3::new(1.0, 0.0, 0.0);
    s.light(light(
        LightType::Point,
        (30.0, 30.0),
        Height::At(10.0),
        50.0,
        x,
        1.5,
        false,
    ));
    s.waves = vec![Shockwave {
        x: 60.0,
        y: 32.0,
        radius: 14.0,
        width: 6.0,
        strength: 0.0,
        light: LinearRgba::new(1.0, 0.6, 0.3, 1.0),
        light_intensity: 2.0,
    }];
    s
}

fn smoke() -> Scene {
    let mut s = Scene::new("smoke", iso(), settings(0.0, true));
    boxed(&mut s.obstacles, (14.0, 8.0), (4.0, 4.0), 8.0);
    let x = Vec3::new(1.0, 0.0, 0.0);
    s.light(light(
        LightType::Point,
        (10.0, 12.0),
        Height::At(6.0),
        30.0,
        x,
        2.0,
        true,
    ));
    s.smoke = true;
    s.polar = true;
    s
}

/// `Puff()` of the reference: 16 x 16, alpha 1 - r over the radius, a colour that must not
/// matter.
fn puff() -> Vec<u8> {
    let mut px = vec![0u8; 16 * 16 * 4];
    for y in 0..16 {
        for x in 0..16 {
            let dx = (x as f32 + 0.5) / 8.0 - 1.0;
            let dy = (y as f32 + 0.5) / 8.0 - 1.0;
            let r = (dx * dx + dy * dy).sqrt();
            let a = if r >= 1.0 {
                0
            } else {
                ((1.0 - r) * 255.0 + 0.5) as u8
            };
            px[(y * 16 + x) * 4..][..4].copy_from_slice(&[10, 200, 30, a]);
        }
    }
    px
}

/// The draws of the reference's `Smoke`: a flat slab of steam, two shaped puffs of smoke.
fn smoke_draws(shape: TextureRef<'_>) -> [MediumItem<'_>; 3] {
    let d = |rect, density, bottom, top, medium| MediumDraw {
        rect,
        density,
        bottom,
        top,
        medium,
    };
    [
        MediumItem {
            draw: d([40.0, 30.0, 40.0, 14.0], 0.4, 0.0, 6.0, 3),
            shape: None,
        },
        MediumItem {
            draw: d([20.0, 10.0, 24.0, 24.0], 0.8, 2.0, 30.0, 1),
            shape: Some(shape),
        },
        MediumItem {
            draw: d([30.5, 18.25, 17.0, 13.0], 0.5, 6.0, 20.0, 1),
            shape: Some(shape),
        },
    ]
}

/// Bytes of `t` after the frame (queued on `cmd`).
struct Readback<'d> {
    name: &'static str,
    download: knorke_gpu::DownloadBuffer<'d>,
    half: bool,
}

fn queue<'d>(
    gpu: Gpu<'d>,
    cmd: &mut CommandBuffer<'_>,
    out: &mut Vec<Readback<'d>>,
    name: &'static str,
    t: Option<TextureRef<'_>>,
    half: bool,
) {
    let t = t.unwrap_or_else(|| panic!("{name} is missing"));
    out.push(Readback {
        name,
        download: queue_download(gpu, cmd, t),
        half,
    });
}

/// Differing texels (RGBA16F) or bytes (the others) and the largest difference.
fn diff(got: &[u8], want: &[u8], half: bool) -> (usize, f32) {
    if half {
        return rgba16f_diff(got, want);
    }
    assert_eq!(got.len(), want.len());
    let n = got.iter().zip(want).filter(|(a, b)| a != b).count();
    let max = got
        .iter()
        .zip(want)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    (n, f32::from(max))
}

#[test]
fn medium_scenes_match_chiaro() {
    with_chiaro_pack(|gpu, pack| {
        // One renderer per scene: the reference renders each scene as the first frame of a new
        // context, and the occlusion heights of an earlier frame must not help.
        let fresh = || Light2dRenderer::new(gpu, pack.to_vec()).unwrap();
        let mut renderer = fresh();
        let out_dir = scene_out();
        let bench = std::env::var("KNORKE_SCENE_BENCH")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|&n| n > 0);
        let color = sampled_rgba8(gpu, W, H, &checker());
        let target = texture(
            gpu,
            (W, H),
            SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
            SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
            None,
        );
        let mut failures = Vec::new();
        let shape = sampled_rgba8(gpu, 16, 16, &puff());
        let mut shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let mut batch = MediumBatch::new(gpu, &shaders).unwrap();
        let kinds = LayerSet::of(Layer::Scene).with(Layer::Medium);
        for s in [shafts(), sun(), ring(), smoke()] {
            renderer = fresh();
            let mut layers = None;
            if s.smoke {
                // The reference's first pass clears the layers, then CHI_DrawMediums.
                let l = layers.insert(Layers::new(gpu, W, H, kinds).unwrap());
                let items = smoke_draws(shape.texture_ref());
                let step = s.settings.light.resolved_medium_height_step();
                batch.prepare(&mut shaders, &items).unwrap();
                let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
                drop(l.clear_pass(&mut cmd, kinds).unwrap());
                batch.record(&mut cmd, l, step, &items).unwrap();
                let down = queue_download(gpu, &mut cmd, l.texture(Layer::Medium).unwrap());
                cmd.submit_and_wait().unwrap();
                let want = golden_bytes("medium/stage", "smoke.layer.bin");
                let (n, max) = diff(&read_download(down), &want, false);
                eprintln!("smoke.layer: {n} differing (max {max})");
                if n > 0 {
                    failures.push(format!("smoke.layer: {n} differing, max {max}"));
                }
            }
            let uploaded = s.layer.as_ref().map(|px| sampled_rgba8(gpu, W, H, px));
            let layer = uploaded
                .as_ref()
                .map(|t| t.texture_ref())
                .or_else(|| layers.as_ref().and_then(|l| l.texture(Layer::Medium)));
            let mut list = ShockwaveList::default();
            list.set(&s.waves).unwrap();
            let post = post_projection(&s.projection);
            let mut waves = ShockwaveFrame::default();
            collect_shockwaves(&list, &post, W, H, &mut waves);
            let frame = Light2dFrame {
                medium: layer,
                shockwaves: (!s.waves.is_empty()).then_some((&post, &waves)),
                ..Light2dFrame::new(color.texture_ref(), target.texture_ref(), &s.projection)
            };
            let stores = SceneStores {
                zones: s.zones.as_ref(),
                ..SceneStores::new(&s.lights, &s.obstacles)
            };
            let prepared = renderer.prepare(&frame, &stores, &s.settings).unwrap();
            assert!(
                prepared.light.stages.contains(PassStages::MEDIUM),
                "{}",
                s.name
            );
            let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
            let record = renderer
                .record(&mut cmd, &frame, &s.settings, prepared)
                .unwrap();
            eprintln!("{}: {:?}", s.name, record.medium);
            let m = renderer.plan().medium();
            let mut reads = Vec::new();
            queue(gpu, &mut cmd, &mut reads, "scatter", m.scatter(), true);
            queue(gpu, &mut cmd, &mut reads, "filtered", m.filtered(), true);
            queue(gpu, &mut cmd, &mut reads, "guides", m.guides(), false);
            if s.polar {
                queue(gpu, &mut cmd, &mut reads, "polar", m.polar_atlas(), true);
            }
            if s.sun {
                queue(gpu, &mut cmd, &mut reads, "sun", m.sun_heights(), false);
            }
            queue(
                gpu,
                &mut cmd,
                &mut reads,
                "final",
                Some(frame.target),
                false,
            );
            cmd.submit_and_wait().unwrap();
            for r in reads {
                let got = read_download(r.download);
                // The rows' shafts and sun heights differ from Chiaro's field march at shadow
                // edges: re-blessed where they do (tests/golden/rows).
                let name = format!("{}.{}.bin", s.name, r.name);
                let want = expected_bytes("medium/stage", &name, &got);
                if r.name == "final" {
                    let strict =
                        compare_rgba8(&got, &want, W, H, &[], &ParityBudget::STRICT).unwrap();
                    strict.print(s.name, "STRICT");
                    if !strict.within(&ParityBudget::STRICT) {
                        failures.push(format!("{}.final: {strict:?}", s.name));
                    }
                    if let Some(dir) = &out_dir {
                        std::fs::create_dir_all(dir).unwrap();
                        save_png_rgba8(&dir.join(format!("{}.rust.png", s.name)), &got, W, H);
                        save_png_rgba8(&dir.join(format!("{}.chiaro.png", s.name)), &want, W, H);
                    }
                }
                let (n, max) = diff(&got, &want, r.half);
                eprintln!("{}.{}: {n} differing (max {max})", s.name, r.name);
                if n > 0 {
                    failures.push(format!("{}.{}: {n} differing, max {max}", s.name, r.name));
                }
            }
            // The host times frames (wall clock is a host tool, not simulation).
            #[allow(clippy::disallowed_methods)]
            if let Some(n) = bench {
                let mut us = Vec::with_capacity(n);
                for _ in 0..n {
                    let t0 = std::time::Instant::now();
                    let prepared = renderer.prepare(&frame, &stores, &s.settings).unwrap();
                    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
                    renderer
                        .record(&mut cmd, &frame, &s.settings, prepared)
                        .unwrap();
                    cmd.submit_and_wait().unwrap();
                    us.push(t0.elapsed().as_secs_f64() * 1e6);
                }
                us.sort_by(f64::total_cmp);
                eprintln!(
                    "bench {} (Rust), {n} frames: median {:.1} us, p95 {:.1} us",
                    s.name,
                    us[n / 2],
                    us[n * 95 / 100]
                );
            }
        }
        // A frame without haze and medium layer: the stage records nothing and keeps no target.
        let s = shafts();
        let plain = settings(0.0, true);
        let frame = Light2dFrame::new(color.texture_ref(), target.texture_ref(), &s.projection);
        let stores = SceneStores::new(&s.lights, &s.obstacles);
        let prepared = renderer.prepare(&frame, &stores, &plain).unwrap();
        assert!(!prepared.light.stages.contains(PassStages::MEDIUM));
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        let record = renderer.record(&mut cmd, &frame, &plain, prepared).unwrap();
        cmd.submit_and_wait().unwrap();
        let m = renderer.plan().medium();
        assert_eq!(record.medium, Default::default());
        assert!(!m.is_active() && m.scatter().is_none() && m.filtered().is_none());
        assert!(m.polar_atlas().is_none() && m.sun_heights().is_none());
        assert!(
            failures.is_empty(),
            "differences from Chiaro: {failures:#?}"
        );
    });
}
