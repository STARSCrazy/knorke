//! Contract: the medium world map of `knorke-light2d` (P29) allocates nothing on the heap in the
//! steady state: once warm, `PassPlan::prepare` and `PassPlan::record` of an unchanged scene with
//! a medium layer, a shadowed spot light over an obstacle and a shadowed directional light (a row
//! light and a sun map, ADR 0019, drawn with the `_medium` row programs) and an unshadowed
//! directional light (Chiaro's `*_medium` program) make no Rust allocation and create no GPU
//! object (ADR 0006). Command buffer acquire and submit are the host's and stay outside.
//!
//! Needs the feature `gpu` of this crate, a Vulkan device and the shader pack of
//! `cargo xtask shaders`; skips without them (a failure with `KNORKE_REQUIRE_GPU=1`).
//!
//! Own test binary with a single test, so the process-wide allocation counter is not disturbed
//! by other tests running concurrently.
#![cfg(feature = "gpu")]

use knorke_contract_tests::counting_alloc::{Counting, allocations};

use knorke_gpu::CommandBuffer;
use knorke_gpu::test_support::{sampled_rgba8, with_chiaro_pack};
use knorke_light2d::codec::encode_medium_pixel;
use knorke_light2d::light::{LightDesc, LightType};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::pass_plan::{PassFrame, PassPlan, PassStages, SceneStores};
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{LightStore, NoResources, ObstacleStore, Projection};
use knorke_math::{At, Vec2, Vec3};

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn steady_state_medium_map_allocates_nothing() {
    with_chiaro_pack(|gpu, pack| {
        let (w, h) = (96u32, 48u32);

        let mut lights = LightStore::new();
        let mut spot = LightDesc::new(LightType::Spot);
        spot.position = At::new(Vec2::new(40.0, 24.0));
        spot.radius = 40.0;
        spot.cast_shadows = true;
        lights.create(&spot, &NoResources).unwrap();
        let mut sun = LightDesc::new(LightType::Directional);
        sun.direction = Vec3::new(-0.7, 0.15, -0.7);
        sun.cast_shadows = false;
        lights.create(&sun, &NoResources).unwrap();
        let moon = LightDesc {
            direction: Vec3::new(0.5, 0.4, -0.75),
            cast_shadows: true,
            ..sun.clone()
        };
        lights.create(&moon, &NoResources).unwrap();
        let mut obstacles = ObstacleStore::new();
        obstacles
            .create_shape(&ShapeDesc {
                common: ObstacleCommon {
                    position: At::new(Vec2::new(60.0, 24.0)),
                    height: 6.0,
                    ..Default::default()
                },
                shape: Shape::Box {
                    width: 4.0,
                    depth: 8.0,
                },
            })
            .unwrap();
        let settings = LightSettings::default();
        let step = settings.resolved_medium_height_step();
        let mut layer = vec![0u8; (w * h * 4) as usize];
        for y in 10..30 {
            for x in 20..50 {
                let px = encode_medium_pixel(0.5, 0.0, 12.0, 1, step).unwrap();
                let i = ((y * w + x) * 4) as usize;
                layer[i..i + 4].copy_from_slice(&px);
            }
        }
        let scene = sampled_rgba8(gpu, w, h, &vec![255u8; (w * h * 4) as usize]);
        let medium = sampled_rgba8(gpu, w, h, &layer);
        let projection = Projection::IDENTITY;
        let frame = PassFrame {
            medium: Some(medium.texture_ref()),
            ..PassFrame::new(scene.texture_ref(), &projection)
        };
        let stores = SceneStores::new(&lights, &obstacles);

        let mut plan = PassPlan::new(gpu, pack.to_vec()).unwrap();
        let frames = |n: u32, plan: &mut PassPlan<'_>| -> u64 {
            let mut counted = 0;
            for _ in 0..n {
                let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
                let before = allocations();
                let prepared = plan.prepare(&frame, &stores, &settings).unwrap();
                let record = plan.record(&mut cmd, &frame, &settings).unwrap();
                counted += allocations() - before;
                assert!(prepared.stages.contains(PassStages::MEDIUM_MAP));
                let rows = (record.lighting.row_lights, record.lighting.row_suns);
                // The sun without a shadow too: the sun program draws it without a map.
                assert_eq!(rows, (1, 2), "the spot light, the moon and the sun");
                cmd.submit_and_wait().unwrap();
            }
            counted
        };
        frames(3, &mut plan); // warm: map targets, pipelines, light programs
        let live = gpu.counter().live();
        let allocations = frames(5, &mut plan);
        assert_eq!(
            allocations, 0,
            "steady-state frames with a medium map allocated"
        );
        assert_eq!(gpu.counter().live(), live, "no GPU object per frame");
    });
}
