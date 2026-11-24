//! Contract: the pass plan of `knorke-light2d` (P31) allocates nothing on the heap in the steady
//! state: once warm, `PassPlan::prepare` and `PassPlan::record` of an unchanged scene with
//! lights, a shadow-casting light over an obstacle, ambient zones and a sprite mask make no Rust
//! allocation (ADR 0006). Command buffer acquire and submit are the host's and stay outside.
//!
//! Needs the feature `gpu` of this crate, a Vulkan device and the shader pack of
//! `cargo xtask shaders`; skips without them (a failure with `KNORKE_REQUIRE_GPU=1`).
//!
//! Own test binary with a single test, so the process-wide allocation counter is not disturbed
//! by other tests running concurrently.
#![cfg(feature = "gpu")]
#![allow(unsafe_code)] // counting allocator

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use knorke_gpu::sys::gpu::{
    SDL_GPU_SHADERFORMAT_SPIRV, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_SAMPLER,
    SDL_GPUBufferUsageFlags,
};
use knorke_gpu::{
    CommandBuffer, Device, Gpu, LiveCounter, Texture, TextureDesc, UploadBuffer, init_video,
};
use knorke_light2d::ambient_zone::{AmbientZoneParams, ZoneShape, make_stored_zone};
use knorke_light2d::ambient_zone_store::AmbientZoneStore;
use knorke_light2d::light::{LightDesc, LightType};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::pass_plan::{PassFrame, PassPlan, SceneStores};
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{LightStore, NoResources, ObstacleStore, Projection};
use knorke_math::{At, Vec2, Vec3};

struct Counting;
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
// SAFETY: forwards to the system allocator unchanged; only counts calls.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
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
fn steady_state_pass_plan_allocates_nothing() {
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
    for (x, shadows) in [(30.0, false), (70.0, true)] {
        let mut d = LightDesc::new(LightType::Point);
        d.position = At::new(Vec2::new(x, 14.0));
        d.radius = 30.0;
        d.cast_shadows = shadows;
        lights.create(&d, &NoResources).unwrap();
    }
    let mut obstacles = ObstacleStore::new();
    let square = [
        Vec2::new(60.0, 8.0),
        Vec2::new(64.0, 8.0),
        Vec2::new(64.0, 20.0),
        Vec2::new(60.0, 20.0),
    ];
    obstacles
        .create_shape(&ShapeDesc {
            common: ObstacleCommon::default(),
            shape: Shape::Polygon(&square),
        })
        .unwrap();
    let mut zones = AmbientZoneStore::new();
    let zone = AmbientZoneParams {
        shape: ZoneShape::Box,
        x: 24.0,
        y: 24.0,
        width: 28.0,
        depth: 20.0,
        ambient: [0.25, 0.5, 1.0],
        soft_edge: 4.0,
        ..Default::default()
    };
    zones.create(make_stored_zone(&zone).unwrap()).unwrap();
    let settings = LightSettings {
        ambient: Vec3::new(0.3, 0.35, 0.4),
        ..LightSettings::default()
    };
    let scene = texture(gpu, w, h, &vec![255u8; (w * h * 4) as usize]);
    let mut sprite_px = vec![0u8; (w * h * 4) as usize];
    for y in 30..40 {
        for x in 10..20 {
            sprite_px[((y * w + x) * 4 + 3) as usize] = 255;
        }
    }
    let sprite = texture(gpu, w, h, &sprite_px);
    let projection = Projection::IDENTITY;
    let frame = PassFrame {
        occlusion: Some(sprite.texture_ref()),
        ..PassFrame::new(scene.texture_ref(), &projection)
    };
    let stores = SceneStores {
        zones: Some(&zones),
        ..SceneStores::new(&lights, &obstacles)
    };

    let mut plan = PassPlan::new(gpu, pack).unwrap();
    let frames = |n: u32, plan: &mut PassPlan<'_>| -> u64 {
        let mut counted = 0;
        for _ in 0..n {
            let mut cmd = CommandBuffer::acquire(&device).unwrap();
            let before = ALLOCATIONS.load(Ordering::Relaxed);
            plan.prepare(&frame, &stores, &settings).unwrap();
            plan.record(&mut cmd, &frame, &settings).unwrap();
            counted += ALLOCATIONS.load(Ordering::Relaxed) - before;
            cmd.submit_and_wait().unwrap();
        }
        counted
    };
    frames(3, &mut plan); // warm: targets, pipelines, buffers, cull lists
    let live = counter.live();
    let allocations = frames(5, &mut plan);
    assert_eq!(
        allocations, 0,
        "steady-state frames of the pass plan allocated"
    );
    assert_eq!(counter.live(), live, "no GPU object per frame");
    drop(plan);
    drop((scene, sprite));
    assert_eq!(counter.live(), 0, "GPU objects leaked");
}
