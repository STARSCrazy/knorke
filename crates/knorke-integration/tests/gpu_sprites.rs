//! Sprites (P34) on Vulkan: the cases of Chiaro's `tests/gpu/SpriteTests.cpp` that apply to the
//! GPU batch (material layer equals the converted map placed at the destination, also scaled
//! and mirrored; flat entries without a normal map; the batch equals single draws; errors name
//! the field; a standing sprite writes the entries of `encode_surface_sprite`), plus what the redesign adds (maps are not blended) and the checks of the
//! crate's own shaders.
//!
//! The GPU tests need a Vulkan device and the shader pack of `cargo xtask shaders` with this
//! crate's shaders in it; they skip without them (a failure with `KNORKE_REQUIRE_GPU=1`).
#![cfg(feature = "gpu")]

use std::path::PathBuf;

use knorke_gpu::kpak::source_checksum;
use knorke_gpu::test_support::{read_texture, spirv, with_chiaro_pack};
use knorke_gpu::{CommandBuffer, Gpu, Samplers, ShaderLibrary};
use knorke_integration::layers::{Layer, LayerSet, Layers};
use knorke_integration::sprites::{
    Flip, SPRITE_INVALID, SPRITE_MISSING_LAYER, SPRITE_NOT_PREPARED, Sprite, SpriteBatch,
    SpriteDesc, SpriteDraw,
};
use knorke_light2d::Projection;
use knorke_light2d::codec::{
    AlphaSource, Material, NormalMapFlags, NormalMapOptions, convert_normal_map,
};
use knorke_light2d::surface_geometry::{FaceKind, SpriteShape, SurfaceFace, encode_surface_sprite};
use knorke_math::Vec2;

const W: u32 = 32;
const H: u32 = 24;
/// Side of the test sprites.
const S: u32 = 8;
/// Height step of standing sprites.
const STEP: f32 = 0.1;

fn kinds() -> LayerSet {
    LayerSet::of(Layer::Material)
        .with(Layer::Occlusion)
        .with(Layer::Surface)
}

/// Sprite colour: every texel different, a few of them transparent.
fn color() -> Vec<u8> {
    (0..S * S)
        .flat_map(|i| {
            let (x, y) = (i % S, i / S);
            let alpha = if (x + 2 * y) % 5 == 0 { 0 } else { 255 };
            [(x * 30) as u8, (y * 30) as u8, (i * 3) as u8, alpha]
        })
        .collect()
}

/// A normal map with a different direction in every texel.
fn normal() -> Vec<u8> {
    (0..S * S)
        .flat_map(|i| [(60 + i * 2) as u8, (200 - i * 2) as u8, 230, 255])
        .collect()
}

/// What `convert_normal_map` makes of the test sprite for `material` and `flip`.
fn converted(normal: Option<&[u8]>, material: Material, flip: Flip) -> Vec<u8> {
    let flat = [128, 128, 255, 255].repeat((S * S) as usize);
    let color = color();
    let options = NormalMapOptions {
        flags: NormalMapFlags {
            flip_x: flip.x,
            flip_y: flip.y,
            ..Default::default()
        },
        material,
        alpha: Some(AlphaSource {
            rgba: &color,
            pitch: (S * 4) as usize,
        }),
        ..Default::default()
    };
    let mut out = vec![0u8; (S * S * 4) as usize];
    convert_normal_map(
        normal.unwrap_or(&flat),
        S,
        S,
        (S * 4) as usize,
        &options,
        &mut out,
    )
    .unwrap();
    out
}

/// `layer` with the entries (alpha above 0) of the `S` x `S` image `texels` placed at `at`,
/// each `scale` pixels wide.
fn place(layer: &mut [u8], texels: &[u8], at: (u32, u32), scale: u32) {
    for y in 0..S * scale {
        for x in 0..S * scale {
            let from = (((y / scale) * S + x / scale) * 4) as usize;
            let to = (((at.1 + y) * W + at.0 + x) * 4) as usize;
            if texels[from + 3] > 0 {
                layer[to..to + 4].copy_from_slice(&texels[from..from + 4]);
            }
        }
    }
}

struct Rig<'d> {
    gpu: Gpu<'d>,
    shaders: ShaderLibrary<'d>,
    samplers: Samplers<'d>,
    batch: SpriteBatch<'d>,
    layers: Layers<'d>,
}

impl<'d> Rig<'d> {
    fn new(gpu: Gpu<'d>, pack: &[u8]) -> Self {
        let shaders = ShaderLibrary::new(gpu, pack.to_vec()).unwrap();
        Self {
            gpu,
            samplers: Samplers::new(gpu).unwrap(),
            batch: SpriteBatch::new(gpu, &shaders).unwrap(),
            layers: Layers::new(gpu, W, H, kinds()).unwrap(),
            shaders,
        }
    }

    fn sprite(&self, desc: &SpriteDesc<'_>) -> Sprite<'d> {
        let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
        let sprite = Sprite::new(self.gpu, &mut cmd, desc, kinds(), STEP).unwrap();
        cmd.submit_and_wait().unwrap();
        sprite
    }

    /// Clears every layer and draws each batch of `batches` in order.
    fn draw(&mut self, batches: &[&[SpriteDraw<'_>]]) {
        self.draw_cleared(LayerSet::ALL, batches);
    }

    /// Clears `clear` and draws each batch of `batches` in order.
    fn draw_cleared(&mut self, clear: LayerSet, batches: &[&[SpriteDraw<'_>]]) {
        let mut cmd = CommandBuffer::acquire(self.gpu.device()).unwrap();
        drop(self.layers.clear_pass(&mut cmd, clear).unwrap());
        for draws in batches {
            self.batch.prepare(&mut self.shaders, draws).unwrap();
            self.batch
                .record(&mut cmd, &self.samplers, &mut self.layers, draws)
                .unwrap();
        }
        cmd.submit_and_wait().unwrap();
    }

    fn read(&self, layer: Layer) -> Vec<u8> {
        read_texture(self.gpu, self.layers.texture(layer).unwrap())
    }
}

fn empty() -> Vec<u8> {
    vec![0u8; (W * H * 4) as usize]
}

#[test]
fn the_material_layer_equals_the_converted_map_placed_at_the_destination() {
    with_chiaro_pack(|gpu, pack| {
        let mut rig = Rig::new(gpu, pack);
        let (color, normal) = (color(), normal());
        let sprite = rig.sprite(&SpriteDesc {
            normal_map: Some(&normal),
            material: Material::SATIN,
            ..SpriteDesc::new(S, S, &color)
        });
        for (scale, at) in [(1, (4, 3)), (3, (5, 0))] {
            for flip in [(false, false), (true, false), (false, true), (true, true)] {
                let flip = Flip {
                    x: flip.0,
                    y: flip.1,
                };
                let dst = [at.0, at.1, S * scale, S * scale].map(|v| v as f32);
                rig.draw(&[&[SpriteDraw {
                    flip,
                    ..SpriteDraw::new(&sprite, dst)
                }]]);
                let mut want = empty();
                place(
                    &mut want,
                    &converted(Some(&normal), Material::SATIN, flip),
                    at,
                    scale,
                );
                assert!(rig.read(Layer::Material) == want, "x{scale} {flip:?}");
            }
        }
    });
}

#[test]
fn the_colour_is_blended_by_its_alpha_and_follows_source_rectangle_and_flip() {
    with_chiaro_pack(|gpu, pack| {
        let mut rig = Rig::new(gpu, pack);
        let color = color();
        let sprite = rig.sprite(&SpriteDesc::new(S, S, &color));
        assert!(sprite.texture(Layer::Material).is_none());
        // The right half of the sprite, mirrored, at (10, 6).
        rig.draw(&[&[SpriteDraw {
            src: Some([4.0, 0.0, 4.0, 8.0]),
            flip: Flip { x: true, y: false },
            ..SpriteDraw::new(&sprite, [10.0, 6.0, 4.0, 8.0])
        }]]);
        let scene = rig.read(Layer::Scene);
        for y in 0..H {
            for x in 0..W {
                let got = &scene[((y * W + x) * 4) as usize..][..4];
                let inside = (10..14).contains(&x) && (6..14).contains(&y);
                let texel = inside.then(|| {
                    let (sx, sy) = (7 - (x - 10), y - 6);
                    &color[((sy * S + sx) * 4) as usize..][..4]
                });
                // The layer's alpha stays opaque; a transparent texel leaves the clear colour.
                let want = match texel {
                    Some(t) if t[3] == 255 => [t[0], t[1], t[2], 255],
                    _ => [0, 0, 0, 255],
                };
                assert_eq!(got, want, "({x}, {y})");
            }
        }
        // No material and no occlusion part: those layers stay empty and undrawn.
        assert!(rig.read(Layer::Material) == empty());
        assert!(rig.read(Layer::Occlusion) == empty());
    });
}

#[test]
fn maps_replace_what_lies_under_them_and_a_shadow_caster_marks_the_occlusion_layer() {
    with_chiaro_pack(|gpu, pack| {
        let mut rig = Rig::new(gpu, pack);
        let color = color();
        let dry = rig.sprite(&SpriteDesc {
            material: Material::SATIN,
            ..SpriteDesc::new(S, S, &color)
        });
        // Wet entries have alpha 224: a blend would mix them with the dry ones under them.
        let wet = rig.sprite(&SpriteDesc {
            material: Material::ASPHALT_WET,
            casts_shadow: true,
            ..SpriteDesc::new(S, S, &color)
        });
        let dst = [2.0, 2.0, 8.0, 8.0];
        let target = rig.layers.texture(Layer::Scene).unwrap();
        let projection = Default::default();
        assert!(
            rig.layers
                .frame(target, &projection, LayerSet::NONE)
                .occlusion
                .is_some()
        );

        // A frame that clears scene and material only and draws no shadow caster.
        let lit = LayerSet::of(Layer::Scene).with(Layer::Material);
        rig.draw_cleared(lit, &[&[SpriteDraw::new(&dry, dst)]]);
        let target = rig.layers.texture(Layer::Scene).unwrap();
        let frame = rig.layers.frame(target, &projection, LayerSet::NONE);
        assert!(
            frame.occlusion.is_none(),
            "nothing drew into the occlusion layer"
        );

        rig.draw(&[&[SpriteDraw::new(&dry, dst), SpriteDraw::new(&wet, dst)]]);
        let mut want = empty();
        place(
            &mut want,
            &converted(None, Material::ASPHALT_WET, Flip::default()),
            (2, 2),
            1,
        );
        let material = rig.read(Layer::Material);
        assert!(material == want);
        assert!(material.chunks(4).any(|t| t[3] == 224));

        let mut shape = empty();
        let caster: Vec<u8> = color
            .chunks(4)
            .flat_map(|c| [0, 0, 0, if c[3] >= 128 { 255 } else { 0 }])
            .collect();
        place(&mut shape, &caster, (2, 2), 1);
        assert!(rig.read(Layer::Occlusion) == shape);
        let target = rig.layers.texture(Layer::Scene).unwrap();
        assert!(
            rig.layers
                .frame(target, &projection, LayerSet::NONE)
                .occlusion
                .is_some()
        );
    });
}

#[test]
fn a_standing_sprite_writes_the_entries_of_encode_surface_sprite() {
    with_chiaro_pack(|gpu, pack| {
        let mut rig = Rig::new(gpu, pack);
        let color = color();
        let world = 10.0;
        let sprite = rig.sprite(&SpriteDesc {
            standing: true,
            standing_height: world,
            casts_shadow: true,
            ..SpriteDesc::new(S, S, &color)
        });
        assert!(sprite.texture(Layer::Material).is_none());
        assert!(sprite.texture(Layer::Surface).is_some());
        rig.draw(&[&[SpriteDraw::new(&sprite, [3.0, 2.0, 8.0, 8.0])]]);
        let s = S as f32;
        let shape = SpriteShape {
            rgba: &color,
            width: S,
            height: S,
            pitch: (S * 4) as usize,
            foot: [Vec2::new(0.0, s), Vec2::new(s, s)],
        };
        let projection = Projection {
            z_axis: Vec2::new(0.0, -s / world),
            ..Projection::IDENTITY
        };
        let mut surface = vec![0u8; (S * S * 4) as usize];
        let face = [SurfaceFace::new(FaceKind::Standing)];
        encode_surface_sprite(&shape, &face, None, &projection, STEP, &mut surface).unwrap();
        let mut want = empty();
        place(&mut want, &surface, (3, 2), 1);
        assert!(rig.read(Layer::Surface) == want);
        // Heights rise from the bottom edge: the top row stands near `world`.
        assert!(surface[7] == 255 && surface[4] > 90, "{:?}", &surface[4..8]); // texel (1, 0)
        assert!(rig.read(Layer::Material) == empty());
    });
}

#[test]
fn a_batch_equals_single_draws() {
    with_chiaro_pack(|gpu, pack| {
        let mut rig = Rig::new(gpu, pack);
        let (color, normal) = (color(), normal());
        let a = rig.sprite(&SpriteDesc {
            normal_map: Some(&normal),
            casts_shadow: true,
            ..SpriteDesc::new(S, S, &color)
        });
        let b = rig.sprite(&SpriteDesc {
            material: Material::METAL_POLISHED,
            ..SpriteDesc::new(S, S, &color)
        });
        let draws = [
            SpriteDraw::new(&a, [1.0, 1.0, 16.0, 16.0]),
            SpriteDraw {
                flip: Flip { x: true, y: true },
                ..SpriteDraw::new(&b, [9.0, 5.0, 8.0, 8.0])
            },
            SpriteDraw::new(&a, [20.0, 12.0, 8.0, 8.0]),
        ];
        let all = [Layer::Scene, Layer::Material, Layer::Occlusion];
        rig.draw(&[&draws]);
        let batched = all.map(|l| rig.read(l));
        rig.draw(&[&draws[..1], &draws[1..2], &draws[2..]]);
        assert!(all.map(|l| rig.read(l)) == batched);
        assert!(batched[1] != empty());
    });
}

#[test]
fn errors_name_the_field_and_the_missing_layer() {
    with_chiaro_pack(|gpu, pack| {
        let mut rig = Rig::new(gpu, pack);
        let color = color();
        let good = SpriteDesc::new(S, S, &color);
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        let mut refused = |desc: SpriteDesc<'_>, layers: LayerSet| {
            let e = Sprite::new(gpu, &mut cmd, &desc, layers, STEP).unwrap_err();
            (e.code, e.message)
        };
        let shiny = SpriteDesc {
            material: Material::SATIN,
            ..good
        };
        let (code, message) = refused(shiny, Layer::Occlusion.into());
        assert_eq!(code, SPRITE_MISSING_LAYER);
        assert!(message.contains("Material"), "{message}");
        let caster = SpriteDesc {
            casts_shadow: true,
            ..good
        };
        let (code, message) = refused(caster, Layer::Material.into());
        assert_eq!(code, SPRITE_MISSING_LAYER);
        assert!(message.contains("Occlusion"), "{message}");
        let standing = SpriteDesc {
            standing: true,
            ..good
        };
        let (code, message) = refused(standing, Layer::Material.into());
        assert_eq!(code, SPRITE_MISSING_LAYER);
        assert!(message.contains("Surface"), "{message}");
        for (desc, field) in [
            (SpriteDesc::new(0, S, &[]), "empty"),
            (SpriteDesc::new(S, S, &color[4..]), "color"),
            (
                SpriteDesc {
                    normal_map: Some(&color[..8]),
                    ..good
                },
                "normal_map",
            ),
            (
                SpriteDesc {
                    material: Material {
                        gloss: 1.5,
                        ..Material::SATIN
                    },
                    ..good
                },
                "material.gloss",
            ),
            (
                SpriteDesc {
                    standing_height: -1.0,
                    ..good
                },
                "standing_height",
            ),
        ] {
            let (code, message) = refused(desc, kinds());
            assert_eq!(code, SPRITE_INVALID, "{field}");
            assert!(message.contains(field), "{field}: {message}");
        }
        let sprite = Sprite::new(gpu, &mut cmd, &good, kinds(), STEP).unwrap();
        cmd.submit_and_wait().unwrap();

        let draws = [
            SpriteDraw::new(&sprite, [0.0, 0.0, 8.0, 8.0]),
            SpriteDraw {
                src: Some([4.0, 0.0, 5.0, 8.0]),
                ..SpriteDraw::new(&sprite, [0.0, 0.0, 8.0, 8.0])
            },
        ];
        let e = rig.batch.prepare(&mut rig.shaders, &draws).unwrap_err();
        assert_eq!(e.code, SPRITE_INVALID);
        assert!(e.message.contains("draws[1].src"), "{}", e.message);
        let mut cmd = CommandBuffer::acquire(gpu.device()).unwrap();
        let e = rig
            .batch
            .record(&mut cmd, &rig.samplers, &mut rig.layers, &draws)
            .unwrap_err();
        assert_eq!(e.code, SPRITE_NOT_PREPARED);
        cmd.submit_and_wait().unwrap();
    });
}

// ---- the crate's own shaders ------------------------------------------------------------------

fn shader_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("shaders")
        .join(sub)
}

/// The committed blobs were compiled from the committed sources
/// (`cargo xtask shaders --compile crates/knorke-integration/shaders`). `chiaro/` holds the
/// Chiaro programs still in use as Chiaro's shadercross blobs; recompiling them with DXC
/// changes some SPIR-V and drops their MSL, so a change there is a deliberate step.
#[test]
fn compiled_shaders_are_up_to_date() {
    for dir in ["", "chiaro/"] {
        let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(shader_dir(&format!("{dir}src")))
            .unwrap()
            .map(|e| e.unwrap().path())
            .map(|p| {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                (name, std::fs::read(&p).unwrap())
            })
            .collect();
        files.sort();
        let sum = source_checksum(files.iter().map(|(n, b)| (n.as_str(), b.as_slice())));
        let recorded =
            std::fs::read_to_string(shader_dir(&format!("{dir}compiled")).join("sources.sum"))
                .unwrap();
        assert_eq!(format!("{sum:016x}"), recorded.trim(), "shaders/{dir}");
    }
}

/// The hand-written reflection files match what the SPIR-V declares, at the descriptor sets
/// SDL expects (vertex resources in set 0, vertex uniforms in 1, fragment resources in 2).
#[test]
fn reflection_files_match_the_spirv() {
    let bindings = |name: &str| {
        let code = std::fs::read(shader_dir("compiled").join(format!("{name}.spv"))).unwrap();
        let json = std::fs::read_to_string(shader_dir("compiled").join(format!("{name}.json")));
        let found: Vec<(String, (u32, u32))> = spirv::decode(&code).bindings.into_iter().collect();
        (found, json.unwrap())
    };
    let (vert, json) = bindings("sprite.vert");
    let named: Vec<(&str, (u32, u32))> = vert.iter().map(|(n, b)| (n.as_str(), *b)).collect();
    assert_eq!(named, [("Quads", (0, 0)), ("SpriteParams", (1, 0))]);
    assert!(json.contains("\"storage_buffers\": 1") && json.contains("\"uniform_buffers\": 1"));
    for name in ["sprite.frag", "sprite_map.frag"] {
        let (frag, json) = bindings(name);
        let named: Vec<(&str, (u32, u32))> = frag.iter().map(|(n, b)| (n.as_str(), *b)).collect();
        assert_eq!(
            named,
            [("Sprite", (2, 0)), ("SpriteSampler", (2, 0))],
            "{name}"
        );
        assert!(json.contains("\"samplers\": 1") && json.contains("\"uniform_buffers\": 0"));
    }
}
