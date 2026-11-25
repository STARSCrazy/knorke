//! Contract: pattern obstacles (P28) allocate nothing on the heap in the steady state: once warm,
//! `PassPlan::prepare` and `PassPlan::record` of an unchanged scene with a pattern box, a rotated
//! pattern polygon and a tile grid with a pattern table under a shadow-casting light make no Rust
//! allocation (ADR 0006), both when the mask is redrawn every frame (a sprite mask) and when it
//! is reused. Command buffer acquire and submit are the host's and stay outside.
//!
//! Needs the feature `gpu` of this crate, a Vulkan device and the shader pack of
//! `cargo xtask shaders`; skips without them (a failure with `KNORKE_REQUIRE_GPU=1`).
//!
//! Own test binary with a single test, so the process-wide allocation counter is not disturbed
//! by other tests running concurrently.
#![cfg(feature = "gpu")]

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use std::path::Path;

use knorke_gpu::sys::gpu::{
    SDL_GPU_SHADERFORMAT_SPIRV, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER,
    SDL_GPUBufferUsageFlags,
};
use knorke_gpu::{
    CommandBuffer, Device, Gpu, LiveCounter, Texture, TextureDesc, UploadBuffer, init_video,
};
use knorke_light2d::light::{LightDesc, LightType};
use knorke_light2d::obstacle::{CellRect, ObstacleCommon, Shape, ShapeDesc, TileGridDesc};
use knorke_light2d::pass_plan::{PassFrame, PassPlan, SceneStores};
use knorke_light2d::pattern_store::{PatternDesc, Sheet, SheetFormat, raw_id};
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{LightStore, NoResources, ObstacleStore, PatternStore, Projection};
use knorke_math::{At, Vec2};

#[global_allocator]
static GLOBAL: Counting = Counting;

fn texture<'d>(gpu: Gpu<'d>, w: u32, h: u32, pixels: &[u8]) -> Texture<'d> {
    let desc = TextureDesc::new(
        w,
        h,
        SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
        SDL_GPU_TEXTUREUSAGE_SAMPLER,
    );
    let texture = Texture::new(gpu, desc).unwrap();
    let mut up = UploadBuffer::new(gpu, SDL_GPUBufferUsageFlags(0), pixels.len() as u32, "px");
    up.reserve(pixels.len() as u64).unwrap();
    up.map().unwrap().bytes()[..pixels.len()].copy_from_slice(pixels);
    let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
    cmd.copy_pass()
        .unwrap()
        .upload_to_texture(&up, texture.texture_ref(), false)
        .unwrap();
    cmd.submit_and_wait().unwrap();
    texture
}

#[test]
fn steady_state_pattern_obstacles_allocate_nothing() {
    let require = std::env::var("KNORKE_REQUIRE_GPU").is_ok_and(|v| v == "1");
    let pack =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shaders/knorke-chiaro.spirv.kpak");
    let Ok(pack) = std::fs::read(&pack) else {
        assert!(!require, "no shader pack at {}", pack.display());
        eprintln!("skipped: no shader pack (cargo xtask shaders)");
        return;
    };
    init_video(Some(c"vulkan"));
    let device = match Device::create(SDL_GPU_SHADERFORMAT_SPIRV, false) {
        Ok(d) => d,
        Err(e) => {
            assert!(!require, "no Vulkan device ({})", e.message);
            eprintln!("skipped: no Vulkan device ({})", e.message);
            return;
        }
    };
    let counter = LiveCounter::new();
    let gpu = Gpu::new(&device, &counter);
    let (w, h) = (96u32, 48u32);

    // Two 4 x 4 patterns: vertical slats and a checker.
    let sheet_px: Vec<u8> = (0..32)
        .flat_map(|i| {
            let (x, y) = (i % 8, i / 8);
            let closed = if x < 4 { x % 2 == 0 } else { (x + y) % 2 == 0 };
            [255, 255, 255, if closed { 255 } else { 0 }]
        })
        .collect();
    let sheet = Sheet {
        width: 8,
        height: 4,
        pitch: 32,
        format: SheetFormat::Rgba8,
        pixels: &sheet_px,
    };
    let mut patterns = PatternStore::new();
    let mut ids = Vec::new();
    patterns
        .create(
            &sheet,
            &[PatternDesc::new(0, 0, 4, 4), PatternDesc::new(4, 0, 4, 4)],
            &mut ids,
        )
        .unwrap();
    let (slats, checker) = (raw_id(ids[0]), raw_id(ids[1]));

    let mut obstacles = ObstacleStore::new();
    let tri = [
        Vec2::new(-4.0, -5.0),
        Vec2::new(5.0, -1.0),
        Vec2::new(0.0, 6.0),
    ];
    for (x, rotation, height, shape, pattern) in [
        (
            30.0,
            0.0,
            8.0,
            Shape::Box {
                width: 6.0,
                depth: 6.0,
            },
            slats,
        ),
        (60.0, 25.0, 0.0, Shape::Polygon(&tri), checker),
    ] {
        let common = ObstacleCommon {
            position: At::new(Vec2::new(x, 24.0)),
            rotation,
            height,
            ..Default::default()
        };
        let id = obstacles
            .create_shape(&ShapeDesc { common, shape })
            .unwrap();
        obstacles.set_pattern(id, pattern, &patterns).unwrap();
    }
    let solid = [1u8, 1, 1, 0];
    let mut grid = TileGridDesc::new(2, 2, &solid);
    grid.common.position = At::new(Vec2::new(10.0, 8.0));
    grid.cell_size = Vec2::new(3.0, 3.0);
    let grid = obstacles.create_tile_grid(&grid).unwrap();
    obstacles.set_pattern(grid, checker, &patterns).unwrap();
    obstacles
        .set_tile_pattern_table(grid, &[slats], &patterns)
        .unwrap();
    let rect = CellRect {
        x0: 0,
        y0: 0,
        x1: 2,
        y1: 2,
    };
    obstacles
        .set_tile_pattern_cells(grid, rect, &[0, 1, 1, 0])
        .unwrap();

    let mut lights = LightStore::new();
    let mut d = LightDesc::new(LightType::Point);
    d.position = At::new(Vec2::new(45.0, 20.0));
    d.radius = 60.0;
    d.cast_shadows = true;
    lights.create(&d, &NoResources).unwrap();
    let settings = LightSettings::default();
    let scene = texture(gpu, w, h, &vec![255u8; (w * h * 4) as usize]);
    let mut sprite_px = vec![0u8; (w * h * 4) as usize];
    for y in 30..40 {
        for x in 80..90 {
            sprite_px[((y * w + x) * 4 + 3) as usize] = 255;
        }
    }
    let sprite = texture(gpu, w, h, &sprite_px);
    let projection = Projection::IDENTITY;
    let stores = SceneStores {
        patterns: Some(&patterns),
        ..SceneStores::new(&lights, &obstacles)
    };

    // With the sprite the mask (and the pattern id texture) is redrawn every frame; without it
    // the mask and the pattern list are reused.
    for sprite in [Some(sprite.texture_ref()), None] {
        let frame = PassFrame {
            occlusion: sprite,
            ..PassFrame::new(scene.texture_ref(), &projection)
        };
        let mut plan = PassPlan::new(gpu, pack.clone()).unwrap();
        let mut frames = |n: u32| -> u64 {
            let mut counted = 0;
            for _ in 0..n {
                let mut cmd = CommandBuffer::acquire(&device).unwrap();
                let before = allocations();
                plan.prepare(&frame, &stores, &settings).unwrap();
                plan.record(&mut cmd, &frame, &settings).unwrap();
                counted += allocations() - before;
                cmd.submit_and_wait().unwrap();
            }
            counted
        };
        frames(3); // warm: targets, atlas, pipelines, buffers, lists
        let live = counter.live();
        let allocated = frames(5);
        assert_eq!(
            allocated,
            0,
            "steady-state pattern frames allocated (sprite {})",
            sprite.is_some()
        );
        assert_eq!(counter.live(), live, "no GPU object per frame");
        let programs = plan
            .lighting()
            .program(knorke_light2d::lighting::DrawProgram::Shadowed);
        assert!(
            programs.is_some_and(|p| p.has(knorke_light2d::light_program::LightFeature::Pattern)),
            "the patterns are not drawn"
        );
    }
    drop((scene, sprite));
    assert_eq!(counter.live(), 0, "GPU objects leaked");
}
