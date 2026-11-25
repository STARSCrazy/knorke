// Origin: chiaro @ 9bc5b9b, src/integration/Layers.hpp and Layers.cpp (the GPU layers:
// CreateTextures, ColorTargets, Fill, ClearColor, Stale / MarkRendered) and CHI_LayerKind of
// include/Chiaro/Chiaro.h.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust redesign) and this notice may not be
// removed or altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Scene layers (P33): the buffers a 2D frame is drawn into, owned in one place, in the format
//! the light expects and cleared to their empty values.
//!
//! The host draws its frame into the layers and hands them over as a [`Light2dFrame`]:
//!
//! 1. [`Layers::clear_pass`] opens a render pass on up to four layers, cleared; the host draws
//!    in it. More layers take further passes.
//! 2. [`Light2dRenderer::note_layers`](crate::light2d::Light2dRenderer::note_layers) reports
//!    layers that were not cleared since the last frame (debug check `LAYERS`).
//! 3. [`Layers::frame`] fills a frame from the layers; a field set afterwards overrides its
//!    layer.
//!
//! # Redesign against Chiaro's `Layers`
//!
//! - GPU layers only: no `SDL_Renderer` layers (ADR 0014), so no Begin/End and no queue.
//! - No submit: clearing is a render pass on the host's command buffer (ADR 0002).
//! - The layers do not know a renderer. A frame is a value built from them, so "a buffer entry
//!   overrides its layer" is struct update syntax and `skip_layers` is an argument.
//! - Sprites, surface geometry and medium clouds are not methods here; they draw into the
//!   layers from outside (P34, P35).

use knorke_gpu::sys::gpu::{
    SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM, SDL_GPU_TEXTUREUSAGE_COLOR_TARGET,
    SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUTextureFormat,
};
use knorke_gpu::{
    ColorTarget, CommandBuffer, Gpu, Load, MAX_COLOR_TARGETS, RenderPass, Texture, TextureDesc,
    TextureRef,
};
use knorke_light2d::Projection;
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::light2d::Light2dFrame;

/// Format of every layer.
pub const LAYER_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R8G8B8A8_UNORM;

/// [`Layers::clear_pass`] with no created layer among the kinds, or with more than a render
/// pass takes.
pub const LAYER_TARGETS: DiagnosticCode = DiagnosticCode::new("knorke.integration.layers.targets");

/// One layer (`CHI_LayerKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    /// The scene colour; always created. Empty: opaque black.
    Scene,
    /// Height and world normal. Empty: 0.
    Surface,
    /// Detail normal and material. Empty: 0.
    Material,
    /// Self-lighting. Empty: 0.
    Glow,
    /// Sprite shadow casters. Empty: 0.
    Occlusion,
    /// Unlit HUD over the result. Empty: 0.
    Overlay,
    /// Density, height span and kind of medium. Empty: 0. No stage reads it yet (P29).
    Medium,
}

impl Layer {
    /// Every layer, in the order of [`LayerSet`] bits and of [`Layers::clear_pass`] targets.
    pub const ALL: [Layer; 7] = [
        Self::Scene,
        Self::Surface,
        Self::Material,
        Self::Glow,
        Self::Occlusion,
        Self::Overlay,
        Self::Medium,
    ];

    /// The value an empty layer holds.
    pub const fn clear_color(self) -> [f32; 4] {
        match self {
            Self::Scene => [0.0, 0.0, 0.0, 1.0],
            _ => [0.0; 4],
        }
    }
}

/// A set of layers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LayerSet(u8);

impl LayerSet {
    /// No layer.
    pub const NONE: Self = Self(0);
    /// Every layer.
    pub const ALL: Self = Self(0x7F);

    /// Only `layer`.
    pub const fn of(layer: Layer) -> Self {
        Self(1 << layer as u8)
    }

    /// `self` and `layer`.
    #[must_use]
    pub const fn with(self, layer: Layer) -> Self {
        Self(self.0 | Self::of(layer).0)
    }

    /// Whether `layer` is in the set.
    pub const fn contains(self, layer: Layer) -> bool {
        self.0 & Self::of(layer).0 != 0
    }
}

impl From<Layer> for LayerSet {
    fn from(layer: Layer) -> Self {
        Self::of(layer)
    }
}

impl core::ops::BitOr for LayerSet {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// The scene layers of one size (see the [module docs](self)).
#[derive(Debug)]
pub struct Layers<'d> {
    gpu: Gpu<'d>,
    textures: [Option<Texture<'d>>; 7],
    /// Layers cleared or marked since the first clear after the last frame; all before the
    /// first clear, when their content is unknown.
    drawn: LayerSet,
    /// A layer was cleared since the last [`take_stale`](Self::take_stale).
    cleared: bool,
}

fn create<'d>(
    gpu: Gpu<'d>,
    width: u32,
    height: u32,
    kinds: LayerSet,
) -> Result<[Option<Texture<'d>>; 7], Diagnostic> {
    let desc = TextureDesc::new(
        width,
        height,
        LAYER_FORMAT,
        SDL_GPU_TEXTUREUSAGE_COLOR_TARGET | SDL_GPU_TEXTUREUSAGE_SAMPLER,
    );
    let mut out = [const { None }; 7];
    for layer in Layer::ALL {
        if kinds.contains(layer) {
            out[layer as usize] = Some(Texture::new(gpu, desc)?);
        }
    }
    Ok(out)
}

impl<'d> Layers<'d> {
    /// Layers of `width` x `height` texels: the scene and those of `kinds`.
    ///
    /// # Errors
    /// A texture could not be created (also: a zero size).
    pub fn new(gpu: Gpu<'d>, width: u32, height: u32, kinds: LayerSet) -> Result<Self, Diagnostic> {
        Ok(Self {
            gpu,
            textures: create(gpu, width, height, kinds.with(Layer::Scene))?,
            drawn: LayerSet::ALL,
            cleared: false,
        })
    }

    /// Recreates every layer at the new size; their content is gone.
    ///
    /// # Errors
    /// As [`new`](Self::new); the layers then keep their size and content.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), Diagnostic> {
        self.textures = create(self.gpu, width, height, self.kinds())?;
        self.drawn = LayerSet::ALL;
        Ok(())
    }

    /// The layers that exist.
    pub fn kinds(&self) -> LayerSet {
        Layer::ALL
            .into_iter()
            .filter(|l| self.textures[*l as usize].is_some())
            .fold(LayerSet::NONE, LayerSet::with)
    }

    /// The texture of `layer`, `None` if it was not created.
    pub fn texture(&self, layer: Layer) -> Option<TextureRef<'_>> {
        self.textures[layer as usize]
            .as_ref()
            .map(Texture::texture_ref)
    }

    /// Opens a render pass on the created layers among `kinds`, in [`Layer::ALL`] order, each
    /// cleared to its empty value. The first clear after a frame starts a new one.
    ///
    /// # Errors
    /// None of `kinds` exists or more than [`MAX_COLOR_TARGETS`] do ([`LAYER_TARGETS`]), or the
    /// pass could not be opened.
    pub fn clear_pass<'c>(
        &mut self,
        cmd: &'c mut CommandBuffer<'_>,
        kinds: LayerSet,
    ) -> Result<RenderPass<'c>, Diagnostic> {
        let mut found = Layer::ALL.into_iter().filter_map(|layer| {
            let t = self.texture(layer).filter(|_| kinds.contains(layer))?;
            Some(ColorTarget::new(t, Load::Clear(layer.clear_color())))
        });
        let wrong = |n: usize| {
            Diagnostic::error(
                LAYER_TARGETS,
                format!("a layer pass takes 1 to {MAX_COLOR_TARGETS} created layers, got {n}"),
            )
        };
        let mut targets = [found.next().ok_or_else(|| wrong(0))?; MAX_COLOR_TARGETS];
        let mut n = 1;
        for target in found {
            if let Some(slot) = targets.get_mut(n) {
                *slot = target;
            }
            n += 1;
        }
        if n > MAX_COLOR_TARGETS {
            return Err(wrong(n));
        }
        let pass = cmd.render_pass(&targets[..n])?;
        let targeted = LayerSet(kinds.0 & self.kinds().0);
        self.drawn = if self.cleared {
            self.drawn | targeted
        } else {
            targeted
        };
        self.cleared = true;
        Ok(pass)
    }

    /// Records that the host wrote `kinds` in another way than through
    /// [`clear_pass`](Self::clear_pass) (an upload, a pass of its own).
    pub fn mark_drawn(&mut self, kinds: LayerSet) {
        self.drawn = self.drawn | kinds;
    }

    /// A frame into `target` with every layer in its slot, except those in `skip` and an
    /// occlusion layer nothing was drawn into since its clear: an empty one gives the same
    /// image, but as a sprite mask it would rebuild the shadow field every frame (E79).
    /// The scene is never skipped.
    pub fn frame<'a>(
        &'a self,
        target: TextureRef<'a>,
        projection: &'a Projection,
        skip: LayerSet,
    ) -> Light2dFrame<'a> {
        let slot = |layer: Layer| self.texture(layer).filter(|_| !skip.contains(layer));
        let scene = self
            .texture(Layer::Scene)
            .expect("the scene layer always exists");
        Light2dFrame {
            glow: slot(Layer::Glow),
            material: slot(Layer::Material),
            surface: slot(Layer::Surface),
            occlusion: slot(Layer::Occlusion).filter(|_| self.drawn.contains(Layer::Occlusion)),
            overlay: slot(Layer::Overlay),
            ..Light2dFrame::new(scene, target, projection)
        }
    }

    /// Whether no layer was cleared since the last call (or ever): the layers still hold the
    /// previous frame. Call it once per rendered frame.
    pub fn take_stale(&mut self) -> bool {
        !core::mem::take(&mut self.cleared)
    }
}
