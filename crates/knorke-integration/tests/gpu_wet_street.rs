//! The Chiaro demo scene `wet_street` (examples/wet_street_demo of chiaro @ 9bc5b9b, zlib
//! license) end to end through [`Layers`], [`SurfaceBatch`] and [`Light2dRenderer`], compared
//! with the P3 golden `tests/golden/chiaro/wet_street.png` under `ParityBudget::STRICT`: heights
//! and surface normals, wet materials, footprint shadows, soft near field, half-resolution
//! shadows, bloom, soft tonemap and integer nearest scaling 640x360 -> 1280x720.
//!
//! The demo paints its colour and material layers with `SDL_Renderer` (flat-coloured projected
//! quads). Knorke has no such renderer, so those two layers come from fixtures dumped by
//! `tests/golden/chiaro/wet_street/wet_street_ref.cpp` (the demo's scene code plus layer dumps;
//! its frame is byte-identical to the golden) and are uploaded into the layers. The surface layer
//! is drawn by Knorke's [`SurfaceBatch`] as the demo's `DrawSurfaces` does and must equal the
//! C++ surface dump byte for byte.
//!
//! The test plays the host (ADR 0002). It skips without a Vulkan device or the shader pack of
//! `cargo xtask shaders` (a failure with `KNORKE_REQUIRE_GPU=1`). `KNORKE_SCENE_OUT=<dir>` writes
//! the frame, the surface layer and amplified diff images there.
#![cfg(feature = "gpu")]

use std::path::PathBuf;

use knorke_contract_tests::parity::{ParityBudget, compare_rgba8, load_png_rgba8, save_png_rgba8};
use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
    SDL_GPUBufferUsageFlags,
};
use knorke_gpu::test_support::{read_texture, texture, with_chiaro_pack, workspace_root};
use knorke_gpu::{CommandBuffer, ShaderLibrary, Texture, UploadBuffer};
use knorke_integration::layers::{Layer, LayerSet, Layers};
use knorke_integration::light2d::{Light2dRenderer, Light2dSettings};
use knorke_integration::surface::SurfaceBatch;
use knorke_light2d::light::{Height, LightDesc, LightType};
use knorke_light2d::obstacle::ShapeDesc;
use knorke_light2d::pass_plan::SceneStores;
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::{LightSettings, Quality, QualitySettings, Resolutions};
use knorke_light2d::surface_geometry::{SurfaceBox, SurfaceGeometry};
use knorke_light2d::{LightStore, NoResources, ObstacleStore};
use knorke_math::{At, Vec2, Vec3};
use knorke_post::output_rect::ScaleMode;
use knorke_post::settings::{Filter, PostSettings, Tonemap};

const W: u32 = 640;
const H: u32 = 360;
const OUT: (u32, u32) = (1280, 720);

// The scene constants of examples/wet_street_demo/main.cpp (chiaro @ 9bc5b9b).
const HEIGHT_STEP: f32 = 0.05;
const FIGURE_HEIGHT: f32 = 1.8;
const FOOTPRINT: f32 = 0.45;
const WETNESS: f32 = 0.8;
/// 0.70710678f in the demo, the same f32.
const DIAG: f32 = std::f32::consts::FRAC_1_SQRT_2;
const FIGURES: [(f32, f32); 4] = [(3.5, 3.0), (5.0, 4.6), (8.0, 2.8), (9.6, 4.4)];
const WALL_LAMPS: [(f32, f32); 2] = [(2.0, 0.25), (10.0, 0.25)];
const STREET_LAMPS: [(f32, f32); 3] = [(1.0, 6.5), (6.5, 6.5), (12.0, 6.5)];

/// The foot line of a post or figure facing the viewer (+x +y): `Left` and `Right` of the demo.
fn across((x, y): (f32, f32), half: f32) -> (Vec3, Vec3) {
    (
        Vec3::new(x - half * DIAG, y + half * DIAG, 0.0),
        Vec3::new(x + half * DIAG, y - half * DIAG, 0.0),
    )
}

/// `DrawSurfaces` of the demo: the building box, the wall lamp plates, the posts, the figures.
fn surfaces(p: &Projection) -> SurfaceGeometry {
    let mut g = SurfaceGeometry::new();
    let building = SurfaceBox {
        center: At::new(Vec2::new(6.0, -2.0)),
        width: 16.0,
        depth: 4.0,
        height: 6.0,
    };
    g.push_boxes(p, HEIGHT_STEP, &[building]).unwrap();
    for (x, y) in WALL_LAMPS {
        let (a, b) = (Vec3::new(x - 0.15, y, 2.5), Vec3::new(x + 0.15, y, 2.5));
        g.push_wall(p, HEIGHT_STEP, a, b, 2.7).unwrap();
    }
    for at in STREET_LAMPS {
        let (a, b) = across(at, 0.08);
        g.push_wall(p, HEIGHT_STEP, a, b, 4.5).unwrap();
    }
    for at in FIGURES {
        let (a, b) = across(at, 0.2);
        g.push_wall(p, HEIGHT_STEP, a, b, FIGURE_HEIGHT).unwrap();
    }
    g
}

/// `CHI_PointLight(x, y, radius, color, intensity).Height(h).SourceSize(s)`.
fn point(
    (x, y): (f32, f32),
    radius: f32,
    color: Vec3,
    intensity: f32,
    height: f32,
    source: f32,
) -> LightDesc {
    let mut d = LightDesc::new(LightType::Point);
    d.position = At::new(Vec2::new(x, y));
    d.radius = radius;
    d.color = color;
    d.intensity = intensity;
    d.height = Height::At(height);
    d.source_radius = source;
    d
}

fn lights() -> LightStore {
    let mut lights = LightStore::new();
    for at in WALL_LAMPS {
        let mut d = point(at, 6.0, Vec3::new(0.75, 0.9, 1.0), 4.0, 2.6, 0.25);
        d.soft_near_field = true;
        lights.create(&d, &NoResources).unwrap();
    }
    for at in STREET_LAMPS {
        let d = point(at, 11.0, Vec3::new(1.0, 0.78, 0.5), 1.6, 4.5, 0.15);
        lights.create(&d, &NoResources).unwrap();
    }
    lights
}

fn obstacles() -> ObstacleStore {
    let mut obstacles = ObstacleStore::new();
    let mut building = ShapeDesc::rect(16.0, 4.0);
    building.common.position = At::new(Vec2::new(6.0, -2.0));
    building.common.height = 6.0;
    obstacles.create_shape(&building).unwrap();
    for (x, y) in FIGURES {
        let mut figure = ShapeDesc::circle(FOOTPRINT);
        figure.common.position = At::new(Vec2::new(x, y));
        figure.common.height = FIGURE_HEIGHT;
        figure.common.shadow_footprint = true;
        obstacles.create_shape(&figure).unwrap();
    }
    obstacles
}

/// The demo's `Context` setters on Chiaro's defaults (src/core/Settings.cpp `DefaultSettings`):
/// `SetAmbient`, `SetScaling(INTEGER, NEAREST)`, `SetTonemap(SOFT)` (default knee 0.8),
/// `SetHeightStep`, `SetBloom(0.6)` (enables bloom), `SetWetness`, and the custom quality
/// (light 1, shadow 0.5, the medium keeps its default 0.5).
fn settings() -> Light2dSettings {
    let mut s = Light2dSettings {
        light: LightSettings {
            ambient: Vec3::new(0.10, 0.11, 0.15),
            quality: QualitySettings {
                quality: Quality::Custom(Resolutions {
                    light: 1.0,
                    shadow: 0.5,
                    medium: 0.5,
                }),
                ..QualitySettings::default()
            },
            surface_height_step: HEIGHT_STEP,
            ..LightSettings::default()
        },
        post: PostSettings::default(),
    };
    s.light.material.wetness = WETNESS;
    s.post.output.scale_mode = ScaleMode::Integer;
    s.post.output.filter = Filter::Nearest;
    s.post.output.tonemap = Tonemap::Soft { knee: 0.8 };
    s.post.bloom.enabled = true;
    s.post.bloom.intensity = 0.6;
    s
}

fn fixture(name: &str) -> Vec<u8> {
    load_png_rgba8(
        &workspace_root().join(format!("tests/golden/chiaro/wet_street/{name}.png")),
        W,
        H,
    )
}

/// Max channel difference per pixel x32, red where it exceeds the STRICT tolerance.
fn diff_image(a: &[u8], b: &[u8]) -> Vec<u8> {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .flat_map(|(a, b)| {
            let m = (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap();
            let v = m.saturating_mul(32);
            let g = if m > 2 { 0 } else { v };
            [v, g, g, 255]
        })
        .collect()
}

#[test]
fn wet_street_matches_chiaro() {
    with_chiaro_pack(|gpu, pack| {
        let out = std::env::var("KNORKE_SCENE_OUT").ok().map(PathBuf::from);
        let projection = Projection::iso(At::new(Vec2::new(5.0, 1.5)), 24.0, 1.0, W, H);
        let mut shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let mut batch = SurfaceBatch::new(gpu, &shaders).unwrap();
        let mut layers = Layers::new(
            gpu,
            W,
            H,
            LayerSet::of(Layer::Surface).with(Layer::Material),
        )
        .unwrap();
        let mut renderer = Light2dRenderer::new(gpu, pack.to_vec()).unwrap();
        let target: Texture<'_> = texture(
            gpu,
            OUT,
            SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
            SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
            None,
        );

        // The host's frame: clear, colour and material from the C++ renderer dumps, surface
        // geometry on top of the cleared surface layer.
        let (scene, material) = (fixture("scene"), fixture("material"));
        let mut uploads = [&scene, &material].map(|px| {
            let mut up =
                UploadBuffer::new(gpu, SDL_GPUBufferUsageFlags(0), px.len() as u32, "fixture");
            up.reserve(px.len() as u64).unwrap();
            up.map().unwrap().bytes()[..px.len()].copy_from_slice(px);
            up
        });
        let geometry = surfaces(&projection);
        batch.prepare(&mut shaders, &geometry).unwrap();
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        drop(layers.clear_pass(&mut cmd, LayerSet::ALL).unwrap());
        {
            let mut copy = cmd.copy_pass().unwrap();
            for (up, layer) in uploads.iter_mut().zip([Layer::Scene, Layer::Material]) {
                copy.upload_to_texture(up, layers.texture(layer).unwrap(), false)
                    .unwrap();
            }
        }
        layers.mark_drawn(LayerSet::of(Layer::Scene).with(Layer::Material));
        batch.record(&mut cmd, &mut layers, &geometry).unwrap();
        cmd.submit_and_wait().unwrap();

        let surface = read_texture(gpu, layers.texture(Layer::Surface).unwrap());
        let surface_ref = fixture("surface");
        let surface_parity =
            compare_rgba8(&surface, &surface_ref, W, H, &[], &ParityBudget::STRICT).unwrap();
        surface_parity.print("wet_street surface layer", "exact");

        renderer.note_layers(&mut layers);
        let (lights, obstacles) = (lights(), obstacles());
        let stores = SceneStores::new(&lights, &obstacles);
        let settings = settings();
        let frame = layers.frame(target.texture_ref(), &projection, LayerSet::NONE);
        let prepared = renderer.prepare(&frame, &stores, &settings).unwrap();
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        renderer
            .record(&mut cmd, &frame, &settings, prepared)
            .unwrap();
        cmd.submit_and_wait().unwrap();
        let pixels = read_texture(gpu, target.texture_ref());

        let golden = load_png_rgba8(
            &workspace_root().join("tests/golden/chiaro/wet_street.png"),
            OUT.0,
            OUT.1,
        );
        let strict =
            compare_rgba8(&pixels, &golden, OUT.0, OUT.1, &[], &ParityBudget::STRICT).unwrap();
        strict.print("wet_street", "STRICT");
        if let Some(dir) = &out {
            std::fs::create_dir_all(dir).unwrap();
            save_png_rgba8(&dir.join("wet_street.rust.png"), &pixels, OUT.0, OUT.1);
            let diff = diff_image(&pixels, &golden);
            save_png_rgba8(&dir.join("wet_street.diff.png"), &diff, OUT.0, OUT.1);
            save_png_rgba8(&dir.join("wet_street.surface.png"), &surface, W, H);
            let diff = diff_image(&surface, &surface_ref);
            save_png_rgba8(&dir.join("wet_street.surface.diff.png"), &diff, W, H);
        }
        assert!(
            surface == surface_ref,
            "the surface layer differs from Chiaro's: {surface_parity:?}"
        );
        assert!(
            strict.within(&ParityBudget::STRICT),
            "STRICT parity missed: {strict:?}"
        );
    });
}
