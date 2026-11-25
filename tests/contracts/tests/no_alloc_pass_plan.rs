//! Contract: the pass plan of `knorke-light2d` (P31) allocates nothing on the heap in the steady
//! state: once warm, `PassPlan::prepare` and `PassPlan::record` of an unchanged scene with
//! lights, a shadow-casting light over an obstacle, one with a caster filter (E53), ambient
//! zones, a shockwave ring light (P21), a sprite mask and every debug check (the GPU one with
//! its readback included) make no Rust allocation (ADR 0006). Command buffer acquire and submit
//! are the host's and stay outside.
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
use knorke_light2d::ambient_zone::{AmbientZoneParams, ZoneShape, make_stored_zone};
use knorke_light2d::ambient_zone_store::AmbientZoneStore;
use knorke_light2d::frame_warnings::DebugChecks;
use knorke_light2d::light::{LightDesc, LightType, ShadowFilter};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::pass_plan::{PassFrame, PassPlan, SceneStores};
use knorke_light2d::ring_light::RingLight;
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{LightStore, NoResources, ObstacleStore, Projection};
use knorke_math::{At, Vec2, Vec3};

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn steady_state_pass_plan_allocates_nothing() {
    with_chiaro_pack(|gpu, pack| {
        let (w, h) = (96u32, 48u32);

        let mut lights = LightStore::new();
        // The third light shadows with static casters only: its caster class mask and field (E53).
        for (x, shadows, filter) in [
            (30.0, false, ShadowFilter::All),
            (70.0, true, ShadowFilter::All),
            (50.0, true, ShadowFilter::StaticOnly),
        ] {
            let mut d = LightDesc::new(LightType::Point);
            d.position = At::new(Vec2::new(x, 14.0));
            d.radius = 30.0;
            d.cast_shadows = shadows;
            d.shadow_filter = filter;
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
        let scene = sampled_rgba8(gpu, w, h, &vec![255u8; (w * h * 4) as usize]);
        let mut sprite_px = vec![0u8; (w * h * 4) as usize];
        for y in 30..40 {
            for x in 10..20 {
                sprite_px[((y * w + x) * 4 + 3) as usize] = 255;
            }
        }
        let sprite = sampled_rgba8(gpu, w, h, &sprite_px);
        // An empty material buffer, so the GPU debug check has something to read.
        let material = sampled_rgba8(gpu, w, h, &vec![0u8; (w * h * 4) as usize]);
        let projection = Projection::IDENTITY;
        // One shockwave ring light (P21): the ring pass runs every frame, too.
        let rings = [RingLight {
            scene_rect: [10.0, 10.0, 40.0, 40.0],
            ring: [24.0, 24.0, 10.0, 0.5],
            color: [1.0, 0.5, 0.2, 0.0],
        }];
        let frame = PassFrame {
            occlusion: Some(sprite.texture_ref()),
            material: Some(material.texture_ref()),
            ring_lights: &rings,
            ..PassFrame::new(scene.texture_ref(), &projection)
        };
        let stores = SceneStores {
            zones: Some(&zones),
            ..SceneStores::new(&lights, &obstacles)
        };

        let mut plan = PassPlan::new(gpu, pack.to_vec()).unwrap();
        plan.set_debug_checks(DebugChecks::ALL);
        let frames = |n: u32, plan: &mut PassPlan<'_>| -> u64 {
            let mut counted = 0;
            for _ in 0..n {
                let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
                let before = allocations();
                plan.prepare(&frame, &stores, &settings).unwrap();
                plan.record(&mut cmd, &frame, &settings).unwrap();
                counted += allocations() - before;
                cmd.submit_and_wait().unwrap();
            }
            counted
        };
        frames(3, &mut plan); // warm: targets, pipelines, buffers, cull lists, readback slots
        let live = gpu.counter().live();
        let allocations = frames(5, &mut plan);
        assert_eq!(
            allocations, 0,
            "steady-state frames of the pass plan allocated"
        );
        assert_eq!(gpu.counter().live(), live, "no GPU object per frame");
        drop(plan);
    });
}
