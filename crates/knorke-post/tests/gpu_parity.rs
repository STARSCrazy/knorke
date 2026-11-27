//! P20 on a real GPU (feature `gpu`): the bloom chain and the output pass of `knorke-post` against
//! C++ Chiaro reference images (`tests/golden/chiaro/post`, made by `post_ref.c` with C++ Chiaro
//! 9bc5b9b on Vulkan) under `ParityBudget::STRICT`.
//!
//! The test plays the host (ADR 0002): it owns device, textures and command buffers, submits and
//! reads back through the safe API of `knorke-gpu` (no `unsafe`). The linear HDR input is made like
//! Chiaro does without lights: the scene times the ambient light, by the composite shader from the
//! same pack (so the HDR buffer is Chiaro's own, bit for bit). Three scenes exercise the stage:
//!
//! - `bloom_aces`: fit scaling with letterbox, linear filter, exposure, ACES, bloom (4 levels), overlay;
//! - `soft_grade_crt`: integer scaling, nearest filter, soft tonemap, tint, saturation, contrast, vignette,
//!   CRT (bend, scanlines, mask, overlay on the screen), dither, bloom with snap, one shockwave;
//! - `srgb_stretch`: stretch, linear scene colours, an `_SRGB` target and overlay, bloom with a
//!   coarsest weight.
//!
//! Scenes and settings mirror `post_ref.c`; change both together. The test skips (passes with a
//! note) without a Vulkan device. `KNORKE_POST_OUT=<dir>` writes each image and an amplified diff.
#![cfg(feature = "gpu")]

use std::path::{Path, PathBuf};

use knorke_contract_tests::parity::{ParityBudget, compare_rgba8, load_png_rgba8, save_with_diff};
use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB,
    SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
    SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUTextureFormat,
};
use knorke_gpu::test_support::{texture, with_vulkan};
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, DownloadBuffer, Filter, Gpu, Load, PipelineCache,
    ProgramDesc, RenderTarget, SamplerBinding, Samplers, ShaderLibrary, StageBindings, TextureRef,
};
use knorke_math::Vec2;
use knorke_post::output_rect::ScaleMode;
use knorke_post::pass::{POST_UNSUPPORTED_TARGET, PostFrame, PostRenderer, REQUIRED_SHADERS};
use knorke_post::settings::{
    BloomSettings, ColorSpace, CrtSettings, Filter as PostFilter, PostSettings, Tonemap,
    VignetteSettings,
};
use knorke_post::shockwave::{
    Projection, Shockwave, ShockwaveFrame, ShockwaveList, collect_shockwaves,
};

const SW: u32 = 192;
const SH: u32 = 128;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/chiaro/post")
}

/// One scene: target, settings, ambient light and whether it has the wave and an sRGB target.
struct Scene {
    name: &'static str,
    target: (u32, u32),
    target_format: SDL_GPUTextureFormat,
    overlay_format: SDL_GPUTextureFormat,
    settings: PostSettings,
    ambient: f32,
    wave: bool,
}

fn scenes() -> [Scene; 3] {
    // bloom_aces
    let mut a = PostSettings::default();
    a.output.scale_mode = ScaleMode::Fit;
    a.output.filter = PostFilter::Linear;
    a.output.letterbox = [0.1, 0.2, 0.3];
    a.output.exposure_ev = 0.25;
    a.output.tonemap = Tonemap::Aces;
    a.bloom = BloomSettings {
        enabled: true,
        threshold: 1.0,
        knee: 0.5,
        intensity: 0.8,
        radius: 24.0,
    };
    // soft_grade_crt
    let mut b = PostSettings::default();
    b.output.scale_mode = ScaleMode::Integer;
    b.output.filter = PostFilter::Nearest;
    b.output.letterbox = [0.05, 0.05, 0.1];
    b.output.exposure_ev = -0.2;
    b.output.tonemap = Tonemap::Soft { knee: 0.7 };
    b.output.tint = [1.1, 1.0, 0.9];
    b.output.saturation = 1.3;
    b.output.contrast = 1.15;
    b.output.vignette = VignetteSettings {
        strength: 0.5,
        radius: 0.6,
        softness: 1.2,
        color: [0.2, 0.0, 0.0],
    };
    b.output.dither = true;
    b.bloom = BloomSettings {
        enabled: true,
        threshold: 1.5,
        knee: 0.5,
        intensity: 0.5,
        radius: 12.0,
    };
    b.crt = CrtSettings {
        overlay: true,
        curvature: 0.3,
        corner_radius: 0.15,
        scanlines: 0.4,
        scanline_period: 1.0,
        mask: 0.25,
        mask_pitch: 3.0,
        brightness: 1.3,
    };
    // srgb_stretch
    let mut c = PostSettings::default();
    c.output.scale_mode = ScaleMode::Stretch;
    c.output.filter = PostFilter::Linear;
    c.output.scene_color_space = ColorSpace::Linear;
    c.output.saturation = 0.8;
    c.bloom = BloomSettings {
        enabled: true,
        threshold: 0.5,
        knee: 0.5,
        intensity: 2.0,
        radius: 100.0,
    };
    let unorm = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;
    let srgb = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM_SRGB;
    [
        Scene {
            name: "bloom_aces",
            target: (320, 200),
            target_format: unorm,
            overlay_format: unorm,
            settings: a,
            ambient: 3.0,
            wave: false,
        },
        Scene {
            name: "soft_grade_crt",
            target: (400, 300),
            target_format: unorm,
            overlay_format: unorm,
            settings: b,
            ambient: 2.5,
            wave: true,
        },
        Scene {
            name: "srgb_stretch",
            target: (256, 256),
            target_format: srgb,
            overlay_format: srgb,
            settings: c,
            ambient: 1.5,
            wave: false,
        },
    ]
}

/// The 192x128 RGBA8 scene of `post_ref.c`.
fn scene_pixels() -> Vec<u8> {
    let mut px = vec![0u8; (SW * SH * 4) as usize];
    for y in 0..SH as i32 {
        for x in 0..SW as i32 {
            let i = ((y * SW as i32 + x) * 4) as usize;
            let mut c = [
                (x * 255 / 191) as u8,
                (y * 255 / 127) as u8,
                if (x / 16 + y / 16) & 1 == 1 { 200 } else { 40 },
                255,
            ];
            if (20..36).contains(&x) && (20..36).contains(&y) {
                c[..3].fill(255);
            }
            let (dx, dy) = (x - 150, y - 40);
            if dx * dx + dy * dy < 100 {
                c[..3].copy_from_slice(&[255, 200, 120]);
            }
            px[i..i + 4].copy_from_slice(&c);
        }
    }
    px
}

/// The premultiplied overlay of `post_ref.c`.
fn overlay_pixels(w: u32, h: u32) -> Vec<u8> {
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            if (10..110).contains(&x) && (10..40).contains(&y) {
                px[i..i + 4].copy_from_slice(&[102, 26, 26, 128]);
            } else if x >= w - 60 && x < w - 20 && y >= h - 50 && y < h - 20 {
                px[i..i + 4].copy_from_slice(&[0, 255, 0, 255]);
            }
        }
    }
    px
}

/// The composite shader of Chiaro without lights: HDR = scene (decoded) times the ambient light,
/// as `CHI_Render` makes the input of the output stage.
struct Composer<'d> {
    shaders: ShaderLibrary<'d>,
    pipelines: PipelineCache<'d>,
    samplers: Samplers<'d>,
    light: RenderTarget<'d>,
    hdr: RenderTarget<'d>,
}

impl<'d> Composer<'d> {
    const HDR: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;

    fn new(gpu: Gpu<'d>, pack: Vec<u8>) -> Self {
        let mut shaders = ShaderLibrary::new(gpu, pack).unwrap();
        let mut pipelines = PipelineCache::new(
            &shaders,
            &[ProgramDesc::new(
                "fullscreen.vert",
                "composite.frag",
                Blend::Opaque,
            )],
        )
        .unwrap();
        pipelines.get(&mut shaders, 0, Self::HDR).unwrap();
        Self {
            shaders,
            pipelines,
            samplers: Samplers::new(gpu).unwrap(),
            light: RenderTarget::new(gpu),
            hdr: RenderTarget::new(gpu),
        }
    }

    fn hdr(&self) -> TextureRef<'_> {
        self.hdr.texture_ref().unwrap()
    }

    fn compose(
        &mut self,
        cmd: &mut CommandBuffer<'_>,
        scene: TextureRef<'_>,
        ambient: f32,
        decode_srgb: bool,
    ) {
        let _ = &self.shaders;
        self.light.ensure(SW, SH, Self::HDR).unwrap();
        self.hdr.ensure(SW, SH, Self::HDR).unwrap();
        let light = self.light.texture_ref().unwrap();
        let hdr = self.hdr.texture_ref().unwrap();
        let nearest = self.samplers.sampler(Filter::Nearest);
        let linear = self.samplers.sampler(Filter::Linear);
        // Light buffer cleared to the ambient light.
        drop(
            cmd.render_pass(&[ColorTarget::new(
                light,
                Load::Clear([ambient, ambient, ambient, 0.0]),
            )
            .cycled(true)])
                .unwrap(),
        );
        // CompositeParams (160 bytes): decode sRGB, light buffer on, no glow, ambient, rest 0.
        let mut block = [0u8; 160];
        let words: [u32; 4] = [u32::from(decode_srgb), 1, 0, 0];
        for (i, w) in words.iter().enumerate() {
            block[4 * i..4 * i + 4].copy_from_slice(&w.to_le_bytes());
        }
        for (i, v) in [ambient, ambient, ambient, 1.0].iter().enumerate() {
            block[16 + 4 * i..20 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        block[36..40].copy_from_slice(&1.0f32.to_le_bytes()); // glow scale
        let pair = SamplerBinding::new;
        let samplers = [
            pair(scene, nearest),
            pair(light, linear),
            pair(scene, nearest),
            pair(scene, nearest),
            pair(scene, linear),
            pair(scene, nearest),
        ];
        let b = Bindings {
            vertex: StageBindings::default(),
            fragment: StageBindings {
                samplers: &samplers,
                uniforms: &[&block],
                ..Default::default()
            },
        };
        let pipeline = self.pipelines.pipeline(0, Self::HDR).unwrap();
        let mut pass = cmd
            .render_pass(&[ColorTarget::new(hdr, Load::DontCare).cycled(true)])
            .unwrap();
        pass.bind_program(pipeline, &b).unwrap();
        pass.draw_fullscreen().unwrap();
    }
}

fn pack() -> Vec<u8> {
    std::fs::read(data_dir().join("post.spirv.kpak")).expect("tests/data/post.spirv.kpak")
}

#[test]
fn post_pass_matches_chiaro() {
    with_vulkan(|gpu| {
        let mut renderer = PostRenderer::new(gpu, pack()).unwrap();
        let mut composer = Composer::new(gpu, pack());
        let out = std::env::var("KNORKE_POST_OUT").ok().map(PathBuf::from);
        let mut failures = Vec::new();

        let sampled = SDL_GPU_TEXTUREUSAGE_SAMPLER;
        let scene_tex = texture(
            gpu,
            (SW, SH),
            SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
            sampled,
            Some(&scene_pixels()),
        );

        for scene in scenes() {
            let (w, h) = scene.target;
            let overlay = texture(
                gpu,
                scene.target,
                scene.overlay_format,
                sampled,
                Some(&overlay_pixels(w, h)),
            );
            let target = texture(
                gpu,
                scene.target,
                scene.target_format,
                SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
                None,
            );
            // The wave of post_ref.c under the identity projection.
            let projection = Projection {
                origin: Vec2::new(0.0, 0.0),
                x_axis: Vec2::new(1.0, 0.0),
                y_axis: Vec2::new(0.0, 1.0),
                z_axis: Vec2::new(0.0, 0.0),
            };
            let mut waves = ShockwaveFrame::default();
            if scene.wave {
                let mut list = ShockwaveList::default();
                list.set(&[Shockwave {
                    x: 96.0,
                    y: 64.0,
                    radius: 30.0,
                    width: 24.0,
                    strength: 5.0,
                    ..Shockwave::default()
                }])
                .unwrap();
                collect_shockwaves(&list, &projection, SW, SH, &mut waves);
                assert_eq!(waves.distortion_count, 1);
            }

            // Frame 1 records HDR, bloom and output into the host's command buffer.
            let mut download = DownloadBuffer::new(gpu, w * h * 4).unwrap();
            let mut run = |renderer: &mut PostRenderer<'_>, read: Option<&DownloadBuffer<'_>>| {
                let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
                composer.compose(
                    &mut cmd,
                    scene_tex.texture_ref(),
                    scene.ambient,
                    scene.settings.output.scene_color_space == ColorSpace::Srgb,
                );
                let frame = PostFrame {
                    hdr: composer.hdr(),
                    overlay: Some(overlay.texture_ref()),
                    target: target.texture_ref(),
                    shockwaves: scene.wave.then_some((&projection, &waves)),
                };
                let prepared = renderer.prepare(&frame, &scene.settings).unwrap();
                renderer
                    .record(&mut cmd, &frame, &scene.settings, prepared)
                    .unwrap();
                if let Some(d) = read {
                    cmd.copy_pass()
                        .unwrap()
                        .download_texture(target.texture_ref(), d)
                        .unwrap();
                }
                cmd.submit_and_wait().unwrap();
                prepared
            };
            let prepared = run(&mut renderer, Some(&download));
            let allocations = renderer.bloom_allocations();
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            download.read(&mut pixels).unwrap();
            // Frame 2 with the same size allocates no bloom level again.
            run(&mut renderer, None);
            assert_eq!(renderer.bloom_allocations(), allocations, "{}", scene.name);
            eprintln!(
                "{}: {} bloom levels",
                scene.name,
                prepared.bloom_plan().map_or(0, |p| p.levels)
            );

            let reference = load_png_rgba8(&golden_dir().join(format!("{}.png", scene.name)), w, h);
            let strict =
                compare_rgba8(&pixels, &reference, w, h, &[], &ParityBudget::STRICT).unwrap();
            strict.print(scene.name, "STRICT");
            if !strict.within(&ParityBudget::STRICT) {
                let loose =
                    compare_rgba8(&pixels, &reference, w, h, &[], &ParityBudget::LOOSE).unwrap();
                loose.print(scene.name, "LOOSE");
                failures.push(format!("{}: {strict:?}", scene.name));
            }
            if let Some(dir) = &out {
                save_with_diff(dir, scene.name, &pixels, &reference, (w, h));
            }
        }
        assert!(failures.is_empty(), "STRICT parity missed: {failures:#?}");
    });
}

#[test]
fn prepare_refuses_bad_settings_and_unsupported_targets() {
    with_vulkan(|gpu| {
        {
            let mut renderer = PostRenderer::new(gpu, pack()).unwrap();
            let hdr = texture(
                gpu,
                (16, 16),
                Composer::HDR,
                SDL_GPU_TEXTUREUSAGE_SAMPLER,
                None,
            );
            let target = texture(
                gpu,
                (16, 16),
                SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM,
                SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
                None,
            );
            let frame = PostFrame {
                hdr: hdr.texture_ref(),
                overlay: None,
                target: target.texture_ref(),
                shockwaves: None,
            };
            // Settings are checked first, with the stable code of the field.
            let mut bad = PostSettings::default();
            bad.bloom.radius = 0.5;
            let e = renderer.prepare(&frame, &bad).unwrap_err();
            assert!(
                e.code.as_str().starts_with("knorke.post.settings."),
                "{e:?}"
            );
            // A target format the output shader cannot encode for.
            let depth = texture(
                gpu,
                (16, 16),
                knorke_gpu::sys::gpu::SDL_GPU_TEXTUREFORMAT_R8_UNORM,
                SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
                None,
            );
            let frame = PostFrame {
                target: depth.texture_ref(),
                ..frame
            };
            let e = renderer
                .prepare(&frame, &PostSettings::default())
                .unwrap_err();
            assert_eq!(e.code, POST_UNSUPPORTED_TARGET);
            // Without bloom no level texture is ever created.
            assert_eq!(renderer.bloom_allocations(), 0);
        }
    });
}

#[test]
fn pack_must_hold_the_post_shaders() {
    let pack = pack();
    let kpak = knorke_gpu::kpak::ShaderPack::parse(&pack).unwrap();
    for name in REQUIRED_SHADERS {
        assert!(kpak.find(name).is_some(), "pack lacks {name}");
    }
}
