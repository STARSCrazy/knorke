//! The renderer: owns textures, palette, options and the frame buffers.
//! One frame: `begin` -> any number of `draw_*` -> `end` (or, with a light pass,
//! `light_samples` -> light pass of the host -> `end_lit`): fog, palette, dithering, outlines.
//!
//! Origin: pixelraum `src/renderer/mod.rs` @ 81927d8. The light pass (shadow maps, light
//! selection, per-pixel light) moved to `knorke-lighting`; the renderer hands it G-buffer
//! columns ([`LightSamples`]) and takes one light value per sample back ([`FrameLight`]).

mod backdrop;
mod casters;
mod draw;
mod owners;
mod particles;
mod post;
mod shade;
mod textures;

pub use casters::{SOLID_CASTER, ShadowBillboard, ShadowCaster};
pub use owners::ParticleLight;
use shade::{Shade, shade_rows};

use knorke_types::{Column, Parallel, Serial};

use crate::anim::TexEffects;
use crate::blend::{BLEND_ADD, BLEND_ALPHA, Blend};
use crate::blend::{Frag, NO_FRAG, step_alpha};
use crate::camera::{Camera, View};
use crate::grid::{GridMap, Quad, SHADOW_SOLID, SIDE_SUN_ONLY};
use crate::math::{Vec3, Vec3Ext};
use crate::palette::{BAYER4, Palette};
use crate::particles::{
    EMIT_MAX, EMIT_RAIN, EMIT_SNOW, Emitter, PARTICLE_ADDITIVE, PARTICLE_BUDGET, PARTICLE_EMISSIVE,
    Particle, emit_one, emit_range, indoors,
};
use crate::pick::{PICK_GRID, PICK_MESH, PICK_SPRITE, Pick, PickEntry};
use crate::raster::{ClipVert, GBuffer, Material, NO_OWNER, Proj, draw_triangle};
use crate::texture::Texture;

/// Look switches. Every effect can be switched off on its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// Colour where nothing was drawn and no sky is set (0xRRGGBB).
    pub background: u32,
    /// A light pass lights this frame (`end_lit`): record shadow casters and billboard axes, light
    /// surfaces with the light values handed in. Off = unchanged images.
    pub lighting: bool,
    /// Half-size of the sun shadow box in world units (grid cells within it are recorded as
    /// casters); 0 = use `draw_distance`, else 32. See [`Renderer::shadow_half`].
    pub shadow_size: f32,
    /// Light steps (toon look); 0 = smooth light.
    pub light_levels: u32,
    /// Legacy directional shading by surface normal (ignored with `lighting`): direction the light
    /// travels.
    pub sun_dir: [f32; 3],
    /// Strength 0..1 of the legacy directional shading (0 = off).
    pub sun_strength: f32,
    /// Distance darkening: full light up to `fog_start`.
    pub fog_start: f32,
    /// `fog_min` from `fog_end` on. Off if end <= start.
    pub fog_end: f32,
    /// Light that remains at `fog_end` and beyond (0..1).
    pub fog_min: f32,
    /// Colour the distance fades to (0 = black, like Doom).
    pub fog_color: u32,
    /// Reduce the colours to the palette (always done for indexed output when a palette is set).
    pub quantize: bool,
    /// Bayer dithering before the palette reduction.
    pub dither: bool,
    /// Dither strength in colour steps (0..255).
    pub dither_strength: f32,
    /// Snap the camera to this world step (0 = off).
    pub snap: f32,
    /// Outlines from depth jumps and normal creases.
    pub outlines: bool,
    /// Relative depth jump that counts as an edge (0.1 = 10 %).
    pub outline_depth: f32,
    /// Normal crease: edge if the dot product of two neighbour normals is below this.
    pub outline_normal: f32,
    /// Outline colour (0xRRGGBB).
    pub outline_color: u32,
    /// Record per pixel what was drawn, for `Renderer::pick`; off = no cost. Never changes the image.
    pub pick: bool,
    /// Grid cells farther than this (world units) are skipped; 0 = no limit.
    pub draw_distance: f32,
    /// Far surfaces take smaller texture levels that keep dark lines (joints, cracks) instead of
    /// sampling the full texture, where such lines flicker or vanish.
    pub texture_lod: bool,
    /// lod_bias > 0 picks smaller levels earlier (softer), < 0 later (more flicker).
    pub lod_bias: f32,
    /// Between two levels, an ordered dither pattern picks the farther one for a growing share of
    /// the pixels (soft band); false = nearest level, hard boundary.
    pub lod_dither: bool,
    /// One level per grid face instead of per pixel (no change of picture within a wall).
    pub face_lod: bool,
    /// Area-preserving contour reduction for textures with a contour mask: 0 = a masked texel
    /// always wins its block (joints cover every far texel they touch), 1 = a joint keeps about its
    /// true area share on the far levels (thinned, every second texel and so on), between blends.
    /// Changing it rebuilds the levels of masked textures on the next frame. Only with texture_lod.
    pub contour_falloff: f32,
    /// See-through casters (BLEND_ALPHA sprites, triangle lists, grid walls with alpha < 1) are
    /// recorded for a stippled shadow with their alpha. false = no shadow.
    pub shadow_stipple: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            background: 0x000000,
            light_levels: 0,
            fog_start: 0.0,
            fog_end: 0.0,
            fog_min: 0.0,
            fog_color: 0x000000,
            sun_dir: [0.0, -1.0, 0.0],
            sun_strength: 0.0,
            quantize: false,
            dither: false,
            dither_strength: 24.0,
            snap: 0.0,
            outlines: false,
            outline_depth: 0.15,
            outline_normal: 0.5,
            outline_color: 0x000000,
            pick: false,
            draw_distance: 0.0,
            texture_lod: false,
            lod_bias: 0.0,
            face_lod: false,
            contour_falloff: 0.0,
            lod_dither: true,
            lighting: false,
            shadow_size: 0.0,
            shadow_stipple: true,
        }
    }
}

/// [`Sprite::flags`]: turn towards the camera.
pub const SPRITE_BILLBOARD: u32 = 1;
/// [`Sprite::flags`]: mirror horizontally.
pub const SPRITE_FLIP_X: u32 = 2;
/// [`Sprite::flags`]: the sprite casts no shadow.
pub const SPRITE_NO_SHADOW: u32 = 4;

/// A flat image standing in the world (Doom-style). Position is the bottom centre.
#[derive(Clone, Copy, Debug)]
pub struct Sprite {
    /// Bottom centre in world space.
    pub position: Vec3,
    /// Width in world units.
    pub width: f32,
    /// Height in world units.
    pub height: f32,
    /// Texture id.
    pub texture: i32,
    /// `SPRITE_*` flags.
    pub flags: u32,
    /// Facing for non-billboard sprites (same convention as the camera yaw: 0 faces south, towards +z viewers).
    pub yaw: f32,
    /// Light level 0..1 (multiplies the light).
    pub light: f32,
    /// Reported by `Renderer::pick`.
    pub tag: u32,
    /// Opacity 0..1, used with `blend` BLEND_ALPHA or BLEND_ADD; ignored with BLEND_NONE.
    pub alpha: f32,
    /// BLEND_NONE (opaque, default), BLEND_ALPHA or BLEND_ADD. Transparent sprites cast no shadow
    /// and are not pickable.
    pub blend: i32,
}

/// Mesh vertex.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Vertex {
    /// Position x.
    pub x: f32,
    /// Position y.
    pub y: f32,
    /// Position z.
    pub z: f32,
    /// Texture coordinate u.
    pub u: f32,
    /// Texture coordinate v.
    pub v: f32,
}

/// [`LightSamples::kind`] values (the same numbers as `knorke_lighting::sample`).
pub mod sample {
    /// Nothing drawn.
    pub const EMPTY: u8 = 0;
    /// Drawn, not lit by the light pass (pre-lit particles, additive glow).
    pub const UNLIT: u8 = 1;
    /// A lit surface.
    pub const LIT: u8 = 2;
    /// A lit billboard sprite (point lights treat it as facing them).
    pub const BILLBOARD: u8 = 3;
}

/// The frame's surfaces for a light pass, as G-buffer columns of equal length: first one sample
/// per screen pixel (sample `i` = pixel `i`), then one per transparent fragment.
#[derive(Clone, Copy, Debug)]
pub struct LightSamples<'a> {
    /// Pixels per row.
    pub width: usize,
    /// Camera position this frame.
    pub eye: Vec3,
    /// Sample kind ([`sample`]).
    pub kind: &'a [u8],
    /// World position.
    pub position: &'a [Vec3],
    /// Unit normal facing the eye.
    pub normal: &'a [Vec3],
    /// Billboard bottom centre for [`sample::BILLBOARD`] samples.
    pub axis: &'a [Vec3],
    /// Screen pixel `y * width + x`.
    pub pixel: &'a [u32],
    /// Number of screen samples (`width * height`).
    pub screen: usize,
    /// Shadow casters recorded this frame.
    pub casters: &'a [ShadowCaster],
    /// Billboard shadow casters recorded this frame.
    pub billboards: &'a [ShadowBillboard],
    /// Particles drawn this frame (lit as a whole, see [`FrameLight::particles`]).
    pub particles: &'a [ParticleLight],
}

/// Light handed back by the light pass for [`Renderer::end_lit`].
#[derive(Clone, Copy, Debug)]
pub struct FrameLight<L, P> {
    /// One linear light factor per sample of [`LightSamples`] (0..1, may exceed 1).
    pub samples: L,
    /// One light factor per particle of [`LightSamples::particles`] (used for lit ones).
    pub particles: P,
    /// Factor for the background and for backdrops with `BACKDROP_TINT`.
    pub sky_tint: [f32; 3],
}

/// The software renderer. Owns its textures and its frame buffers (colour, palette index, depth,
/// normals); the host reads them after [`end`](Self::end) / [`end_lit`](Self::end_lit).
pub struct Renderer {
    /// Frame width in pixels.
    pub width: usize,
    /// Frame height in pixels.
    pub height: usize,
    /// Look switches.
    pub options: Options,
    textures: Vec<Texture>,
    palette: Option<Palette>,
    sky: Option<(usize, f32)>,
    gbuf: GBuffer,
    view: Option<View>,
    rgba: Vec<u8>,
    indexed: Vec<u8>,
    normals_out: Vec<f32>,
    quads: Vec<Quad>,
    /// Renderer time in seconds (texture animation, scrolling, particles).
    time: f64,
    casters: Vec<ShadowCaster>,
    billboards: Vec<ShadowBillboard>,
    /// Bumped on every texture change; part of the shadow cache keys (alpha test).
    tex_gen: u64,
    /// Per texture: no base-level texel is transparent (shadow rasterizing skips the alpha test).
    tex_opaque: Vec<bool>,
    /// Animations and scroll speeds per texture id.
    effects: TexEffects,
    /// What each pick id (index + 1) stands for, this frame.
    picks: Vec<PickEntry>,
    /// Transparent draw calls so far this frame (Blend::order).
    blend_order: u32,
    /// Particles drawn so far this frame (PARTICLE_BUDGET).
    particle_count: usize,
    /// Persistent emitters by id (ids never reused); drawn in id order.
    emitters: std::collections::BTreeMap<u32, Emitter>,
    next_emitter: u32,
    /// Persistent backdrops by id (ids never reused).
    backdrops: std::collections::BTreeMap<u32, crate::backdrop::Backdrop>,
    next_backdrop: u32,
    /// Between begin and end: draw calls that only add to the frame are accepted.
    frame_open: bool,
    /// Particles dropped so far this frame; `stats` of the last finished frame.
    dropped_particles: u32,
    stats: Stats,
    /// This frame's particles, lit in `end`; index = owner of their pixels.
    particle_lights: Vec<ParticleLight>,
    /// Final colour factor per particle of the frame (`light_particles`).
    particle_tints: Vec<[f32; 3]>,
    /// This frame's billboard sprites (bottom centre); index = owner of their pixels.
    billboard_axes: Vec<Vec3>,
    /// Backdrop ray angles per pixel (`backdrop::composite`) and the view directions they belong to.
    sky_memo: (Vec<u32>, Vec<[f32; 2]>),
    /// Backdrop layers of the frame (reused).
    layers: Vec<crate::backdrop::Layer>,
    /// Backdrop (order, id) pairs of the frame (reused).
    layer_order: Vec<(i32, u32)>,
    /// Fragment order of one pixel while shading (reused, serial path).
    frag_order: Vec<u32>,
    /// Outline edge mask (reused).
    edge: Vec<bool>,
    /// Light pass columns (`light_samples`), reused between frames.
    samples: SampleColumns,
}

/// Columns behind [`LightSamples`].
#[derive(Default)]
struct SampleColumns {
    kind: Vec<u8>,
    position: Vec<Vec3>,
    normal: Vec<Vec3>,
    axis: Vec<Vec3>,
    pixel: Vec<u32>,
}

/// Counters for the last finished frame (`Renderer::stats`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Transparent fragments over the per-frame cap (blend::MAX_FRAGS), not drawn.
    pub dropped_fragments: u32,
    /// Particles over the per-frame budget (particles::PARTICLE_BUDGET), not drawn.
    pub dropped_particles: u32,
}

/// World position of the surface seen through pixel (x, y) at view depth `depth`.
pub fn world_pos(view: &View, x: usize, y: usize, depth: f32) -> Vec3 {
    let r = view.ray(x, y);
    view.eye + r * (depth / r.dot(view.forward))
}

fn rgb_of(c: u32) -> [f32; 3] {
    [
        ((c >> 16) & 255) as f32,
        ((c >> 8) & 255) as f32,
        (c & 255) as f32,
    ]
}

/// Colour 0xRRGGBB times strength as 0..1 per channel.
pub(crate) fn color_scaled(c: u32, strength: f32) -> [f32; 3] {
    let ch = |s: u32| ((c >> s) & 255) as f32 / 255.0 * strength;
    [ch(16), ch(8), ch(0)]
}

/// Step the light to `levels` steps by its strongest channel, keeping the hue. 0 levels = unchanged.
pub(crate) fn step_light(l: [f32; 3], levels: u32) -> [f32; 3] {
    if levels == 0 {
        return l;
    }
    let m = l[0].max(l[1]).max(l[2]);
    if !(m > 0.0) {
        return [0.0; 3];
    }
    let n = levels as f32;
    let s = (m * n).round() / n / m;
    l.map(|v| v * s)
}

/// No light pass: an empty light column.
const NO_LIGHT: FrameLight<&[[f32; 3]], &[[f32; 3]]> = FrameLight {
    samples: &[],
    particles: &[],
    sky_tint: [1.0; 3],
};

impl Renderer {
    /// A renderer for frames of `width x height` pixels (1..=8192 each).
    pub fn new(width: usize, height: usize) -> Option<Renderer> {
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return None;
        }
        let n = width * height;
        Some(Renderer {
            width,
            height,
            options: Options::default(),
            textures: Vec::new(),
            palette: None,
            sky: None,
            gbuf: GBuffer::new(width, height),
            view: None,
            rgba: vec![0; n * 4],
            indexed: vec![0; n],
            normals_out: vec![0.0; n * 3],
            quads: Vec::new(),
            time: 0.0,
            casters: Vec::new(),
            billboards: Vec::new(),
            tex_gen: 0,
            tex_opaque: Vec::new(),
            effects: TexEffects::default(),
            picks: Vec::new(),
            blend_order: 0,
            particle_count: 0,
            emitters: std::collections::BTreeMap::new(),
            next_emitter: 0,
            backdrops: std::collections::BTreeMap::new(),
            next_backdrop: 0,
            frame_open: false,
            dropped_particles: 0,
            stats: Stats::default(),
            particle_lights: Vec::new(),
            particle_tints: Vec::new(),
            billboard_axes: Vec::new(),
            sky_memo: (Vec::new(), Vec::new()),
            layers: Vec::new(),
            layer_order: Vec::new(),
            frag_order: Vec::new(),
            edge: Vec::new(),
            samples: SampleColumns::default(),
        })
    }

    /// Changes the frame size; false (and nothing changes) for an invalid size.
    pub fn resize(&mut self, width: usize, height: usize) -> bool {
        if width == self.width && height == self.height {
            self.view = None;
            self.frame_open = false;
            return true;
        }
        match Renderer::new(width, height) {
            Some(r) => {
                self.width = width;
                self.height = height;
                self.gbuf = r.gbuf;
                self.rgba = r.rgba;
                self.indexed = r.indexed;
                self.normals_out = r.normals_out;
                self.view = None;
                self.frame_open = false;
                true
            }
            None => false,
        }
    }

    /// Renderer time in seconds: texture animations, scrolling and particle emitters.
    pub fn set_time(&mut self, t: f64) {
        self.time = t;
    }

    /// Renderer time in seconds.
    pub fn time(&self) -> f64 {
        self.time
    }

    /// Starts a frame seen by `cam`: clears the frame buffers.
    pub fn begin(&mut self, cam: &Camera) {
        self.casters.clear();
        self.billboards.clear();
        self.picks.clear();
        self.particle_lights.clear();
        self.billboard_axes.clear();
        self.blend_order = 0;
        self.particle_count = 0;
        self.dropped_particles = 0;
        self.gbuf.pick = self.options.pick;
        self.gbuf.clear();
        self.view = Some(View::new(cam, self.width, self.height, self.options.snap));
        self.frame_open = true;
        if self.options.texture_lod {
            let f = self.options.contour_falloff;
            for (i, t) in self.textures.iter_mut().enumerate() {
                if t.contour_falloff != f && t.contour.is_some() && !self.effects.is_anim(i) {
                    t.set_contour_falloff(f);
                }
            }
        }
    }

    /// Half-size of the sun shadow box (also the range in which grid cells are recorded as
    /// casters): `shadow_size`, else `draw_distance`, else 32. A light pass uses it for its sun map.
    pub fn shadow_half(&self) -> f32 {
        let o = &self.options;
        if o.shadow_size > 0.0 {
            o.shadow_size
        } else if o.draw_distance > 0.0 {
            o.draw_distance
        } else {
            32.0
        }
    }

    /// Next pick id for `e`; 0 (nothing recorded) while picking is off.
    fn pick_id(&mut self, e: PickEntry) -> u32 {
        if !self.gbuf.pick {
            return 0;
        }
        self.picks.push(e);
        self.picks.len() as u32
    }

    /// What the last frame drew under pixel (x, y); None off screen, where nothing was drawn,
    /// or when that frame was drawn without `Options::pick`.
    pub fn pick(&self, x: i32, y: i32) -> Option<Pick> {
        let view = self.view.as_ref()?;
        if !self.gbuf.pick
            || x < 0
            || y < 0
            || x as usize >= self.width
            || y as usize >= self.height
        {
            return None;
        }
        let (x, y) = (x as usize, y as usize);
        let i = y * self.width + x;
        let e = self.picks.get((self.gbuf.id[i] as usize).checked_sub(1)?)?;
        let (depth, normal) = match self.gbuf.prelit_any && self.gbuf.prelit[i] {
            true => *self.gbuf.pick_under.get(&i)?,
            false => (self.gbuf.depth[i], self.gbuf.normal[i]),
        };
        Some(Pick {
            kind: e.kind,
            tag: e.tag,
            cell_x: e.cell_x,
            cell_z: e.cell_z,
            level: e.level,
            side: e.side,
            part: e.part,
            position: world_pos(view, x, y, depth),
            depth,
            normal,
        })
    }

    /// Fills the light pass columns of the frame (G-buffer per pixel and per transparent
    /// fragment). Call after the draws and before [`light_samples`](Self::light_samples) /
    /// [`end_lit`](Self::end_lit); false outside a frame. Kinds are [`sample::LIT`] /
    /// [`sample::BILLBOARD`] only with `Options::lighting`. The columns are kept between frames
    /// (no allocation once warm).
    pub fn prepare_light_samples(&mut self) -> bool {
        let Some(view) = self.view else { return false };
        let lighting = self.options.lighting;
        let (w, h) = (self.width, self.height);
        let g = &self.gbuf;
        let s = &mut self.samples;
        let n = w * h + g.frags.frags.len();
        s.kind.clear();
        s.position.clear();
        s.normal.clear();
        s.axis.clear();
        s.pixel.clear();
        s.kind.resize(n, sample::EMPTY);
        s.position.resize(n, Vec3::default());
        s.normal.resize(n, Vec3::default());
        s.axis.resize(n, Vec3::default());
        s.pixel.resize(n, 0);
        let axes = &self.billboard_axes;
        let bill = g.owner_any && !axes.is_empty();
        let mut put = |j: usize, i: usize, z: f32, normal: Vec3, unlit: bool, owner: u32| {
            let (x, y) = (i % w, i / w);
            let pos = world_pos(&view, x, y, z);
            s.position[j] = pos;
            s.pixel[j] = i as u32;
            if unlit || !lighting {
                s.kind[j] = sample::UNLIT;
                return;
            }
            let mut n = normal;
            if n.dot(pos - view.eye) > 0.0 {
                n = -n;
            }
            s.normal[j] = n;
            match axes.get(owner as usize).filter(|_| bill) {
                Some(&a) => {
                    s.kind[j] = sample::BILLBOARD;
                    s.axis[j] = a;
                }
                None => s.kind[j] = sample::LIT,
            }
        };
        for i in 0..w * h {
            let z = g.depth[i];
            if z.is_finite() {
                let prelit = g.prelit_any && g.prelit[i];
                put(i, i, z, g.normal[i], prelit, g.owner[i]);
            }
        }
        g.frags.for_each_frag(|i, k, f| {
            put(
                w * h + k,
                i,
                f.depth,
                f.normal,
                f.prelit || f.mode == BLEND_ADD,
                f.owner,
            );
        });
        true
    }

    /// The columns filled by the last [`prepare_light_samples`](Self::prepare_light_samples),
    /// with the frame's casters and particles; `None` before the first frame.
    pub fn light_samples(&self) -> Option<LightSamples<'_>> {
        let view = self.view?;
        let (w, h) = (self.width, self.height);
        let s = &self.samples;
        Some(LightSamples {
            width: w,
            eye: view.eye,
            kind: &s.kind,
            position: &s.position,
            normal: &s.normal,
            axis: &s.axis,
            pixel: &s.pixel,
            screen: w * h,
            casters: &self.casters,
            billboards: &self.billboards,
            particles: &self.particle_lights,
        })
    }

    /// Finishes the frame without a light pass: fog, sky, palette, dithering, outlines.
    pub fn end(&mut self) {
        self.end_lit(NO_LIGHT, &Serial);
    }

    /// Finishes the frame with the light values of a light pass over [`light_samples`]
    /// (`light.samples` empty = no light pass), shading rows in chunks on `par`. The image does
    /// not depend on the executor.
    ///
    /// [`light_samples`]: Self::light_samples
    pub fn end_lit<L: Column<[f32; 3]> + Sync, P: Column<[f32; 3]>>(
        &mut self,
        light: FrameLight<L, P>,
        par: &impl Parallel,
    ) {
        let Some(view) = self.view else { return };
        self.frame_open = false;
        let o = self.options;
        let (w, h) = (self.width, self.height);
        let lit = o.lighting && light.samples.len() == w * h + self.gbuf.frags.frags.len();
        if !self.particle_lights.is_empty() {
            self.light_particles(&light.particles, lit);
        }
        self.stats = Stats {
            dropped_fragments: self.gbuf.frags.dropped,
            dropped_particles: self.dropped_particles,
        };
        let mut layers = std::mem::take(&mut self.layers);
        let mut layer_order = std::mem::take(&mut self.layer_order);
        self.backdrop_layers(&view, &mut layers, &mut layer_order);
        self.layer_order = layer_order;
        let ctx = Shade {
            view,
            o,
            lit,
            width: w,
            g: &self.gbuf,
            textures: &self.textures,
            backdrops: &layers,
            backdrop_look: crate::backdrop::Look {
                tint: o.lighting.then_some(light.sky_tint),
                fog: (o.fog_end > o.fog_start)
                    .then(|| (o.fog_min.clamp(0.0, 1.0), rgb_of(o.fog_color))),
            },
            palette: self.palette.as_ref(),
            light: &light.samples,
            sky_tint: light.sky_tint,
            fog_c: rgb_of(o.fog_color),
            legacy_sun: Vec3::new(o.sun_dir[0], o.sun_dir[1], o.sun_dir[2]).normalized(),
        };
        // ray angles of the backdrops, kept while the view directions stay the same
        let sky_on = !layers.is_empty();
        if sky_on {
            let b = |f: f32| f.to_bits();
            let key = [
                w as u32,
                h as u32,
                b(view.forward.x),
                b(view.forward.y),
                b(view.forward.z),
                b(view.right.x),
                b(view.right.y),
                b(view.right.z),
                b(view.up.x),
                b(view.up.y),
                b(view.up.z),
                b(view.focal),
                b(view.cx),
                b(view.cy),
            ];
            if self.sky_memo.0[..] != key[..] || self.sky_memo.1.len() != w * h {
                self.sky_memo.0.clear();
                self.sky_memo.0.extend_from_slice(&key);
                self.sky_memo.1.clear();
                self.sky_memo.1.resize(w * h, [f32::NAN; 2]);
            }
        }
        // Fixed bands of SHADE_BAND rows. Every pixel is computed on its own by exactly one
        // chunk, so neither the split nor the executor changes the result.
        let sky = if sky_on {
            Some(&mut self.sky_memo.1[..])
        } else {
            None
        };
        let (rgba, indexed, order) = (
            &mut self.rgba[..],
            &mut self.indexed[..],
            &mut self.frag_order,
        );
        if par.threads() <= 1 {
            let bands = rgba
                .chunks_mut(SHADE_BAND * w * 4)
                .zip(indexed.chunks_mut(SHADE_BAND * w));
            let mut sky = sky.map(|s| s.chunks_mut(SHADE_BAND * w));
            for (k, (rgba, idx)) in bands.enumerate() {
                shade_rows(
                    &ctx,
                    k * SHADE_BAND,
                    rgba,
                    idx,
                    sky.as_mut().and_then(|s| s.next()),
                    order,
                );
            }
        } else {
            let mut sky = sky.map(|s| s.chunks_mut(SHADE_BAND * w));
            let mut bands: Vec<Band> = rgba
                .chunks_mut(SHADE_BAND * w * 4)
                .zip(indexed.chunks_mut(SHADE_BAND * w))
                .map(|(rgba, idx)| Band {
                    rgba,
                    idx,
                    sky: sky.as_mut().and_then(|s| s.next()),
                })
                .collect();
            let ctx = &ctx;
            par.for_each_chunk_mut(&mut bands, 1, |k, b| {
                let b = &mut b[0];
                shade_rows(
                    ctx,
                    k * SHADE_BAND,
                    b.rgba,
                    b.idx,
                    b.sky.as_deref_mut(),
                    &mut Vec::new(),
                );
            });
        }
        self.layers = layers;
        if o.outlines {
            self.apply_outlines();
        }
        for i in 0..w * h {
            let n = self.gbuf.normal[i];
            self.normals_out[i * 3..i * 3 + 3].copy_from_slice(&[n.x, n.y, n.z]);
        }
    }

    /// Drops the backdrop ray angles kept between frames (tests: compare against a frame
    /// shaded without the cache; the image is the same either way).
    #[doc(hidden)]
    pub fn forget_backdrop_cache(&mut self) {
        self.sky_memo = (Vec::new(), Vec::new());
    }

    /// True if some backdrop ray angle is kept (tests of the cache).
    #[doc(hidden)]
    pub fn backdrop_cache_used(&self) -> bool {
        self.sky_memo.1.iter().any(|k| !k[0].is_nan())
    }

    /// Counters for the last frame finished with `end`.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// RGBA colours of the last frame, row by row.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
    /// Palette indices of the last frame (with a palette).
    pub fn indexed(&self) -> &[u8] {
        &self.indexed
    }
    /// View depth per pixel of the last frame (infinity where nothing was drawn).
    pub fn depth(&self) -> &[f32] {
        &self.gbuf.depth
    }
    /// World normals per pixel of the last frame, three floats each.
    pub fn normals(&self) -> &[f32] {
        &self.normals_out
    }
    /// True if a palette is set.
    pub fn has_palette(&self) -> bool {
        self.palette.is_some()
    }
}

/// One band of rows for a parallel shading chunk.
struct Band<'a> {
    rgba: &'a mut [u8],
    idx: &'a mut [u8],
    sky: Option<&'a mut [[f32; 2]]>,
}

/// Rows per band of the shading pass.
const SHADE_BAND: usize = 2;

/// Animation frame for a fixed frame rate ("choppy" on purpose): which of `count` frames shows at `time`.
pub fn anim_frame(time: f64, fps: f32, count: u32) -> u32 {
    if count == 0 || fps <= 0.0 || !time.is_finite() {
        return 0;
    }
    ((time * fps as f64).floor() as i64).rem_euclid(count as i64) as u32
}

/// Round a time down to the frame grid of `fps`.
pub fn quantize_time(time: f64, fps: f32) -> f64 {
    if fps <= 0.0 {
        return time;
    }
    (time * fps as f64).floor() / fps as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall_sprite(flags: u32) -> Sprite {
        Sprite {
            position: Vec3::new(0.0, 0.0, -3.0),
            width: 1.0,
            height: 1.0,
            texture: 0,
            flags,
            yaw: 0.0,
            light: 1.0,
            tag: 0,
            alpha: 1.0,
            blend: crate::blend::BLEND_NONE,
        }
    }

    fn setup(lighting: bool) -> Renderer {
        let mut r = Renderer::new(16, 16).unwrap();
        r.add_texture(Texture::from_rgba(1, 1, &[255, 255, 255, 255]).unwrap());
        r.options.lighting = lighting;
        r.begin(&Camera::default());
        r
    }

    #[test]
    fn casters_recorded_only_when_lighting() {
        let mut r = setup(false);
        r.draw_sprite(&wall_sprite(0));
        assert_eq!(r.casters.len(), 0);
        assert_eq!(r.billboards.len(), 0);

        let mut r = setup(true);
        r.draw_sprite(&wall_sprite(0));
        assert_eq!(r.casters.len(), 2);
        r.draw_sprite(&wall_sprite(SPRITE_BILLBOARD));
        assert_eq!(r.billboards.len(), 1);
        r.draw_sprite(&wall_sprite(SPRITE_BILLBOARD | SPRITE_NO_SHADOW));
        r.draw_sprite(&wall_sprite(SPRITE_NO_SHADOW));
        assert_eq!(r.billboards.len(), 1);
        assert_eq!(r.casters.len(), 2);
        r.begin(&Camera::default());
        assert_eq!(r.casters.len() + r.billboards.len(), 0);
    }

    #[test]
    fn grid_records_casters_beyond_draw_distance() {
        let mut g = GridMap::new(40, 1, 1.0);
        for x in 0..40 {
            let c = g.cell_mut(x, 0).unwrap();
            c.floor_texture = 0;
        }
        let mut r = setup(true);
        r.options.draw_distance = 4.0;
        r.options.shadow_size = 12.0;
        r.draw_grid(&g);
        let near = r.casters.len();
        r.begin(&Camera::default());
        r.options.shadow_size = 30.0;
        r.draw_grid(&g);
        assert!(r.casters.len() > near && near > 0);
    }

    #[test]
    fn world_pos_has_requested_view_depth() {
        let cam = Camera {
            position: Vec3::new(1.0, 0.5, 2.0),
            yaw: 0.3,
            pitch: -0.2,
            ..Camera::default()
        };
        let v = View::new(&cam, 40, 30, 0.0);
        for (x, y) in [(0, 0), (39, 29), (20, 15)] {
            let p = world_pos(&v, x, y, 3.0);
            assert!((v.to_view(p).z - 3.0).abs() < 1e-4);
        }
    }

    #[test]
    fn light_samples_cover_pixels_and_fragments() {
        let mut r = setup(true);
        r.draw_sprite(&wall_sprite(0));
        r.draw_sprite(&Sprite {
            blend: BLEND_ALPHA,
            alpha: 0.5,
            position: Vec3::new(0.0, 0.0, -2.0),
            ..wall_sprite(0)
        });
        assert!(r.prepare_light_samples());
        let s = r.light_samples().unwrap();
        assert_eq!(s.screen, 256);
        assert!(
            s.kind.len() > s.screen,
            "fragments follow the screen samples"
        );
        assert!(
            s.kind[..s.screen].contains(&sample::LIT)
                && s.kind[..s.screen].contains(&sample::EMPTY)
        );
        assert!(s.kind[s.screen..].iter().all(|&k| k == sample::LIT));
        assert_eq!(
            s.casters.len(),
            4,
            "opaque quad plus the stippled see-through one"
        );
    }
}
