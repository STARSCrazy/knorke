//! The Chiaro demo scenes as resolved scene texts (`iso_night`, `smoke_blast`): parsed into
//! Knorke's stores through the public API, drawn into [`Layers`] by the helpers and lit by
//! [`Light2dRenderer`], compared with the P3 goldens under `ParityBudget::STRICT`. Used by
//! `gpu_iso_night.rs` and `gpu_smoke_blast.rs` (`#[path]`, next to `scene_common`).
//!
//! The demos paint their colour and glow layers with `SDL_Renderer`, so those come as PNG
//! fixtures dumped by the C++ reference programs (`tests/golden/chiaro/<demo>/*_ref.cpp`); from
//! the helper-drawn layers on (surface geometry and standing sprites, the medium clouds) and the
//! whole light Knorke computes. The helper-drawn surface and medium layers must equal Chiaro's
//! dumps byte for byte.
//!
//! Scene text: one item per line, fields separated by spaces, handles as 1-based indices into the
//! items of their kind (0 = none), floats as written by `%.9g` (exact for f32):
//! - `profile` peak peak_c90 spread nadir cutoff cutoff_fade gain0..3 (`CHI_ProfileShape`);
//! - `animation` type start period minimum low_rgb duty count attack hold decay;
//! - `mask window_hall`; `sheet w h` with `row` lines of alpha bytes (hex; RGB is white);
//!   `pattern` x y w h repeat_width repeat_height;
//! - `light` / `frame_light`: the fields of `CHI_LightDesc` in declaration order;
//! - `sample` light-of-the-animated-group intensity color_rgb rotation (Chiaro's sample at the
//!   scene's time);
//! - `shape` type flags x y rotation height base opacity radius width depth pattern;
//! - `zone` type flags ambient_rgb soft_edge priority height base x y rotation radius width depth;
//! - the surface pass in call order: `figure` w h world_height with `figure_row` lines of alpha
//!   bytes, `figure_draw` x y w h, `surface_box` x y width depth height, `surface_wall` a_xyz b_xyz
//!   top, `surface_floor` x,y pairs and height (drawn by [`SpriteBatch`] and [`SurfaceBatch`]);
//! - `medium_draw` x y w h density bottom top medium puff: one `CHI_MediumDraw` in call order
//!   (`AddPuff`/`AddFogPatch` of `SmokePuffs.hpp`), its shape the puff of that 1-based index into
//!   `tests/golden/chiaro/medium/puffs.png` (drawn by [`MediumBatch`]);
//! - `output` saturation contrast tint_rgb vignette radius softness vignette_rgb dither and `crt`
//!   flags curvature corner scanlines period mask pitch brightness (a graded frame);
//! - `explosion` x y age power reach and `shockwave` x y radius width strength light_rgb
//!   intensity: Chiaro's `CHI_ExplosionShockwave` of these arguments.
#![allow(dead_code)]

use knorke_contract_tests::parity::{self, ParityBudget, Rect, compare_rgba8};
use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
};
use knorke_gpu::test_support::{
    golden_bytes, read_texture, sampled_rgba8, texture, with_chiaro_pack,
};
use knorke_gpu::{CommandBuffer, Gpu, Samplers, ShaderLibrary, Texture, TextureRef};
use knorke_integration::layers::{Layer, LayerSet, Layers};
use knorke_integration::light2d::{Light2dRenderer, Light2dSettings, post_projection};
use knorke_integration::medium::{MediumBatch, MediumItem};
use knorke_integration::sprites::{Sprite, SpriteBatch, SpriteDesc, SpriteDraw};
use knorke_integration::surface::SurfaceBatch;
use knorke_light2d::ambient_zone::{AmbientZoneParams, ZoneShape, make_stored_zone};
use knorke_light2d::ambient_zone_store::AmbientZoneStore;
use knorke_light2d::animation::AnimationType;
use knorke_light2d::light::{Height, LightDesc, LightId, LightType, MaskMode};
use knorke_light2d::mask::{MaskPreset, MaskShape};
use knorke_light2d::medium_draw::{MediumDraw, smoke_puff};
use knorke_light2d::obstacle::{ObstacleCommon, Shape, ShapeDesc};
use knorke_light2d::pass_plan::{PassRecord, SceneStores};
use knorke_light2d::pattern_store::{PatternDesc, Sheet, SheetFormat, raw_id};
use knorke_light2d::profile::{ProfileShape, build_shape_profile};
use knorke_light2d::profile_store::ProfileStore;
use knorke_light2d::projection::Projection;
use knorke_light2d::surface_geometry::{SurfaceBox, SurfaceGeometry};
use knorke_light2d::{
    AnimationDesc, AnimationStore, LightStore, MaskStore, ObstacleStore, PatternStore,
    StoreResources,
};
use knorke_math::{At, Vec2, Vec3};
use knorke_post::shockwave::{
    Projection as PostProjection, Shockwave, ShockwaveFrame, ShockwaveList, collect_shockwaves,
    make_explosion_shockwave,
};

use crate::scene_common;

/// Scene size of both demos.
pub(crate) const W: u32 = 640;
pub(crate) const H: u32 = 360;
/// The `--screenshot` size.
pub(crate) const OUT: (u32, u32) = (1280, 720);
/// `kHeightStep` of both demos.
pub(crate) const HEIGHT_STEP: f32 = 0.1;

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

/// The stores of one scene, built from its text through the public API.
pub(crate) struct Scene {
    profiles: ProfileStore,
    animations: AnimationStore,
    masks: MaskStore,
    pub(crate) patterns: PatternStore,
    pub(crate) lights: LightStore,
    /// The lights in the order of the text.
    pub(crate) light_ids: Vec<LightId>,
    pub(crate) obstacles: ObstacleStore,
    zones: AmbientZoneStore,
    /// The surface pass: the helper geometry in call order.
    surface: SurfaceGeometry,
    /// Whether the text has a surface pass (its layer is then compared with Chiaro's).
    surface_pass: bool,
    /// The figure sprite: width, height, world height and its RGBA (white where opaque).
    figure: Option<(u32, u32, f32, Vec<u8>)>,
    /// Where the surface pass draws the figure's surface entry, `[x, y, w, h]`.
    figure_draws: Vec<[f32; 4]>,
    /// The medium draws in call order with the 1-based index of their puff shape (0 = none).
    pub(crate) media: Vec<(MediumDraw, usize)>,
    /// The `output` and `crt` fields of a graded variant.
    pub(crate) grade: Option<(Vec<f32>, Vec<f32>)>,
    /// The frame's shockwaves (`explosion`), checked against Chiaro's (`shockwave`).
    waves: Vec<Shockwave>,
}

impl Scene {
    /// The scene of `text` under `p`, its animations sampled at `time` as Chiaro's.
    pub(crate) fn parse(text: &str, p: &Projection, time: f64) -> Self {
        let mut s = Self {
            profiles: ProfileStore::new(),
            animations: AnimationStore::new(),
            masks: MaskStore::new(),
            patterns: PatternStore::new(),
            lights: LightStore::new(),
            light_ids: Vec::new(),
            obstacles: ObstacleStore::new(),
            zones: AmbientZoneStore::new(),
            surface: SurfaceGeometry::new(),
            surface_pass: false,
            figure: None,
            figure_draws: Vec::new(),
            media: Vec::new(),
            grade: None,
            waves: Vec::new(),
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
                        s.light_ids.push(s.lights.create(&d, &res).unwrap());
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
                "figure" => s.figure = Some((num(f[0]), num(f[1]), num(f[2]), Vec::new())),
                "figure_row" => {
                    let px = &mut s.figure.as_mut().unwrap().3;
                    for k in 0..f[0].len() / 2 {
                        let a = u8::from_str_radix(&f[0][2 * k..2 * k + 2], 16).unwrap();
                        px.extend_from_slice(&[a; 4]);
                    }
                }
                "figure_draw" => {
                    s.surface_pass = true;
                    s.figure_draws.push([0, 1, 2, 3].map(|i| num(f[i])));
                }
                "surface_box" => {
                    s.surface_pass = true;
                    let v: Vec<f32> = f.iter().map(|s| num(s)).collect();
                    let b = SurfaceBox {
                        center: At::new(Vec2::new(v[0], v[1])),
                        width: v[2],
                        depth: v[3],
                        height: v[4],
                    };
                    s.surface.push_boxes(p, HEIGHT_STEP, &[b]).unwrap();
                }
                "surface_wall" => {
                    s.surface_pass = true;
                    let top = num(f[6]);
                    let (a, b) = (vec3(&f[0..3]), vec3(&f[3..6]));
                    s.surface.push_wall(p, HEIGHT_STEP, a, b, top).unwrap();
                }
                "surface_floor" => {
                    s.surface_pass = true;
                    let v: Vec<f32> = f.iter().map(|s| num(s)).collect();
                    let (xy, z) = v.split_at(v.len() - 1);
                    let points: Vec<_> = xy
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|[x, y]| At::new(Vec2::new(*x, *y)))
                        .collect();
                    s.surface.push_floor(p, HEIGHT_STEP, &points, z[0]).unwrap();
                }
                "medium_draw" => {
                    let v: Vec<f32> = f[..7].iter().map(|s| num(s)).collect();
                    let draw = MediumDraw {
                        rect: [v[0], v[1], v[2], v[3]],
                        density: v[4],
                        bottom: v[5],
                        top: v[6],
                        medium: num(f[7]),
                    };
                    s.media.push((draw, num(f[8])));
                }
                "output" => s.grade = Some((f.iter().map(|s| num(s)).collect(), Vec::new())),
                "crt" => s.grade.as_mut().unwrap().1 = f.iter().map(|s| num(s)).collect(),
                "explosion" => {
                    let v: Vec<f32> = f.iter().map(|s| num(s)).collect();
                    let origin = Vec2::new(v[0], v[1]);
                    s.waves
                        .push(make_explosion_shockwave(origin, v[2], v[3], v[4]));
                }
                "shockwave" => {
                    let v: Vec<f32> = f.iter().map(|s| num(s)).collect();
                    let w = s.waves.last_mut().expect("`explosion` comes first");
                    let got = [w.x, w.y, w.radius, w.width, w.strength];
                    let got = [
                        &got[..],
                        &[w.light.r, w.light.g, w.light.b, w.light_intensity],
                    ]
                    .concat();
                    let want = [v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8]];
                    assert_eq!(got, want, "the explosion's shockwave differs from Chiaro's");
                }
                _ => panic!("unknown scene item {kind}"),
            }
        }
        let res = StoreResources::new(&s.animations, &s.masks).with_profiles(&s.profiles);
        s.lights
            .set_frame_lights(frame_lights.as_slice(), &res)
            .unwrap();
        // The copied animation descs sample as Chiaro's (seed: the light's slot index).
        assert_eq!(samples.len(), animated.len());
        for (f, (seed, d)) in samples.iter().zip(&animated) {
            let a = s
                .animations
                .sample(d.animation, time, d.animation_offset, *seed as u32);
            let want = [num(f[1]), num(f[2]), num(f[3]), num(f[4]), num::<f32>(f[5])];
            let got = [a.intensity, a.color.x, a.color.y, a.color.z, a.rotation];
            assert_eq!(got, want, "animation sample of animated light {}", f[0]);
        }
        s
    }

    pub(crate) fn stores(&self) -> SceneStores<'_> {
        SceneStores {
            zones: Some(&self.zones),
            profiles: Some(&self.profiles),
            masks: Some(&self.masks),
            animations: Some(&self.animations),
            patterns: Some(&self.patterns),
            ..SceneStores::new(&self.lights, &self.obstacles)
        }
    }

    /// Changes light `id` with `change`.
    pub(crate) fn change_light(&mut self, id: LightId, change: impl FnOnce(&mut LightDesc)) {
        let mut d = self.lights.get(id).unwrap().clone();
        change(&mut d);
        let res = StoreResources::new(&self.animations, &self.masks).with_profiles(&self.profiles);
        self.lights.update(id, &d, &res).unwrap();
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

/// The eight puff shapes of `SmokePuffs.hpp` (`PuffSet::Create`: 128 x 128, white, alpha the
/// shape), from the fixture that stacks them vertically. [`smoke_puff`] makes the same shapes:
/// one alpha byte of 524288 is one higher (puff 2, pixel 41, 51; probably `det::atan2` against
/// MSVC's `atan2f`, not traced), so the scenes draw the fixture to stay bit-equal and the generator is checked here.
fn puffs(gpu: Gpu<'_>) -> Vec<Texture<'_>> {
    let px = scene_common::fixture("medium", "puffs", (128, 8 * 128));
    let mut made = vec![0u8; px.len()];
    for (i, p) in made.chunks_mut(128 * 128 * 4).enumerate() {
        smoke_puff(i as u32 * 977 + 13, 128, p);
    }
    let off: Vec<_> = (made.iter().zip(&px).enumerate())
        .filter(|(_, (a, b))| a != b)
        .map(|(i, (a, b))| (i, *a, *b))
        .collect();
    assert_eq!(
        off,
        [(157_351, 111, 110)],
        "smoke_puff against Chiaro's puffs"
    );
    (px.chunks(128 * 128 * 4))
        .map(|p| sampled_rgba8(gpu, 128, 128, p))
        .collect()
}

/// One frame of a demo to render and compare.
pub(crate) struct Demo<'a> {
    /// Fixture directory under `tests/golden/chiaro`.
    pub(crate) dir: &'a str,
    /// The scene text `<dir>/<variant>.txt`.
    pub(crate) variant: &'a str,
    /// The golden `tests/golden/chiaro/<golden>.png`, also the name in reports.
    pub(crate) golden: &'a str,
    /// The layers besides the medium layer, which is added where the scene draws clouds.
    pub(crate) kinds: LayerSet,
    /// Suffix of this variant's colour, glow and surface fixtures.
    pub(crate) layer_suffix: &'a str,
    /// World to scene.
    pub(crate) projection: Projection,
    /// Animation time of the frame.
    pub(crate) time: f64,
    /// The overlay (target size, premultiplied).
    pub(crate) overlay: Option<Vec<u8>>,
    /// Target rects left out of the comparison (the HUD text).
    pub(crate) ignore: &'a [Rect],
    /// The fixture the clouds drawn by [`MediumBatch`] must equal: Chiaro's GPU-layer path
    /// (`DrawMediumsGpu`), which `MediumBatch` ports.
    pub(crate) medium_gpu: &'a str,
    /// The demo's own medium layer (renderer layers, `FlushMedium`) where it differs from the
    /// GPU-layer path: uploaded over the batch's clouds before the light, like the colour layer.
    pub(crate) medium_renderer: Option<&'a str>,
}

impl Demo<'_> {
    /// Renders the frame with the settings `settings` gives for the parsed scene and compares
    /// it with the golden under `STRICT`, the helper-drawn layers with Chiaro's exactly.
    pub(crate) fn check(&self, settings: impl Fn(&Scene) -> Light2dSettings) {
        with_chiaro_pack(|gpu, pack| self.render(gpu, pack, &settings));
    }

    /// Reads `layer` back and compares it byte for byte (alpha included: the medium layer's
    /// density) with the fixture `<name>`; `None` if equal, else the differing bytes and the
    /// largest difference.
    fn layer_parity(
        &self,
        gpu: Gpu<'_>,
        layers: &Layers<'_>,
        layer: Layer,
        name: &str,
    ) -> Option<String> {
        let got = read_texture(gpu, layers.texture(layer).unwrap());
        let want = scene_common::fixture(self.dir, name, (W, H));
        let diff = got.iter().zip(&want).filter(|(a, b)| a != b);
        let n = diff.clone().count();
        let max = diff.map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        eprintln!(
            "{} {layer:?} layer vs {name}: {n} differing bytes (max {max})",
            self.golden
        );
        if let Some(dir) = parity::scene_out() {
            let file = format!("{}.{layer:?}", self.golden);
            parity::save_with_diff(&dir, &file, &got, &want, (W, H));
        }
        (n > 0).then(|| format!("{layer:?} layer vs {name}: {n} differing bytes, max {max}"))
    }

    /// Builds the scene, fills its layers and hands `body` what renders frames of it. Returns
    /// how the helper-drawn layers differ from Chiaro's.
    pub(crate) fn with_frames(
        &self,
        gpu: Gpu<'_>,
        pack: &[u8],
        settings: &dyn Fn(&Scene) -> Light2dSettings,
        body: &mut dyn FnMut(&mut DemoFrames<'_, '_>),
    ) -> Vec<String> {
        let text = golden_bytes(self.dir, &format!("{}.txt", self.variant));
        let text = String::from_utf8(text).expect("the scene text is UTF-8");
        let projection = &self.projection;
        let scene = Scene::parse(&text, projection, self.time);
        let settings = settings(&scene);
        let clouds = !scene.media.is_empty();
        let kinds = self.kinds;
        let all = if clouds {
            kinds.with(Layer::Medium)
        } else {
            kinds
        };
        let mut layers = Layers::new(gpu, W, H, all).unwrap();
        let mut renderer = Light2dRenderer::new(gpu, pack.to_vec()).unwrap();
        let target: Texture<'_> = texture(
            gpu,
            OUT,
            SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
            SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
            None,
        );
        let overlay = (self.overlay.as_ref()).map(|px| sampled_rgba8(gpu, OUT.0, OUT.1, px));

        let suffix = self.layer_suffix;
        let fixture =
            |layer: &str| scene_common::fixture(self.dir, &format!("{layer}{suffix}"), (W, H));
        let (color, glow) = (fixture("scene"), fixture("glow"));
        let shapes = puffs(gpu);
        let media: Vec<_> = (scene.media.iter())
            .map(|(draw, puff)| MediumItem {
                draw: *draw,
                shape: puff.checked_sub(1).map(|k| shapes[k].texture_ref()),
            })
            .collect();
        let mut shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        let samplers = Samplers::new(gpu).unwrap();
        let mut sprites = SpriteBatch::new(gpu, &shaders).unwrap();
        let mut surface = SurfaceBatch::new(gpu, &shaders).unwrap();
        let mut medium = MediumBatch::new(gpu, &shaders).unwrap();
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        let figure = scene.figure.as_ref().map(|(w, h, height, px)| {
            let desc = SpriteDesc {
                standing: true,
                standing_height: *height,
                ..SpriteDesc::new(*w, *h, px)
            };
            Sprite::new(gpu, &mut cmd, &desc, kinds, HEIGHT_STEP).unwrap()
        });
        let draws: Vec<_> = (scene.figure_draws.iter())
            .map(|dst| SpriteDraw::new(figure.as_ref().unwrap(), *dst))
            .collect();
        sprites.prepare(&mut shaders, &draws).unwrap();
        surface.prepare(&mut shaders, &scene.surface).unwrap();
        medium.prepare(&mut shaders, &media).unwrap();
        // A pass takes at most four layers: with material and medium layer there are five.
        let others = (Layer::ALL.into_iter())
            .filter(|&l| l != Layer::Medium)
            .fold(LayerSet::NONE, LayerSet::with);
        drop(layers.clear_pass(&mut cmd, others).unwrap());
        if clouds {
            drop(
                layers
                    .clear_pass(&mut cmd, LayerSet::of(Layer::Medium))
                    .unwrap(),
            );
        }
        // Chiaro queues the helper geometry and draws it when the target changes, after the
        // figures the renderer drew in between: sprites first, geometry on top. The sprites also
        // write their colour, which the colour fixture then replaces.
        sprites
            .record(&mut cmd, &samplers, &mut layers, &draws)
            .unwrap();
        let _uploads = scene_common::upload_layers(
            gpu,
            &mut cmd,
            &mut layers,
            &[(Layer::Scene, &color), (Layer::Glow, &glow)],
        );
        surface
            .record(&mut cmd, &mut layers, &scene.surface)
            .unwrap();
        // The clouds (`Layers::DrawMediums` after the scene).
        let step = settings.light.resolved_medium_height_step();
        medium.record(&mut cmd, &mut layers, step, &media).unwrap();
        cmd.submit_and_wait().unwrap();
        // The helper-drawn surface and medium layers equal Chiaro's (without a surface pass or
        // clouds the fixture stays unread).
        let mut parity = Vec::new();
        if scene.surface_pass {
            let name = format!("surface{suffix}");
            parity.push(self.layer_parity(gpu, &layers, Layer::Surface, &name));
        }
        if clouds {
            parity.push(self.layer_parity(gpu, &layers, Layer::Medium, self.medium_gpu));
        }
        let renderer_medium = self.medium_renderer.map(|name| {
            let px = scene_common::fixture(self.dir, name, (W, H));
            let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
            let up =
                scene_common::upload_layers(gpu, &mut cmd, &mut layers, &[(Layer::Medium, &px)]);
            cmd.submit_and_wait().unwrap();
            up
        });

        let mut list = ShockwaveList::default();
        list.set(&scene.waves).unwrap();
        let post = post_projection(projection);
        let mut waves = ShockwaveFrame::default();
        collect_shockwaves(&list, &post, W, H, &mut waves);
        renderer.note_layers(&mut layers);
        body(&mut DemoFrames {
            gpu,
            layers: &mut layers,
            target: target.texture_ref(),
            overlay: overlay.as_ref().map(Texture::texture_ref),
            shockwaves: (!scene.waves.is_empty()).then_some((&post, &waves)),
            time: self.time,
            scene,
            projection: *projection,
            settings,
            renderer,
            skip: LayerSet::NONE,
            occlusion: None,
        });
        drop(renderer_medium);
        parity.into_iter().flatten().collect()
    }

    fn render(&self, gpu: Gpu<'_>, pack: &[u8], settings: &dyn Fn(&Scene) -> Light2dSettings) {
        let mut pixels = Vec::new();
        let parity = self.with_frames(gpu, pack, settings, &mut |f| pixels = f.frame().0);
        let golden = scene_common::expected(".", self.golden, &pixels, OUT);
        let strict = compare_rgba8(
            &pixels,
            &golden,
            OUT.0,
            OUT.1,
            self.ignore,
            &ParityBudget::STRICT,
        )
        .unwrap();
        strict.print(self.golden, "STRICT");
        if let Some(dir) = parity::scene_out() {
            parity::save_with_diff(&dir, self.golden, &pixels, &golden, OUT);
        }
        assert!(
            parity.is_empty(),
            "helper-drawn layers differ from Chiaro's: {parity:?}"
        );
        assert!(
            strict.within(&ParityBudget::STRICT),
            "STRICT parity missed: {strict:?}"
        );
        // STRICT lets every pixel be off by 2; these scenes are bit-identical, and a slightly
        // wrong haze or a missing dither moves a third of the pixels by 1 and stays within it.
        assert_eq!(strict.max_diff, 0, "not bit-identical: {strict:?}");
    }
}

/// A demo's scene with its filled layers and a renderer: renders frames of it, between which a
/// test changes the stores, the camera or the settings.
pub(crate) struct DemoFrames<'a, 'd> {
    pub(crate) gpu: Gpu<'d>,
    pub(crate) layers: &'a mut Layers<'d>,
    target: TextureRef<'a>,
    overlay: Option<TextureRef<'a>>,
    shockwaves: Option<(&'a PostProjection, &'a ShockwaveFrame)>,
    time: f64,
    pub(crate) scene: Scene,
    /// The camera of the light; the layers stay drawn for the demo's.
    pub(crate) projection: Projection,
    pub(crate) settings: Light2dSettings,
    pub(crate) renderer: Light2dRenderer<'d>,
    /// Layers the frames leave out.
    pub(crate) skip: LayerSet,
    /// The application's sprite mask of the frames (scene size), if any.
    pub(crate) occlusion: Option<Texture<'d>>,
}

impl DemoFrames<'_, '_> {
    /// Prepares, records and submits one frame; the target's pixels and what was recorded.
    pub(crate) fn frame(&mut self) -> (Vec<u8>, PassRecord) {
        let record = self.submit(true);
        (read_texture(self.gpu, self.target), record)
    }

    /// Median wall-clock microseconds per frame of `runs` runs of `frames` frames each, the
    /// target not read back; it waits for the GPU every 100 frames (`gpu_lightrow_probe.rs`).
    /// `scroll` moves the camera one mask texel (2 scene pixels) a frame along x, 30 texels to
    /// and fro, and puts it back afterwards. The host times frames: wall clock is a host tool.
    #[allow(clippy::disallowed_methods)]
    pub(crate) fn time_frames(&mut self, frames: usize, runs: usize, scroll: bool) -> f64 {
        let origin = self.projection.origin;
        let mut times: Vec<f64> = (0..runs)
            .map(|_| {
                let start = std::time::Instant::now();
                for k in 1..=frames {
                    if scroll {
                        let step = (k % 60).min(60 - k % 60) as f32;
                        self.projection.origin = At::new(origin.raw() + Vec2::new(2.0 * step, 0.0));
                    }
                    self.submit(k.is_multiple_of(100) || k == frames);
                }
                start.elapsed().as_secs_f64() * 1e6 / frames as f64
            })
            .collect();
        self.projection.origin = origin;
        times.sort_by(f64::total_cmp);
        times[runs / 2]
    }

    fn submit(&mut self, wait: bool) -> PassRecord {
        let mut frame = (self.layers).frame(self.target, &self.projection, self.skip);
        frame.overlay = self.overlay;
        frame.time = self.time;
        frame.shockwaves = self.shockwaves;
        frame.occlusion = self.occlusion.as_ref().map(Texture::texture_ref);
        let prepared = (self.renderer)
            .prepare(&frame, &self.scene.stores(), &self.settings)
            .unwrap();
        let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
        let record = (self.renderer)
            .record(&mut cmd, &frame, &self.settings, prepared)
            .unwrap();
        if wait {
            cmd.submit_and_wait().unwrap();
        } else {
            cmd.submit().unwrap();
        }
        record
    }

    /// The picture a new renderer (shader pack `pack`) gives for the same frame: nothing kept
    /// from earlier frames.
    pub(crate) fn fresh_frame(&mut self, pack: &[u8]) -> Vec<u8> {
        let new = Light2dRenderer::new(self.gpu, pack.to_vec()).unwrap();
        let kept = std::mem::replace(&mut self.renderer, new);
        let (pixels, _) = self.frame();
        self.renderer = kept;
        pixels
    }
}
