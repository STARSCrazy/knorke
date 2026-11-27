//! Pattern obstacles (P28) end to end: the `height` scene of
//! `tests/golden/chiaro/pattern/pattern_ref.cpp` (C++ Chiaro @ 9bc5b9b, zlib license) through
//! `Light2dRenderer` with default settings, compared with Chiaro's final target under
//! `ParityBudget::STRICT`. The scene is mirrored in `crates/knorke-light2d/tests/gpu_patterns.rs`
//! (`scene_height`); change all three together. Skips without a Vulkan device or shader pack;
//! with `KNORKE_REQUIRE_GPU=1` a skip is a failure.
#![cfg(feature = "gpu")]

use knorke_contract_tests::parity::{ParityBudget, compare_rgba8};
use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
};
use knorke_gpu::test_support::{expected_bytes, sampled_rgba8, texture, with_chiaro_pack};
use knorke_gpu::{CommandBuffer, DownloadBuffer};
use knorke_integration::light2d::{Light2dFrame, Light2dRenderer, Light2dSettings};
use knorke_light2d::light::{Height, LightDesc, LightType};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::pass_plan::SceneStores;
use knorke_light2d::pattern_store::{PatternDesc, Sheet, SheetFormat, raw_id};
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{LightStore, NoResources, ObstacleStore, PatternStore};
use knorke_math::{At, Vec2, Vec3};
use knorke_post::settings::PostSettings;

const W: u32 = 96;
const H: u32 = 48;

/// The reference's sheet: slats x 0..3 (left half open), lattice x 4..19.
fn sheet_pixels() -> Vec<u8> {
    let mut px = vec![0u8; 20 * 16 * 4];
    for y in 0..16 {
        for x in 0..20 {
            let closed = if x < 4 {
                x >= 2
            } else {
                let (dx, dy) = ((x - 4) as f32 + 0.5 - 8.0, y as f32 + 0.5 - 8.0);
                ((dx.abs() + dy.abs()) - 7.0).abs() < 1.1
            };
            let i = (y * 20 + x) * 4;
            px[i..i + 4].copy_from_slice(&[255, 255, 255, if closed { 255 } else { 0 }]);
        }
    }
    px
}

/// Fence (slats, height 6) and tower (lattice, height 16) under a spot light and the moon.
fn height_scene(
    patterns: &mut PatternStore,
    obstacles: &mut ObstacleStore,
    lights: &mut LightStore,
) {
    let px = sheet_pixels();
    let sheet = Sheet {
        width: 20,
        height: 16,
        pitch: 80,
        format: SheetFormat::Rgba8,
        pixels: &px,
    };
    let descs = [
        PatternDesc {
            repeat_width: 3.0,
            ..PatternDesc::new(0, 0, 4, 16)
        },
        PatternDesc {
            repeat_width: 8.0,
            repeat_height: 8.0,
            ..PatternDesc::new(4, 0, 16, 16)
        },
    ];
    let mut ids = Vec::new();
    patterns.create(&sheet, &descs, &mut ids).unwrap();
    for (x, width, depth, height, pattern) in [
        (70.0, 1.0, 36.0, 6.0, ids[0]),
        (30.0, 8.0, 8.0, 16.0, ids[1]),
    ] {
        let common = ObstacleCommon {
            position: At::new(Vec2::new(x, 24.0)),
            height,
            ..Default::default()
        };
        let shape = Shape::Box { width, depth };
        let id = obstacles
            .create_shape(&ShapeDesc { common, shape })
            .unwrap();
        obstacles
            .set_pattern(id, raw_id(pattern), patterns)
            .unwrap();
    }
    for (kind, position, height, radius, direction, intensity) in [
        (
            LightType::Spot,
            Vec2::new(42.0, 24.0),
            Height::At(12.0),
            50.0,
            Vec3::new(-1.0, 0.0, -0.4),
            1.5,
        ),
        (
            LightType::Directional,
            Vec2::new(0.0, 0.0),
            Height::Auto,
            30.0,
            Vec3::new(-0.7, 0.15, -0.7),
            0.6,
        ),
    ] {
        let mut d = LightDesc::new(kind);
        d.position = At::new(position);
        d.height = height;
        d.radius = radius;
        d.direction = direction;
        d.intensity = intensity;
        d.inner_angle = 25.0;
        d.outer_angle = 45.0;
        d.cast_shadows = true;
        lights.create(&d, &NoResources).unwrap();
    }
}

#[test]
fn pattern_scene_matches_chiaro_through_light2d_renderer() {
    with_chiaro_pack(|gpu, pack| {
        let mut renderer = Light2dRenderer::new(gpu, pack.to_vec()).unwrap();
        let (mut patterns, mut obstacles, mut lights) =
            (PatternStore::new(), ObstacleStore::new(), LightStore::new());
        height_scene(&mut patterns, &mut obstacles, &mut lights);

        let scene = sampled_rgba8(gpu, W, H, &vec![255u8; (W * H * 4) as usize]);
        let target = texture(
            gpu,
            (W, H),
            SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
            SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
            None,
        );
        let projection = Projection::default();
        let frame = Light2dFrame::new(scene.texture_ref(), target.texture_ref(), &projection);
        let settings = Light2dSettings {
            light: LightSettings {
                ambient: Vec3::new(0.03, 0.03, 0.04),
                ..LightSettings::default()
            },
            post: PostSettings::default(),
        };
        let stores = SceneStores {
            patterns: Some(&patterns),
            ..SceneStores::new(&lights, &obstacles)
        };
        let prepared = renderer.prepare(&frame, &stores, &settings).unwrap();
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        renderer
            .record(&mut cmd, &frame, &settings, prepared)
            .unwrap();
        let download = DownloadBuffer::new(gpu, W * H * 4).unwrap();
        cmd.copy_pass()
            .unwrap()
            .download_texture(frame.target, &download)
            .unwrap();
        cmd.submit_and_wait().unwrap();
        let mut pixels = vec![0u8; (W * H * 4) as usize];
        let mut download = download;
        download.read(&mut pixels).unwrap();

        // The rows differ from Chiaro's light along shadow edges: re-blessed (tests/golden/rows).
        let reference = expected_bytes("pattern", "height.target.bin", &pixels);
        let strict = compare_rgba8(&pixels, &reference, W, H, &[], &ParityBudget::STRICT).unwrap();
        strict.print("pattern height", "STRICT");
        assert!(
            strict.within(&ParityBudget::STRICT),
            "STRICT parity missed: {strict:?}"
        );
    });
}
