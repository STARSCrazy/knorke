//! Contract: `MediumBatch` (P30) allocates nothing on the heap and creates no GPU object in the
//! steady state: once warm, `prepare` and `record` of the same flat and shaped clouds make no
//! Rust allocation (ADR 0006). Command buffer acquire and submit are the host's and stay outside.
//! Own test binary with a single test, so the process-wide allocation counter is undisturbed.
//! Skips without a Vulkan device or the shader pack (a failure with `KNORKE_REQUIRE_GPU=1`).
#![cfg(feature = "gpu")]

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_gpu::test_support::{sampled_rgba8, with_chiaro_pack};
use knorke_gpu::{CommandBuffer, ShaderLibrary};
use knorke_integration::layers::{Layer, LayerSet, Layers};
use knorke_integration::medium::{MediumBatch, MediumItem};
use knorke_light2d::medium_draw::MediumDraw;

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn steady_state_medium_draws_allocate_nothing() {
    with_chiaro_pack(|gpu, pack| {
        let mut shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let mut batch = MediumBatch::new(gpu, &shaders).unwrap();
        let kinds = LayerSet::of(Layer::Scene).with(Layer::Medium);
        let mut layers = Layers::new(gpu, 32, 24, kinds).unwrap();
        let shape = sampled_rgba8(gpu, 2, 2, &[10, 200, 30, 255].repeat(4));
        let cloud = MediumDraw {
            rect: [2.0, 3.0, 20.0, 12.0],
            density: 0.4,
            bottom: 0.0,
            top: 6.0,
            medium: 1,
        };
        let items = [
            MediumItem {
                draw: cloud,
                shape: None,
            },
            MediumItem {
                draw: cloud,
                shape: Some(shape.texture_ref()),
            },
        ];
        let device = gpu.device();
        let mut frames = |n: u32| -> u64 {
            let mut counted = 0;
            for _ in 0..n {
                let mut cmd = CommandBuffer::acquire(device).unwrap();
                let before = allocations();
                batch.prepare(&mut shaders, &items).unwrap();
                drop(layers.clear_pass(&mut cmd, kinds).unwrap());
                batch.record(&mut cmd, &mut layers, 1.0, &items).unwrap();
                counted += allocations() - before;
                cmd.submit_and_wait().unwrap();
            }
            counted
        };
        frames(2); // warm: pipelines
        assert_eq!(frames(5), 0, "steady-state medium draws allocated");
    });
}
