//! Light rows (ADR 0019, plan 17.13) in the pass plan, on the Chiaro demo scene `iso_night`:
//! every light is a row light, obstacles of every kind are runs of the rows, and a lamp's row
//! (a sun's map) is built only when the lamp or the mask around it changed, with the picture of
//! a plan that builds them all.
//!
//! The rows against the reference picture are measured by `gpu_lightrow_probe.rs`.
//!
//! The tests play the host (ADR 0002). They skip without a Vulkan device or the shader pack of
//! `cargo xtask shaders` (a failure with `KNORKE_REQUIRE_GPU=1`).
#![cfg(feature = "gpu")]

#[path = "common/gpu.rs"]
mod scene_common;

#[path = "common/demo_scene.rs"]
mod demo_scene;

#[path = "common/iso_night.rs"]
mod iso_night;

use demo_scene::{DemoFrames, H, OUT, Scene, W};
use knorke_gpu::test_support::{sampled_rgba8, with_chiaro_pack};
use knorke_integration::layers::{Layer, LayerSet};
use knorke_light2d::frame_warnings::Warning;
use knorke_light2d::light::{LightDesc, LightType, MaskMode};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc, TileGridDesc};
use knorke_math::{At, Vec2, Vec3};

/// Largest channel difference of two pictures, the mean over all channels and the share of
/// pixels that differ, in percent.
fn diff(a: &[u8], b: &[u8]) -> (u8, f64, f64) {
    assert_eq!(a.len(), b.len());
    let (mut max, mut sum, mut pixels) = (0, 0u64, 0u64);
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let d = (0..4).map(|i| p[i].abs_diff(q[i]));
        max = max.max(d.clone().max().unwrap());
        sum += d.clone().map(u64::from).sum::<u64>();
        pixels += u64::from(p != q);
    }
    let n = (a.len() / 4) as f64;
    (max, sum as f64 / (4.0 * n), 100.0 * pixels as f64 / n)
}

/// Makes the demo's two pattern obstacles opaque: they keep the lamps reaching them from the
/// rows. Returns them.
fn drop_patterns(scene: &mut Scene) -> Vec<knorke_light2d::ObstacleId> {
    let ids: Vec<_> = (scene.obstacles.iter())
        .filter(|(_, o)| o.patterns.refs > 0)
        .map(|(id, _)| id)
        .collect();
    assert_eq!(ids.len(), 2, "the demo's pattern obstacles");
    for &id in &ids {
        scene.obstacles.set_pattern(id, 0, &scene.patterns).unwrap();
    }
    ids
}

/// The obstacles `ids` with this opacity, or switched off.
fn set_walls(scene: &mut Scene, ids: &[knorke_light2d::ObstacleId], opacity: f32, enabled: bool) {
    for &id in ids {
        let (_, o) = scene.obstacles.iter().find(|(i, _)| *i == id).unwrap();
        let common = ObstacleCommon {
            opacity,
            enabled,
            ..o.common
        };
        let outline = scene.obstacles.outline(id).to_vec();
        let shape = Shape::Polygon(&outline);
        scene
            .obstacles
            .set_shape(id, &ShapeDesc { common, shape })
            .unwrap();
    }
}

/// The whole demo with opaque obstacles: a lamp reaching farther than `MAX_REACH` has a row of
/// 2048 angles; one reaching farther than `WIDE_REACH` is drawn without shadow and warns
/// (`unshadowed_lights`).
#[test]
fn a_lamp_beyond_a_rows_reach_warns() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("hard", 44, "", |demo| {
            demo.with_frames(gpu, pack, &iso_night::settings, &mut |f| {
                drop_patterns(&mut f.scene);
                f.frame();
                let key = (f.renderer.plan().lights().rows.iter())
                    .find(|l| l.shadowed)
                    .map(|l| l.key)
                    .unwrap();
                let id = *(f.scene.light_ids.iter())
                    .find(|id| u64::from(id.index()) == key >> 2)
                    .unwrap();
                let lamp = |f: &DemoFrames<'_, '_>| {
                    *(f.renderer.plan().lights().rows.iter())
                        .find(|l| l.key == key)
                        .unwrap()
                };
                let mut wide = 0.0;
                for (radius, shadowed) in [(1000.0, false), (0.0, true)] {
                    // The second radius: 1200 mask texels (reach is radius / texel + 1 + span).
                    let r = if radius > 0.0 { radius } else { wide };
                    f.scene.change_light(id, |d| d.radius = r);
                    let (_, with) = f.frame();
                    let row = lamp(f);
                    wide = 1200.0 * 1000.0 / (row.reach - 1.0);
                    eprintln!(
                        "iso_night_hard, opaque, a lamp of radius {r} (reach {} texels), {} lights",
                        row.reach, with.lighting.lights_drawn
                    );
                    assert_eq!(row.shadowed, shadowed, "radius {r}");
                    assert!(row.wide(), "radius {r}");
                    let warned = with.warnings.contains(Warning::UnshadowedLights);
                    assert_eq!(warned, !shadowed, "radius {r}");
                }
            });
        });
    });
}

/// The demo as it is: its two pattern fences are runs of the row whose crossings the pattern pass
/// finds, so the lamps that reach them are row lights too; and runs of the moon's map, so the
/// moon takes its shadow from the map.
#[test]
fn pattern_obstacles_are_runs_of_the_row() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("hard", 44, "", |demo| {
            demo.with_frames(gpu, pack, &iso_night::settings, &mut |f| {
                let (_, with) = f.frame();
                let suns = with.lighting.suns;
                eprintln!("iso_night_hard: {suns} sun maps");
                assert_eq!(suns, 1, "the moon has a map");

                // Every shadowed lamp whose disc holds a corner of a fence triangle.
                let mut corners = Vec::new();
                for (id, o) in f.scene.obstacles.iter() {
                    if o.patterns.refs > 0 {
                        f.scene.obstacles.append_world_triangles(id, &mut corners);
                    }
                }
                let near: Vec<u32> = (f.scene.light_ids.iter())
                    .filter(|&&id| {
                        let d = f.scene.lights.get(id).unwrap();
                        d.cast_shadows
                            && corners
                                .iter()
                                .any(|c| (c.raw() - d.position.raw()).length() < d.radius)
                    })
                    .map(|id| id.index())
                    .collect();
                assert!(!near.is_empty(), "a lamp lights a fence");
                let row_keys: Vec<u32> = (f.renderer.plan().lights().rows.iter())
                    .map(|l| (l.key >> 2) as u32)
                    .collect();
                // Five lamps at the fences; the sun's disc holds them too, and one lamp is
                // out of the picture.
                assert!(
                    near.iter().filter(|k| row_keys.contains(k)).count() >= 5,
                    "lamps at the fences {near:?}, row lights {row_keys:?}"
                );
            });
        });
    });
}

/// The demo's fences as walls of 50 % opacity without pattern: a partly transparent run of the
/// row. Behind the walls the picture lies between the frames with opaque walls and without them.
#[test]
fn half_transparent_walls_are_runs_of_the_row() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("hard", 44, "", |demo| {
            demo.with_frames(gpu, pack, &iso_night::settings, &mut |f| {
                let walls = drop_patterns(&mut f.scene);
                let mut ends = Vec::new();
                for (opacity, enabled) in [(1.0, false), (1.0, true)] {
                    set_walls(&mut f.scene, &walls, opacity, enabled);
                    ends.push(f.frame().0);
                }
                set_walls(&mut f.scene, &walls, 0.5, true);
                let on = f.frame().0;

                // The walls' shadow: pixels the opaque walls darken by 48 or more (channel sum).
                let sum = |p: &[u8], i: usize| -> u32 {
                    p[4 * i..4 * i + 3].iter().map(|&c| u32::from(c)).sum()
                };
                let (lit, dark) = (&ends[0], &ends[1]);
                let shadow: Vec<usize> = (0..lit.len() / 4)
                    .filter(|&i| sum(lit, i) >= sum(dark, i) + 48)
                    .collect();
                let at = |p: &[u8]| {
                    let total: f64 = shadow.iter().map(|&i| f64::from(sum(p, i))).sum();
                    total / shadow.len() as f64
                };
                // How far the picture lies from opaque walls (0) to none (1).
                let way =
                    |p: &[u8], lit: &[u8], dark: &[u8]| (at(p) - at(dark)) / (at(lit) - at(dark));
                let row = way(&on, lit, dark);
                eprintln!(
                    "iso_night_hard, 50 % walls: in the walls' shadow ({} pixels) {row:.4} of the \
                     way from opaque walls to none",
                    shadow.len()
                );
                assert!(shadow.len() > 500, "{} pixels", shadow.len());
                // Measured 0.546 (Chiaro's light 0.619; the probe, "hart halb", finds the rows
                // closer to the exact march there).
                assert!(row > 0.2 && row < 0.8, "neither dark nor lit: {row}");
            });
        });
    });
}

/// The masked hall window alone (a point light under the hall's ceiling behind a window mask):
/// a row light with the mask's colour whose ray ends at the window, so the hall's own walls
/// cast nothing; hard (`hard`) and soft with infinitely tall walls (`plain`). The same with the
/// window tilted, and as a projector mask. `KNORKE_ROWS_OUT=<dir>` writes the window's light.
#[test]
fn the_hall_window_is_a_row_light() {
    with_chiaro_pack(|gpu, pack| {
        for (variant, hud) in [("hard", 44), ("plain", 45)] {
            iso_night::with_demo(variant, hud, "", |demo| {
                demo.with_frames(gpu, pack, &iso_night::settings, &mut |f| {
                    for &id in &f.scene.light_ids {
                        let window =
                            |d: &LightDesc| d.mask.is_some() && d.mask_mode == MaskMode::Window;
                        if f.scene.lights.get(id).is_ok_and(|d| !window(d)) {
                            f.scene.lights.destroy(id).unwrap();
                        }
                    }
                    let window = f.scene.light_ids[(f.scene.light_ids.iter())
                        .position(|&id| f.scene.lights.get(id).is_ok())
                        .unwrap()];
                    let original = f.scene.lights.get(window).unwrap().clone();
                    // As it is; tilted out of the wall (a window met over a stretch of the
                    // ground: the near edge of its rectangle begins the row); a projector mask
                    // instead, the lamp moved out of the hall through the window and shining
                    // away from it (the mask tints only; the hall's walls cast).
                    let (u, v) = (original.mask_u, original.mask_v);
                    let normal = Vec3::new(-u.y, u.x, 0.0).normalize_or_zero() * v.length();
                    let lamp = original.position.value;
                    let toward = Vec2::new(original.mask_center.x, original.mask_center.y) - lamp;
                    for case in ["fenster", "schraeg", "projektor"] {
                        f.scene.change_light(window, |d| {
                            *d = original.clone();
                            match case {
                                "schraeg" => d.mask_v = normal * 0.6 + v * 0.8,
                                "projektor" => {
                                    d.mask_mode = MaskMode::Projector;
                                    d.position = At::new(lamp + toward * 2.0);
                                    d.direction = Vec3::new(toward.x, toward.y, -toward.length())
                                        .normalize_or_zero();
                                }
                                _ => {}
                            }
                        });
                        let on = f.frame().0;
                        let masked = (f.renderer.plan().lights().rows.iter())
                            .filter(|l| l.mask.is_some())
                            .count();
                        // The demo's car lights come and go with the frame: the window's own
                        // light is the frame less the one without it.
                        f.scene.change_light(window, |d| d.enabled = false);
                        let dark = f.frame().0;
                        let on: Vec<u8> = (on.iter().zip(&dark))
                            .map(|(x, y)| x.saturating_sub(*y))
                            .collect();
                        let lit = 100.0
                            * (on.as_chunks::<4>().0.iter())
                                .filter(|p| p.iter().any(|&c| c > 0))
                                .count() as f64
                            / (on.len() / 4) as f64;
                        eprintln!(
                            "iso_night_{variant}, the hall window's light, {case}: lighting \
                             {lit:.2} % of the pixels"
                        );
                        if let Some(dir) = std::env::var_os("KNORKE_ROWS_OUT") {
                            let dir = std::path::PathBuf::from(dir);
                            std::fs::create_dir_all(&dir).unwrap();
                            let opaque: Vec<u8> = (on.as_chunks::<4>().0.iter())
                                .flat_map(|p| [p[0], p[1], p[2], 255])
                                .collect();
                            let file = dir.join(format!("{case}_{variant}.png"));
                            knorke_contract_tests::parity::save_png_rgba8(
                                &file, &opaque, OUT.0, OUT.1,
                            );
                        }
                        assert_eq!(masked, 1, "{case}: the hall window is a row light");
                        assert!(lit > 0.1, "{case}: the window lights {lit} % of the pixels");
                    }
                });
            });
        }
    });
}

/// A frame without surface and material buffer draws row lights (no Lambert term), the picture
/// of a new plan. (With the material buffer alone the row light is Chiaro's surface light:
/// `gpu_light2d_scenes`, `minimal_normal_map`, bit-identical.)
#[test]
fn a_frame_without_surface_buffer_draws_row_lights() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("hard", 44, "", |demo| {
            demo.with_frames(gpu, pack, &iso_night::settings, &mut |f| {
                f.skip = LayerSet::of(Layer::Surface).with(Layer::Material);
                let (on, record) = f.frame();
                let (max, ..) = diff(&on, &f.fresh_frame(pack));
                let rows = record.lighting.rows_built;
                eprintln!("iso_night_hard without surface buffer: {rows} rows, {max}/255");
                assert!(rows > 0, "row lights");
                assert_eq!(max, 0, "against a new plan");
            });
        });
    });
}

/// The application's sprite mask (E21) casts row shadows: its heights are in the height texture
/// the rows are built from. Squares of sprite all over the demo (`gpu_lightrow_probe`, "hart
/// normal", measures the same squares against the reference).
#[test]
fn a_sprite_mask_casts_row_shadows() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("hard", 44, "", |demo| {
            demo.with_frames(gpu, pack, &iso_night::settings, &mut |f| {
                let (plain, _) = f.frame();
                let sprite: Vec<u8> = (0..W * H)
                    .flat_map(|i| {
                        let (x, y) = (i % W, i / W);
                        let solid = x % 80 >= 34 && x % 80 < 46 && y % 60 >= 24 && y % 60 < 36;
                        [0, 0, 0, if solid { 255 } else { 0 }]
                    })
                    .collect();
                f.occlusion = Some(sampled_rgba8(f.gpu, W, H, &sprite));
                let (on, record) = f.frame();
                let rows = record.lighting.rows_built;
                let (cast, _, _) = diff(&on, &plain);
                eprintln!(
                    "iso_night_hard with a sprite mask: {rows} rows built; the sprites change up \
                     to {cast}/255"
                );
                assert!(rows > 0, "the sprite mask rebuilds rows");
                assert!(cast > 64, "the sprites cast shadows");
            });
        });
    });
}

/// A row is built when its lamp or the mask in its reach changed, and not otherwise; the moon's
/// map whole when the moon turns or gets a size, in part (the texels whose rays reach the change)
/// when an obstacle moves or the camera pans by whole mask texels, hard and soft. Whatever was kept, the picture is
/// that of a plan that builds every row and map.
#[test]
fn rows_are_built_only_when_lamp_or_mask_change() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("hard", 44, "", |demo| {
            demo.with_frames(gpu, pack, &iso_night::settings, &mut |f| {
                drop_patterns(&mut f.scene);
                // Rows built, sun maps built whole, sun maps updated, sun map texels drawn.
                let check = |f: &mut DemoFrames<'_, '_>, what: &str| {
                    let (pixels, record) = f.frame();
                    let l = record.lighting;
                    assert_eq!(l.suns, 1, "{what}: the moon has a map");
                    let (max, _, share) = diff(&pixels, &f.fresh_frame(pack));
                    eprintln!(
                        "{what}: {} rows built, sun map {} whole / {} in part, {} texels; \
                         against a new plan: largest difference {max}/255, {share:.3} % of the \
                         pixels",
                        l.rows_built, l.suns_built, l.suns_updated, l.sun_texels
                    );
                    assert_eq!(max, 0, "{what}");
                    (l.rows_built, l.suns_built, l.suns_updated, l.sun_texels)
                };
                let rest = (0, 0, 0, 0);

                let (all, suns, _, whole) = check(f, "first frame");
                assert!(all > 1 && suns == 1 && whole > 0);
                assert_eq!(check(f, "resting"), rest);
                let stats = f.renderer.plan().stats();
                assert_eq!((stats.rows_built, stats.suns_built), (u64::from(all), 1));

                // One lamp, a third of a world unit.
                let (id, at) = (f.scene.light_ids.iter())
                    .find_map(|&id| {
                        let d = f.scene.lights.get(id).unwrap();
                        let row = d.kind == LightType::Point && d.cast_shadows;
                        (row && d.profile.is_none() && d.mask.is_none())
                            .then_some((id, (d.position, d.height)))
                    })
                    .unwrap();
                let moved = At::new(at.0.raw() + Vec2::new(0.3, 0.0));
                f.scene.lights.set_position(id, moved, at.1).unwrap();
                assert_eq!(check(f, "one lamp moved"), (1, 0, 0, 0));
                assert_eq!(check(f, "resting"), rest);

                // The moon turns a little: its map, no row.
                let moon = *(f.scene.light_ids.iter())
                    .find(|&&id| f.scene.lights.get(id).unwrap().kind == LightType::Directional)
                    .unwrap();
                f.scene.change_light(moon, |d| {
                    d.direction = (d.direction + Vec3::new(0.02, 0.0, 0.0)).normalize_or_zero();
                });
                assert_eq!(check(f, "the moon turned"), (0, 1, 0, whole));
                assert_eq!(check(f, "resting"), rest);

                // One dynamic obstacle (a static one redraws the whole mask): the lamps that
                // reach its old or new place, the map texels whose rays reach it.
                let (id, pose) = (f.scene.obstacles.iter())
                    .filter(|(_, o)| o.common.dynamic)
                    .map(|(id, o)| (id, (o.common.position, o.common.rotation)))
                    .next()
                    .unwrap();
                let moved = At::new(pose.0.raw() + Vec2::new(0.5, 0.0));
                f.scene.obstacles.set_transform(id, moved, pose.1).unwrap();
                let (built, full, part, texels) = check(f, "one dynamic obstacle moved");
                assert!(built >= 1 && built < all, "{built} of {all}");
                assert!(
                    full == 0 && part == 1 && texels < whole / 4,
                    "{texels} of {whole}"
                );
                assert_eq!(check(f, "resting"), rest);

                // The camera, half a mask texel: the mask stays where it is in the world.
                f.projection.origin = At::new(f.projection.origin.raw() + Vec2::new(1.0, 0.0));
                assert_eq!(check(f, "camera moved half a mask texel"), rest);
                // One whole texel, then twelve by four: only lamps reaching over the mask's rim
                // see new mask; the map rebuilds the texels whose rays reach its rim strips.
                for (what, pan) in [
                    ("camera moved 1 mask texel", Vec2::new(1.0, 0.0)),
                    ("camera moved 12 x 4 mask texels", Vec2::new(23.0, 8.0)),
                ] {
                    f.projection.origin = At::new(f.projection.origin.raw() + pan);
                    let (built, full, part, texels) = check(f, what);
                    assert!(built < all, "{built} of {all}");
                    assert!(
                        full == 0 && part == 1 && texels < whole / 2,
                        "{texels} of {whole}"
                    );
                    assert_eq!(check(f, "resting"), rest);
                }
                // A row sees nothing beyond the mask's rim: the pans above exposed and pushed out
                // no obstacle within a lamp's reach, so they built no row. A hundred texels bring
                // buildings into the reach of lamps that reached over the rim.
                f.projection.origin = At::new(f.projection.origin.raw() + Vec2::new(200.0, 0.0));
                let (built, ..) = check(f, "camera moved 100 mask texels");
                assert!(built > 0 && built < all, "{built} of {all}");
                assert_eq!(check(f, "resting"), rest);

                // A size (plain.txt's moon): the map gains its second target, built whole; then
                // a pan and the obstacle again update it in part.
                f.scene.change_light(moon, |d| d.source_radius = 1.0);
                assert_eq!(check(f, "the moon got a size"), (0, 1, 0, whole));
                assert_eq!(check(f, "resting"), rest);
                f.projection.origin = At::new(f.projection.origin.raw() + Vec2::new(2.0, 0.0));
                let (_, full, part, texels) = check(f, "soft moon, camera moved 1 mask texel");
                assert!(
                    full == 0 && part == 1 && texels < whole / 2,
                    "{texels} of {whole}"
                );
                assert_eq!(check(f, "resting"), rest);
                f.scene.obstacles.set_transform(id, pose.0, pose.1).unwrap();
                let (_, full, part, texels) = check(f, "soft moon, the obstacle moved back");
                assert!(
                    full == 0 && part == 1 && texels < whole / 4,
                    "{texels} of {whole}"
                );
                assert_eq!(check(f, "resting"), rest);

                f.renderer.plan_mut().invalidate();
                assert_eq!(check(f, "invalidated"), (all, 1, 0, whole));
            });
        });
    });
}

/// Every variant of the demo as it is, also panned by a mask texel: the medium stage reads its
/// shaft rows and sun heights from the rows and maps, and each picture is that of a new plan.
/// With `KNORKE_PROBE_FRAMES` set prints the frame times, resting and scrolling (5 runs, median).
#[test]
fn every_variant_draws_the_picture_of_a_new_plan() {
    let frames: usize = std::env::var("KNORKE_PROBE_FRAMES").map_or(0, |v| v.parse().unwrap());
    let variants = [
        ("plain", 45, ""),
        ("hard", 44, ""),
        ("t5", 44, ""),
        ("cutaway", 44, "_cutaway"),
        ("grade_crt", 44, ""),
    ];
    with_chiaro_pack(|gpu, pack| {
        for (variant, hud, suffix) in variants {
            iso_night::with_demo(variant, hud, suffix, |demo| {
                demo.with_frames(gpu, pack, &iso_night::settings, &mut |f| {
                    for i in 0..2 {
                        if i == 1 {
                            // Panned by a mask texel.
                            let o = f.projection.origin.raw();
                            f.projection.origin = At::new(o + Vec2::new(2.0, 0.0));
                        }
                        let (pixels, record) = f.frame();
                        let (max, ..) = diff(&pixels, &f.fresh_frame(pack));
                        let (l, m) = (record.lighting, record.medium);
                        eprintln!(
                            "iso_night_{variant} frame {i}: {} lights, {} sun maps, {} shaft rows, \
                             {} sun blocks; against a new plan {max}/255",
                            l.lights_drawn, l.suns, m.shaft_rows, m.sun_blocks
                        );
                        assert_eq!(max, 0, "{variant} frame {i}");
                    }
                    f.projection = demo_projection();
                    if frames > 0 {
                        let rest = f.time_frames(frames, 5, false);
                        let scroll = f.time_frames(frames, 5, true);
                        eprintln!(
                            "iso_night_{variant}: {frames} frames, median us per frame: resting \
                             {rest:.1}, scrolling {scroll:.1}"
                        );
                    }
                });
            });
        }
    });
}

/// The demo's camera (`iso_night::with_demo`).
fn demo_projection() -> knorke_light2d::Projection {
    knorke_light2d::Projection::iso(At::new(Vec2::new(18.0, 14.0)), 10.0, 1.0, W, H)
}

/// The demo as it is: its moon takes its shadow from a map with the two pattern fences in it
/// (the map keeps their runs), built whole once, kept while nothing changes, updated in part when
/// a half-transparent box appears in the picture or the camera pans by a mask texel, and right
/// after a pattern fence moved. Built whole when a fence loses its pattern (the other's record index moves) and
/// when the moon gets a size. A tile grid with a pattern in the picture is a run of the map too, and
/// so is it moved far outside. Each picture is that of a plan that builds every row and map.
#[test]
fn pattern_and_partial_obstacles_keep_the_suns_map() {
    with_chiaro_pack(|gpu, pack| {
        iso_night::with_demo("hard", 44, "", |demo| {
            demo.with_frames(gpu, pack, &iso_night::settings, &mut |f| {
                // Sun maps, built whole, updated, texels drawn.
                let check = |f: &mut DemoFrames<'_, '_>, what: &str| {
                    let (pixels, record) = f.frame();
                    let l = record.lighting;
                    let (max, _, share) = diff(&pixels, &f.fresh_frame(pack));
                    eprintln!(
                        "{what}: {} sun maps, built {} whole / {} in part, {} texels; against a \
                         new plan: largest difference {max}/255, {share:.3} % of the pixels",
                        l.suns, l.suns_built, l.suns_updated, l.sun_texels
                    );
                    assert_eq!(max, 0, "{what}");
                    (l.suns, l.suns_built, l.suns_updated, l.sun_texels)
                };
                let rest = (1, 0, 0, 0);
                let (suns, full, _, whole) = check(f, "the demo");
                assert_eq!(
                    (suns, full),
                    (1, 1),
                    "the moon has a map with the fences in it"
                );
                assert_eq!(check(f, "resting"), rest);
                let part = |f: &mut DemoFrames<'_, '_>, what: &str, most: u64| {
                    let (suns, full, part, texels) = check(f, what);
                    assert!(
                        suns == 1 && full == 0 && part == 1 && texels < most,
                        "{what}: {texels} of {whole}"
                    );
                    assert_eq!(check(f, "resting"), rest);
                };

                let inside = f.scene.obstacles.iter().next().unwrap().1.common.position;
                let mut glass = ShapeDesc::rect(1.0, 1.0);
                glass.common.height = 2.0;
                glass.common.opacity = 0.5;
                glass.common.dynamic = true;
                glass.common.position = inside;
                f.scene.obstacles.create_shape(&glass).unwrap();
                part(f, "a half-transparent box in the picture", whole / 4);
                let fences: Vec<_> = (f.scene.obstacles.iter())
                    .filter(|(_, o)| o.patterns.refs > 0)
                    .map(|(id, o)| (id, o.common.position, o.patterns.pattern))
                    .collect();
                assert_eq!(fences.len(), 2, "the demo's pattern fences");
                let (fence, at, pattern) = fences[1];
                let moved = At::new(at.raw() + Vec2::new(0.5, 0.0));
                f.scene.obstacles.set_transform(fence, moved, 0.0).unwrap();
                assert_eq!(check(f, "a pattern fence moved").0, 1);
                assert_eq!(check(f, "resting"), rest);
                f.projection.origin = At::new(f.projection.origin.raw() + Vec2::new(2.0, 0.0));
                part(f, "camera moved 1 mask texel", whole / 2);

                f.scene
                    .obstacles
                    .set_pattern(fences[0].0, 0, &f.scene.patterns)
                    .unwrap();
                let (suns, full, _, _) = check(f, "the first fence lost its pattern");
                assert_eq!(
                    (suns, full),
                    (1, 1),
                    "the second fence's record index moved"
                );
                assert_eq!(check(f, "resting"), rest);
                f.scene
                    .obstacles
                    .set_pattern(fences[0].0, pattern, &f.scene.patterns)
                    .unwrap();
                let moon = *(f.scene.light_ids.iter())
                    .find(|&&id| f.scene.lights.get(id).unwrap().kind == LightType::Directional)
                    .unwrap();
                f.scene.change_light(moon, |d| d.source_radius = 1.0);
                assert_eq!(check(f, "a soft moon").1, 1);
                assert_eq!(check(f, "resting"), rest);
                f.projection.origin = At::new(f.projection.origin.raw() + Vec2::new(2.0, 0.0));
                part(f, "soft moon, camera moved 1 mask texel", whole / 2);

                let solid = [1u8; 4];
                let mut grid = TileGridDesc::new(2, 2, &solid);
                grid.common.height = 2.0;
                grid.common.position = inside;
                let id = f.scene.obstacles.create_tile_grid(&grid).unwrap();
                f.scene
                    .obstacles
                    .set_pattern(id, pattern, &f.scene.patterns)
                    .unwrap();
                assert_eq!(check(f, "a tile grid with a pattern in the picture").0, 1);
                let far = At::new(Vec2::new(-500.0, -500.0));
                f.scene.obstacles.set_transform(id, far, 0.0).unwrap();
                assert_eq!(check(f, "the grid far outside").0, 1);
            });
        });
    });
}
