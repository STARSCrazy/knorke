// Origin: chiaro @ 9bc5b9b, src/rendering/OcclusionMask.hpp and OcclusionMask.cpp (Begin, Plan,
// PreparePipelines, RecordMask, RecordShiftCopy, Commit), src/rendering/SdfShadows.hpp and
// SdfShadows.cpp (Prepare, Record, RecordPasses, FieldPass, DrawTexelRects) and
// src/gpu/PipelineCache.cpp (the program table entries MaskClearRects, MaskClearRectsHeight,
// JfaSeed, JfaStep, JfaDistance, JfaSeedRects, JfaStepRects, JfaDistanceRects, OcclusionMask,
// OcclusionMaskHeight); the sprite mask of OcclusionMask.cpp (PreparePipelines, DrawSources,
// SpriteParams).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The GPU stage: mask, height and field textures as ping-pong pairs, the seed targets, and the
//! passes that bring them up to date. Ownership follows ADR 0002: the host lends the command
//! buffer; nothing here submits, presents or reads back.

use super::{JfaParams, RectParams, ShiftCopy, UPDATE_LIMITS, jfa_steps};
use crate::occluder_raster::{
    DomainParams, HEIGHT_FORMAT, INFINITE_TOP, MASK_FORMAT, OccluderGeometry, TileOccluders,
};
use crate::shadow::{DirtyRectList, MaskPlanInput, MaskUpdate, MaskUpdatePlan, OcclusionDomain};
use knorke_gpu::sys::gpu::{
    SDL_GPU_PRIMITIVETYPE_TRIANGLELIST, SDL_GPU_TEXTUREFORMAT_R16_FLOAT,
    SDL_GPU_TEXTUREFORMAT_R32G32_FLOAT, SDL_GPUTextureFormat,
};
use knorke_gpu::sys::rect::SDL_Rect;
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, Gpu, Load, PipelineCache, PipelineRef,
    ProgramDesc, ProgramIndex, RenderTarget, SamplerBinding, SamplerRef, SecondTarget,
    ShaderLibrary, StageBindings, TextureRef,
};
use knorke_math::uniform::write_f32s;
use knorke_types::{Diagnostic, DiagnosticCode};

/// Recording without a successful [`OcclusionMask::prepare`] since the last commit.
pub const NOT_PREPARED: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.occlusion.not_prepared");

/// Format of the jump flooding seeds: the texel coordinate of the nearest seed, `-1` for none.
pub const SEED_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R32G32_FLOAT;
/// Format of the distance field: texels to the nearest obstacle texel, clamped to
/// [`FIELD_BOUND`](super::FIELD_BOUND).
pub const FIELD_FORMAT: SDL_GPUTextureFormat = SDL_GPU_TEXTUREFORMAT_R16_FLOAT;

const fn program(vertex: &'static str, fragment: &'static str) -> ProgramDesc {
    ProgramDesc {
        vertex,
        fragment,
        blend: Blend::Opaque,
        second: None,
        primitive: SDL_GPU_PRIMITIVETYPE_TRIANGLELIST,
    }
}

/// The programs of the stage, to be placed in the host's
/// [`PipelineCache`](knorke_gpu::PipelineCache) table at [`MaskPrograms::at`], next to
/// [`OCCLUDER_PROGRAMS`](crate::occluder_raster::OCCLUDER_PROGRAMS). None blends: the clear
/// programs reset the rectangles of a local update, the field programs overwrite; the sprite
/// programs (the application's occlusion buffer, E21) blend with MAX like the obstacles.
pub const MASK_PROGRAMS: [ProgramDesc; 10] = [
    program("rect.vert", "clear.frag"),
    ProgramDesc {
        second: Some(SecondTarget {
            format: HEIGHT_FORMAT,
            blend: Blend::Opaque,
        }),
        ..program("rect.vert", "clear_height.frag")
    },
    program("fullscreen.vert", "jfa_seed.frag"),
    program("fullscreen.vert", "jfa_step.frag"),
    program("fullscreen.vert", "jfa_distance.frag"),
    program("rect.vert", "jfa_seed.frag"),
    program("rect.vert", "jfa_step.frag"),
    program("rect.vert", "jfa_distance.frag"),
    ProgramDesc {
        blend: Blend::Max,
        ..program("fullscreen.vert", "occlusion_mask.frag")
    },
    ProgramDesc {
        blend: Blend::Max,
        second: Some(SecondTarget {
            format: HEIGHT_FORMAT,
            blend: Blend::Max,
        }),
        ..program("fullscreen.vert", "occlusion_mask_height.frag")
    },
];

/// Size of the sprite mask's uniform block (`cbuffer SpriteParams` of `occlusion_mask.frag`).
const SPRITE_PARAMS_SIZE: usize = 32;

/// The sprite mask's parameters (Chiaro `SpriteParams`): the scene rectangle in mask texels,
/// moved by the anchor remainder like every other mask writer, and the mask size; plus the
/// scissor (rounded outwards) that keeps the rasterizer to the rectangle. The shader clips to
/// it exactly: a fractional viewport would lose the last column or row.
fn sprite_params(d: &OcclusionDomain, remainder: [f32; 2]) -> ([u8; SPRITE_PARAMS_SIZE], SDL_Rect) {
    let rect = [
        d.margin as f32 * d.texels_per_scene[0] - remainder[0],
        d.margin as f32 * d.texels_per_scene[1] - remainder[1],
        d.scene_w as f32 * d.texels_per_scene[0],
        d.scene_h as f32 * d.texels_per_scene[1],
    ];
    let values = [
        rect[0],
        rect[1],
        rect[2],
        rect[3],
        d.width as f32,
        d.height as f32,
        0.0,
        0.0,
    ];
    let mut bytes = [0u8; SPRITE_PARAMS_SIZE];
    write_f32s(&mut bytes, &values);
    let lo = |v: f32, size: u32| v.floor().clamp(0.0, size as f32) as i32;
    let hi = |v: f32, size: u32| v.ceil().clamp(0.0, size as f32) as i32;
    let (x0, y0) = (lo(rect[0], d.width), lo(rect[1], d.height));
    let (x1, y1) = (
        hi(rect[0] + rect[2], d.width),
        hi(rect[1] + rect[3], d.height),
    );
    let scissor = SDL_Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    };
    (bytes, scissor)
}

/// Where [`MASK_PROGRAMS`] sit in a pipeline table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaskPrograms {
    /// The first of the ten programs.
    pub base: ProgramIndex,
}

impl MaskPrograms {
    /// The ten programs at `base..base + 10`, in the order of [`MASK_PROGRAMS`].
    pub const fn at(base: ProgramIndex) -> Self {
        Self { base }
    }

    fn clear(&self, height_mode: bool) -> ProgramIndex {
        self.base + usize::from(height_mode)
    }

    fn sprite(&self, height_mode: bool) -> ProgramIndex {
        self.base + 8 + usize::from(height_mode)
    }

    /// Seed, step and distance program, fullscreen or over rectangles.
    fn field(&self, local: bool) -> [ProgramIndex; 3] {
        let first = self.base + if local { 5 } else { 2 };
        [first, first + 1, first + 2]
    }
}

impl Default for MaskPrograms {
    /// The table that holds only [`MASK_PROGRAMS`].
    fn default() -> Self {
        Self::at(0)
    }
}

/// What one recorded frame did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaskRecord {
    /// Static triangle and cell uploads (as P22 counts them).
    pub uploads: u32,
    /// Draws of obstacle sources into the mask.
    pub draws: u32,
    /// Field texels the distance pass wrote.
    pub field_texels: u64,
}

/// One target of a ping-pong pair or the seeds.
fn view<'a>(t: &'a RenderTarget<'_>, what: &str) -> Result<TextureRef<'a>, Diagnostic> {
    t.texture_ref()
        .ok_or_else(|| Diagnostic::error(NOT_PREPARED, format!("the {what} texture is missing")))
}

fn pipeline<'a>(
    pipelines: &'a PipelineCache<'_>,
    program: ProgramIndex,
    format: SDL_GPUTextureFormat,
) -> Result<PipelineRef<'a>, Diagnostic> {
    pipelines.pipeline(program, format).ok_or_else(|| {
        Diagnostic::error(
            NOT_PREPARED,
            "an occlusion pipeline is missing after prepare",
        )
    })
}

/// The occlusion mask (R the obstacle opacity, G footprint, E25), its distance field (E19) and,
/// in height mode, the height intervals (E22), each as a ping-pong pair kept across frames (E21).
/// The mask covers the scene plus a margin band ([`OcclusionDomain`]).
///
/// [`MaskUpdatePlan`] decides per frame how they change:
/// - nothing changed and the anchor did not move by a whole texel: [`MaskUpdate::Reused`], no
///   pass at all;
/// - [`MaskUpdate::Incremental`]: on a pan, the current textures are copied, shifted, into the
///   other texture of each pair; then the changed rectangles are cleared and every source is
///   drawn again (outside the rectangles it rewrites what is there), and the field is recomputed
///   in the field rectangles only, with seed and step passes over the seed rectangles;
/// - otherwise, or after a frame that was prepared but not committed, [`MaskUpdate::Full`]:
///   cleared, drawn and the field rebuilt with fullscreen passes.
///
/// Per frame: [`prepare`](Self::prepare) (decides; may fail), the obstacle stages'
/// `prepare` (if the update is not `Reused`), [`record`](Self::record) and, once the command
/// buffer holds the frame, [`commit`](Self::commit). The stage is a cache of GPU contents, not
/// simulation state: it has no snapshot; after a restore call [`invalidate`](Self::invalidate).
#[derive(Debug)]
pub struct OcclusionMask<'d> {
    programs: MaskPrograms,
    mask: [RenderTarget<'d>; 2],
    field: [RenderTarget<'d>; 2],
    height: [RenderTarget<'d>; 2],
    seeds: [RenderTarget<'d>; 2],
    /// Index of the pairs' textures that hold the last committed frame.
    current: usize,
    /// Index the prepared frame draws into; becomes `current` in commit.
    target: usize,
    plan: MaskUpdatePlan,
    /// A frame was prepared but not committed: the textures may not match the plan.
    failed: bool,
    /// The prepared frame, until it is recorded or committed.
    prepared: Option<MaskUpdate>,
    recorded: bool,
    last_update: MaskUpdate,
    height_mode: bool,
    height_recorded: bool,
    domain: OcclusionDomain,
    params: DomainParams,
    rects: RectParams,
    /// The prepared frame draws the application's sprite mask (E21).
    sprite: bool,
    /// The anchor remainder of the prepared frame, for the sprite rectangle.
    remainder: [f32; 2],
}

impl<'d> OcclusionMask<'d> {
    /// A stage without textures (the first prepare creates them).
    pub fn new(gpu: Gpu<'d>, programs: MaskPrograms) -> Self {
        let pair = || [RenderTarget::new(gpu), RenderTarget::new(gpu)];
        Self {
            programs,
            mask: pair(),
            field: pair(),
            height: pair(),
            seeds: pair(),
            current: 0,
            target: 0,
            plan: MaskUpdatePlan::new(UPDATE_LIMITS),
            failed: false,
            prepared: None,
            recorded: false,
            last_update: MaskUpdate::Full,
            height_mode: false,
            height_recorded: false,
            domain: OcclusionDomain::new(1, 1, 1.0),
            params: DomainParams::default(),
            rects: RectParams::new(),
            sprite: false,
            remainder: [0.0; 2],
        }
    }

    /// Replaces the full rebuild ratio of the plan (tests reach local updates that are never
    /// cheaper at small sizes; settings).
    pub fn set_full_rebuild_ratio(&mut self, ratio: f64) {
        let mut limits = UPDATE_LIMITS;
        limits.full_rebuild_ratio = ratio;
        self.plan.set_limits(limits);
    }

    /// Forgets what the textures hold: the next frame is [`MaskUpdate::Full`]. Call it after a
    /// snapshot restore of the obstacles or the scene.
    pub fn invalidate(&mut self) {
        self.plan.invalidate();
    }

    /// Ensures the textures of `input.domain`, decides how this frame updates them and creates
    /// the pipelines it draws with. `height_mode` adds the height pair (E22). Until
    /// [`commit`](Self::commit) the textures count as not matching the plan, so a frame that is
    /// prepared but not committed makes the next one rebuild fully.
    ///
    /// # Errors
    /// Creating a texture or a pipeline failed.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        pipelines: &mut PipelineCache<'d>,
        input: &MaskPlanInput<'_>,
        height_mode: bool,
    ) -> Result<MaskUpdate, Diagnostic> {
        let failed_before = self.failed;
        self.failed = true;
        self.prepared = None;
        self.recorded = false;
        self.last_update = MaskUpdate::Full;
        let d = input.domain;
        let mut reallocated = false;
        for i in 0..2 {
            reallocated |= self.mask[i].ensure(d.width, d.height, MASK_FORMAT)?;
            reallocated |= self.field[i].ensure(d.width, d.height, FIELD_FORMAT)?;
            if height_mode {
                reallocated |= self.height[i].ensure(d.width, d.height, HEIGHT_FORMAT)?;
            }
            // The seeds are rewritten from scratch where they are read: no rebuild on change.
            self.seeds[i].ensure(d.width, d.height, SEED_FORMAT)?;
        }
        let mut input = *input;
        input.force_full |= failed_before || reallocated || height_mode != self.height_recorded;
        let update = self.plan.decide(&input);
        // A shift copies into the other texture of each pair; everything else draws in place.
        self.target = if update == MaskUpdate::Incremental && self.plan.shift() != [0, 0] {
            1 - self.current
        } else {
            self.current
        };
        self.last_update = update;
        self.height_mode = height_mode;
        self.domain = d;
        self.params = DomainParams::new(input.projection, &d, &input.anchor);
        self.sprite = input.sprite;
        self.remainder = input.anchor.remainder;

        if update != MaskUpdate::Reused {
            // Every variant every time: the first camera pan must not create pipelines.
            pipelines.get(shaders, self.programs.clear(height_mode), MASK_FORMAT)?;
            for local in [false, true] {
                let [seed, step, distance] = self.programs.field(local);
                pipelines.get(shaders, seed, SEED_FORMAT)?;
                pipelines.get(shaders, step, SEED_FORMAT)?;
                pipelines.get(shaders, distance, FIELD_FORMAT)?;
            }
            if input.sprite {
                pipelines.get(shaders, self.programs.sprite(height_mode), MASK_FORMAT)?;
            }
        }
        self.prepared = Some(update);
        Ok(update)
    }

    /// The domain parameters of the prepared frame, for the obstacle stages (the same values
    /// [`record`](Self::record) draws them with).
    pub fn domain_params(&self) -> &DomainParams {
        &self.params
    }

    /// Records the prepared frame into `cmd`: one copy pass (the pending uploads of `geometry`
    /// and `tiles`, the shift copy), one render pass on the mask (and height texture) that
    /// clears the changed rectangles and draws every source, and the field passes. Nothing for
    /// [`MaskUpdate::Reused`]. The obstacle stages must be prepared for this frame. `nearest`
    /// is the host's nearest-neighbour sampler.
    ///
    /// # Errors
    /// Not prepared, already recorded, or a pass or binding failed (the frame then does not
    /// count; the next one rebuilds fully).
    pub fn record(
        &mut self,
        cmd: &mut CommandBuffer<'_>,
        pipelines: &PipelineCache<'_>,
        nearest: SamplerRef<'_>,
        geometry: &mut OccluderGeometry<'_>,
        tiles: &mut TileOccluders<'_>,
    ) -> Result<MaskRecord, Diagnostic> {
        self.record_with_sprite(cmd, pipelines, nearest, geometry, tiles, None)
    }

    /// [`record`](Self::record) for a frame prepared with
    /// [`MaskPlanInput::sprite`](crate::shadow::MaskPlanInput::sprite) set: after the obstacles
    /// the application's occlusion buffer `sprite` (scene size, alpha at or above 0.5 occludes,
    /// infinitely tall) is drawn into the scene rectangle of the mask (E21).
    ///
    /// # Errors
    /// As [`record`](Self::record), and [`NOT_PREPARED`] if `sprite` is given although the
    /// prepared frame has no sprite mask, or the other way round.
    pub fn record_with_sprite(
        &mut self,
        cmd: &mut CommandBuffer<'_>,
        pipelines: &PipelineCache<'_>,
        nearest: SamplerRef<'_>,
        geometry: &mut OccluderGeometry<'_>,
        tiles: &mut TileOccluders<'_>,
        sprite: Option<TextureRef<'_>>,
    ) -> Result<MaskRecord, Diagnostic> {
        let Some(update) = self.prepared.filter(|_| !self.recorded) else {
            return Err(Diagnostic::error(
                NOT_PREPARED,
                "record needs a prepared, unrecorded frame",
            ));
        };
        if sprite.is_some() != self.sprite {
            return Err(Diagnostic::error(
                NOT_PREPARED,
                "the sprite mask of record does not match the prepared frame",
            ));
        }
        let mut out = MaskRecord::default();
        if update == MaskUpdate::Reused {
            self.recorded = true;
            return Ok(out);
        }
        let local = update == MaskUpdate::Incremental;
        let shift = ShiftCopy::new(self.plan.shift(), self.domain.width, self.domain.height)
            .filter(|_| self.target != self.current);
        if geometry.upload_pending() || tiles.upload_pending() || shift.is_some() {
            let mut copy = cmd.copy_pass()?;
            out.uploads += geometry.record_uploads(&mut copy)?;
            out.uploads += tiles.record_uploads(&mut copy)?;
            if let Some(c) = shift {
                let (cur, tgt) = (self.current, self.target);
                let pairs = [
                    (&self.mask, true),
                    (&self.field, true),
                    (&self.height, self.height_mode),
                ];
                for (pair, used) in pairs {
                    if !used {
                        continue; // a change of the height mode rebuilds fully
                    }
                    // The copy and the redrawn strips together write every texel: may cycle.
                    copy.copy_texture_region(
                        view(&pair[cur], "source")?,
                        c.src,
                        view(&pair[tgt], "target")?,
                        c.dst,
                        c.size,
                        true,
                    )?;
                }
            }
        }

        out.draws = self.record_mask(cmd, pipelines, nearest, geometry, tiles, sprite, local)?;
        out.field_texels = self.record_field(cmd, pipelines, nearest, local)?;
        self.recorded = true;
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)] // the mask pass draws every source: all of it is needed
    fn record_mask(
        &mut self,
        cmd: &mut CommandBuffer<'_>,
        pipelines: &PipelineCache<'_>,
        nearest: SamplerRef<'_>,
        geometry: &OccluderGeometry<'_>,
        tiles: &TileOccluders<'_>,
        sprite: Option<TextureRef<'_>>,
        local: bool,
    ) -> Result<u32, Diagnostic> {
        let t = self.target;
        let (mask_load, height_load) = if local {
            (Load::Load, Load::Load)
        } else {
            (Load::Clear([0.0; 4]), Load::Clear([-INFINITE_TOP; 4]))
        };
        let mask = ColorTarget::new(view(&self.mask[t], "mask")?, mask_load).cycled(!local);
        let targets_buf;
        let targets: &[ColorTarget<'_>] = if self.height_mode {
            let height =
                ColorTarget::new(view(&self.height[t], "height")?, height_load).cycled(!local);
            targets_buf = [mask, height];
            &targets_buf
        } else {
            core::slice::from_ref(&mask)
        };
        let mut pass = cmd.render_pass(targets)?;
        if local {
            // Clear the changed rectangles, then draw every source unclipped.
            let n = self.rects.write(
                self.plan.mask_rects(),
                self.domain.width,
                self.domain.height,
            );
            if n > 0 {
                let clear = pipeline(
                    pipelines,
                    self.programs.clear(self.height_mode),
                    MASK_FORMAT,
                )?;
                let rects: &[u8] = self.rects.bytes();
                let bindings = Bindings {
                    vertex: StageBindings {
                        uniforms: &[rects],
                        ..Default::default()
                    },
                    fragment: StageBindings::default(),
                };
                pass.bind_program(clear, &bindings)?;
                pass.draw_instanced(6, n)?;
            }
        }
        let mut draws = geometry.record_draws(&mut pass, pipelines, &self.params, true, true)?;
        draws += tiles.record_draws(&mut pass, pipelines, nearest, &self.params)?;
        if let Some(sprite) = sprite {
            // Last, like Chiaro's DrawSources: the scissor stays set for the rest of the pass.
            let program = pipeline(
                pipelines,
                self.programs.sprite(self.height_mode),
                MASK_FORMAT,
            )?;
            let (params, scissor) = sprite_params(&self.domain, self.remainder);
            let samplers = [SamplerBinding::new(sprite, nearest)];
            let bindings = Bindings {
                vertex: StageBindings::default(),
                fragment: StageBindings {
                    samplers: &samplers,
                    uniforms: &[&params],
                    ..Default::default()
                },
            };
            pass.set_scissor(&scissor);
            pass.bind_program(program, &bindings)?;
            pass.draw_fullscreen()?;
            draws += 1;
        }
        Ok(draws)
    }

    /// One pass of `program` reading `input` into `target`: fullscreen, or the rectangles of
    /// `rects` only (the rest is kept).
    #[allow(clippy::too_many_arguments)] // one pass of a fixed pipeline: all of it is needed
    fn field_pass(
        cmd: &mut CommandBuffer<'_>,
        target: TextureRef<'_>,
        program: PipelineRef<'_>,
        input: TextureRef<'_>,
        nearest: SamplerRef<'_>,
        params: &JfaParams,
        rects: Option<(&RectParams, u32)>,
        cycle: bool,
    ) -> Result<(), Diagnostic> {
        let load = if rects.is_some() {
            Load::Load
        } else {
            Load::DontCare
        };
        let mut pass = cmd.render_pass(&[ColorTarget::new(target, load).cycled(cycle)])?;
        let samplers = [SamplerBinding::new(input, nearest)];
        let jfa = params.to_bytes();
        let fragment = StageBindings {
            samplers: &samplers,
            uniforms: &[&jfa],
            ..Default::default()
        };
        match rects {
            Some((r, n)) => {
                let bytes: &[u8] = r.bytes();
                let bindings = Bindings {
                    vertex: StageBindings {
                        uniforms: &[bytes],
                        ..Default::default()
                    },
                    fragment,
                };
                pass.bind_program(program, &bindings)?;
                pass.draw_instanced(6, n)
            }
            None => {
                let bindings = Bindings {
                    vertex: StageBindings::default(),
                    fragment,
                };
                pass.bind_program(program, &bindings)?;
                pass.draw_fullscreen()
            }
        }
    }

    /// Seed pass, step passes and distance pass (E19); returns the field texels written.
    fn record_field(
        &mut self,
        cmd: &mut CommandBuffer<'_>,
        pipelines: &PipelineCache<'_>,
        nearest: SamplerRef<'_>,
        local: bool,
    ) -> Result<u64, Diagnostic> {
        let (w, h) = (self.domain.width, self.domain.height);
        let [seed, step, distance] = self.programs.field(local);
        let seed = pipeline(pipelines, seed, SEED_FORMAT)?;
        let step = pipeline(pipelines, step, SEED_FORMAT)?;
        let distance = pipeline(pipelines, distance, FIELD_FORMAT)?;
        let params = JfaParams::new(w, h);
        // Local: seed and step passes cover the seed rectangles, so that no texel of the field
        // rectangles reads a stale seed; the distance pass writes the field rectangles only.
        let mut seed_rects = RectParams::new();
        let mut field_rects = RectParams::new();
        let (seed_area, field_area) = if local {
            let ns = seed_rects.write(self.plan.seed_rects(), w, h);
            let nf = field_rects.write(self.plan.field_rects(), w, h);
            (Some((&seed_rects, ns)), Some((&field_rects, nf)))
        } else {
            (None, None)
        };
        if local && field_area.is_some_and(|(_, n)| n == 0) {
            return Ok(0); // a pan alone: the shifted field is complete
        }
        let t = self.target;
        let seeds = [view(&self.seeds[0], "seed")?, view(&self.seeds[1], "seed")?];
        // The ping-pong seeds are rewritten several times per frame: no cycling.
        Self::field_pass(
            cmd,
            seeds[0],
            seed,
            view(&self.mask[t], "mask")?,
            nearest,
            &params,
            seed_area,
            false,
        )?;
        let mut current = 0;
        for k in jfa_steps() {
            Self::field_pass(
                cmd,
                seeds[1 - current],
                step,
                seeds[current],
                nearest,
                &params.with_step(k as f32),
                seed_area,
                false,
            )?;
            current = 1 - current;
        }
        // A full rebuild overwrites the whole field (cycle); a local one keeps the shifted rest.
        Self::field_pass(
            cmd,
            view(&self.field[t], "field")?,
            distance,
            seeds[current],
            nearest,
            &params.distance(),
            field_area,
            !local,
        )?;
        Ok(if local {
            self.plan.field_rects().area()
        } else {
            u64::from(w) * u64::from(h)
        })
    }

    /// The prepared frame is in the command buffer: its textures are the current ones now.
    /// Without a recorded frame this does nothing (the next frame then rebuilds fully).
    pub fn commit(&mut self) {
        if self.prepared.is_none() || !self.recorded {
            return;
        }
        self.current = self.target;
        self.plan.commit();
        self.failed = false;
        self.height_recorded = self.height_mode;
        self.prepared = None;
        self.recorded = false;
    }

    /// The current mask (R8G8), `None` before the first frame.
    pub fn mask(&self) -> Option<TextureRef<'_>> {
        self.mask[self.current].texture_ref()
    }

    /// The current distance field (R16F), `None` before the first frame.
    pub fn field(&self) -> Option<TextureRef<'_>> {
        self.field[self.current].texture_ref()
    }

    /// The current height texture, `None` unless the last committed frame had heights.
    pub fn height(&self) -> Option<TextureRef<'_>> {
        if self.height_recorded {
            self.height[self.current].texture_ref()
        } else {
            None
        }
    }

    /// How the last prepared frame updated the textures.
    pub fn last_update(&self) -> MaskUpdate {
        self.last_update
    }

    /// The changed rectangles of the last prepared frame (meaningful when it is incremental).
    pub fn mask_rects(&self) -> &DirtyRectList {
        self.plan.mask_rects()
    }

    /// Textures (re)allocated over the stage's lifetime; constant in the steady state.
    pub fn allocations(&self) -> u32 {
        [&self.mask, &self.field, &self.height, &self.seeds]
            .iter()
            .flat_map(|pair| pair.iter())
            .map(RenderTarget::recreated)
            .sum()
    }
}
