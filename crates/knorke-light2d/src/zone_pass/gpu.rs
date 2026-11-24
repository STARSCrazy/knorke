// Origin: chiaro @ 9bc5b9b, src/rendering/AmbientZonePass.hpp, src/rendering/AmbientZonePass.cpp,
// shaders/src/ambient_zone.vert.hlsl, shaders/src/ambient_zone.frag.hlsl.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The ambient zone pass on the GPU. Ownership follows ADR 0002: the pass owns its zone
//! buffer, vertex texture and pure ambient target on the host's device and records into the
//! host's command buffer.

use core::cell::Cell;

use knorke_gpu::sys::gpu::{
    SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ, SDL_GPU_TEXTUREFORMAT_R32G32_FLOAT,
    SDL_GPU_TEXTUREUSAGE_SAMPLER, SDL_GPUBufferUsageFlags, SDL_GPUTextureFormat,
};
use knorke_gpu::{
    Bindings, Blend, ColorTarget, CommandBuffer, CopyPass, Filter, Gpu, Load, PipelineCache,
    ProgramDesc, RenderTarget, SamplerBinding, Samplers, SecondTarget, ShaderLibrary,
    StageBindings, Texture, TextureDesc, TextureRef, UploadBuffer,
};
use knorke_math::Vec3;
use knorke_types::{Diagnostic, DiagnosticCode};

use super::{
    ZONE_VERTEX_WIDTH, ZoneFragParams, ZoneInput, ZonePassParams, fill_zone_vertices, write_zones,
    zone_vertex_bytes, zone_vertex_rows,
};
use crate::ambient_zone::GPU_ZONE_BYTES;
use crate::lighting::{LIGHT_FORMAT, LightingFrame, ambient_clear};

/// Format of the pure ambient target: the light buffer's (Chiaro `kAmbientTargetFormat`).
pub const AMBIENT_TARGET_FORMAT: SDL_GPUTextureFormat = LIGHT_FORMAT;

/// [`AmbientZonePass::record_pass`] for a frame the pass did not prepare.
pub const ZONE_PASS_NOT_PREPARED: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.zone_pass.not_prepared");
/// A resource the zone pass reads is missing after prepare.
pub const ZONE_PASS_INPUT_MISSING: DiagnosticCode =
    DiagnosticCode::new("knorke.light2d.zone_pass.input_missing");

const ZONE_PROGRAM: usize = 0;
const ZONE_DUAL_PROGRAM: usize = 1;

/// The two programs of the pass: the zone fold into the light buffer, and the same with a
/// second target for the pure ambient.
const PROGRAMS: [ProgramDesc; 2] = [
    ProgramDesc::new("ambient_zone.vert", "ambient_zone.frag", Blend::AlphaRgb),
    ProgramDesc {
        second: Some(SecondTarget {
            format: AMBIENT_TARGET_FORMAT,
            blend: Blend::AlphaRgb,
        }),
        ..ProgramDesc::new(
            "ambient_zone.vert",
            "ambient_zone_dual.frag",
            Blend::AlphaRgb,
        )
    },
];

/// Initial zone buffer capacity in zones (Chiaro: 16).
const INITIAL_ZONES: u32 = 16;

/// The ambient zone pass (Chiaro `AmbientZonePass`): uploads the visible zones, the polygon
/// vertex texture and draws one instance per zone into the light buffer.
#[derive(Debug)]
pub struct AmbientZonePass<'d> {
    gpu: Gpu<'d>,
    cache: Option<PipelineCache<'d>>,
    zones: UploadBuffer<'d>,
    vertices: Option<Texture<'d>>,
    vertex_upload: UploadBuffer<'d>,
    rows: u32,
    vertex_version: u64,
    vertex_pending: Cell<bool>,
    ambient_target: RenderTarget<'d>,
    count: u32,
    dual: bool,
}

impl<'d> AmbientZonePass<'d> {
    /// A pass without buffers and textures.
    pub fn new(gpu: Gpu<'d>) -> Self {
        Self {
            gpu,
            cache: None,
            zones: UploadBuffer::new(
                gpu,
                SDL_GPU_BUFFERUSAGE_GRAPHICS_STORAGE_READ,
                INITIAL_ZONES * GPU_ZONE_BYTES as u32,
                "ambient zone",
            ),
            vertices: None,
            vertex_upload: UploadBuffer::new(
                gpu,
                SDL_GPUBufferUsageFlags(0),
                ZONE_VERTEX_WIDTH * 8,
                "ambient zone vertex",
            ),
            rows: 0,
            vertex_version: u64::MAX,
            vertex_pending: Cell::new(false),
            ambient_target: RenderTarget::new(gpu),
            count: 0,
            dual: false,
        }
    }

    /// Uploads the list (mapping can fail here, not in the record), prepares the vertex texture
    /// if a polygon zone is drawn and the pure ambient target of `width` x `height` if `dual`,
    /// and the pipeline. An empty list draws nothing. Returns the number of GPU objects
    /// created (textures and targets).
    ///
    /// # Errors
    /// A buffer, texture, target or pipeline could not be created.
    pub fn prepare(
        &mut self,
        shaders: &mut ShaderLibrary<'d>,
        input: &ZoneInput<'_>,
        width: u32,
        height: u32,
        dual: bool,
    ) -> Result<u32, Diagnostic> {
        self.count = 0;
        self.dual = false;
        let mut created = 0;
        if input.store.polygon_row_count() == 0 {
            // No polygon zone left: the texture goes, the next one creates it again.
            self.release_vertices();
        }
        let list = input.list;
        if list.zones.is_empty() {
            return Ok(0);
        }
        let zones = list.zones.len() as u32;
        self.zones
            .reserve(u64::from(zones) * GPU_ZONE_BYTES as u64)?;
        if list.any_polygon {
            created += self.prepare_vertices(input)?;
        }
        if dual {
            if self
                .ambient_target
                .ensure(width, height, AMBIENT_TARGET_FORMAT)?
            {
                created += 1;
            }
        } else {
            // No reader this frame (no light pass, or not direction mode).
            self.ambient_target.release();
        }
        if self.cache.is_none() {
            self.cache = Some(PipelineCache::new(shaders, &PROGRAMS)?);
        }
        if let Some(cache) = self.cache.as_mut() {
            cache.get(shaders, program(dual), LIGHT_FORMAT)?;
        }
        let mut mapped = self.zones.map()?;
        write_zones(list, mapped.bytes());
        drop(mapped);
        self.count = zones;
        self.dual = dual;
        Ok(created)
    }

    fn prepare_vertices(&mut self, input: &ZoneInput<'_>) -> Result<u32, Diagnostic> {
        let store = input.store;
        let rows = zone_vertex_rows(store.polygon_row_count());
        let mut created = 0;
        if self.vertices.is_none() || rows > self.rows {
            let desc = TextureDesc::new(
                ZONE_VERTEX_WIDTH,
                rows,
                SDL_GPU_TEXTUREFORMAT_R32G32_FLOAT,
                SDL_GPU_TEXTUREUSAGE_SAMPLER,
            );
            self.vertices = Some(Texture::new(self.gpu, desc)?);
            self.rows = rows;
            self.vertex_version = u64::MAX;
            created = 1;
        }
        if self.vertex_version == store.polygon_version() {
            return Ok(created);
        }
        let bytes = zone_vertex_bytes(self.rows);
        self.vertex_upload.reserve(bytes)?;
        let mut mapped = self.vertex_upload.map()?;
        fill_zone_vertices(store, self.rows, &mut mapped.bytes()[..bytes as usize]);
        drop(mapped);
        self.vertex_version = store.polygon_version();
        self.vertex_pending.set(true);
        Ok(created)
    }

    fn release_vertices(&mut self) {
        self.vertices = None;
        self.vertex_upload.reset();
        self.rows = 0;
        self.vertex_pending.set(false);
        self.vertex_version = u64::MAX;
    }

    /// Zones are drawn this frame.
    pub fn is_active(&self) -> bool {
        self.count > 0
    }

    /// Zones drawn this frame.
    pub fn zone_count(&self) -> u32 {
        self.count
    }

    /// The zone list and, if changed, the vertex texture, into an open copy pass.
    ///
    /// # Errors
    /// An upload does not fit its buffer or texture.
    pub fn record_uploads(&self, copy: &mut CopyPass<'_>) -> Result<(), Diagnostic> {
        if self.count == 0 {
            return Ok(());
        }
        copy.upload_to_buffer(&self.zones, self.count * GPU_ZONE_BYTES as u32)?;
        if let (true, Some(texture)) = (self.vertex_pending.get(), self.vertices.as_ref()) {
            copy.upload_to_texture(&self.vertex_upload, texture.texture_ref(), true)?;
            self.vertex_pending.set(false);
        }
        Ok(())
    }

    /// The zone pass into `light` (and the pure ambient target if prepared dual): both start
    /// at `ambient` (alpha 0), the zones fold over it.
    ///
    /// # Errors
    /// Not prepared, or a pass or binding failed.
    pub fn record_pass(
        &self,
        cmd: &mut CommandBuffer<'_>,
        samplers: &Samplers<'_>,
        frame: &LightingFrame<'_>,
        ambient: Vec3,
        light: TextureRef<'_>,
    ) -> Result<(), Diagnostic> {
        if self.count == 0 {
            return Ok(());
        }
        let missing = |what: &str| {
            Diagnostic::error(
                ZONE_PASS_INPUT_MISSING,
                format!("{what} is missing after prepare"),
            )
        };
        let start = ambient_clear(ambient);
        let first = ColorTarget::new(light, Load::Clear(start)).cycled(true);
        let mut targets = [first, first];
        let mut target_count = 1;
        if self.dual {
            let pure = self
                .ambient_target
                .texture_ref()
                .ok_or_else(|| missing("the pure ambient target"))?;
            targets[1] = ColorTarget::new(pure, Load::Clear(start)).cycled(true);
            target_count = 2;
        }
        let cache = self.cache.as_ref().ok_or_else(|| {
            Diagnostic::error(
                ZONE_PASS_NOT_PREPARED,
                "the ambient zone pass was not prepared",
            )
        })?;
        let pipeline = cache
            .pipeline(program(self.dual), LIGHT_FORMAT)
            .ok_or_else(|| missing("the zone pipeline"))?;
        let buffer = self
            .zones
            .buffer_ref()
            .ok_or_else(|| missing("the zone buffer"))?;
        let mut pass = cmd.render_pass(&targets[..target_count])?;

        let (w, h) = (frame.scene.width(), frame.scene.height());
        let vertex = ZonePassParams {
            scene_size: [w as f32, h as f32],
            instance_offset: 0,
            pad: 0,
        }
        .to_bytes();
        let fragment = ZoneFragParams::new(
            frame.projection,
            w,
            h,
            frame.settings.surface_height_step,
            frame.surface.is_some(),
            if self.vertices.is_some() {
                self.rows
            } else {
                0
            },
        )
        .to_bytes();
        // Stand-ins (the scene) for a missing surface buffer or vertex texture: not read then.
        let nearest = samplers.sampler(Filter::Nearest);
        let samplers = [
            SamplerBinding::new(frame.surface.unwrap_or(frame.scene), nearest),
            SamplerBinding::new(
                self.vertices
                    .as_ref()
                    .map_or(frame.scene, Texture::texture_ref),
                nearest,
            ),
        ];
        let storage = [buffer];
        let vertex_uniforms: [&[u8]; 1] = [&vertex];
        let fragment_uniforms: [&[u8]; 1] = [&fragment];
        let bindings = Bindings {
            vertex: StageBindings {
                storage_buffers: &storage,
                uniforms: &vertex_uniforms,
                ..Default::default()
            },
            fragment: StageBindings {
                samplers: &samplers,
                uniforms: &fragment_uniforms,
                ..Default::default()
            },
        };
        pass.bind_program(pipeline, &bindings)?;
        pass.draw_instanced(6, self.count)
    }

    /// Releases everything the store no longer needs: the zone buffer, the vertex texture and
    /// the pure ambient target (a frame without a visible zone holds the same GPU objects as
    /// a scene without zones).
    pub fn release(&mut self) {
        self.ambient_target.release();
        self.release_vertices();
        self.zones.reset();
        self.count = 0;
        self.dual = false;
    }

    /// The pure ambient of the last dual frame.
    pub fn ambient_target(&self) -> Option<TextureRef<'_>> {
        self.ambient_target.texture_ref()
    }

    /// The polygon vertex texture; `None` before the first drawn polygon zone.
    pub fn vertex_texture(&self) -> Option<TextureRef<'_>> {
        self.vertices.as_ref().map(Texture::texture_ref)
    }
}

const fn program(dual: bool) -> usize {
    if dual {
        ZONE_DUAL_PROGRAM
    } else {
        ZONE_PROGRAM
    }
}
