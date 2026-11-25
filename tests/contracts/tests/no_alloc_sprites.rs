//! Contract: the sprite batch of `knorke-integration` (P34) allocates nothing on the heap in the
//! steady state: once warm, `SpriteBatch::prepare` and `SpriteBatch::record` of the same number
//! of draws make no Rust allocation (ADR 0006). Clearing the layers, command buffer acquire and
//! submit are the host's and stay outside.
//!
//! Needs the feature `gpu` of this crate, a Vulkan device and the shader pack of
//! `cargo xtask shaders`; skips without them (a failure with `KNORKE_REQUIRE_GPU=1`).
//!
//! Own test binary with a single test, so the process-wide allocation counter is not disturbed
//! by other tests running concurrently.
#![cfg(feature = "gpu")]

use knorke_contract_tests::counting_alloc::{Counting, allocations};
use knorke_gpu::test_support::with_chiaro_pack;
use knorke_gpu::{CommandBuffer, Samplers, ShaderLibrary};
use knorke_integration::layers::{Layer, LayerSet, Layers};
use knorke_integration::sprites::{Flip, Sprite, SpriteBatch, SpriteDesc, SpriteDraw};
use knorke_light2d::codec::Material;

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn steady_state_sprite_batch_allocates_nothing() {
    with_chiaro_pack(|gpu, pack| {
        let kinds = LayerSet::of(Layer::Material).with(Layer::Occlusion);
        let mut shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let samplers = Samplers::new(gpu).unwrap();
        let mut batch = SpriteBatch::new(gpu, &shaders).unwrap();
        let mut layers = Layers::new(gpu, 64, 48, kinds).unwrap();
        let color = [200u8, 120, 40, 255].repeat(16 * 16);
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        let desc = SpriteDesc {
            material: Material::SATIN,
            casts_shadow: true,
            ..SpriteDesc::new(16, 16, &color)
        };
        let sprite = Sprite::new(gpu, &mut cmd, &desc, kinds, 1.0).unwrap();
        cmd.submit_and_wait().unwrap();
        let draws = [
            SpriteDraw::new(&sprite, [2.0, 2.0, 16.0, 16.0]),
            SpriteDraw {
                flip: Flip { x: true, y: false },
                ..SpriteDraw::new(&sprite, [30.0, 10.0, 32.0, 32.0])
            },
        ];

        let mut frames = |n: u32| -> u64 {
            let mut counted = 0;
            for _ in 0..n {
                let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
                drop(layers.clear_pass(&mut cmd, LayerSet::ALL).unwrap());
                let before = allocations();
                batch.prepare(&mut shaders, &draws).unwrap();
                batch
                    .record(&mut cmd, &samplers, &mut layers, &draws)
                    .unwrap();
                counted += allocations() - before;
                cmd.submit_and_wait().unwrap();
            }
            counted
        };
        frames(3); // warm: pipelines, the quad buffer
        let live = gpu.counter().live();
        assert_eq!(frames(5), 0, "steady-state sprite batches allocated");
        assert_eq!(gpu.counter().live(), live, "no GPU object per frame");
    });
}
