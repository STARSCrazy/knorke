//! P27 on a real GPU: the composite pass (`knorke_light2d::composite::CompositeStage`) after the
//! light pass (P25), read back and compared byte for byte with C++ Chiaro (chiaro @ 9bc5b9b,
//! dumps of `tests/golden/chiaro/composite/composite_ref.cpp`, zlib license), and the composite
//! behaviour of Chiaro's `tests/gpu/GlowTests.cpp` and `RenderTests.cpp` checked on the HDR buffer.
//!
//! The test plays the host (ADR 0002): it owns device, command buffers and the input textures,
//! submits and reads back. It skips (passes with a note) without a Vulkan device or without the
//! shader pack from `cargo xtask shaders` in `target/shaders/`.
#![cfg(feature = "gpu")]

mod common;

use common::{H, W, at, dome_material_pixels};
use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB,
    SDL_GPUTextureFormat,
};
use knorke_gpu::test_support::{
    expected_bytes, half_to_f32, queue_download, read_download, rgba16f_diff, sampled,
    sampled_rgba8, with_chiaro_pack,
};
use knorke_gpu::{CommandBuffer, Gpu, Samplers, ShaderLibrary, Texture};
use knorke_light2d::codec::surface_pixel;
use knorke_light2d::composite::{
    COMPOSITE_INPUT_MISSING, COMPOSITE_NOT_PREPARED, CompositeFrame, CompositeLight,
    CompositeProgram, CompositeStage, InputColorSpace,
};
use knorke_light2d::light::{Height, LightDesc, LightType};
use knorke_light2d::light_culling::{LightList, LightModes, collect_lights};
use knorke_light2d::lighting::{DetailMode, LightingFrame, LightingResult, LightingStage};
use knorke_light2d::settings::{GlowColor, LightSettings, Quality, Resolutions};
use knorke_light2d::{LightStore, NoResources, Projection};
use knorke_math::{Vec3, det};

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        det::pow((c + 0.055) / 1.055, 2.4)
    }
}

// ---- The inputs of the dumper `composite_ref.cpp` ---------------------------------------------

/// `ScenePixels()`: sRGB-encoded gradient with a checker blue channel.
fn scene_pixels() -> Vec<u8> {
    let mut px = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let p = ((y * W + x) * 4) as usize;
            px[p] = (x * 255 / (W - 1)) as u8;
            px[p + 1] = (y * 255 / (H - 1)) as u8;
            px[p + 2] = if ((x / 8) + (y / 8)) & 1 != 0 {
                200
            } else {
                60
            };
            px[p + 3] = 255;
        }
    }
    px
}

/// `GlowPixels()`: orange in a rectangle, strength rising by column; zero elsewhere.
fn glow_pixels() -> Vec<u8> {
    let mut px = vec![0u8; (W * H * 4) as usize];
    for y in 10..30 {
        for x in 50..90 {
            let p = ((y * W + x) * 4) as usize;
            px[p..p + 4].copy_from_slice(&[255, 140, 40, (40 + (x - 50) * 5) as u8]);
        }
    }
    px
}

/// `Surface()`: ground, a plateau of height 6 and a tilted ramp of height 3 (step 1).
fn surface_pixels() -> Vec<u8> {
    let mut px = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let (height, normal) = if (30..70).contains(&x) && (12..36).contains(&y) {
                (6.0, Vec3::new(0.0, 0.0, 1.0))
            } else if (70..90).contains(&x) && (12..36).contains(&y) {
                (3.0, Vec3::new(0.4, 0.0, 0.9))
            } else {
                (0.0, Vec3::new(0.0, 0.0, 1.0))
            };
            let p = ((y * W + x) * 4) as usize;
            px[p..p + 4].copy_from_slice(&surface_pixel(height, 1.0, normal));
        }
    }
    px
}

/// The lights of the dumper's `Lights()` / `Directional()`.
#[derive(Clone, Copy)]
enum Lights {
    /// Two point lights and a spot light.
    Three,
    /// One directional light (folds into the ambient, no light pass).
    Directional,
    /// `IsoLights()`: two point lights and a spot light for the isometric scene.
    Iso,
}

impl Lights {
    fn create(self, store: &mut LightStore) {
        let mut add = |d: LightDesc| {
            store.create(&d, &NoResources).unwrap();
        };
        match self {
            Lights::Three => {
                let mut d = LightDesc::new(LightType::Point);
                d.position = at(20.0, 24.0);
                d.radius = 40.0;
                d.color = Vec3::new(1.0, 0.8, 0.6);
                d.intensity = 1.2;
                add(d);
                let mut d = LightDesc::new(LightType::Point);
                d.position = at(60.0, 10.0);
                d.height = Height::At(8.0);
                d.radius = 35.0;
                d.falloff = 1.5;
                add(d);
                let mut d = LightDesc::new(LightType::Spot);
                d.position = at(84.0, 38.0);
                d.height = Height::At(6.0);
                d.radius = 45.0;
                d.color = Vec3::new(0.5, 0.7, 1.0);
                d.direction = Vec3::new(-1.0, -0.5, -0.3);
                d.inner_angle = 20.0;
                d.outer_angle = 35.0;
                add(d);
            }
            Lights::Iso => {
                let mut d = LightDesc::new(LightType::Point);
                d.position = at(-4.0, 2.0);
                d.height = Height::At(5.0);
                d.radius = 24.0;
                d.color = Vec3::new(1.0, 0.8, 0.6);
                d.intensity = 1.2;
                add(d);
                let mut d = LightDesc::new(LightType::Point);
                d.position = at(8.0, 4.0);
                d.radius = 18.0;
                d.falloff = 1.5;
                add(d);
                let mut d = LightDesc::new(LightType::Spot);
                d.position = at(4.0, -8.0);
                d.height = Height::At(6.0);
                d.radius = 26.0;
                d.color = Vec3::new(0.5, 0.7, 1.0);
                d.direction = Vec3::new(-0.3, 0.8, -0.5);
                d.inner_angle = 15.0;
                d.outer_angle = 35.0;
                add(d);
            }
            Lights::Directional => {
                let mut d = LightDesc::new(LightType::Directional);
                d.intensity = 0.3;
                d.direction = Vec3::new(0.3, 0.4, -1.0);
                add(d);
            }
        }
    }
}

/// One golden scene of `composite_ref.cpp` (`Config` and the `run` lines of `main`).
struct Scene {
    name: &'static str,
    lights: Lights,
    ambient: f32,
    light_resolution: f32,
    glow_scale: f32,
    glow_color: GlowColor,
    glow: bool,
    material: bool,
    /// Isometric projection, surface buffer and wet ground (the `iso` scene).
    iso: bool,
    /// What the light pass must have done (checks the scene does what its comment says).
    detail: DetailMode,
    light_buffer: bool,
}

const SCENES: [Scene; 7] = [
    Scene {
        name: "plain",
        lights: Lights::Three,
        ambient: 0.05,
        light_resolution: 1.0,
        glow_scale: 1.0,
        glow_color: GlowColor::Buffer,
        glow: false,
        material: false,
        iso: false,
        detail: DetailMode::None,
        light_buffer: true,
    },
    Scene {
        name: "glow",
        lights: Lights::Three,
        ambient: 0.05,
        light_resolution: 1.0,
        glow_scale: 2.0,
        glow_color: GlowColor::Buffer,
        glow: true,
        material: false,
        iso: false,
        detail: DetailMode::None,
        light_buffer: true,
    },
    Scene {
        name: "glow_scene",
        lights: Lights::Three,
        ambient: 0.05,
        light_resolution: 1.0,
        glow_scale: 1.5,
        glow_color: GlowColor::Scene,
        glow: true,
        material: false,
        iso: false,
        detail: DetailMode::None,
        light_buffer: true,
    },
    Scene {
        name: "exact",
        lights: Lights::Three,
        ambient: 0.05,
        light_resolution: 1.0,
        glow_scale: 1.0,
        glow_color: GlowColor::Buffer,
        glow: true,
        material: true,
        iso: false,
        detail: DetailMode::Exact,
        light_buffer: true,
    },
    Scene {
        name: "direction",
        lights: Lights::Three,
        ambient: 0.05,
        light_resolution: 0.5,
        glow_scale: 1.0,
        glow_color: GlowColor::Buffer,
        glow: true,
        material: true,
        iso: false,
        detail: DetailMode::Direction,
        light_buffer: true,
    },
    Scene {
        name: "ambient",
        lights: Lights::Directional,
        ambient: 0.2,
        light_resolution: 1.0,
        glow_scale: 1.0,
        glow_color: GlowColor::Buffer,
        glow: false,
        material: false,
        iso: false,
        detail: DetailMode::None,
        light_buffer: false,
    },
    Scene {
        name: "iso",
        lights: Lights::Iso,
        ambient: 0.05,
        light_resolution: 0.5,
        glow_scale: 1.0,
        glow_color: GlowColor::Buffer,
        glow: true,
        material: true,
        iso: true,
        detail: DetailMode::Direction,
        light_buffer: true,
    },
];

impl Scene {
    fn settings(&self) -> LightSettings {
        let a = self.ambient;
        let mut s = LightSettings {
            ambient: Vec3::new(a, a * 1.2, a * 1.4),
            ..LightSettings::default()
        };
        s.glow.scale = self.glow_scale;
        s.glow.color = self.glow_color;
        if self.iso {
            s.material.wetness = 0.6;
        }
        if self.light_resolution < 1.0 {
            s.quality.quality = Quality::Custom(Resolutions {
                light: self.light_resolution,
                shadow: 1.0,
                medium: 0.5,
            });
        }
        s
    }
}

// ---- Host -------------------------------------------------------------------------------------

struct Host<'d> {
    gpu: Gpu<'d>,
    shaders: ShaderLibrary<'d>,
    samplers: Samplers<'d>,
    lighting: LightingStage<'d>,
    composite: CompositeStage<'d>,
    list: LightList,
}

impl<'d> Host<'d> {
    fn new(gpu: Gpu<'d>, pack: &[u8]) -> Self {
        let shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        Self {
            gpu,
            samplers: Samplers::new(gpu).unwrap(),
            lighting: LightingStage::new(gpu),
            composite: CompositeStage::new(gpu, &shaders).unwrap(),
            shaders,
            list: LightList::default(),
        }
    }

    /// One golden scene: light pass, composite, the HDR buffer read back.
    fn frame(&mut self, s: &Scene) -> (LightingResult, CompositeProgram, Vec<u8>) {
        let scene_tex = sampled_rgba8(self.gpu, W, H, &scene_pixels());
        let glow_tex = s
            .glow
            .then(|| sampled_rgba8(self.gpu, W, H, &glow_pixels()));
        let material_tex = s
            .material
            .then(|| sampled_rgba8(self.gpu, W, H, &dome_material_pixels(W, H)));
        let surface_tex = s
            .iso
            .then(|| sampled_rgba8(self.gpu, W, H, &surface_pixels()));
        let settings = s.settings();
        let projection = if s.iso {
            Projection::iso(at(0.0, 0.0), 2.0, 1.0, W, H)
        } else {
            Projection::IDENTITY
        };
        // Chiaro `BuildFrame`: with a surface buffer and no obstacles, the highest encodable height.
        let max_height = if s.iso {
            255.0 * settings.surface_height_step
        } else {
            0.0
        };
        let mut lights = LightStore::new();
        s.lights.create(&mut lights);
        let modes = LightModes {
            height_mode: false,
            normal_mode: s.material,
            max_height,
            ..Default::default()
        };
        collect_lights(
            &lights,
            &settings,
            &projection,
            0.0,
            W,
            H,
            &modes,
            &mut self.list,
        );
        let lighting_frame = LightingFrame {
            scene: scene_tex.texture_ref(),
            surface: surface_tex.as_ref().map(Texture::texture_ref),
            material: material_tex.as_ref().map(Texture::texture_ref),
            projection: &projection,
            settings: &settings,
            max_height,
            lights: &self.list,
            shadows: None,
            profile_atlas: None,
            mask_atlas: None,
            ring_lights: &[],
            medium_map: None,
            medium_runs: false,
        };
        let result = self
            .lighting
            .prepare(&mut self.shaders, &lighting_frame)
            .unwrap();
        let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
        self.lighting
            .record(&mut cmd, &self.samplers, &lighting_frame)
            .unwrap();
        let frame = CompositeFrame {
            scene: scene_tex.texture_ref(),
            glow: glow_tex.as_ref().map(Texture::texture_ref),
            material: material_tex.as_ref().map(Texture::texture_ref),
            surface: surface_tex.as_ref().map(Texture::texture_ref),
            light: CompositeLight::from_stage(&self.lighting, &result),
            medium: None,
            projection: &projection,
            settings: &settings,
            input_space: InputColorSpace::Srgb,
        };
        let prepared = self.composite.prepare(&mut self.shaders, &frame).unwrap();
        self.composite
            .record(&mut cmd, &self.samplers, &frame)
            .unwrap();
        let hdr = self.composite.hdr_buffer().unwrap();
        assert_eq!((hdr.width(), hdr.height()), (W, H));
        let down = queue_download(self.gpu, &mut cmd, hdr);
        cmd.submit_and_wait().unwrap();
        (result, prepared.program, read_download(down))
    }
}

fn with_host(body: impl for<'d> FnOnce(&mut dyn FnMut() -> Host<'d>)) {
    with_chiaro_pack(|gpu, pack| body(&mut || Host::new(gpu, pack)));
}

#[test]
fn hdr_buffers_match_chiaro() {
    with_host(|fresh| {
        let mut failures = Vec::new();
        for scene in &SCENES {
            let mut host = fresh();
            let (result, program, hdr) = host.frame(scene);
            let name = scene.name;
            assert_eq!(result.detail_mode, scene.detail, "{name}: material mode");
            assert_eq!(
                result.light_buffer, scene.light_buffer,
                "{name}: light pass"
            );
            assert_eq!(
                result.use_specular,
                scene.detail == DetailMode::Exact,
                "{name}: specular"
            );
            assert_eq!(program, CompositeProgram::Base, "{name}: program");
            let (n, max) = rgba16f_diff(
                &hdr,
                &expected_bytes("composite", &format!("{name}.hdr.bin"), &hdr),
            );
            eprintln!("{name}: HDR buffer {n} differing texels (max {max})");
            if n > 0 {
                failures.push(format!("{name}: {n} texels, max {max}"));
            }
        }
        assert!(failures.is_empty(), "differs from Chiaro: {failures:?}");
    });
}

// ---- Behaviour (GlowTests.cpp, RenderTests.cpp) -------------------------------------------------

const GW: u32 = 64;
const GH: u32 = 16;

/// A GW x GH image of one colour, with rectangles painted over it.
fn image(fill: [u8; 4], rects: &[(u32, u32, u32, u32, [u8; 4])]) -> Vec<u8> {
    let mut px = fill.repeat((GW * GH) as usize);
    for &(x0, y0, x1, y1, c) in rects {
        for y in y0..y1 {
            for x in x0..x1 {
                let p = ((y * GW + x) * 4) as usize;
                px[p..p + 4].copy_from_slice(&c);
            }
        }
    }
    px
}

/// The composite alone: no light pass, `ambient` as light, optional glow; returns the HDR buffer
/// as linear RGB per pixel.
struct Run<'a> {
    scene: Vec<u8>,
    scene_format: SDL_GPUTextureFormat,
    glow: Option<(Vec<u8>, SDL_GPUTextureFormat)>,
    ambient: Vec3,
    space: InputColorSpace,
    settings: LightSettings,
    _marker: core::marker::PhantomData<&'a ()>,
}

impl Run<'_> {
    fn new() -> Self {
        Self {
            scene: image([255; 4], &[]),
            scene_format: SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
            glow: None,
            ambient: Vec3::ZERO,
            space: InputColorSpace::Srgb,
            settings: LightSettings::default(),
            _marker: core::marker::PhantomData,
        }
    }

    fn render(&self, host: &mut Host<'_>) -> Vec<[f32; 3]> {
        let scene = sampled(host.gpu, GW, GH, self.scene_format, &self.scene);
        let glow = self
            .glow
            .as_ref()
            .map(|(px, f)| sampled(host.gpu, GW, GH, *f, px));
        let projection = Projection::IDENTITY;
        let frame = CompositeFrame {
            scene: scene.texture_ref(),
            glow: glow.as_ref().map(Texture::texture_ref),
            material: None,
            surface: None,
            light: CompositeLight {
                ambient: self.ambient,
                light_buffer: None,
                use_specular: false,
                specular: None,
                direction: None,
                pure_ambient: None,
            },
            medium: None,
            projection: &projection,
            settings: &self.settings,
            input_space: self.space,
        };
        host.composite.prepare(&mut host.shaders, &frame).unwrap();
        let mut cmd = CommandBuffer::acquire(host.gpu.device()).unwrap();
        host.composite
            .record(&mut cmd, &host.samplers, &frame)
            .unwrap();
        let hdr = host.composite.hdr_buffer().unwrap();
        let down = queue_download(host.gpu, &mut cmd, hdr);
        cmd.submit_and_wait().unwrap();
        read_download(down)
            .chunks(8)
            .map(|p| {
                let c = |i: usize| half_to_f32(u16::from_le_bytes([p[i * 2], p[i * 2 + 1]]));
                [c(0), c(1), c(2)]
            })
            .collect()
    }
}

fn px(img: &[[f32; 3]], x: u32, y: u32) -> [f32; 3] {
    img[(y * GW + x) as usize]
}

fn close(got: [f32; 3], want: [f32; 3], tolerance: f32) {
    for c in 0..3 {
        assert!(
            (got[c] - want[c]).abs() <= tolerance,
            "got {got:?}, want {want:?}"
        );
    }
}

/// sRGB-encoded 8-bit value of a linear value.
fn encode(linear: f32) -> u8 {
    let s = if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * det::pow(linear, 1.0 / 2.4) - 0.055
    };
    (s * 255.0).round() as u8
}

#[test]
fn the_glow_buffer_adds_colour_times_strength_times_scale() {
    // GlowTests.cpp: scale 4, both 8-bit formats; exposure is the output stage's, so the HDR values
    // are the products themselves.
    for format in [
        SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
        SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB,
    ] {
        with_host(|fresh| {
            let mut host = fresh();
            let half_byte = encode(0.5);
            let g = srgb_to_linear(f32::from(half_byte) / 255.0);
            let mut run = Run::new();
            run.settings.glow.scale = 4.0;
            run.glow = Some((
                image(
                    [0; 4],
                    &[
                        (0, 0, 16, GH, [255, half_byte, 0, 255]), // full strength
                        (16, 0, 32, GH, [255, half_byte, 0, 51]), // A = 51: a fifth
                        (32, 0, 48, GH, [255, half_byte, 0, 0]),  // A = 0: nothing
                    ],
                ),
                format,
            ));
            let out = run.render(&mut host);
            close(px(&out, 8, 8), [4.0, 4.0 * g, 0.0], 0.05);
            close(px(&out, 24, 8), [0.8, 0.8 * g, 0.0], 0.01);
            close(px(&out, 40, 8), [0.0; 3], 0.001);
            close(px(&out, 56, 8), [0.0; 3], 0.001);
        });
    }
}

#[test]
fn the_glow_buffer_follows_the_input_colour_space() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut run = Run::new();
        run.space = InputColorSpace::Linear;
        run.glow = Some((
            image([255, 128, 0, 255], &[]),
            SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
        ));
        let out = run.render(&mut host);
        // Linear values: (1, 128/255, 0), no decode.
        close(px(&out, 8, 8), [1.0, 128.0 / 255.0, 0.0], 0.002);
    });
}

#[test]
fn the_scene_colour_source_uses_only_the_strength_of_the_buffer() {
    for source in [GlowColor::Buffer, GlowColor::Scene] {
        with_host(|fresh| {
            let mut host = fresh();
            let mut run = Run::new();
            run.scene = image([255, 0, 255, 255], &[]); // magenta scene
            run.settings.glow.scale = 2.0;
            run.settings.glow.color = source;
            run.glow = Some((
                image([0, 255, 0, 255], &[]), // green buffer
                SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
            ));
            let got = px(&run.render(&mut host), 8, 8);
            match source {
                GlowColor::Buffer => close(got, [0.0, 2.0, 0.0], 0.005),
                GlowColor::Scene => close(got, [2.0, 0.0, 2.0], 0.005),
            }
        });
    }
}

#[test]
fn glow_does_not_depend_on_the_ambient() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut run = Run::new();
        run.ambient = Vec3::new(0.25, 0.25, 0.25);
        let without = run.render(&mut host);
        run.glow = Some((
            image([255, 255, 255, 255], &[]),
            SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
        ));
        let with = run.render(&mut host);
        for x in [2, 10, 30, 40, 60] {
            let (a, b) = (px(&without, x, 8), px(&with, x, 8));
            assert!((a[0] - 0.25).abs() < 0.002, "{a:?}");
            for c in 0..3 {
                assert!((b[c] - a[c] - 1.0).abs() <= 0.003, "x {x}: {a:?} {b:?}");
            }
        }
    });
}

#[test]
fn an_srgb_scene_texture_is_not_decoded_twice() {
    // RenderTests.cpp: 128 in an _SRGB texture is decoded by the sampler once; the shader's
    // decode would square it again.
    with_host(|fresh| {
        let mut host = fresh();
        let mut run = Run::new();
        run.ambient = Vec3::new(1.0, 1.0, 1.0);
        run.scene = image([128, 128, 128, 255], &[]);
        run.scene_format = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB;
        let once = srgb_to_linear(128.0 / 255.0);
        close(px(&run.render(&mut host), 8, 8), [once; 3], 0.002);
        // The same bytes in a UNORM texture are decoded by the shader: the same value.
        run.scene_format = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;
        close(px(&run.render(&mut host), 8, 8), [once; 3], 0.002);
    });
}

#[test]
fn a_linear_input_colour_space_skips_the_decode() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut run = Run::new();
        run.ambient = Vec3::new(1.0, 1.0, 1.0);
        run.scene = image([128, 128, 128, 255], &[]);
        run.space = InputColorSpace::Linear;
        close(px(&run.render(&mut host), 8, 8), [128.0 / 255.0; 3], 0.002);
    });
}

#[test]
fn without_a_light_buffer_the_ambient_is_the_light() {
    with_host(|fresh| {
        let mut host = fresh();
        let mut run = Run::new();
        run.ambient = Vec3::new(0.5, 0.25, 0.125);
        run.scene = image([255, 255, 255, 255], &[]);
        close(px(&run.render(&mut host), 3, 3), [0.5, 0.25, 0.125], 0.001);
    });
}

#[test]
fn the_hdr_target_is_reallocated_only_when_the_scene_size_changes() {
    with_host(|fresh| {
        let mut host = fresh();
        let projection = Projection::IDENTITY;
        let settings = LightSettings::default();
        let mut allocations = Vec::new();
        for (w, h) in [(8, 8), (8, 8), (16, 8), (16, 8)] {
            let scene = sampled_rgba8(host.gpu, w, h, &vec![255; (w * h * 4) as usize]);
            let frame = CompositeFrame {
                scene: scene.texture_ref(),
                glow: None,
                material: None,
                surface: None,
                light: CompositeLight {
                    ambient: Vec3::ONE,
                    light_buffer: None,
                    use_specular: false,
                    specular: None,
                    direction: None,
                    pure_ambient: None,
                },
                medium: None,
                projection: &projection,
                settings: &settings,
                input_space: InputColorSpace::Srgb,
            };
            let r = host.composite.prepare(&mut host.shaders, &frame).unwrap();
            let hdr = host.composite.hdr_buffer().unwrap();
            assert_eq!((hdr.width(), hdr.height()), (w, h));
            allocations.push(r.allocations);
        }
        assert_eq!(allocations, [1, 0, 1, 0]);
        // One program, one pipeline.
        assert_eq!(host.composite.pipeline_count(), 1);
    });
}

#[test]
fn missing_inputs_and_missing_prepare_are_diagnostics() {
    with_host(|fresh| {
        let mut host = fresh();
        let scene = sampled_rgba8(host.gpu, 8, 8, &[255; 8 * 8 * 4]);
        let projection = Projection::IDENTITY;
        let settings = LightSettings::default();
        let mut frame = CompositeFrame {
            scene: scene.texture_ref(),
            glow: None,
            material: None,
            surface: None,
            light: CompositeLight {
                ambient: Vec3::ONE,
                light_buffer: None,
                use_specular: true,
                specular: None,
                direction: None,
                pure_ambient: None,
            },
            medium: None,
            projection: &projection,
            settings: &settings,
            input_space: InputColorSpace::Srgb,
        };
        // The specular buffer needs the material buffer (and itself).
        let e = host
            .composite
            .prepare(&mut host.shaders, &frame)
            .unwrap_err();
        assert_eq!(e.code, COMPOSITE_INPUT_MISSING);
        // A failed prepare leaves nothing to record.
        let mut cmd = CommandBuffer::acquire(host.gpu.device()).unwrap();
        let e = host
            .composite
            .record(&mut cmd, &host.samplers, &frame)
            .unwrap_err();
        assert_eq!(e.code, COMPOSITE_NOT_PREPARED);
        frame.light.use_specular = false;
        host.composite.prepare(&mut host.shaders, &frame).unwrap();
        // A frame that picks another program than the prepared one is refused.
        frame.light.direction = Some(scene.texture_ref());
        frame.light.pure_ambient = Some(scene.texture_ref());
        let e = host
            .composite
            .record(&mut cmd, &host.samplers, &frame)
            .unwrap_err();
        assert_eq!(e.code, COMPOSITE_NOT_PREPARED);
    });
}
