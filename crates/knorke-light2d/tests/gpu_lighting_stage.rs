//! P25 on a real GPU: the light pass (`knorke_light2d::lighting::LightingStage`) after the
//! culling (P8) and the occlusion stages (P22, P23), read back and compared byte for byte with
//! C++ Chiaro (chiaro @ 9bc5b9b, dumps of `tests/golden/chiaro/light/light_ref.cpp`, zlib
//! license), and the light behaviour of Chiaro's `tests/gpu/LightingTests.cpp` checked on the
//! light buffer.
//!
//! The test plays the host (ADR 0002): it owns device, command buffers, the obstacle and
//! occlusion stages, submits and reads back. It skips (passes with a note) without a Vulkan
//! device or without the shader pack from `cargo xtask shaders` in `target/shaders/`.
#![cfg(feature = "gpu")]

mod common;

use common::light_host::{L, L0, Scene, with_host};
use common::{H, W, at};
use knorke_gpu::CommandBuffer;
use knorke_gpu::test_support::{expected_bytes, rgba16f_diff, sampled_rgba8};
use knorke_light2d::light::{Height, LightType};
use knorke_light2d::light_culling::LightList;
use knorke_light2d::light_math::attenuation;
use knorke_light2d::lighting::{DetailMode, LightingFrame, LightingRecord};
use knorke_light2d::settings::LightSettings;
use knorke_light2d::{NoResources, Projection};
use knorke_math::Vec3;

/// A 96 x 48 scene of `light_ref.cpp`: grey ambient and a light resolution.
fn scene(name: &'static str, ambient: f32, light_resolution: f32) -> Scene {
    Scene::golden(name, Vec3::new(ambient, ambient, ambient), light_resolution)
}

/// `SceneFlat` of `light_ref.cpp`.
fn scene_flat() -> Scene {
    let mut s = scene("flat", 0.0, 1.0);
    s.settings.ambient = Vec3::new(0.05, 0.06, 0.07);
    s.lights(&[
        L {
            x: 20.0,
            y: 24.0,
            radius: 30.0,
            color: Vec3::new(1.0, 0.8, 0.6),
            intensity: 1.2,
            ..L0
        },
        L {
            x: 48.0,
            y: 10.0,
            height: Height::At(8.0),
            radius: 20.0,
            falloff: 1.5,
            ..L0
        },
        L {
            kind: LightType::Spot,
            x: 70.0,
            y: 30.0,
            radius: 40.0,
            direction: Vec3::new(-1.0, 0.3, -0.2),
            inner: 15.0,
            outer: 35.0,
            ..L0
        },
        L {
            kind: LightType::Line,
            x: 40.0,
            y: 40.0,
            radius: 18.0,
            color: Vec3::new(0.4, 0.7, 1.0),
            axis: Vec3::new(12.0, 0.0, 0.0),
            ..L0
        },
        L {
            kind: LightType::Directional,
            intensity: 0.1,
            direction: Vec3::new(0.3, 0.4, -1.0),
            ..L0
        },
    ]);
    s
}

fn obstacles(s: &mut Scene, box_h: f32, tri_h: f32, tri_opacity: f32) {
    s.polygon(
        &[40.0, 14.0, 48.0, 14.0, 48.0, 30.0, 40.0, 30.0],
        box_h,
        1.0,
    )
    .polygon(&[60.0, 8.0, 72.0, 12.0, 64.0, 22.0], tri_h, tri_opacity)
    .circle(4.0, 26.0, 34.0, 0.0);
}

/// `SceneShadow` of `light_ref.cpp`.
fn scene_shadow() -> Scene {
    let mut s = scene("shadow", 0.03, 1.0);
    obstacles(&mut s, 0.0, 0.0, 0.6);
    s.lights(&[
        L {
            x: 18.0,
            y: 20.0,
            radius: 50.0,
            shadows: true,
            ..L0
        },
        L {
            x: 80.0,
            y: 36.0,
            radius: 45.0,
            color: Vec3::new(1.0, 0.7, 0.5),
            source: 3.0,
            shadows: true,
            ..L0
        },
        L {
            kind: LightType::Spot,
            x: 50.0,
            y: 44.0,
            radius: 40.0,
            source: 1.0,
            shadows: true,
            direction: Vec3::new(0.0, -1.0, 0.0),
            inner: 20.0,
            outer: 40.0,
            ..L0
        },
        L {
            kind: LightType::Rect,
            x: 30.0,
            y: 6.0,
            radius: 30.0,
            color: Vec3::new(0.6, 0.8, 1.0),
            shadows: true,
            direction: Vec3::new(0.0, 1.0, 0.0),
            axis: Vec3::new(6.0, 0.0, 0.0),
            half_width: 2.0,
            ..L0
        },
        L {
            x: 88.0,
            y: 6.0,
            radius: 15.0,
            ..L0
        },
    ]);
    s
}

/// `SceneHeight` of `light_ref.cpp`.
fn scene_height() -> Scene {
    let mut s = scene("height", 0.0, 1.0);
    s.settings.ambient = Vec3::new(0.04, 0.04, 0.05);
    obstacles(&mut s, 6.0, 3.0, 1.0);
    s.lights(&[
        L {
            x: 18.0,
            y: 20.0,
            height: Height::At(10.0),
            radius: 50.0,
            source: 1.0,
            shadows: true,
            ..L0
        },
        L {
            kind: LightType::Line,
            x: 70.0,
            y: 40.0,
            height: Height::At(4.0),
            radius: 30.0,
            shadows: true,
            axis: Vec3::new(0.0, 6.0, 0.0),
            ..L0
        },
        L {
            kind: LightType::Directional,
            intensity: 0.6,
            source: 1.0,
            shadows: true,
            direction: Vec3::new(0.6, 0.3, -0.8),
            ..L0
        },
    ]);
    s
}

/// `SceneMaterial` of `light_ref.cpp`.
fn scene_material(name: &'static str, light_resolution: f32) -> Scene {
    let mut s = scene(name, 0.05, light_resolution);
    s.material = true;
    s.dome_material = true;
    s.polygon(&[70.0, 30.0, 78.0, 30.0, 78.0, 40.0, 70.0, 40.0], 6.0, 1.0)
        .circle(3.0, 20.0, 10.0, 4.0);
    s.lights(&[
        L {
            x: 20.0,
            y: 24.0,
            radius: 40.0,
            color: Vec3::new(1.0, 0.9, 0.8),
            ..L0
        },
        L {
            x: 60.0,
            y: 10.0,
            radius: 50.0,
            source: 2.0,
            shadows: true,
            ..L0
        },
        L {
            kind: LightType::Spot,
            x: 90.0,
            y: 44.0,
            height: Height::At(6.0),
            radius: 50.0,
            color: Vec3::new(0.5, 0.7, 1.0),
            shadows: true,
            direction: Vec3::new(-1.0, -0.5, -0.3),
            inner: 20.0,
            outer: 35.0,
            ..L0
        },
        L {
            kind: LightType::Directional,
            intensity: 0.4,
            source: 1.0,
            shadows: true,
            direction: Vec3::new(-0.5, 0.4, -0.7),
            ..L0
        },
    ]);
    s
}

#[test]
fn light_buffers_match_chiaro() {
    with_host(|fresh| {
        let mut failures = Vec::new();
        for scene in [
            scene_flat(),
            scene_shadow(),
            scene_height(),
            scene_material("exact", 1.0),
            scene_material("direction", 0.5),
        ] {
            let mut host = fresh();
            let out = host.frame(&scene, 0.0);
            let name = scene.name;
            let (n, max) = rgba16f_diff(
                &out.light,
                &expected_bytes("light", &format!("{name}.light.bin"), &out.light),
            );
            eprintln!("{name}: light buffer {} differing texels (max {max})", n);
            if n > 0 {
                failures.push(format!("{name}.light: {n} texels, max {max}"));
            }
            let second = match out.result.detail_mode {
                DetailMode::Exact => Some("specular"),
                DetailMode::Direction => Some("direction"),
                DetailMode::None => None,
            };
            if let Some(kind) = second {
                let (n, max) = rgba16f_diff(
                    &out.second,
                    &expected_bytes("light", &format!("{name}.{kind}.bin"), &out.second),
                );
                eprintln!("{name}: {kind} buffer {n} differing texels (max {max})");
                if n > 0 {
                    failures.push(format!("{name}.{kind}: {n} texels, max {max}"));
                }
            }
        }
        assert!(failures.is_empty(), "not bit identical: {failures:#?}");
    });
}

/// The same frame twice gives the same bytes, and the second prepare allocates nothing.
#[test]
fn frames_repeat_bit_identically() {
    with_host(|fresh| {
        let mut host = fresh();
        let scene = scene_material("exact", 1.0);
        let a = host.frame(&scene, 0.0);
        let b = host.frame(&scene, 0.0);
        assert!(a.result.allocations > 0);
        assert_eq!(b.result.allocations, 0);
        assert_eq!(a.light, b.light);
        assert_eq!(a.second, b.second);
        assert_eq!(a.record, b.record);
        assert_eq!(a.record.lights_drawn, 3);
        assert_eq!(a.record.rows_built, 2);
    });
}

// ---- Chiaro tests/gpu/LightingTests.cpp, checked on the light buffer -----------------------

/// Light at (lx, ly, lz) seen from the centre of pixel (x, y), identity projection.
fn att_at_pixel(x: u32, y: u32, lx: f32, ly: f32, lz: f32, r: f32) -> f32 {
    let dx = x as f32 + 0.5 - lx;
    let dy = y as f32 + 0.5 - ly;
    attenuation((dx * dx + dy * dy + lz * lz).sqrt(), r, 2.0)
}

fn dark(name: &'static str) -> Scene {
    scene(name, 0.0, 1.0)
}

fn near(got: f32, want: f32) -> bool {
    (got - want).abs() <= 2e-3 + want.abs() * 2e-3
}

#[test]
fn ambient_alone_needs_no_light_pass() {
    with_host(|fresh| {
        let mut host = fresh();
        let s = scene("ambient", 0.25, 1.0);
        let out = host.frame(&s, 0.0);
        assert!(!out.result.light_buffer);
        assert_eq!(out.record, LightingRecord::default());
        assert_eq!(out.result.ambient, Vec3::new(0.25, 0.25, 0.25));
    });
}

#[test]
fn point_light_falls_off_smoothly_and_reaches_zero_at_its_radius() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("point");
        s.light(L {
            x: 32.0,
            y: 32.0,
            height: Height::At(0.0),
            radius: 32.0,
            ..L0
        });
        let out = host.frame(&s, 0.0);
        for (x, y) in [(32, 32), (48, 32), (32, 40)] {
            assert!(near(
                out.red(x, y),
                att_at_pixel(x, y, 32.0, 32.0, 0.0, 32.0)
            ));
        }
        assert_eq!(out.red(0, 0), 0.0);
        assert_eq!(out.record.lights_drawn, 1);
    });
}

#[test]
fn a_raised_light_is_dimmer_directly_below() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("raised");
        s.light(L {
            x: 32.0,
            y: 32.0,
            height: Height::At(16.0),
            radius: 32.0,
            ..L0
        });
        let out = host.frame(&s, 0.0);
        let want = att_at_pixel(32, 32, 32.0, 32.0, 16.0, 32.0);
        assert!(want < 0.8);
        assert!(near(out.red(32, 32), want));
    });
}

#[test]
fn spot_light_only_lights_its_cone() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("spot");
        s.light(L {
            kind: LightType::Spot,
            x: 32.0,
            y: 24.0,
            radius: 32.0,
            direction: Vec3::new(1.0, 0.0, 0.0),
            ..L0
        });
        let out = host.frame(&s, 0.0);
        assert!(near(
            out.red(48, 24),
            att_at_pixel(48, 24, 32.0, 24.0, 0.0, 32.0)
        ));
        assert_eq!(out.red(16, 24), 0.0); // behind
        assert_eq!(out.red(32, 40), 0.0); // 90 degrees off
    });
}

#[test]
fn directional_light_without_shadows_stays_in_the_ambient() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("directional");
        s.light(L {
            kind: LightType::Directional,
            intensity: 0.5,
            direction: Vec3::new(0.0, 0.0, -1.0),
            ..L0
        });
        let out = host.frame(&s, 0.0);
        assert!(!out.result.light_buffer);
        assert_eq!(out.result.ambient, Vec3::new(0.5, 0.5, 0.5));
    });
}

#[test]
fn lights_live_in_world_space_zoom_scales_their_footprint() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("zoom");
        s.projection = Projection::ortho(at(32.0, 24.0), 2.0, W, H);
        s.light(L {
            x: 32.0,
            y: 24.0,
            height: Height::At(0.0),
            radius: 16.0, // 32 scene pixels
            ..L0
        });
        let out = host.frame(&s, 0.0);
        // Scene pixel (64, 24) centre -> world (32 + 16.5 / 2, 24 + 0.5 / 2).
        let (wx, wy) = (32.0 + 16.5 / 2.0, 24.0 + 0.5 / 2.0);
        let d = ((wx - 32.0f32) * (wx - 32.0) + (wy - 24.0f32) * (wy - 24.0)).sqrt();
        assert!(near(out.red(64, 24), attenuation(d, 16.0, 2.0)));
        assert_eq!(out.red(0, 0), 0.0);
    });
}

#[test]
fn light_buffer_and_scene_share_the_vertical_orientation() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("orientation");
        s.light(L {
            x: 32.0,
            y: 12.0,
            height: Height::At(0.0),
            radius: 20.0,
            ..L0
        });
        let out = host.frame(&s, 0.0);
        assert!(near(
            out.red(32, 4),
            att_at_pixel(32, 4, 32.0, 12.0, 0.0, 20.0)
        ));
        assert!(near(
            out.red(32, 28),
            att_at_pixel(32, 28, 32.0, 12.0, 0.0, 20.0)
        ));
        assert_eq!(out.red(32, 40), 0.0);
    });
}

#[test]
fn many_lights_grow_the_buffers_once_and_then_stay_stable() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("many");
        for i in 0..1500u32 {
            s.light(L {
                x: (i % 96) as f32,
                y: ((i * 7) % 48) as f32,
                radius: 2.0,
                intensity: 0.01,
                ..L0
            });
        }
        let a = host.frame(&s, 0.0);
        assert_eq!(a.record.lights_drawn, 1500);
        let b = host.frame(&s, 0.0);
        assert_eq!(b.result.allocations, 0);
        assert_eq!(a.light, b.light);
    });
}

#[test]
fn light_buffer_follows_the_resolution_scale() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = scene("scale", 0.0, 0.5);
        s.light(L {
            x: 32.0,
            y: 24.0,
            radius: 32.0,
            ..L0
        });
        let a = host.frame(&s, 0.0);
        assert_eq!(a.light_size, (48, 24));
        assert_eq!(a.result.allocations, 1);
        assert!(a.red(16, 12) > 0.95);
        let b = host.frame(&s, 0.0);
        assert_eq!(b.result.allocations, 0);
    });
}

#[test]
fn disabled_lights_are_not_drawn() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut s = dark("disabled");
        let mut d = L0.desc();
        d.enabled = false;
        s.lights.create(&d, &NoResources).unwrap();
        let out = host.frame(&s, 0.0);
        assert!(!out.result.light_buffer);
        assert_eq!(out.record.lights_drawn, 0);
    });
}

#[test]
fn record_without_prepare_fails() {
    with_host(|fresh| {
        let host = fresh();
        let scene = sampled_rgba8(host.gpu, 4, 4, &[255u8; 64]);
        let list = LightList::default();
        let settings = LightSettings::default();
        let frame = LightingFrame {
            scene: scene.texture_ref(),
            surface: None,
            material: None,
            projection: &Projection::IDENTITY,
            settings: &settings,
            max_height: 0.0,
            lights: &list,
            shadows: None,
            profile_atlas: None,
            mask_atlas: None,
            ring_lights: &[],
            medium_map: None,
            medium_runs: false,
        };
        let mut cmd = CommandBuffer::acquire(host.gpu.device()).unwrap();
        let e = host
            .lighting
            .record(&mut cmd, &host.samplers, &frame)
            .unwrap_err();
        assert_eq!(e.code, knorke_light2d::lighting::LIGHTING_NOT_PREPARED);
    });
}
