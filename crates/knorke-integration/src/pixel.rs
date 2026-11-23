//! Profile `scene.render.lighting` for the software path: `knorke-render` draws, `knorke-lighting`
//! lights, this module joins them (ADR 0013).
//!
//! Frame order:
//! 1. `Renderer::begin` and the draws (the renderer records G-buffer, casters, particles).
//! 2. [`light_frame`]: `Renderer::light_samples` → `LightScene::prepare` (pick lights, shadow
//!    maps) → `LightScene::shade` (one light value per sample) → `LightScene::light_points`
//!    (particles).
//! 3. `Renderer::end_lit` with those light values: fog, blending, palette, outlines.
//!
//! No data is converted per pixel: the G-buffer columns go to the light pass as the slices the
//! renderer keeps, and the light values come back as a column of the light scene's own state.
//! Only the shadow casters are copied into reused buffers (the two crates must not share a type).
//!
//! [`LitRenderer`] bundles both with the API pixelraum had (one `Options` with every switch,
//! `end()` doing everything) for hosts and tests migrating from pixelraum.

use std::ops::{Deref, DerefMut};

use knorke_lighting as lighting;
use knorke_lighting::{
    BillboardCaster, Caster, CasterTextures, LightOptions, LightScene, PointLight, Sun, Surfaces,
};
use knorke_render as render;
use knorke_render::{Camera, FrameLight, Renderer};
use knorke_types::{Parallel, Serial, project};

// The sample kinds are the same numbers on both sides.
const _: () = {
    assert!(render::sample::EMPTY == lighting::sample::EMPTY);
    assert!(render::sample::UNLIT == lighting::sample::UNLIT);
    assert!(render::sample::LIT == lighting::sample::LIT);
    assert!(render::sample::BILLBOARD == lighting::sample::BILLBOARD);
    assert!(render::SOLID_CASTER == lighting::SOLID_CASTER);
};

/// The renderer's textures as seen by the shadow alpha test.
pub struct RenderTextures<'a> {
    textures: &'a [render::Texture],
    opaque: &'a [bool],
    generation: u64,
}

impl<'a> RenderTextures<'a> {
    /// The textures of `r`.
    pub fn of(r: &'a Renderer) -> Self {
        RenderTextures {
            textures: r.textures(),
            opaque: r.textures_opaque(),
            generation: r.texture_generation(),
        }
    }
}

impl CasterTextures for RenderTextures<'_> {
    fn size(&self, id: usize) -> Option<(usize, usize)> {
        self.textures.get(id).map(|t| (t.width, t.height))
    }
    fn alpha(&self, id: usize, u: f32, v: f32, wrap: bool) -> u8 {
        let t = &self.textures[id];
        (if wrap {
            t.sample(u, v)
        } else {
            t.sample_clamp(u, v)
        })[3]
    }
    fn opaque(&self, id: usize) -> bool {
        self.opaque.get(id).copied().unwrap_or(false)
    }
    fn generation(&self) -> u64 {
        self.generation
    }
}

/// Buffers of [`light_frame`] kept between frames (no allocation once warm).
#[derive(Debug, Default)]
pub struct LightBuffers {
    casters: Vec<Caster>,
    billboards: Vec<BillboardCaster>,
    particles: Vec<[f32; 3]>,
}

/// Runs the light pass of `lights` over the frame `r` has drawn and finishes the frame
/// (`Renderer::end_lit`). The sun shadow box follows `Renderer::shadow_half`. Without
/// `Options::lighting` this is `Renderer::end`. Same image for every executor.
pub fn light_frame(
    r: &mut Renderer,
    lights: &mut LightScene,
    buf: &mut LightBuffers,
    par: &impl Parallel,
) {
    if !r.options.lighting {
        r.end();
        return;
    }
    lights.options.shadow_half = r.shadow_half();
    if !r.prepare_light_samples() {
        return;
    }
    {
        let Some(s) = r.light_samples() else { return };
        buf.casters.clear();
        buf.casters.extend(s.casters.iter().map(|c| Caster {
            tri: c.tri,
            uv: c.uv,
            texture: c.texture,
            wrap: c.wrap,
            alpha: c.alpha,
            sun_only: c.sun_only,
        }));
        buf.billboards.clear();
        buf.billboards
            .extend(s.billboards.iter().map(|b| BillboardCaster {
                base: b.base,
                width: b.width,
                height: b.height,
                texture: b.texture,
                flip: b.flip,
                scroll: b.scroll,
                alpha: b.alpha,
            }));
        let surfaces = Surfaces {
            width: s.width,
            eye: s.eye,
            kind: s.kind,
            position: s.position,
            normal: s.normal,
            axis: s.axis,
            pixel: s.pixel,
            screen: s.screen,
        };
        let textures = RenderTextures::of(r);
        lights.prepare(&surfaces, &buf.casters, &buf.billboards, &textures, par);
        lights.shade(&surfaces, par);
        buf.particles.clear();
        buf.particles.resize(s.particles.len(), [0.0; 3]);
        lights.light_points(
            project(s.particles, |p: &render::ParticleLight| &p.pos),
            &mut buf.particles[..],
        );
    }
    let sky_tint = lights.sky_tint();
    r.end_lit(
        FrameLight {
            samples: lights.light(),
            particles: &buf.particles[..],
            sky_tint,
        },
        par,
    );
}

/// Every switch of pixelraum's `Options`: the renderer's look and the light pass's shadows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// Colour where nothing was drawn and no sky is set (0xRRGGBB).
    pub background: u32,
    /// Lighting pass with sun, ambient and point lights; off = unchanged images.
    pub lighting: bool,
    /// Half-size of the sun shadow box in world units; 0 = use `draw_distance`, else 32.
    pub shadow_size: f32,
    /// Sun shadow map size in texels.
    pub shadow_res: usize,
    /// Point light shadow cube face size in texels.
    pub point_shadow_res: usize,
    /// Shadow bias in world units; the slope term is 2x this scaled by 1 - n.l.
    pub shadow_bias: f32,
    /// Soft sun shadow edges: width of the penumbra in world units, 0 = hard edges.
    pub shadow_soft: f32,
    /// The same for shadowed point lights.
    pub point_shadow_soft: f32,
    /// Light steps (toon look); 0 = smooth light.
    pub light_levels: u32,
    /// Legacy directional shading (ignored with `lighting`): direction the light travels.
    pub sun_dir: [f32; 3],
    /// Strength 0..1 of the legacy directional shading (0 = off).
    pub sun_strength: f32,
    /// Distance darkening: full light up to `fog_start`.
    pub fog_start: f32,
    /// `fog_min` from `fog_end` on. Off if end <= start.
    pub fog_end: f32,
    /// Light that remains at `fog_end` and beyond.
    pub fog_min: f32,
    /// Colour the distance fades to.
    pub fog_color: u32,
    /// Reduce the colours to the palette.
    pub quantize: bool,
    /// Bayer dithering before the palette reduction.
    pub dither: bool,
    /// Dither strength in colour steps.
    pub dither_strength: f32,
    /// Snap the camera to this world step (0 = off).
    pub snap: f32,
    /// Outlines from depth jumps and normal creases.
    pub outlines: bool,
    /// Relative depth jump that counts as an edge.
    pub outline_depth: f32,
    /// Normal crease threshold.
    pub outline_normal: f32,
    /// Outline colour.
    pub outline_color: u32,
    /// Record per pixel what was drawn, for `pick`.
    pub pick: bool,
    /// Grid cells farther than this are skipped; 0 = no limit.
    pub draw_distance: f32,
    /// Texture levels for far surfaces.
    pub texture_lod: bool,
    /// Level bias.
    pub lod_bias: f32,
    /// Dither between two levels.
    pub lod_dither: bool,
    /// One level per grid face.
    pub face_lod: bool,
    /// Area-preserving contour reduction (0..1).
    pub contour_falloff: f32,
    /// Worker threads for shadow maps and shading; 0 = all cores (at most 8), 1 = none. Same
    /// image for any count.
    pub threads: usize,
    /// See-through casters throw a stippled shadow.
    pub shadow_stipple: bool,
}

impl Default for Options {
    fn default() -> Self {
        let r = render::Options::default();
        let l = LightOptions::default();
        Options {
            background: r.background,
            lighting: r.lighting,
            shadow_size: r.shadow_size,
            shadow_res: l.shadow_res,
            point_shadow_res: l.point_shadow_res,
            shadow_bias: l.shadow_bias,
            shadow_soft: l.shadow_soft,
            point_shadow_soft: l.point_shadow_soft,
            light_levels: r.light_levels,
            sun_dir: r.sun_dir,
            sun_strength: r.sun_strength,
            fog_start: r.fog_start,
            fog_end: r.fog_end,
            fog_min: r.fog_min,
            fog_color: r.fog_color,
            quantize: r.quantize,
            dither: r.dither,
            dither_strength: r.dither_strength,
            snap: r.snap,
            outlines: r.outlines,
            outline_depth: r.outline_depth,
            outline_normal: r.outline_normal,
            outline_color: r.outline_color,
            pick: r.pick,
            draw_distance: r.draw_distance,
            texture_lod: r.texture_lod,
            lod_bias: r.lod_bias,
            lod_dither: r.lod_dither,
            face_lod: r.face_lod,
            contour_falloff: r.contour_falloff,
            threads: 0,
            shadow_stipple: r.shadow_stipple,
        }
    }
}

impl Options {
    /// The renderer's part.
    pub fn render(&self) -> render::Options {
        render::Options {
            background: self.background,
            lighting: self.lighting,
            shadow_size: self.shadow_size,
            light_levels: self.light_levels,
            sun_dir: self.sun_dir,
            sun_strength: self.sun_strength,
            fog_start: self.fog_start,
            fog_end: self.fog_end,
            fog_min: self.fog_min,
            fog_color: self.fog_color,
            quantize: self.quantize,
            dither: self.dither,
            dither_strength: self.dither_strength,
            snap: self.snap,
            outlines: self.outlines,
            outline_depth: self.outline_depth,
            outline_normal: self.outline_normal,
            outline_color: self.outline_color,
            pick: self.pick,
            draw_distance: self.draw_distance,
            texture_lod: self.texture_lod,
            lod_bias: self.lod_bias,
            lod_dither: self.lod_dither,
            face_lod: self.face_lod,
            contour_falloff: self.contour_falloff,
            shadow_stipple: self.shadow_stipple,
        }
    }

    /// The light pass's part (`shadow_half` is set from the renderer each frame).
    pub fn light(&self) -> LightOptions {
        LightOptions {
            shadow_half: LightOptions::default().shadow_half,
            shadow_res: self.shadow_res,
            point_shadow_res: self.point_shadow_res,
            shadow_bias: self.shadow_bias,
            shadow_soft: self.shadow_soft,
            point_shadow_soft: self.point_shadow_soft,
        }
    }
}

/// Counters of the last finished frame (renderer and light pass).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Transparent fragments over the per-frame cap, not drawn.
    pub dropped_fragments: u32,
    /// Particles over the per-frame budget, not drawn.
    pub dropped_particles: u32,
    /// Point lights used for the frame.
    pub lights_used: u32,
    /// Shadow maps rebuilt this frame (sun map and each point light's cube count one each).
    pub shadow_maps_rebuilt: u32,
}

/// Renderer plus light scene with pixelraum's API. Everything the renderer offers is reachable
/// through `Deref`; lights, time and the combined [`Options`] are here. The renderer's options
/// follow [`LitRenderer::options`] whenever the renderer is reached mutably.
pub struct LitRenderer {
    /// All switches; copied into renderer and light scene before use.
    pub options: Options,
    render: Renderer,
    lights: LightScene,
    buf: LightBuffers,
    lights_used: u32,
    rebuilt: u32,
}

impl LitRenderer {
    /// A renderer for frames of `width x height` pixels (1..=8192 each).
    pub fn new(width: usize, height: usize) -> Option<LitRenderer> {
        Some(LitRenderer {
            options: Options::default(),
            render: Renderer::new(width, height)?,
            lights: LightScene::new(),
            buf: LightBuffers::default(),
            lights_used: 0,
            rebuilt: 0,
        })
    }

    /// The light scene.
    pub fn lights(&self) -> &LightScene {
        &self.lights
    }

    /// The light scene, mutably.
    pub fn lights_mut(&mut self) -> &mut LightScene {
        &mut self.lights
    }

    /// Time in seconds for animations, particles and light flicker.
    pub fn set_time(&mut self, t: f64) {
        self.render.set_time(t);
        self.lights.set_time(t);
    }

    /// Sky colour and strength (multiplies the sky with lighting).
    pub fn set_sky_tint(&mut self, color: u32, strength: f32) {
        self.lights.set_sky_tint(color, strength);
    }

    /// Sets or removes the sun.
    pub fn set_sun(&mut self, s: Option<Sun>) {
        self.lights.set_sun(s);
    }

    /// Ambient light colour and strength.
    pub fn set_ambient(&mut self, color: u32, strength: f32) {
        self.lights.set_ambient(color, strength);
    }

    /// Creates a persistent point light; ids are never reused.
    pub fn create_light(&mut self, l: PointLight) -> u32 {
        self.lights.create_light(l)
    }

    /// Changes a light; false for an unknown id.
    pub fn set_light(&mut self, id: u32, l: PointLight) -> bool {
        self.lights.set_light(id, l)
    }

    /// Removes a light; false for an unknown id.
    pub fn destroy_light(&mut self, id: u32) -> bool {
        self.lights.destroy_light(id)
    }

    /// Light `id`, if it exists.
    pub fn light(&self, id: u32) -> Option<&PointLight> {
        self.lights.light_of(id)
    }

    /// Starts a frame seen by `cam`.
    pub fn begin(&mut self, cam: &Camera) {
        self.sync();
        self.render.begin(cam);
    }

    /// Finishes the frame: light pass (with `Options::lighting`), fog, palette, outlines. Runs on
    /// `Options::threads` threads; the image is the same for any count.
    pub fn end(&mut self) {
        match self.options.threads {
            1 => self.end_with(&Serial),
            0 => self.end_with(&knorke_exec::ScopedThreads::new(
                std::thread::available_parallelism().map_or(std::num::NonZeroUsize::MIN, |n| {
                    n.min(std::num::NonZeroUsize::new(8).unwrap_or(n))
                }),
            )),
            n => self.end_with(&knorke_exec::ScopedThreads::new(
                std::num::NonZeroUsize::new(n).unwrap_or(std::num::NonZeroUsize::MIN),
            )),
        }
    }

    /// [`end`](Self::end) on the executor `par`.
    pub fn end_with(&mut self, par: &impl Parallel) {
        self.sync();
        self.lights.options = LightOptions {
            shadow_half: self.lights.options.shadow_half,
            ..self.options.light()
        };
        let lighting = self.options.lighting;
        light_frame(&mut self.render, &mut self.lights, &mut self.buf, par);
        let st = self.lights.stats();
        (self.lights_used, self.rebuilt) = if lighting {
            (st.lights_used, st.shadow_maps_rebuilt)
        } else {
            (0, 0)
        };
    }

    /// Counters of the last finished frame.
    pub fn stats(&self) -> Stats {
        let s = self.render.stats();
        Stats {
            dropped_fragments: s.dropped_fragments,
            dropped_particles: s.dropped_particles,
            lights_used: self.lights_used,
            shadow_maps_rebuilt: self.rebuilt,
        }
    }

    fn sync(&mut self) {
        self.render.options = self.options.render();
    }
}

impl Deref for LitRenderer {
    type Target = Renderer;
    fn deref(&self) -> &Renderer {
        &self.render
    }
}

impl DerefMut for LitRenderer {
    /// Draw calls see the current options (as they did in pixelraum, where there was only one).
    fn deref_mut(&mut self) -> &mut Renderer {
        self.sync();
        &mut self.render
    }
}

/// Everything pixelraum's crate root exported, with [`LitRenderer`] as `Renderer` and the
/// combined [`Options`]/[`Stats`]: `use knorke_integration::pixel::prelude::*` replaces
/// `use pixelraum::*`.
pub mod prelude {
    pub use super::{LitRenderer as Renderer, Normalized, Options, Stats};
    pub use knorke_lighting as light;
    pub use knorke_lighting::{MAX_LIGHTS, PointLight, Sun};
    pub use knorke_render::*;
}

/// pixelraum's `Vec3::normalized` (unit vector, zero for the zero vector) for code migrating
/// from it; `knorke_math::Vec3::normalize_or_zero` treats lengths up to `f32::EPSILON` as zero.
pub trait Normalized {
    /// Unit vector, or zero for the zero vector.
    fn normalized(self) -> Self;
}

impl Normalized for knorke_math::Vec3 {
    fn normalized(self) -> Self {
        let l = self.length();
        if l > 0.0 {
            self * (1.0 / l)
        } else {
            knorke_math::Vec3::ZERO
        }
    }
}
