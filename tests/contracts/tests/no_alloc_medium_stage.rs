//! Contract: the medium stage of `knorke-light2d` (P30) allocates nothing on the heap in the
//! steady state: once warm, `PassPlan::prepare` and `PassPlan::record` of an unchanged scene with
//! haze, a medium layer, a shadowed spot light over an obstacle (a light shaft row) and a shadowed
//! directional light (a sun height block) make no Rust allocation and create no GPU object
//! (ADR 0006). Command buffer acquire and submit are the host's and stay outside.
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
use knorke_light2d::codec::encode_medium_pixel;
use knorke_light2d::light::{LightDesc, LightType};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::pass_plan::{PassFrame, PassPlan, PassStages, SceneStores};
use knorke_light2d::settings::{LightSettings, Quality, Resolutions};
use knorke_light2d::{LightStore, NoResources, ObstacleStore, Projection};
use knorke_math::{At, Vec2, Vec3};

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
fn steady_state_medium_stage_allocates_nothing() {
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

    let mut lights = LightStore::new();
    let mut spot = LightDesc::new(LightType::Spot);
    spot.position = At::new(Vec2::new(10.0, 12.0));
    spot.radius = 40.0;
    spot.cast_shadows = true;
    lights.create(&spot, &NoResources).unwrap();
    let mut sun = LightDesc::new(LightType::Directional);
    sun.direction = Vec3::new(-0.7, 0.15, -0.7);
    sun.cast_shadows = true;
    lights.create(&sun, &NoResources).unwrap();
    let mut obstacles = ObstacleStore::new();
    obstacles
        .create_shape(&ShapeDesc {
            common: ObstacleCommon {
                position: At::new(Vec2::new(14.0, 8.0)),
                height: 6.0,
                ..Default::default()
            },
            shape: Shape::Box {
                width: 4.0,
                depth: 8.0,
            },
        })
        .unwrap();
    // A second, lower box: obstacles of different tops give the occlusion heights the sun
    // block marches through.
    obstacles
        .create_shape(&ShapeDesc {
            common: ObstacleCommon {
                position: At::new(Vec2::new(24.0, 4.0)),
                height: 4.0,
                ..Default::default()
            },
            shape: Shape::Box {
                width: 6.0,
                depth: 3.0,
            },
        })
        .unwrap();
    let mut settings = LightSettings::default();
    settings.haze.density = 0.2;
    settings.quality.quality = Quality::Custom(Resolutions {
        light: 1.0,
        shadow: 0.5,
        medium: 0.5,
    });
    let step = settings.resolved_medium_height_step();
    let mut layer = vec![0u8; (w * h * 4) as usize];
    for y in 10..30 {
        for x in 20..50 {
            let px = encode_medium_pixel(0.5, 0.0, 12.0, 1, step).unwrap();
            let i = ((y * w + x) * 4) as usize;
            layer[i..i + 4].copy_from_slice(&px);
        }
    }
    let scene = texture(gpu, w, h, &vec![255u8; (w * h * 4) as usize]);
    let medium = texture(gpu, w, h, &layer);
    // A height axis: the sun's shadow needs the height mode for its block (as `sun` of
    // `tests/golden/chiaro/medium/medium_stage_ref.cpp`).
    let projection = Projection {
        origin: At::new(Vec2::new(48.3, 10.6)),
        x_axis: Vec2::new(1.6, 0.8),
        y_axis: Vec2::new(-1.6, 0.8),
        z_axis: Vec2::new(0.0, -1.2),
    };
    let frame = PassFrame {
        medium: Some(medium.texture_ref()),
        ..PassFrame::new(scene.texture_ref(), &projection)
    };
    let stores = SceneStores::new(&lights, &obstacles);

    let mut plan = PassPlan::new(gpu, pack).unwrap();
    let frames = |n: u32, plan: &mut PassPlan<'_>| -> u64 {
        let mut counted = 0;
        for _ in 0..n {
            let mut cmd = CommandBuffer::acquire(&device).unwrap();
            let before = allocations();
            let prepared = plan.prepare(&frame, &stores, &settings).unwrap();
            let out = plan.record(&mut cmd, &frame, &settings).unwrap();
            counted += allocations() - before;
            assert!(prepared.stages.contains(PassStages::MEDIUM));
            assert!(
                out.medium.shaft_rows > 0 && out.medium.sun_blocks > 0,
                "{:?}",
                out.medium
            );
            cmd.submit_and_wait().unwrap();
        }
        counted
    };
    frames(3, &mut plan); // warm: stage targets, atlases, pipelines
    let live = counter.live();
    let allocations = frames(5, &mut plan);
    assert_eq!(
        allocations, 0,
        "steady-state frames with the medium stage allocated"
    );
    assert_eq!(counter.live(), live, "no GPU object per frame");
    drop(plan);
    drop((scene, medium));
    assert_eq!(counter.live(), 0, "GPU objects leaked");
}
