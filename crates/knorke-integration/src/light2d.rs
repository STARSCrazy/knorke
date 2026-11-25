//! Profile `scene.render.light2d` for the GPU path: the pass plan of `knorke-light2d` lights the
//! scene into an HDR buffer, the post pass of `knorke-post` turns it into the host's target
//! (ADR 0014: the two crates do not know each other; this module joins them).
//!
//! Frame order, every step on the command buffer the host lends (ADR 0002):
//! 1. [`Light2dRenderer::prepare`]: `PassPlan::prepare` (culls lights and zones, decides every
//!    stage, creates what the frame needs), then `PostRenderer::prepare` on the plan's HDR
//!    buffer. Everything that can fail fails here.
//! 2. [`Light2dRenderer::record`]: `PassPlan::record` (occlusion, atlases, medium map, zones and lights,
//!    composite), then `PostRenderer::record` (bloom, tonemap, grade, CRT, shockwaves) into the
//!    target. The host submits and presents.
//!
//! One host setting feeds both crates: the scene colour space of the output settings
//! ([`Light2dSettings::post`], `output.scene_color_space`) is the composite's input colour space
//! as well, so scene colours are decoded exactly once.

use std::borrow::Cow;

use knorke_gpu::{CommandBuffer, Gpu, TextureRef};
use knorke_light2d::Projection;
use knorke_light2d::composite::InputColorSpace;
use knorke_light2d::frame_warnings::{Warning, WarningSet};
use knorke_light2d::medium_table::MediumTable;
use knorke_light2d::pass_plan::{PassFrame, PassPlan, PassPrepared, PassRecord, SceneStores};
use knorke_light2d::settings::LightSettings;
use knorke_post::pass::{PostFrame, PostRenderer, Prepared as PostPrepared};
use knorke_post::settings::{ColorSpace, PostSettings};
use knorke_post::shockwave::{Projection as PostProjection, ShockwaveFrame};
use knorke_types::{Diagnostic, DiagnosticCode};

use crate::layers::Layers;

/// Recording without a successful [`Light2dRenderer::prepare`].
pub const NOT_PREPARED: DiagnosticCode =
    DiagnosticCode::new("knorke.integration.light2d.not_prepared");

/// The settings of the whole frame: the light and the output.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Light2dSettings {
    /// Light, shadows, material, zones (`knorke-light2d`).
    pub light: LightSettings,
    /// Output, bloom, CRT (`knorke-post`); its `output.scene_color_space` is also the colour
    /// space the composite decodes the scene and glow colours from.
    pub post: PostSettings,
}

impl Light2dSettings {
    /// The composite's input colour space, from the one host setting.
    pub fn input_space(&self) -> InputColorSpace {
        input_space(self.post.output.scene_color_space)
    }
}

/// The light crate's name for the output's scene colour space.
pub fn input_space(space: ColorSpace) -> InputColorSpace {
    match space {
        ColorSpace::Srgb => InputColorSpace::Srgb,
        ColorSpace::Linear => InputColorSpace::Linear,
    }
}

/// One frame as the host hands it over: the scene buffers, overlay and target, all the host's.
#[derive(Clone, Copy, Debug)]
pub struct Light2dFrame<'a> {
    /// Scene colour (8-bit RGBA, `SAMPLER` usage); its size is the scene size.
    pub scene: TextureRef<'a>,
    /// Glow buffer (scene size).
    pub glow: Option<TextureRef<'a>>,
    /// Material buffer (scene size).
    pub material: Option<TextureRef<'a>>,
    /// Surface buffer (scene size).
    pub surface: Option<TextureRef<'a>>,
    /// Application sprite mask (scene size; alpha at or above 0.5 occludes).
    pub occlusion: Option<TextureRef<'a>>,
    /// Medium layer (scene size; density, height span and kind of medium per pixel). With a
    /// light it builds the medium world map that dims the lights (P29).
    pub medium: Option<TextureRef<'a>>,
    /// The media table the medium layer picks from; `None`: the presets.
    pub media: Option<&'a MediumTable>,
    /// HUD overlay over the whole target (premultiplied alpha).
    pub overlay: Option<TextureRef<'a>>,
    /// The output target (`COLOR_TARGET` usage).
    pub target: TextureRef<'a>,
    /// World to scene projection.
    pub projection: &'a Projection,
    /// Animation time in seconds, from the host's tick.
    pub time: f64,
    /// The shockwaves culled for this frame (`knorke_post::shockwave::collect_shockwaves`) with
    /// the projection they were culled under ([`post_projection`] of `projection`).
    pub shockwaves: Option<(&'a PostProjection, &'a ShockwaveFrame)>,
}

/// The output crate's copy of a projection (the crates share no type, ADR 0014).
pub fn post_projection(p: &Projection) -> PostProjection {
    PostProjection {
        origin: p.origin.raw(),
        x_axis: p.x_axis,
        y_axis: p.y_axis,
        z_axis: p.z_axis,
    }
}

impl<'a> Light2dFrame<'a> {
    /// A frame of `scene` into `target` under `projection`, without other buffers.
    pub fn new(scene: TextureRef<'a>, target: TextureRef<'a>, projection: &'a Projection) -> Self {
        Self {
            scene,
            glow: None,
            material: None,
            surface: None,
            occlusion: None,
            medium: None,
            media: None,
            overlay: None,
            target,
            projection,
            time: 0.0,
            shockwaves: None,
        }
    }

    fn pass(&self, settings: &Light2dSettings) -> PassFrame<'a> {
        PassFrame {
            scene: self.scene,
            glow: self.glow,
            material: self.material,
            surface: self.surface,
            occlusion: self.occlusion,
            medium: self.medium,
            media: self.media,
            projection: self.projection,
            time: self.time,
            input_space: settings.input_space(),
        }
    }
}

/// What [`Light2dRenderer::prepare`] decided.
#[derive(Clone, Copy, Debug)]
pub struct Light2dPrepared {
    /// The light's decision.
    pub light: PassPrepared,
    post: PostPrepared,
}

/// The pass plan and the post pass, one after the other.
#[derive(Debug)]
pub struct Light2dRenderer<'d> {
    plan: PassPlan<'d>,
    post: PostRenderer<'d>,
}

impl<'d> Light2dRenderer<'d> {
    /// Both renderers from one shader pack (`knorke-chiaro.<format>.kpak`).
    ///
    /// # Errors
    /// The pack is invalid or misses a shader of a stage.
    pub fn new(gpu: Gpu<'d>, pack: impl Into<Cow<'static, [u8]>>) -> Result<Self, Diagnostic> {
        let pack = pack.into();
        Ok(Self {
            plan: PassPlan::new(gpu, pack.clone())?,
            post: PostRenderer::new(gpu, pack)?,
        })
    }

    /// The pass plan (debug checks, inspection, invariants, stats).
    pub fn plan(&self) -> &PassPlan<'d> {
        &self.plan
    }

    /// The pass plan, mutably (debug checks, invalidate after a restore).
    pub fn plan_mut(&mut self) -> &mut PassPlan<'d> {
        &mut self.plan
    }

    /// Tells the light whether `layers` were cleared since the last frame; uncleared ones are
    /// the warning `StaleLayers` of the next [`record`](Self::record) (debug check `LAYERS`).
    /// Call it once per frame drawn from layers, before [`Layers::frame`] (the frame borrows
    /// the layers).
    pub fn note_layers(&mut self, layers: &mut Layers<'_>) {
        if layers.take_stale() {
            self.plan
                .add_pending_warnings(WarningSet::of(Warning::StaleLayers));
        }
    }

    /// The post pass.
    pub fn post(&self) -> &PostRenderer<'d> {
        &self.post
    }

    /// Prepares the light and then the output of the frame.
    ///
    /// # Errors
    /// The light's or the output's diagnostic (`knorke.light2d.*`, `knorke.post.*`).
    pub fn prepare(
        &mut self,
        frame: &Light2dFrame<'_>,
        stores: &SceneStores<'_>,
        settings: &Light2dSettings,
    ) -> Result<Light2dPrepared, Diagnostic> {
        let light = self
            .plan
            .prepare(&frame.pass(settings), stores, &settings.light)?;
        let hdr = self
            .plan
            .hdr()
            .ok_or_else(|| Diagnostic::error(NOT_PREPARED, "the pass plan has no HDR buffer"))?;
        let post = self.post.prepare(&post_frame(frame, hdr), &settings.post)?;
        Ok(Light2dPrepared { light, post })
    }

    /// Records the light and the output into the host's command buffer. `frame` and
    /// `settings` must be the ones of [`prepare`](Self::prepare).
    ///
    /// # Errors
    /// Not prepared, or a pass failed.
    pub fn record(
        &mut self,
        cmd: &mut CommandBuffer<'_>,
        frame: &Light2dFrame<'_>,
        settings: &Light2dSettings,
        prepared: Light2dPrepared,
    ) -> Result<PassRecord, Diagnostic> {
        let out = self
            .plan
            .record(cmd, &frame.pass(settings), &settings.light)?;
        let hdr = self
            .plan
            .hdr()
            .ok_or_else(|| Diagnostic::error(NOT_PREPARED, "the pass plan has no HDR buffer"))?;
        self.post
            .record(cmd, &post_frame(frame, hdr), &settings.post, prepared.post)?;
        Ok(out)
    }
}

fn post_frame<'a>(frame: &Light2dFrame<'a>, hdr: TextureRef<'a>) -> PostFrame<'a> {
    PostFrame {
        hdr,
        overlay: frame.overlay,
        target: frame.target,
        shockwaves: frame.shockwaves,
    }
}
