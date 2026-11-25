//! Contract: the surface geometry of `knorke-light2d` and its GPU batch in `knorke-integration`
//! (P35) allocate nothing on the heap in the steady state: once warm, refilling a
//! `SurfaceGeometry` of the same size and `SurfaceBatch::prepare` / `record` make no Rust
//! allocation (ADR 0006). Clearing the layers, command buffer acquire and submit are the host's
//! and stay outside.
//!
//! Needs the feature `gpu` of this crate, a Vulkan device and the shader pack of
//! `cargo xtask shaders`; skips without them (a failure with `KNORKE_REQUIRE_GPU=1`).
//!
//! Own test binary with a single test, so the process-wide allocation counter is not disturbed
//! by other tests running concurrently.
#![cfg(feature = "gpu")]

use std::path::Path;

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_gpu::sys::gpu::SDL_GPU_SHADERFORMAT_SPIRV;
use knorke_gpu::{CommandBuffer, Device, Gpu, LiveCounter, ShaderLibrary, init_video};
use knorke_integration::layers::{Layer, LayerSet, Layers};
use knorke_integration::surface::SurfaceBatch;
use knorke_light2d::Projection;
use knorke_light2d::surface_geometry::{SurfaceBox, SurfaceGeometry};
use knorke_math::{At, Vec2, Vec3};

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn steady_state_surface_geometry_allocates_nothing() {
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

    let projection = Projection::iso(At::new(Vec2::ZERO), 4.0, 1.0, 64, 48);
    let mut shaders = ShaderLibrary::new(gpu, pack).unwrap();
    let mut batch = SurfaceBatch::new(gpu, &shaders).unwrap();
    let mut layers = Layers::new(gpu, 64, 48, Layer::Surface.into()).unwrap();
    let mut geometry = SurfaceGeometry::new();
    let boxes = [SurfaceBox {
        center: At::new(Vec2::new(1.0, 1.0)),
        width: 3.0,
        depth: 2.0,
        height: 4.0,
    }];
    let floor = [(-4.0, -4.0), (4.0, -4.0), (4.0, 4.0)].map(|(x, y)| At::new(Vec2::new(x, y)));

    let mut frames = |n: u32| -> u64 {
        let mut counted = 0;
        for _ in 0..n {
            let mut cmd = CommandBuffer::acquire(&device).unwrap();
            drop(layers.clear_pass(&mut cmd, LayerSet::ALL).unwrap());
            let before = allocations();
            geometry.clear();
            geometry.push_floor(&projection, 0.1, &floor, 0.0).unwrap();
            geometry.push_boxes(&projection, 0.1, &boxes).unwrap();
            let (a, b) = (Vec3::new(-3.0, 3.0, 0.0), Vec3::new(3.0, 3.0, 0.0));
            geometry.push_wall(&projection, 0.1, a, b, 2.0).unwrap();
            batch.prepare(&mut shaders, &geometry).unwrap();
            batch.record(&mut cmd, &mut layers, &geometry).unwrap();
            counted += allocations() - before;
            cmd.submit_and_wait().unwrap();
        }
        counted
    };
    frames(3); // warm: the pipeline, the vertex buffer, the geometry's capacity
    let live = counter.live();
    assert_eq!(frames(5), 0, "steady-state surface geometry allocated");
    assert_eq!(counter.live(), live, "no GPU object per frame");
    drop((batch, layers, shaders));
    assert_eq!(counter.live(), 0, "GPU objects leaked");
}
