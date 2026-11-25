//! The Chiaro demo scene `iso_night` (examples/iso_night_demo of chiaro @ 9bc5b9b, zlib license)
//! at `--time 5` in its two variants without media, end to end through [`Layers`] and
//! [`Light2dRenderer`], compared with the P3 goldens `tests/golden/chiaro/iso_night_<variant>.png`
//! under `ParityBudget::STRICT` outside the HUD rect of `manifest.toml`:
//! - `plain` (`--no-heights --no-surface --no-haze --no-fog --no-moon-shadows`): infinitely tall
//!   obstacles, flat ground, no moon shadows, soft shadows;
//! - `hard` (`--hard --no-haze`): real heights, surface heights and normals, hard shadows.
//!
//! Both have point, spot, line, rect and directional lights, light profiles (hat and LED street
//! lamps), the hall window mask, animations (fire, beacon, flashes, envelope), the car's frame
//! lights, the hall's ambient zone, dynamic obstacles, the two pattern obstacles, glow and bloom.
//! The haze is off and nothing is drawn into the medium layer, so Chiaro runs no medium stage
//! (`MediumStage.hpp`: only with haze density > 0 or a medium layer; `Layers::Fill` leaves out a
//! medium layer nothing drew into).
//!
//! The demo paints its layers with `SDL_Renderer`, so the colour and glow layers come from fixtures
//! (the same in both variants: no toggle changes a colour) dumped by
//! `tests/golden/chiaro/iso_night/iso_night_ref.cpp`, which compiles the demo's `Scene.cpp`
//! unchanged, renders a frame byte-identical to the golden outside the HUD, and writes the resolved
//! scene of each variant as `<variant>.txt` (format below). Every store is built here from that
//! text through Knorke's public API.
//!
//! The demo's HUD panel reaches past the manifest's HUD rect in `plain` (its longest line, the
//! toggles, is one character longer than in `hard`): it is drawn as the overlay, without its text,
//! which lies inside the rect.
//!
//! The test plays the host (ADR 0002). It skips without a Vulkan device or the shader pack of
//! `cargo xtask shaders` (a failure with `KNORKE_REQUIRE_GPU=1`). `KNORKE_SCENE_OUT=<dir>` writes
//! the frames and amplified diff images there.
//!
//! Scene text: one item per line, fields separated by spaces, handles as 1-based indices into the
//! items of their kind (0 = none), floats as written by `%.9g` (exact for f32):
//! - `profile` peak peak_c90 spread nadir cutoff cutoff_fade gain0..3 (`CHI_ProfileShape`);
//! - `animation` type start period minimum low_rgb duty count attack hold decay;
//! - `mask window_hall`; `sheet w h` with `row` lines of alpha bytes (hex; RGB is white);
//!   `pattern` x y w h repeat_width repeat_height;
//! - `light` / `frame_light`: the fields of `CHI_LightDesc` in declaration order;
//! - `sample` light-of-the-animated-group intensity color_rgb rotation (Chiaro's sample at t = 5);
//! - `shape` type flags x y rotation height base opacity radius width depth pattern;
//! - `zone` type flags ambient_rgb soft_edge priority height base x y rotation radius width depth.
#![cfg(feature = "gpu")]

#[path = "common/gpu.rs"]
mod scene_common;

use knorke_contract_tests::parity::{ParityBudget, Rect, compare_rgba8};
use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
};
use knorke_gpu::test_support::{
    golden_bytes, read_texture, sampled_rgba8, texture, with_chiaro_pack,
};
use knorke_gpu::{CommandBuffer, Texture};
use knorke_integration::layers::{Layer, LayerSet, Layers};
use knorke_integration::light2d::{Light2dRenderer, Light2dSettings};
use knorke_light2d::ambient_zone::{AmbientZoneParams, ZoneShape, make_stored_zone};
use knorke_light2d::ambient_zone_store::AmbientZoneStore;
use knorke_light2d::animation::AnimationType;
use knorke_light2d::light::{Height, LightDesc, LightType, MaskMode};
use knorke_light2d::mask::{MaskPreset, MaskShape};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::pass_plan::SceneStores;
use knorke_light2d::pattern_store::{PatternDesc, Sheet, SheetFormat, raw_id};
use knorke_light2d::profile::{ProfileShape, build_shape_profile};
use knorke_light2d::profile_store::ProfileStore;
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::{GlowColor, LightSettings, Quality, QualitySettings, Resolutions};
use knorke_light2d::{
    AnimationDesc, AnimationStore, LightStore, MaskStore, ObstacleStore, PatternStore,
    StoreResources,
};
use knorke_math::{At, Vec2, Vec3};
use knorke_post::output_rect::ScaleMode;
use knorke_post::settings::{Filter, PostSettings};

const W: u32 = 640;
const H: u32 = 360;
const OUT: (u32, u32) = (1280, 720);
const TIME: f64 = 5.0;
/// `ignore_rects` of both variants in `tests/golden/chiaro/manifest.toml`.
const HUD: Rect = Rect {
    x: 0,
    y: 0,
    w: 372,
    h: 152,
};

/// The lines of a scene text, split into fields.
fn items(text: &str) -> impl Iterator<Item = (&str, Vec<&str>)> {
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut f = l.split(' ');
            (f.next().unwrap(), f.collect())
        })
}

fn num<T: std::str::FromStr>(s: &str) -> T
where
    T::Err: std::fmt::Debug,
{
    s.parse().unwrap()
}

fn vec3(f: &[&str]) -> Vec3 {
    Vec3::new(num(f[0]), num(f[1]), num(f[2]))
}

/// The 1-based index field `s` into `ids` (0 = none).
fn handle<T: Copy>(ids: &[T], s: &str) -> Option<T> {
    num::<usize>(s).checked_sub(1).map(|i| ids[i])
}

/// The stores of one variant, built from its scene text through the public API.
struct Scene {
    profiles: ProfileStore,
    animations: AnimationStore,
    masks: MaskStore,
    patterns: PatternStore,
    lights: LightStore,
    obstacles: ObstacleStore,
    zones: AmbientZoneStore,
}

impl Scene {
    fn parse(text: &str) -> Self {
        let mut s = Self {
            profiles: ProfileStore::new(),
            animations: AnimationStore::new(),
            masks: MaskStore::new(),
            patterns: PatternStore::new(),
            lights: LightStore::new(),
            obstacles: ObstacleStore::new(),
            zones: AmbientZoneStore::new(),
        };
        let (mut profiles, mut animations, mut masks) = (Vec::new(), Vec::new(), Vec::new());
        let (mut sheet, mut pattern_descs, mut patterns) = (Vec::new(), Vec::new(), Vec::new());
        let (mut frame_lights, mut samples) = (Vec::new(), Vec::new());
        let mut animated = Vec::new(); // (light index, desc) of every animated light
        for (kind, f) in items(text) {
            match kind {
                "profile" => {
                    let v: Vec<f32> = f.iter().map(|s| num(s)).collect();
                    let shape = ProfileShape {
                        peak_angle: v[0],
                        peak_angle_c90: v[1],
                        spread_angle: v[2],
                        nadir: v[3],
                        cutoff_angle: v[4],
                        cutoff_fade_angle: v[5],
                        gain: [v[6], v[7], v[8], v[9]],
                    };
                    let values = build_shape_profile(&shape).unwrap();
                    profiles.push(s.profiles.create(values).unwrap());
                }
                "animation" => {
                    let kind = match f[0] {
                        "0" => AnimationType::Pulse,
                        "1" => AnimationType::Flash,
                        "2" => AnimationType::Beacon,
                        "3" => AnimationType::Fire,
                        "4" => AnimationType::Envelope,
                        t => panic!("animation type {t}"),
                    };
                    let d = AnimationDesc {
                        start: num(f[1]),
                        period: num(f[2]),
                        minimum: num(f[3]),
                        low_color: vec3(&f[4..7]),
                        duty: num(f[7]),
                        count: num(f[8]),
                        attack: num(f[9]),
                        hold: num(f[10]),
                        decay: num(f[11]),
                        ..AnimationDesc::new(kind)
                    };
                    animations.push(s.animations.create(&d).unwrap());
                }
                "mask" => {
                    assert_eq!(f, ["window_hall"]);
                    let mut texels = Vec::new();
                    MaskShape::preset(MaskPreset::WindowHall).build(&mut texels);
                    masks.push(s.masks.create(texels).unwrap());
                }
                "sheet" => assert_eq!(f, ["20", "16"]),
                "row" => {
                    for k in 0..f[0].len() / 2 {
                        let alpha = u8::from_str_radix(&f[0][2 * k..2 * k + 2], 16).unwrap();
                        sheet.extend_from_slice(&[255, 255, 255, alpha]);
                    }
                }
                "pattern" => {
                    let r: Vec<u32> = f[..4].iter().map(|s| num(s)).collect();
                    pattern_descs.push(PatternDesc {
                        repeat_width: num(f[4]),
                        repeat_height: num(f[5]),
                        ..PatternDesc::new(r[0], r[1], r[2], r[3])
                    });
                }
                "light" | "frame_light" => {
                    let d = light(&f, &profiles, &animations, &masks);
                    let res =
                        StoreResources::new(&s.animations, &s.masks).with_profiles(&s.profiles);
                    if kind == "frame_light" {
                        frame_lights.push(d);
                    } else {
                        if d.animation.is_some() {
                            animated.push((s.lights.len(), d.clone()));
                        }
                        s.lights.create(&d, &res).unwrap();
                    }
                }
                "sample" => samples.push(f),
                "shape" => {
                    if patterns.is_empty() && !pattern_descs.is_empty() {
                        let sheet = Sheet {
                            width: 20,
                            height: 16,
                            pitch: 80,
                            format: SheetFormat::Rgba8,
                            pixels: &sheet,
                        };
                        s.patterns
                            .create(&sheet, &pattern_descs, &mut patterns)
                            .unwrap();
                    }
                    let (desc, pattern) = shape(&f);
                    let id = s.obstacles.create_shape(&desc).unwrap();
                    if let Some(p) = handle(&patterns, pattern) {
                        s.obstacles.set_pattern(id, raw_id(p), &s.patterns).unwrap();
                    }
                }
                "zone" => {
                    let zone = zone(&f);
                    s.zones.create(make_stored_zone(&zone).unwrap()).unwrap();
                }
                _ => panic!("unknown scene item {kind}"),
            }
        }
        let res = StoreResources::new(&s.animations, &s.masks).with_profiles(&s.profiles);
        s.lights
            .set_frame_lights(frame_lights.as_slice(), &res)
            .unwrap();
        // The copied animation descs sample as Chiaro's at t = 5 (seed: the light's slot index).
        assert_eq!(samples.len(), animated.len());
        for (f, (seed, d)) in samples.iter().zip(&animated) {
            let a = s
                .animations
                .sample(d.animation, TIME, d.animation_offset, *seed as u32);
            let want = [num(f[1]), num(f[2]), num(f[3]), num(f[4]), num::<f32>(f[5])];
            let got = [a.intensity, a.color.x, a.color.y, a.color.z, a.rotation];
            assert_eq!(got, want, "animation sample of animated light {}", f[0]);
        }
        s
    }

    fn stores(&self) -> SceneStores<'_> {
        SceneStores {
            zones: Some(&self.zones),
            profiles: Some(&self.profiles),
            masks: Some(&self.masks),
            animations: Some(&self.animations),
            patterns: Some(&self.patterns),
            ..SceneStores::new(&self.lights, &self.obstacles)
        }
    }
}

/// A `CHI_LightDesc` line as a [`LightDesc`].
fn light(
    f: &[&str],
    profiles: &[knorke_light2d::light::ProfileId],
    animations: &[knorke_light2d::AnimationId],
    masks: &[knorke_light2d::MaskId],
) -> LightDesc {
    let kind = match f[0] {
        "0" => LightType::Point,
        "1" => LightType::Spot,
        "2" => LightType::Directional,
        "3" => LightType::Line,
        "4" => LightType::Rect,
        t => panic!("light type {t}"),
    };
    // CHI_LIGHT_NO_SHADOWS, _DISABLED, _TWO_SIDED, _SOFT_NEAR_FIELD; the scene uses no other bit.
    let flags: u32 = num(f[1]);
    assert_eq!(flags & !0xF, 0, "unmapped light flags {flags:#x}");
    let v = |i: usize| -> f32 { num(f[i]) };
    let mut d = LightDesc::new(kind);
    d.cast_shadows = flags & 1 == 0;
    d.enabled = flags & 2 == 0;
    d.two_sided = flags & 4 != 0;
    d.soft_near_field = flags & 8 != 0;
    d.position = At::new(Vec2::new(v(2), v(3)));
    // CHI_HEIGHT_AUTO is -1.
    d.height = if v(4) < 0.0 {
        Height::Auto
    } else {
        Height::At(v(4))
    };
    d.color = vec3(&f[5..8]);
    d.intensity = v(8);
    d.specular = v(9);
    d.haze = v(10);
    d.radius = v(11);
    d.falloff = v(12);
    d.source_radius = v(13);
    d.flicker = v(14);
    d.flicker_speed = v(15);
    d.direction = vec3(&f[16..19]);
    d.inner_angle = v(19);
    d.outer_angle = v(20);
    d.axis = vec3(&f[21..24]);
    d.half_width = v(24);
    d.profile = handle(profiles, f[25]);
    d.profile_rotation = v(26);
    d.animation = handle(animations, f[27]);
    d.animation_offset = v(28);
    d.mask = handle(masks, f[29]);
    d.mask_mode = if f[30] == "1" {
        MaskMode::Window
    } else {
        MaskMode::Projector
    };
    // CHI_MASK_ANGLE_AUTO is 0.
    d.mask_angle = Some(v(31)).filter(|&a| a != 0.0);
    d.mask_rotation = v(32);
    d.mask_center = vec3(&f[33..36]);
    d.mask_u = vec3(&f[36..39]);
    d.mask_v = vec3(&f[39..42]);
    d
}

/// A `CHI_ShapeDesc` line as a [`ShapeDesc`] and its pattern index field.
fn shape<'f>(f: &[&'f str]) -> (ShapeDesc<'static>, &'f str) {
    // CHI_OBSTACLE_DYNAMIC, _DISABLED, _FOOTPRINT_SHADOW.
    let flags: u32 = num(f[1]);
    assert_eq!(flags & !0x7, 0, "unmapped obstacle flags {flags:#x}");
    let v = |i: usize| -> f32 { num(f[i]) };
    let common = ObstacleCommon {
        position: At::new(Vec2::new(v(2), v(3))),
        rotation: v(4),
        height: v(5),
        base_height: v(6),
        opacity: v(7),
        dynamic: flags & 1 != 0,
        enabled: flags & 2 == 0,
        shadow_footprint: flags & 4 != 0,
    };
    let shape = match f[0] {
        "1" => Shape::Circle { radius: v(8) },
        "2" => Shape::Box {
            width: v(9),
            depth: v(10),
        },
        t => panic!("shape type {t}"),
    };
    (ShapeDesc { common, shape }, f[11])
}

/// A `CHI_AmbientZoneDesc` line (a box) as [`AmbientZoneParams`].
fn zone(f: &[&str]) -> AmbientZoneParams {
    assert_eq!(f[0], "2", "only box zones are mapped");
    // CHI_AMBIENT_ZONE_DISABLED, _RELATIVE.
    let flags: u32 = num(f[1]);
    assert_eq!(flags & !0x3, 0, "unmapped zone flags {flags:#x}");
    let v = |i: usize| -> f32 { num(f[i]) };
    AmbientZoneParams {
        shape: ZoneShape::Box,
        disabled: flags & 1 != 0,
        relative: flags & 2 != 0,
        ambient: [v(2), v(3), v(4)],
        soft_edge: v(5),
        priority: num(f[6]),
        // CHI_HEIGHT_INFINITE is 0: no top.
        height: Some(v(7)).filter(|&h| h != 0.0),
        base_height: v(8),
        x: v(9),
        y: v(10),
        rotation: v(11),
        radius: v(12),
        width: v(13),
        depth: v(14),
        ..AmbientZoneParams::default()
    }
}

/// The settings of the demo for these variants: `NightScene::Create` (ambient, height step, glow
/// scale and scene colour, bloom 0.8 with radius 12) and `main.cpp` (integer nearest scaling,
/// custom quality with light 1 and shadow 0.5, the medium keeping the preset's 0.5; haze off).
fn settings() -> Light2dSettings {
    let mut s = Light2dSettings {
        light: LightSettings {
            ambient: Vec3::new(0.03, 0.035, 0.05),
            quality: QualitySettings {
                quality: Quality::Custom(Resolutions {
                    light: 1.0,
                    shadow: 0.5,
                    medium: 0.5,
                }),
                ..QualitySettings::default()
            },
            surface_height_step: 0.1,
            ..LightSettings::default()
        },
        post: PostSettings::default(),
    };
    s.light.glow.scale = 2.5;
    s.light.glow.color = GlowColor::Scene;
    s.light.haze.density = 0.0;
    s.light.haze.color = Vec3::new(0.8, 0.85, 1.0);
    s.post.output.scale_mode = ScaleMode::Integer;
    s.post.output.filter = Filter::Nearest;
    s.post.bloom.enabled = true;
    s.post.bloom.intensity = 0.8;
    s.post.bloom.radius = 12.0;
    s
}

/// The demo's HUD panel (`demo::DrawHudPanel` at scale 1): black at alpha 170 behind 12 lines of
/// 8-pixel characters, `longest` the length of the longest one. Black, so straight alpha is
/// premultiplied already. The text lies inside the HUD rect and is left out.
fn hud_panel(longest: u32) -> Vec<u8> {
    let (w, h) = (12 + 8 * longest, 10 + 11 * 12);
    let mut px = vec![0u8; (OUT.0 * OUT.1 * 4) as usize];
    for y in 6..6 + h {
        for x in 6..6 + w {
            px[((y * OUT.0 + x) * 4 + 3) as usize] = 170;
        }
    }
    px
}

/// Renders `variant` with the layers `kinds` and compares it with its golden; `hud_longest` is
/// the length of the HUD's longest line.
fn check(variant: &str, kinds: LayerSet, hud_longest: u32) {
    with_chiaro_pack(|gpu, pack| {
        let text = golden_bytes("iso_night", &format!("{variant}.txt"));
        let text = String::from_utf8(text).expect("the scene text is UTF-8");
        let scene = Scene::parse(&text);
        let projection = Projection::iso(At::new(Vec2::new(18.0, 14.0)), 10.0, 1.0, W, H);
        let mut layers = Layers::new(gpu, W, H, kinds).unwrap();
        let mut renderer = Light2dRenderer::new(gpu, pack.to_vec()).unwrap();
        let target: Texture<'_> = texture(
            gpu,
            OUT,
            SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
            SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
            None,
        );
        let overlay = sampled_rgba8(gpu, OUT.0, OUT.1, &hud_panel(hud_longest));

        let fixture = |layer: &str| scene_common::fixture("iso_night", layer, (W, H));
        let (color, glow) = (fixture("scene"), fixture("glow"));
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        drop(layers.clear_pass(&mut cmd, LayerSet::ALL).unwrap());
        let _uploads = scene_common::upload_layers(
            gpu,
            &mut cmd,
            &mut layers,
            &[(Layer::Scene, &color), (Layer::Glow, &glow)],
        );
        cmd.submit_and_wait().unwrap();

        renderer.note_layers(&mut layers);
        let settings = settings();
        let mut frame = layers.frame(target.texture_ref(), &projection, LayerSet::NONE);
        frame.overlay = Some(overlay.texture_ref());
        frame.time = TIME;
        let prepared = renderer
            .prepare(&frame, &scene.stores(), &settings)
            .unwrap();
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        renderer
            .record(&mut cmd, &frame, &settings, prepared)
            .unwrap();
        cmd.submit_and_wait().unwrap();
        let pixels = read_texture(gpu, target.texture_ref());

        let golden = scene_common::fixture(".", &format!("iso_night_{variant}"), OUT);
        let strict = compare_rgba8(
            &pixels,
            &golden,
            OUT.0,
            OUT.1,
            &[HUD],
            &ParityBudget::STRICT,
        )
        .unwrap();
        strict.print(&format!("iso_night_{variant}"), "STRICT");
        if let Some(dir) = scene_common::scene_out() {
            scene_common::save_with_diff(
                &dir,
                &format!("iso_night_{variant}"),
                &pixels,
                &golden,
                OUT,
            );
        }
        assert!(
            strict.within(&ParityBudget::STRICT),
            "STRICT parity missed: {strict:?}"
        );
    });
}

#[test]
fn iso_night_plain_matches_chiaro() {
    // The toggles line "M moon shadows off  F soft  P running  Z haze" is the longest.
    check("plain", LayerSet::of(Layer::Surface).with(Layer::Glow), 45);
}
