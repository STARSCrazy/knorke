// Origin: chiaro @ 9bc5b9b, src/integration/Sprite.hpp and Sprite.cpp (CreateSprite: which maps
// a sprite gets and how they are converted; DrawSprites: the validation, the layer order and the
// pre-mirrored material variants) and CHI_SpriteDesc / CHI_SpriteDraw of include/Chiaro/Chiaro.h.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust redesign) and this notice may not be
// removed or altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Sprites (P34): a colour texture and its maps, drawn together into the
//! [`Layers`](crate::layers::Layers).
//!
//! A [`Sprite`] converts its maps once on the CPU and uploads them once. A [`SpriteBatch`]
//! draws sprites: the colour into the scene layer (blended by its alpha), the material into the
//! material layer and the shadow shape into the occlusion layer, all at the same rectangle.
//!
//! # Redesign against Chiaro
//!
//! - Chiaro drew sprites with `SDL_Renderer` and left GPU layers to the application. Here the
//!   batch draws on the GPU with Knorke's own shaders (`shaders/src/sprite*.hlsl`), into the
//!   host's command buffer (ADR 0002).
//! - Map texels are written exactly where the sprite has an entry and not blended, so a wet
//!   material (alpha below 255) keeps its bytes. Chiaro's blend mixed such texels.
//! - Standing sprites (a surface entry) come with the surface helpers (P35).

use knorke_gpu::sys::gpu::{
    SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, SDL_GPU_TEXTUREUSAGE_SAMPLER,
    SDL_GPUBufferUsageFlags,
};
use knorke_gpu::{
    Bindings, Blend, CommandBuffer, Filter, Gpu, PipelineCache, ProgramDesc, SamplerBinding,
    Samplers, ShaderLibrary, StageBindings, Texture, TextureDesc, TextureRef, UploadBuffer,
};
use knorke_light2d::codec::{
    AlphaSource, Material, NormalMapFlags, NormalMapOptions, convert_normal_map,
};
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::layers::{LAYER_FORMAT, Layer, LayerSet, Layers};

/// A sprite description or a draw is invalid; the message names the field.
pub const SPRITE_INVALID: DiagnosticCode = DiagnosticCode::new("knorke.integration.sprite.invalid");
/// A sprite needs a layer the layers were created without.
pub const SPRITE_MISSING_LAYER: DiagnosticCode =
    DiagnosticCode::new("knorke.integration.sprite.missing_layer");
/// [`SpriteBatch::record`] without a successful [`SpriteBatch::prepare`] of the same draws.
pub const SPRITE_NOT_PREPARED: DiagnosticCode =
    DiagnosticCode::new("knorke.integration.sprite.not_prepared");

fn invalid(message: String) -> Diagnostic {
    Diagnostic::error(SPRITE_INVALID, message)
}

/// What a sprite is made of. Every image is `width * height` RGBA pixels, tightly packed, and
/// is read during [`Sprite::new`] only.
#[derive(Clone, Copy, Debug)]
pub struct SpriteDesc<'a> {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The colour, drawn into the scene layer blended by its alpha. Its alpha is the shape of
    /// every map (128 and above).
    pub color: &'a [u8],
    /// RGB = `n * 0.5 + 0.5`, green up unless `green_down`. Needs the material layer.
    pub normal_map: Option<&'a [u8]>,
    /// Greyscale (R is read): scales `material.specular` per pixel, 255 = 1.
    pub specular_map: Option<&'a [u8]>,
    /// Constant material. A sprite without a normal map gets a material entry (flat normal)
    /// only if this is not matte or a specular map is given.
    pub material: Material,
    /// The normal map has y pointing down (DirectX style).
    pub green_down: bool,
    /// Writes its alpha shape into the occlusion layer. Needs the occlusion layer.
    pub casts_shadow: bool,
}

impl<'a> SpriteDesc<'a> {
    /// A matte sprite of `color` without maps.
    pub fn new(width: u32, height: u32, color: &'a [u8]) -> Self {
        Self {
            width,
            height,
            color,
            normal_map: None,
            specular_map: None,
            material: Material::MATTE,
            green_down: false,
            casts_shadow: false,
        }
    }
}

/// Mirroring of a draw. Mirrored material normals are converted at creation, so flips stay
/// exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Flip {
    /// Mirror left and right.
    pub x: bool,
    /// Mirror top and bottom.
    pub y: bool,
}

impl Flip {
    const fn index(self) -> usize {
        self.x as usize | (self.y as usize) << 1
    }
}

/// A sprite: textures for the colour and each map.
#[derive(Debug)]
pub struct Sprite<'d> {
    size: (u32, u32),
    color: Texture<'d>,
    /// One pre-mirrored variant per [`Flip`], or none.
    material: Option<[Texture<'d>; 4]>,
    occlusion: Option<Texture<'d>>,
}

/// Creates a sampled texture of `pixels` and records its upload on `cmd`.
fn upload<'d>(
    gpu: Gpu<'d>,
    cmd: &mut CommandBuffer<'_>,
    (w, h): (u32, u32),
    pixels: &[u8],
) -> Result<Texture<'d>, Diagnostic> {
    let desc = TextureDesc::new(w, h, LAYER_FORMAT, SDL_GPU_TEXTUREUSAGE_SAMPLER);
    let texture = Texture::new(gpu, desc)?;
    let mut staging = UploadBuffer::new(
        gpu,
        SDL_GPUBufferUsageFlags(0),
        pixels.len() as u32,
        "sprite",
    );
    staging.reserve(pixels.len() as u64)?;
    staging.map()?.bytes()[..pixels.len()].copy_from_slice(pixels);
    cmd.copy_pass()?
        .upload_to_texture(&staging, texture.texture_ref(), false)?;
    // SDL keeps the staging buffer alive until `cmd` has run.
    Ok(texture)
}

impl<'d> Sprite<'d> {
    /// Converts the maps and records the uploads on `cmd`; the sprite can be drawn in command
    /// buffers submitted after it. `layers` are the kinds of the layers it will be drawn into.
    ///
    /// # Errors
    /// [`SPRITE_INVALID`] for an empty sprite, an image of the wrong length or a material out
    /// of range, [`SPRITE_MISSING_LAYER`] for a map whose layer `layers` lacks, or a texture
    /// could not be created.
    pub fn new(
        gpu: Gpu<'d>,
        cmd: &mut CommandBuffer<'_>,
        desc: &SpriteDesc<'_>,
        layers: LayerSet,
    ) -> Result<Self, Diagnostic> {
        let size = (desc.width, desc.height);
        let pixels = desc.width as usize * desc.height as usize;
        if pixels == 0 {
            return Err(invalid(format!(
                "a sprite must not be empty (got {}x{})",
                desc.width, desc.height
            )));
        }
        for (name, image) in [
            ("color", Some(desc.color)),
            ("normal_map", desc.normal_map),
            ("specular_map", desc.specular_map),
        ] {
            if let Some(image) = image
                && image.len() != pixels * 4
            {
                return Err(invalid(format!(
                    "{name} has {} bytes, but a {}x{} sprite needs {}",
                    image.len(),
                    desc.width,
                    desc.height,
                    pixels * 4
                )));
            }
        }
        let m = desc.material;
        if !((0.0..=1.0).contains(&m.specular) && (0.0..=1.0).contains(&m.gloss)) {
            return Err(invalid(format!(
                "material.specular and material.gloss must be in [0, 1] (got {}, {})",
                m.specular, m.gloss
            )));
        }
        let has_material =
            desc.normal_map.is_some() || desc.specular_map.is_some() || m.specular > 0.0;
        for (needed, layer, what) in [
            (
                has_material,
                Layer::Material,
                "a material entry (normal_map, specular_map or material.specular > 0)",
            ),
            (desc.casts_shadow, Layer::Occlusion, "casts_shadow"),
        ] {
            if needed && !layers.contains(layer) {
                return Err(Diagnostic::error(
                    SPRITE_MISSING_LAYER,
                    format!("{what} needs the {layer:?} layer"),
                ));
            }
        }

        let color = upload(gpu, cmd, size, desc.color)?;
        let mut scratch = vec![0u8; pixels * 4];
        let material = if has_material {
            let flat = [128, 128, 255, 255].repeat(pixels);
            let specular: Option<Vec<u8>> = desc.specular_map.map(|grey| {
                grey.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|p| (f32::from(p[0]) * m.specular).round() as u8)
                    .collect()
            });
            let mut variant = |flip: usize| {
                let options = NormalMapOptions {
                    flags: NormalMapFlags {
                        green_down: desc.green_down,
                        flip_x: flip & 1 != 0,
                        flip_y: flip & 2 != 0,
                    },
                    material: m,
                    specular_map: specular.as_deref(),
                    alpha: Some(AlphaSource {
                        rgba: desc.color,
                        pitch: desc.width as usize * 4,
                    }),
                    ..Default::default()
                };
                convert_normal_map(
                    desc.normal_map.unwrap_or(&flat),
                    desc.width,
                    desc.height,
                    desc.width as usize * 4,
                    &options,
                    &mut scratch,
                )
                .map_err(|e| e.to_diagnostic())?;
                upload(gpu, cmd, size, &scratch)
            };
            Some([variant(0)?, variant(1)?, variant(2)?, variant(3)?])
        } else {
            None
        };
        let occlusion = if desc.casts_shadow {
            let texels = scratch.as_chunks_mut::<4>().0.iter_mut();
            for (out, c) in texels.zip(desc.color.as_chunks::<4>().0) {
                *out = [0, 0, 0, if c[3] >= 128 { 255 } else { 0 }];
            }
            Some(upload(gpu, cmd, size, &scratch)?)
        } else {
            None
        };
        Ok(Self {
            size,
            color,
            material,
            occlusion,
        })
    }

    /// Size in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The texture the sprite draws into `layer` (the material unflipped), `None` for a part it
    /// does not have.
    pub fn texture(&self, layer: Layer) -> Option<TextureRef<'_>> {
        self.part(layer, Flip::default())
    }

    fn part(&self, layer: Layer, flip: Flip) -> Option<TextureRef<'_>> {
        match layer {
            Layer::Scene => Some(&self.color),
            Layer::Material => self.material.as_ref().map(|m| &m[flip.index()]),
            Layer::Occlusion => self.occlusion.as_ref(),
            _ => None,
        }
        .map(Texture::texture_ref)
    }
}

/// One draw of a [`SpriteBatch`].
#[derive(Clone, Copy, Debug)]
pub struct SpriteDraw<'a> {
    /// The sprite.
    pub sprite: &'a Sprite<'a>,
    /// Part of the sprite as `[x, y, w, h]` in sprite pixels; `None`: the whole sprite.
    pub src: Option<[f32; 4]>,
    /// Where it goes, as `[x, y, w, h]` in scene pixels.
    pub dst: [f32; 4],
    /// Mirroring.
    pub flip: Flip,
}

impl<'a> SpriteDraw<'a> {
    /// The whole sprite at `dst`, not mirrored.
    pub fn new(sprite: &'a Sprite<'a>, dst: [f32; 4]) -> Self {
        Self {
            sprite,
            src: None,
            dst,
            flip: Flip::default(),
        }
    }
}

/// The layers a batch draws into, in draw order, with the program of each.
const TARGETS: [(Layer, usize); 3] = [
    (Layer::Scene, 0),
    (Layer::Material, 1),
    (Layer::Occlusion, 1),
];

/// Bytes of one quad in the storage buffer (`struct Quad` of `sprite.vert.hlsl`).
const QUAD_BYTES: usize = 32;
/// Quads per draw: the one of the colour and the occlusion (mirrored by its texture
/// coordinates), then the one of the material (its texture is mirrored already).
const QUADS_PER_DRAW: usize = 2;

/// Draws sprites into [`Layers`]. Per batch [`prepare`](Self::prepare) (may fail), then
/// [`record`](Self::record) into the host's command buffer. Steady state: no allocation.
#[derive(Debug)]
pub struct SpriteBatch<'d> {
    pipelines: PipelineCache<'d>,
    quads: UploadBuffer<'d>,
    /// Number of draws of the last successful prepare.
    prepared: Option<usize>,
}

impl<'d> SpriteBatch<'d> {
    /// Resolves the sprite shaders in `shaders`; fails if the pack lacks one.
    pub fn new(gpu: Gpu<'d>, shaders: &ShaderLibrary<'d>) -> Result<Self, Diagnostic> {
        let programs = [
            ProgramDesc::new("sprite.vert", "sprite.frag", Blend::AlphaRgb),
            ProgramDesc::new("sprite.vert", "sprite_map.frag", Blend::Opaque),
        ];
        Ok(Self {
            pipelines: PipelineCache::new(shaders, &programs)?,
            quads: UploadBuffer::new(
                gpu,
                SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ,
                (64 * QUADS_PER_DRAW * QUAD_BYTES) as u32,
                "sprite quads",
            ),
            prepared: None,
        })
    }

    /// Validates `draws`, fills the quad buffer and creates the pipelines.
    ///
    /// # Errors
    /// [`SPRITE_INVALID`] for a source rectangle outside its sprite (the message names the
    /// draw), or the buffer or a pipeline could not be created. Nothing is drawn then.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        draws: &[SpriteDraw<'_>],
    ) -> Result<(), Diagnostic> {
        self.prepared = None;
        for (i, d) in draws.iter().enumerate() {
            let (w, h) = (d.sprite.size.0 as f32, d.sprite.size.1 as f32);
            if let Some([x, y, sw, sh]) = d.src
                && !(sw > 0.0 && sh > 0.0 && x >= 0.0 && y >= 0.0 && x + sw <= w && y + sh <= h)
            {
                return Err(invalid(format!(
                    "draws[{i}].src ({x}, {y}, {sw}, {sh}) must lie inside the {w}x{h} sprite"
                )));
            }
        }
        for program in 0..2 {
            self.pipelines.get(shaders, program, LAYER_FORMAT)?;
        }
        if draws.is_empty() {
            self.prepared = Some(0);
            return Ok(());
        }
        self.quads
            .reserve((draws.len() * QUADS_PER_DRAW * QUAD_BYTES) as u64)?;
        let mut mapped = self.quads.map()?;
        let out = mapped.bytes();
        for (d, quads) in draws
            .iter()
            .zip(&mut out.as_chunks_mut::<{ QUADS_PER_DRAW * QUAD_BYTES }>().0[..])
        {
            let (w, h) = (d.sprite.size.0 as f32, d.sprite.size.1 as f32);
            let [x, y, sw, sh] = d.src.unwrap_or([0.0, 0.0, w, h]);
            let (mut u, mut v) = ([x / w, (x + sw) / w], [y / h, (y + sh) / h]);
            // The material variant is mirrored already: unflipped from the mirrored rectangle.
            let mirrored = [
                if d.flip.x { 1.0 - u[1] } else { u[0] },
                if d.flip.y { 1.0 - v[1] } else { v[0] },
                if d.flip.x { 1.0 - u[0] } else { u[1] },
                if d.flip.y { 1.0 - v[0] } else { v[1] },
            ];
            if d.flip.x {
                u.swap(0, 1);
            }
            if d.flip.y {
                v.swap(0, 1);
            }
            let floats = d
                .dst
                .into_iter()
                .chain([u[0], v[0], u[1], v[1]])
                .chain(d.dst)
                .chain(mirrored);
            for (bytes, f) in quads.as_chunks_mut::<4>().0.iter_mut().zip(floats) {
                *bytes = f.to_le_bytes();
            }
        }
        self.prepared = Some(draws.len());
        Ok(())
    }

    /// Records the prepared `draws` into `cmd`: the quad upload, then one render pass per layer
    /// that a sprite writes and `layers` has (scene, material, occlusion), each in draw order.
    /// A layer drawn into counts as drawn.
    ///
    /// # Errors
    /// Not prepared for these draws ([`SPRITE_NOT_PREPARED`]), or a pass or binding failed.
    pub fn record(
        &mut self,
        cmd: &mut CommandBuffer<'_>,
        samplers: &Samplers<'_>,
        layers: &mut Layers<'_>,
        draws: &[SpriteDraw<'_>],
    ) -> Result<(), Diagnostic> {
        if self.prepared.take() != Some(draws.len()) {
            return Err(Diagnostic::error(
                SPRITE_NOT_PREPARED,
                "the sprite batch was not prepared for these draws",
            ));
        }
        if draws.is_empty() {
            return Ok(());
        }
        let missing =
            |what: &str| Diagnostic::error(SPRITE_NOT_PREPARED, format!("{what} is missing"));
        cmd.copy_pass()?.upload_to_buffer(
            &self.quads,
            (draws.len() * QUADS_PER_DRAW * QUAD_BYTES) as u32,
        )?;
        let quads = self
            .quads
            .buffer_ref()
            .ok_or_else(|| missing("the quad buffer"))?;
        for (layer, program) in TARGETS {
            let Some(target) = layers.texture(layer) else {
                continue;
            };
            let size = [target.width() as f32, target.height() as f32];
            if !draws.iter().any(|d| d.sprite.part(layer, d.flip).is_some()) {
                continue;
            }
            let pipeline = self
                .pipelines
                .pipeline(program, LAYER_FORMAT)
                .ok_or_else(|| missing("a sprite pipeline"))?;
            // Colour is filtered, maps hold codes and are not.
            let filter = if layer == Layer::Scene {
                Filter::Linear
            } else {
                Filter::Nearest
            };
            let mut pass = layers.draw_pass(cmd, layer)?;
            for (i, d) in draws.iter().enumerate() {
                let Some(texture) = d.sprite.part(layer, d.flip) else {
                    continue;
                };
                let quad = i * QUADS_PER_DRAW + usize::from(layer == Layer::Material);
                let mut params = [0u8; 16];
                let values = [size[0], size[1], quad as f32, 0.0];
                for (bytes, f) in params.as_chunks_mut::<4>().0.iter_mut().zip(values) {
                    *bytes = f.to_le_bytes();
                }
                pass.bind_program(
                    pipeline,
                    &Bindings {
                        vertex: StageBindings {
                            storage_buffers: &[quads],
                            uniforms: &[&params],
                            ..Default::default()
                        },
                        fragment: StageBindings {
                            samplers: &[SamplerBinding::new(texture, samplers.sampler(filter))],
                            ..Default::default()
                        },
                    },
                )?;
                pass.draw(6, 1, 0, 0)?;
            }
        }
        Ok(())
    }
}
