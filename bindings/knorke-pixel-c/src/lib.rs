//! C interface of the pixel renderer: `pr_*` functions and `PR_*` structs of
//! `include/pixelraum.h`, a drop-in for pixelraum's C API (ADR 0024). Built on
//! `knorke_integration::pixel` (renderer + light pass with pixelraum's API).
//!
//! Origin: pixelraum @ 81927d8, `src/capi.rs` (MIT); ported 1:1, only the imports and the palette
//! flag (knorke-render has no `has_palette`) changed. Mirrors `include/pixelraum.h` exactly; keep
//! both in sync. The API is frozen until `knorke-abi` (plan 17.1).
// FFI boundary (ADR 0005): every `unsafe fn` here takes C pointers whose validity the caller
// promises as documented in the header; the bodies only turn non-null pointers into references
// or slices of the documented length.
#![allow(unsafe_code)]
#![allow(
    clippy::missing_safety_doc,
    non_camel_case_types,
    missing_docs,
    clippy::too_many_arguments,
    unsafe_op_in_unsafe_fn
)]

use knorke_integration::pixel::prelude::{
    self as px, Camera, Cell, GridMap, Options, PointLight, Renderer, Sprite, Sun, Texture, Vec3,
    Vertex, Wall,
    particles::{Emitter, Particle},
};
use std::slice;

/// Version of the C API (`PR_VERSION` in the header): 0x00MMmmpp.
pub const VERSION: u32 = px::PIXELRAUM_VERSION;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PR_Options {
    pub struct_size: u32,
    pub background: u32,
    pub lighting: i32,
    pub shadow_size: f32,
    pub shadow_res: i32,
    pub point_shadow_res: i32,
    pub shadow_bias: f32,
    pub shadow_soft: f32,
    pub point_shadow_soft: f32,
    pub light_levels: i32,
    pub sun_dir: [f32; 3],
    pub sun_strength: f32,
    pub fog_start: f32,
    pub fog_end: f32,
    pub fog_min: f32,
    pub fog_color: u32,
    pub quantize: i32,
    pub dither: i32,
    pub dither_strength: f32,
    pub snap: f32,
    pub outlines: i32,
    pub outline_depth: f32,
    pub outline_normal: f32,
    pub outline_color: u32,
    pub pick: i32,
    pub draw_distance: f32,
    pub texture_lod: i32,
    pub lod_bias: f32,
    pub lod_dither: i32,
    pub face_lod: i32,
    pub contour_falloff: f32,
    pub threads: i32,
    pub shadow_stipple: i32,
}

impl From<Options> for PR_Options {
    fn from(o: Options) -> Self {
        PR_Options {
            struct_size: std::mem::size_of::<PR_Options>() as u32,
            background: o.background,
            light_levels: o.light_levels as i32,
            fog_start: o.fog_start,
            fog_end: o.fog_end,
            fog_min: o.fog_min,
            fog_color: o.fog_color,
            sun_dir: o.sun_dir,
            sun_strength: o.sun_strength,
            quantize: o.quantize as i32,
            dither: o.dither as i32,
            dither_strength: o.dither_strength,
            snap: o.snap,
            outlines: o.outlines as i32,
            outline_depth: o.outline_depth,
            outline_normal: o.outline_normal,
            outline_color: o.outline_color,
            pick: o.pick as i32,
            draw_distance: o.draw_distance,
            texture_lod: o.texture_lod as i32,
            lod_bias: o.lod_bias,
            face_lod: o.face_lod as i32,
            contour_falloff: o.contour_falloff,
            lod_dither: o.lod_dither as i32,
            threads: o.threads.min(i32::MAX as usize) as i32,
            lighting: o.lighting as i32,
            shadow_size: o.shadow_size,
            shadow_res: o.shadow_res.min(i32::MAX as usize) as i32,
            point_shadow_res: o.point_shadow_res.min(i32::MAX as usize) as i32,
            shadow_bias: o.shadow_bias,
            shadow_soft: o.shadow_soft,
            point_shadow_soft: o.point_shadow_soft,
            shadow_stipple: o.shadow_stipple as i32,
        }
    }
}

impl From<PR_Options> for Options {
    fn from(o: PR_Options) -> Self {
        Options {
            background: o.background,
            light_levels: o.light_levels.max(0) as u32,
            fog_start: o.fog_start,
            fog_end: o.fog_end,
            fog_min: o.fog_min,
            fog_color: o.fog_color,
            sun_dir: o.sun_dir,
            sun_strength: o.sun_strength,
            quantize: o.quantize != 0,
            dither: o.dither != 0,
            dither_strength: o.dither_strength,
            snap: o.snap,
            outlines: o.outlines != 0,
            outline_depth: o.outline_depth,
            outline_normal: o.outline_normal,
            outline_color: o.outline_color,
            pick: o.pick != 0,
            draw_distance: o.draw_distance,
            texture_lod: o.texture_lod != 0,
            lod_bias: o.lod_bias,
            face_lod: o.face_lod != 0,
            contour_falloff: o.contour_falloff,
            lod_dither: o.lod_dither != 0,
            threads: o.threads.clamp(0, 256) as usize,
            lighting: o.lighting != 0,
            shadow_size: o.shadow_size,
            shadow_res: o.shadow_res.clamp(1, 4096) as usize,
            point_shadow_res: o.point_shadow_res.clamp(1, 4096) as usize,
            shadow_bias: o.shadow_bias,
            shadow_soft: o.shadow_soft,
            point_shadow_soft: o.point_shadow_soft,
            shadow_stipple: o.shadow_stipple != 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PR_Camera {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub fov_y: f32,
    pub near_clip: f32,
    pub shift_x: f32,
    pub shift_y: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PR_Wall {
    pub texture: i32,
    pub open: f32,
    pub alpha: f32,
    pub back_texture: i32,
    pub leaf_texture: i32,
    pub door_mode: i32,
    pub leaf_u0: f32,
    pub leaf_u1: f32,
    pub leaf_top: f32,
    pub swing_angle: f32,
}

impl From<&Wall> for PR_Wall {
    fn from(w: &Wall) -> Self {
        PR_Wall {
            texture: w.texture,
            open: w.open,
            alpha: w.alpha,
            back_texture: w.back_texture,
            leaf_texture: w.leaf_texture,
            door_mode: w.door_mode,
            leaf_u0: w.leaf_u0,
            leaf_u1: w.leaf_u1,
            leaf_top: w.leaf_top,
            swing_angle: w.swing_angle,
        }
    }
}

impl From<&PR_Wall> for Wall {
    fn from(w: &PR_Wall) -> Self {
        Wall {
            texture: w.texture,
            open: w.open,
            alpha: w.alpha,
            back_texture: w.back_texture,
            leaf_texture: w.leaf_texture,
            door_mode: w.door_mode,
            leaf_u0: w.leaf_u0,
            leaf_u1: w.leaf_u1,
            leaf_top: w.leaf_top,
            swing_angle: w.swing_angle,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PR_Cell {
    pub floor_height: f32,
    pub ceiling_height: f32,
    pub floor_texture: i32,
    pub ceiling_texture: i32,
    pub side_texture: i32,
    pub light: f32,
    pub walls: [PR_Wall; 4],
    pub floor_corners: [f32; 4],
    pub ceiling_corners: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PR_Sprite {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub width: f32,
    pub height: f32,
    pub texture: i32,
    pub flags: u32,
    pub yaw: f32,
    pub light: f32,
    pub tag: u32,
    pub alpha: f32,
    pub blend: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PR_Pick {
    pub kind: i32,
    pub tag: u32,
    pub cell_x: i32,
    pub cell_z: i32,
    pub side: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub depth: f32,
    pub nx: f32,
    pub ny: f32,
    pub nz: f32,
    pub part: i32,
    pub level: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct PR_GridHit {
    pub hit: i32,
    pub cell_x: i32,
    pub cell_z: i32,
    pub side: i32,
    pub distance: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub part: i32,
    pub level: i32,
}

/// The renderer, whether a frame is open (between pr_begin and pr_end) and whether a palette is
/// set (knorke-render keeps that private; pixelraum asked `Renderer::has_palette`).
pub struct PR_Renderer(Renderer, bool, bool);
pub struct PR_Grid(GridMap);

unsafe fn r<'a>(p: *mut PR_Renderer) -> Option<&'a mut Renderer> {
    p.as_mut().map(|x| &mut x.0)
}

#[unsafe(no_mangle)]
pub extern "C" fn pr_version() -> u32 {
    VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn pr_renderer_create(
    header_version: u32,
    width: i32,
    height: i32,
) -> *mut PR_Renderer {
    // major and minor must match the library the caller was compiled against; the patch may differ
    if header_version >> 8 != VERSION >> 8 || width <= 0 || height <= 0 {
        return std::ptr::null_mut();
    }
    match Renderer::new(width as usize, height as usize) {
        Some(x) => Box::into_raw(Box::new(PR_Renderer(x, false, false))),
        None => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_renderer_destroy(p: *mut PR_Renderer) {
    if !p.is_null() {
        drop(Box::from_raw(p));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_renderer_resize(p: *mut PR_Renderer, width: i32, height: i32) -> i32 {
    match r(p) {
        Some(x) if width > 0 && height > 0 => x.resize(width as usize, height as usize) as i32,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_default_options(out: *mut PR_Options) {
    if let Some(o) = out.as_mut() {
        *o = Options::default().into();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_set_options(p: *mut PR_Renderer, o: *const PR_Options) -> i32 {
    let (Some(x), Some(o)) = (r(p), o.as_ref()) else {
        return 0;
    };
    // read only the size field first: a caller with an older/newer struct may have passed fewer bytes
    let size = std::ptr::read_unaligned(o as *const PR_Options as *const u32);
    if size as usize != std::mem::size_of::<PR_Options>() {
        return 0;
    }
    x.options = (*o).into();
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_get_options(p: *mut PR_Renderer, out: *mut PR_Options) {
    if let (Some(x), Some(o)) = (r(p), out.as_mut()) {
        *o = x.options.into();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_set_palette(p: *mut PR_Renderer, rgb: *const u8, count: i32) -> i32 {
    let Some(pr) = p.as_mut() else { return 0 };
    if rgb.is_null() || count <= 0 {
        pr.2 = false;
        return pr.0.set_palette(&[]) as i32;
    }
    if count > 256 {
        return 0;
    }
    let ok =
        pr.0.set_palette(slice::from_raw_parts(rgb, count as usize * 3));
    // a rejected palette leaves the old one in place, as in knorke-render
    pr.2 |= ok;
    ok as i32
}

unsafe fn texture_rgba(w: i32, h: i32, rgba: *const u8) -> Option<Texture> {
    if w <= 0 || h <= 0 || rgba.is_null() {
        return None;
    }
    Texture::from_rgba(
        w as usize,
        h as usize,
        slice::from_raw_parts(rgba, (w * h * 4) as usize),
    )
}

unsafe fn texture_indexed(
    w: i32,
    h: i32,
    px: *const u8,
    pal: *const u8,
    n: i32,
    transp: i32,
) -> Option<Texture> {
    if w <= 0 || h <= 0 || px.is_null() || pal.is_null() || n <= 0 || n > 256 {
        return None;
    }
    Texture::from_indexed(
        w as usize,
        h as usize,
        slice::from_raw_parts(px, (w * h) as usize),
        slice::from_raw_parts(pal, n as usize * 3),
        if (0..256).contains(&transp) {
            Some(transp as u8)
        } else {
            None
        },
    )
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_texture_rgba(
    p: *mut PR_Renderer,
    w: i32,
    h: i32,
    rgba: *const u8,
) -> i32 {
    match (r(p), texture_rgba(w, h, rgba)) {
        (Some(x), Some(t)) => x.add_texture(t),
        _ => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_texture_indexed(
    p: *mut PR_Renderer,
    w: i32,
    h: i32,
    pixels: *const u8,
    palette_rgb: *const u8,
    palette_count: i32,
    transparent_index: i32,
) -> i32 {
    match (
        r(p),
        texture_indexed(w, h, pixels, palette_rgb, palette_count, transparent_index),
    ) {
        (Some(x), Some(t)) => x.add_texture(t),
        _ => -1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_texture_update_rgba(
    p: *mut PR_Renderer,
    id: i32,
    w: i32,
    h: i32,
    rgba: *const u8,
) -> i32 {
    match (r(p), texture_rgba(w, h, rgba)) {
        (Some(x), Some(t)) => x.replace_texture(id, t) as i32,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_texture_update_indexed(
    p: *mut PR_Renderer,
    id: i32,
    w: i32,
    h: i32,
    pixels: *const u8,
    palette_rgb: *const u8,
    palette_count: i32,
    transparent_index: i32,
) -> i32 {
    match (
        r(p),
        texture_indexed(w, h, pixels, palette_rgb, palette_count, transparent_index),
    ) {
        (Some(x), Some(t)) => x.replace_texture(id, t) as i32,
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_texture_add_level_indexed(
    p: *mut PR_Renderer,
    id: i32,
    w: i32,
    h: i32,
    pixels: *const u8,
    palette_rgb: *const u8,
    palette_count: i32,
    transparent_index: i32,
) -> i32 {
    match (
        r(p),
        texture_indexed(w, h, pixels, palette_rgb, palette_count, transparent_index),
    ) {
        (Some(x), Some(t)) => x.add_texture_level(id, t) as i32,
        _ => 0,
    }
}

/// Mask slice of `w * h` bytes, or None for a null pointer; Err for bad sizes.
unsafe fn mask_arg<'a>(mask: *const u8, w: i32, h: i32) -> Result<Option<&'a [u8]>, ()> {
    if mask.is_null() {
        return Ok(None);
    }
    if w <= 0 || h <= 0 {
        return Err(());
    }
    Ok(Some(std::slice::from_raw_parts(
        mask,
        w as usize * h as usize,
    )))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_texture_set_contour(
    p: *mut PR_Renderer,
    id: i32,
    mask: *const u8,
    w: i32,
    h: i32,
) -> i32 {
    pr_texture_set_level_contour(p, id, 0, mask, w, h)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_texture_set_level_contour(
    p: *mut PR_Renderer,
    id: i32,
    level: i32,
    mask: *const u8,
    w: i32,
    h: i32,
) -> i32 {
    let Some(x) = r(p) else { return 0 };
    if level < 0 {
        return 0;
    }
    // the size must match the level (checked against the mask length and the level size)
    let Some(t) = x.texture(id) else { return 0 };
    let lt = if level == 0 {
        Some(t)
    } else {
        t.mips.get(level as usize - 1)
    };
    let Some(lt) = lt else { return 0 };
    if !mask.is_null() && (w as usize != lt.width || h as usize != lt.height) {
        return 0;
    }
    match mask_arg(mask, w, h) {
        Ok(m) => x.set_texture_level_contour(id, level as usize, m) as i32,
        Err(()) => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_texture_contour_auto(
    p: *mut PR_Renderer,
    id: i32,
    strength: f32,
) -> i32 {
    match r(p) {
        Some(x) => x.texture_contour_auto(id, strength) as i32,
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_set_sky(p: *mut PR_Renderer, texture: i32, repeats: f32) {
    if let Some(x) = r(p) {
        x.set_sky(texture, repeats);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_begin(p: *mut PR_Renderer, cam: *const PR_Camera) {
    if let (Some(pr), Some(c)) = (p.as_mut(), cam.as_ref()) {
        pr.1 = true;
        pr.0.begin(&Camera {
            position: Vec3::new(c.x, c.y, c.z),
            yaw: c.yaw,
            pitch: c.pitch,
            fov_y: c.fov_y,
            near: c.near_clip,
            shift_x: c.shift_x,
            shift_y: c.shift_y,
        });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_draw_grid(p: *mut PR_Renderer, g: *const PR_Grid) {
    if let (Some(x), Some(g)) = (r(p), g.as_ref()) {
        x.draw_grid(&g.0);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_draw_sprite(p: *mut PR_Renderer, s: *const PR_Sprite) {
    if let (Some(x), Some(s)) = (r(p), s.as_ref()) {
        x.draw_sprite(&Sprite {
            position: Vec3::new(s.x, s.y, s.z),
            width: s.width,
            height: s.height,
            texture: s.texture,
            flags: s.flags,
            yaw: s.yaw,
            light: s.light,
            tag: s.tag,
            alpha: s.alpha,
            blend: s.blend,
        });
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_draw_triangles(
    p: *mut PR_Renderer,
    verts: *const Vertex,
    count: i32,
    texture: i32,
    light: f32,
    cull: i32,
) {
    if let Some(x) = r(p)
        && !verts.is_null()
        && count > 0
    {
        x.draw_triangles(
            slice::from_raw_parts(verts, count as usize),
            texture,
            light,
            cull != 0,
        );
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PR_Sun {
    pub dir: [f32; 3],
    pub color: u32,
    pub strength: f32,
    pub shadows: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PR_Light {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub color: u32,
    pub strength: f32,
    pub radius: f32,
    pub flicker: f32,
    pub seed: u32,
    pub shadows: i32,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_set_sun(p: *mut PR_Renderer, sun: *const PR_Sun) {
    if let Some(x) = r(p) {
        x.set_sun(sun.as_ref().map(|s| Sun {
            dir: Vec3::new(s.dir[0], s.dir[1], s.dir[2]),
            color: s.color,
            strength: s.strength,
            shadows: s.shadows != 0,
        }));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_set_ambient(p: *mut PR_Renderer, color: u32, strength: f32) {
    if let Some(x) = r(p) {
        x.set_ambient(color, strength);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_set_sky_tint(p: *mut PR_Renderer, color: u32, strength: f32) {
    if let Some(x) = r(p) {
        x.set_sky_tint(color, strength);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_set_time(p: *mut PR_Renderer, time: f64) {
    if let Some(x) = r(p) {
        x.set_time(time);
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PR_Particle {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub size: f32,
    pub color: u32,
    pub texture: i32,
    pub flags: u32,
    pub alpha: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PR_Emitter {
    pub kind: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub size_x: f32,
    pub size_y: f32,
    pub size_z: f32,
    pub rate: f32,
    pub speed: f32,
    pub size: f32,
    pub color: u32,
    pub texture: i32,
    pub wind_x: f32,
    pub wind_z: f32,
    pub seed: u32,
}

fn emitter_from_c(e: &PR_Emitter) -> Emitter {
    Emitter {
        kind: e.kind,
        position: Vec3::new(e.x, e.y, e.z),
        box_size: Vec3::new(e.size_x, e.size_y, e.size_z),
        rate: e.rate,
        speed: e.speed,
        size: e.size,
        color: e.color,
        texture: e.texture,
        wind: [e.wind_x, e.wind_z],
        seed: e.seed,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_draw_particles(
    p: *mut PR_Renderer,
    list: *const PR_Particle,
    count: i32,
) {
    let Some(x) = r(p) else { return };
    if list.is_null() || count <= 0 {
        return;
    }
    // only the first PARTICLE_BUDGET can be drawn; draw_particles counts the rest as dropped
    // (pixelraum counted them here; knorke-render keeps drop_particles private)
    let ps: Vec<Particle> = slice::from_raw_parts(list, count as usize)
        .iter()
        .map(|q| Particle {
            position: Vec3::new(q.x, q.y, q.z),
            size: q.size,
            color: q.color,
            texture: q.texture,
            flags: q.flags,
            alpha: q.alpha,
        })
        .collect();
    x.draw_particles(&ps);
}

/// Persistent emitter; id >= 0, or -1 on a bad pointer or when ids are used up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_emitter_create(p: *mut PR_Renderer, e: *const PR_Emitter) -> i32 {
    let (Some(x), Some(e)) = (r(p), e.as_ref()) else {
        return -1;
    };
    let id = x.create_emitter(emitter_from_c(e));
    if id > i32::MAX as u32 {
        x.destroy_emitter(id);
        return -1;
    }
    id as i32
}

/// 1 = changed, 0 = unknown id or bad pointer.
///
/// Not supported yet: knorke-render has no `Renderer::set_emitter` (pixelraum had one), so this
/// always returns 0 and the emitter stays as it was. Kept for link compatibility; Wizardry does
/// not call it. Destroy and create again instead (the id changes).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_emitter_set(
    _p: *mut PR_Renderer,
    _id: i32,
    _e: *const PR_Emitter,
) -> i32 {
    0
}

/// 1 = removed, 0 = unknown id or bad pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_emitter_destroy(p: *mut PR_Renderer, id: i32) -> i32 {
    let Some(x) = r(p) else { return 0 };
    (id >= 0 && x.destroy_emitter(id as u32)) as i32
}

/// Between pr_begin and pr_end; grid NULL = no indoor culling of rain and snow.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_draw_emitters(p: *mut PR_Renderer, g: *const PR_Grid) {
    if let Some(x) = r(p) {
        x.draw_emitters(g.as_ref().map(|g| &g.0));
    }
}

fn light_from_c(l: &PR_Light) -> PointLight {
    PointLight {
        pos: Vec3::new(l.x, l.y, l.z),
        color: l.color,
        strength: l.strength,
        radius: l.radius,
        flicker: l.flicker,
        seed: l.seed,
        shadows: l.shadows != 0,
    }
}

/// Persistent light; id >= 0, or -1 on a bad pointer or when ids are used up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_light_create(p: *mut PR_Renderer, light: *const PR_Light) -> i32 {
    let (Some(x), Some(l)) = (r(p), light.as_ref()) else {
        return -1;
    };
    let id = x.create_light(light_from_c(l));
    if id > i32::MAX as u32 {
        x.destroy_light(id);
        return -1;
    }
    id as i32
}

/// 1 = changed, 0 = unknown id or bad pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_light_set(p: *mut PR_Renderer, id: i32, light: *const PR_Light) -> i32 {
    let (Some(x), Some(l)) = (r(p), light.as_ref()) else {
        return 0;
    };
    (id >= 0 && x.set_light(id as u32, light_from_c(l))) as i32
}

/// 1 = removed, 0 = unknown id or bad pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_light_destroy(p: *mut PR_Renderer, id: i32) -> i32 {
    let Some(x) = r(p) else { return 0 };
    (id >= 0 && x.destroy_light(id as u32)) as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_draw_triangles_ex(
    p: *mut PR_Renderer,
    verts: *const Vertex,
    count: i32,
    texture: i32,
    light: f32,
    cull: i32,
    cast_shadow: i32,
    tag: u32,
    alpha: f32,
    blend: i32,
) {
    if let Some(x) = r(p)
        && !verts.is_null()
        && count > 0
    {
        x.draw_triangles_ex(
            slice::from_raw_parts(verts, count as usize),
            texture,
            light,
            cull != 0,
            cast_shadow != 0,
            tag,
            alpha,
            blend,
        );
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_end(p: *mut PR_Renderer) {
    if let Some(pr) = p.as_mut() {
        pr.0.end();
        pr.1 = false;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_pixels_rgba(p: *mut PR_Renderer) -> *const u8 {
    r(p).map_or(std::ptr::null(), |x| x.rgba().as_ptr())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_pixels_indexed(p: *mut PR_Renderer) -> *const u8 {
    match p.as_mut() {
        Some(pr) if pr.2 => pr.0.indexed().as_ptr(),
        _ => std::ptr::null(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_depth(p: *mut PR_Renderer) -> *const f32 {
    r(p).map_or(std::ptr::null(), |x| x.depth().as_ptr())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_normals(p: *mut PR_Renderer) -> *const f32 {
    r(p).map_or(std::ptr::null(), |x| x.normals().as_ptr())
}

/// 1 = something drawn under (x, y) in the last frame (needs options.pick); `out` is zeroed otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_pick(p: *mut PR_Renderer, x: i32, y: i32, out: *mut PR_Pick) -> i32 {
    let Some(out) = out.as_mut() else { return 0 };
    *out = PR_Pick::default();
    let Some(k) = r(p).and_then(|r| r.pick(x, y)) else {
        return 0;
    };
    *out = PR_Pick {
        kind: k.kind,
        tag: k.tag,
        cell_x: k.cell_x,
        cell_z: k.cell_z,
        side: k.side,
        x: k.position.x,
        y: k.position.y,
        z: k.position.z,
        depth: k.depth,
        nx: k.normal.x,
        ny: k.normal.y,
        nz: k.normal.z,
        part: k.part,
        level: k.level,
    };
    1
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct PR_Stats {
    pub dropped_fragments: u32,
    pub dropped_particles: u32,
    pub lights_used: u32,
    pub shadow_maps_rebuilt: u32,
}

/// Counters of the last pr_end; 1 = ok, 0 on a NULL pointer (`out` zeroed when given).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_stats(p: *mut PR_Renderer, out: *mut PR_Stats) -> i32 {
    let Some(out) = out.as_mut() else { return 0 };
    *out = PR_Stats::default();
    let Some(x) = r(p) else { return 0 };
    let s = x.stats();
    *out = PR_Stats {
        dropped_fragments: s.dropped_fragments,
        dropped_particles: s.dropped_particles,
        lights_used: s.lights_used,
        shadow_maps_rebuilt: s.shadow_maps_rebuilt,
    };
    1
}

/// 1 = the ray hits a grid face within max_dist; `out` is zeroed otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_grid_raycast(
    g: *const PR_Grid,
    origin: *const f32,
    dir: *const f32,
    max_dist: f32,
    out: *mut PR_GridHit,
) -> i32 {
    let Some(out) = out.as_mut() else { return 0 };
    *out = PR_GridHit::default();
    let Some(g) = g.as_ref() else { return 0 };
    if origin.is_null() || dir.is_null() {
        return 0;
    }
    let o = slice::from_raw_parts(origin, 3);
    let d = slice::from_raw_parts(dir, 3);
    let Some(h) = g.0.raycast(
        Vec3::new(o[0], o[1], o[2]),
        Vec3::new(d[0], d[1], d[2]),
        max_dist,
    ) else {
        return 0;
    };
    *out = PR_GridHit {
        hit: 1,
        cell_x: h.cell_x,
        cell_z: h.cell_z,
        side: h.side,
        distance: h.distance,
        x: h.point.x,
        y: h.point.y,
        z: h.point.z,
        part: h.part,
        level: h.level,
    };
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn pr_grid_create(
    width: i32,
    depth: i32,
    levels: i32,
    cell_size: f32,
) -> *mut PR_Grid {
    if width <= 0
        || depth <= 0
        || width > 4096
        || depth > 4096
        || !(1..=64).contains(&levels)
        || cell_size.is_nan()
        || cell_size <= 0.0
        || width as i64 * depth as i64 * levels as i64 > 1 << 24
    {
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(PR_Grid(GridMap::with_levels(
        width as usize,
        depth as usize,
        levels as usize,
        cell_size,
    ))))
}

/// Storeys of the grid (as given to pr_grid_create); 0 for NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_grid_levels(g: *const PR_Grid) -> i32 {
    g.as_ref().map_or(0, |g| g.0.levels as i32)
}

/// The cell (x, z, level) or None for out-of-range indices.
fn grid_cell_mut(g: &mut PR_Grid, x: i32, z: i32, level: i32) -> Option<&mut Cell> {
    if x < 0 || z < 0 || level < 0 {
        return None;
    }
    g.0.cell_at_mut(x as usize, z as usize, level as usize)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_grid_destroy(g: *mut PR_Grid) {
    if !g.is_null() {
        drop(Box::from_raw(g));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_default_wall(out: *mut PR_Wall) {
    if let Some(o) = out.as_mut() {
        *o = (&Wall::default()).into();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_default_cell(out: *mut PR_Cell) {
    if let Some(o) = out.as_mut() {
        *o = cell_to_c(&Cell::default());
    }
}

fn cell_to_c(c: &Cell) -> PR_Cell {
    PR_Cell {
        floor_height: c.floor_height,
        ceiling_height: c.ceiling_height,
        floor_texture: c.floor_texture,
        ceiling_texture: c.ceiling_texture,
        side_texture: c.side_texture,
        light: c.light,
        walls: c.walls.each_ref().map(PR_Wall::from),
        floor_corners: c.floor_corners,
        ceiling_corners: c.ceiling_corners,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_grid_set_cell(
    g: *mut PR_Grid,
    x: i32,
    z: i32,
    level: i32,
    c: *const PR_Cell,
) -> i32 {
    let (Some(g), Some(c)) = (g.as_mut(), c.as_ref()) else {
        return 0;
    };
    match grid_cell_mut(g, x, z, level) {
        Some(cell) => {
            *cell = Cell {
                floor_height: c.floor_height,
                ceiling_height: c.ceiling_height,
                floor_texture: c.floor_texture,
                ceiling_texture: c.ceiling_texture,
                side_texture: c.side_texture,
                light: c.light,
                walls: c.walls.each_ref().map(Wall::from),
                floor_corners: c.floor_corners,
                ceiling_corners: c.ceiling_corners,
            };
            1
        }
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_grid_get_cell(
    g: *const PR_Grid,
    x: i32,
    z: i32,
    level: i32,
    out: *mut PR_Cell,
) -> i32 {
    let (Some(g), Some(o)) = (g.as_ref(), out.as_mut()) else {
        return 0;
    };
    if level < 0 {
        return 0;
    }
    match g.0.cell_at(x as i64, z as i64, level as usize) {
        Some(c) => {
            *o = cell_to_c(c);
            1
        }
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_grid_set_wall(
    g: *mut PR_Grid,
    x: i32,
    z: i32,
    level: i32,
    side: i32,
    texture: i32,
    open: f32,
) -> i32 {
    let Some(g) = g.as_mut() else { return 0 };
    if !(0..4).contains(&side) {
        return 0;
    }
    match grid_cell_mut(g, x, z, level) {
        Some(c) => {
            // keeps the other fields (alpha, back texture, leaf, door mode)
            let w = &mut c.walls[side as usize];
            w.texture = texture;
            w.open = open;
            1
        }
        None => 0,
    }
}

/// Animated texture over `count` existing texture ids; returns its own texture id or -1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_texture_anim(
    p: *mut PR_Renderer,
    frames: *const i32,
    count: i32,
    fps: f32,
) -> i32 {
    let Some(x) = r(p) else { return -1 };
    if frames.is_null() || count <= 0 {
        return -1;
    }
    x.add_texture_anim(std::slice::from_raw_parts(frames, count as usize), fps)
}

/// Scroll speed of texture `id` in texture coordinates per second; 1 = ok, 0 = unknown id.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_texture_scroll(p: *mut PR_Renderer, id: i32, du: f32, dv: f32) -> i32 {
    match r(p) {
        Some(x) => x.set_texture_scroll(id, du, dv) as i32,
        None => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn pr_anim_frame(time: f64, fps: f32, count: i32) -> i32 {
    px::anim_frame(time, fps, count.max(0) as u32) as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn pr_quantize_time(time: f64, fps: f32) -> f64 {
    px::quantize_time(time, fps)
}

// ---- Backdrops (background layers) ----

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PR_Backdrop {
    pub kind: i32,
    pub texture: i32,
    pub order: i32,
    pub flags: u32,
    pub azimuth: f32,
    pub elevation: f32,
    pub height: f32,
    pub repeats: f32,
    pub size: f32,
    pub blend: i32,
    pub alpha: f32,
}

fn backdrop_from_c(b: &PR_Backdrop) -> px::Backdrop {
    px::Backdrop {
        kind: b.kind,
        texture: b.texture,
        order: b.order,
        flags: b.flags,
        azimuth: b.azimuth,
        elevation: b.elevation,
        height: b.height,
        repeats: b.repeats,
        size: b.size,
        blend: b.blend,
        alpha: b.alpha,
    }
}

/// Persistent background layer; id >= 0, or -1 on a bad pointer, an invalid backdrop or when ids are used up.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_backdrop_create(p: *mut PR_Renderer, b: *const PR_Backdrop) -> i32 {
    let (Some(x), Some(b)) = (r(p), b.as_ref()) else {
        return -1;
    };
    match x.create_backdrop(backdrop_from_c(b)) {
        Some(id) if id <= i32::MAX as u32 => id as i32,
        Some(id) => {
            x.destroy_backdrop(id);
            -1
        }
        None => -1,
    }
}

/// 1 = changed, 0 = unknown id, invalid backdrop or bad pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_backdrop_set(
    p: *mut PR_Renderer,
    id: i32,
    b: *const PR_Backdrop,
) -> i32 {
    let (Some(x), Some(b)) = (r(p), b.as_ref()) else {
        return 0;
    };
    (id >= 0 && x.set_backdrop(id as u32, backdrop_from_c(b))) as i32
}

/// 1 = removed, 0 = unknown id or bad pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pr_backdrop_destroy(p: *mut PR_Renderer, id: i32) -> i32 {
    let Some(x) = r(p) else { return 0 };
    (id >= 0 && x.destroy_backdrop(id as u32)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;

    fn cam() -> PR_Camera {
        PR_Camera {
            x: 1.5,
            y: 0.5,
            z: 2.5,
            yaw: 0.0,
            pitch: 0.0,
            fov_y: 60.0,
            near_clip: 0.05,
            shift_x: 0.0,
            shift_y: 0.0,
        }
    }

    unsafe fn scene() -> (*mut PR_Renderer, *mut PR_Grid) {
        let r = pr_renderer_create(VERSION, 32, 32);
        let tex = [200u8, 200, 200, 255];
        let t = pr_texture_rgba(r, 1, 1, tex.as_ptr());
        let mut o: PR_Options = std::mem::zeroed();
        pr_default_options(&mut o);
        o.lighting = 1;
        pr_set_options(r, &o);
        pr_set_ambient(r, 0xffffff, 0.1);
        let g = pr_grid_create(3, 3, 1, 1.0);
        let mut c: PR_Cell = std::mem::zeroed();
        pr_default_cell(&mut c);
        c.floor_texture = t;
        c.ceiling_texture = t;
        for z in 0..3 {
            for x in 0..3 {
                c.walls[0].texture = if z == 0 { t } else { -1 };
                pr_grid_set_cell(g, x, z, 0, &c);
            }
        }
        (r, g)
    }

    unsafe fn frame(r: *mut PR_Renderer, g: *mut PR_Grid) -> Vec<u8> {
        let c = cam();
        pr_begin(r, &c);
        pr_draw_grid(r, g);
        pr_end(r);
        slice::from_raw_parts(pr_pixels_rgba(r), 32 * 32 * 4).to_vec()
    }

    #[test]
    fn capi_draws_after_end_ignored_and_stats() {
        unsafe {
            let (r, g) = scene();
            let base = frame(r, g);
            let depth = slice::from_raw_parts(pr_depth(r), 32 * 32).to_vec();
            let dot = PR_Particle {
                x: 1.5,
                y: 0.5,
                z: 1.5,
                size: 0.4,
                color: 0xff0000,
                texture: -1,
                flags: 1,
                alpha: 1.0,
            };
            let s = PR_Sprite {
                x: 1.5,
                y: 0.0,
                z: 1.0,
                width: 0.6,
                height: 1.0,
                texture: 0,
                flags: 1,
                yaw: 0.0,
                light: 1.0,
                tag: 1,
                alpha: 1.0,
                blend: 0,
            };
            let tri = [
                Vertex {
                    x: 1.0,
                    y: 0.0,
                    z: 1.0,
                    u: 0.0,
                    v: 0.0,
                },
                Vertex {
                    x: 2.0,
                    y: 0.0,
                    z: 1.0,
                    u: 1.0,
                    v: 0.0,
                },
                Vertex {
                    x: 1.5,
                    y: 1.0,
                    z: 1.0,
                    u: 0.0,
                    v: 1.0,
                },
            ];
            pr_draw_grid(r, g);
            pr_draw_sprite(r, &s);
            pr_draw_triangles(r, tri.as_ptr(), 3, 0, 1.0, 0);
            pr_draw_triangles_ex(r, tri.as_ptr(), 3, 0, 1.0, 0, 1, 2, 0.5, 1);
            pr_draw_particles(r, &dot, 1);
            pr_draw_emitters(r, g);
            assert_eq!(
                slice::from_raw_parts(pr_pixels_rgba(r), 32 * 32 * 4),
                base.as_slice()
            );
            assert_eq!(
                slice::from_raw_parts(pr_depth(r), 32 * 32),
                depth.as_slice()
            );
            assert_eq!(frame(r, g), base, "nothing carried into the next frame");
            let mut st = PR_Stats {
                lights_used: 9,
                ..PR_Stats::default()
            };
            assert_eq!(pr_stats(ptr::null_mut(), &mut st), 0);
            assert_eq!(st, PR_Stats::default());
            assert_eq!(pr_stats(r, ptr::null_mut()), 0);
            let many = vec![dot; px::particles::PARTICLE_BUDGET + 3];
            pr_begin(r, &cam());
            pr_draw_particles(r, many.as_ptr(), many.len() as i32);
            pr_end(r);
            assert_eq!(pr_stats(r, &mut st), 1);
            assert_eq!(st.dropped_particles, 3);
            pr_grid_destroy(g);
            pr_renderer_destroy(r);
        }
    }

    #[test]
    fn capi_particles_and_emitters() {
        unsafe {
            let (r, g) = scene();
            let base = frame(r, g);
            let e = PR_Emitter {
                kind: px::particles::EMIT_SPARKS,
                x: 1.5,
                y: 0.2,
                z: 1.5,
                size_x: 0.3,
                size_y: 0.0,
                size_z: 0.3,
                rate: 50.0,
                speed: 0.0,
                size: 0.1,
                color: 0xffa040,
                texture: -1,
                wind_x: 0.0,
                wind_z: 0.0,
                seed: 3,
            };
            assert_eq!(pr_emitter_create(ptr::null_mut(), &e), -1);
            assert_eq!(pr_emitter_create(r, ptr::null()), -1);
            let id = pr_emitter_create(r, &e);
            assert!(id >= 0);
            pr_set_time(r, 1.0);
            let c = cam();
            let draw = |grid: *const PR_Grid| {
                pr_begin(r, &c);
                pr_draw_grid(r, g);
                pr_draw_emitters(r, grid);
                pr_end(r);
                slice::from_raw_parts(pr_pixels_rgba(r), 32 * 32 * 4).to_vec()
            };
            let sparks = draw(ptr::null());
            assert_ne!(sparks, base);
            assert_eq!(draw(g), sparks, "sparks are not culled by the grid");
            // pixelraum: pr_emitter_set(rate 0) == 1 and the frame is `base` again. knorke-render
            // has no set_emitter yet, so the call reports 0 and nothing changes.
            assert_eq!(pr_emitter_set(r, id, &PR_Emitter { rate: 0.0, ..e }), 0);
            assert_eq!(draw(g), sparks, "unchanged emitter");
            assert_eq!(pr_emitter_destroy(r, id), 1);
            assert_eq!(pr_emitter_destroy(r, id), 0);
            assert_eq!(pr_emitter_set(r, id, &e), 0);
            assert!(pr_emitter_create(r, &e) > id, "ids are not reused");
            pr_draw_emitters(ptr::null_mut(), g);
            let dot = PR_Particle {
                x: 1.5,
                y: 0.5,
                z: 1.5,
                size: 0.2,
                color: 0xff0000,
                texture: -1,
                flags: 1,
                alpha: 1.0,
            };
            pr_begin(r, &c);
            pr_draw_particles(r, ptr::null(), 4);
            pr_draw_particles(r, &dot, -1);
            pr_draw_particles(r, &dot, 1);
            pr_end(r);
            pr_draw_particles(ptr::null_mut(), &dot, 1);
            pr_grid_destroy(g);
            pr_renderer_destroy(r);
        }
    }

    #[test]
    fn capi_persistent_lights() {
        unsafe {
            let (r, g) = scene();
            let l = PR_Light {
                x: 1.5,
                y: 0.8,
                z: 1.5,
                color: 0xffcc88,
                strength: 1.0,
                radius: 4.0,
                flicker: 0.0,
                seed: 1,
                shadows: 1,
            };
            let base = frame(r, g);
            let id = pr_light_create(r, &l);
            assert!(id >= 0);
            let lit = frame(r, g);
            assert_ne!(base, lit);
            assert_eq!(frame(r, g), lit, "light stays across frames");
            let dim = PR_Light { strength: 0.3, ..l };
            assert_eq!(pr_light_set(r, id, &dim), 1);
            let dimmed = frame(r, g);
            assert!(dimmed != lit && dimmed != base);
            assert_eq!(pr_light_destroy(r, id), 1);
            assert_eq!(frame(r, g), base, "light gone after destroy");
            assert_eq!(pr_light_set(r, id, &l), 0, "destroyed id");
            assert_eq!(pr_light_destroy(r, id), 0);
            assert_eq!(pr_light_destroy(r, -1), 0);
            assert_eq!(pr_light_set(r, -1, &l), 0);
            let id2 = pr_light_create(r, &l);
            assert!(id2 >= 0 && id2 != id);
            // null safety
            assert_eq!(pr_light_create(std::ptr::null_mut(), &l), -1);
            assert_eq!(pr_light_create(r, std::ptr::null()), -1);
            assert_eq!(pr_light_set(std::ptr::null_mut(), id2, &l), 0);
            assert_eq!(pr_light_set(r, id2, std::ptr::null()), 0);
            assert_eq!(pr_light_destroy(std::ptr::null_mut(), id2), 0);
            // PR_Options round trip and clamping
            let mut o: PR_Options = std::mem::zeroed();
            pr_get_options(r, &mut o);
            o.shadow_res = 0;
            o.point_shadow_res = 1_000_000;
            pr_set_options(r, &o);
            pr_get_options(r, &mut o);
            assert_eq!((o.shadow_res, o.point_shadow_res, o.lighting), (1, 4096, 1));
            pr_grid_destroy(g);
            pr_renderer_destroy(r);
        }
    }

    #[test]
    fn texture_anim_and_scroll() {
        unsafe {
            let r = pr_renderer_create(VERSION, 8, 8);
            let px = [10u8, 20, 30, 255];
            let a = pr_texture_rgba(r, 1, 1, px.as_ptr());
            let b = pr_texture_rgba(r, 1, 1, px.as_ptr());
            let frames = [a, b];
            assert_eq!(pr_texture_anim(r, frames.as_ptr(), 0, 4.0), -1);
            assert_eq!(pr_texture_anim(r, std::ptr::null(), 2, 4.0), -1);
            assert_eq!(pr_texture_anim(r, frames.as_ptr(), 2, -1.0), -1);
            let anim = pr_texture_anim(r, frames.as_ptr(), 2, 4.0);
            assert_eq!(anim, 2);
            let bad = [anim];
            assert_eq!(pr_texture_anim(r, bad.as_ptr(), 1, 4.0), -1);
            assert_eq!(pr_texture_scroll(r, anim, 0.5, 0.0), 1);
            assert_eq!(pr_texture_scroll(r, 7, 0.5, 0.0), 0);
            assert_eq!(pr_texture_scroll(std::ptr::null_mut(), 0, 0.5, 0.0), 0);
            pr_renderer_destroy(r);
        }
    }

    #[test]
    fn texture_contour_calls() {
        unsafe {
            let r = pr_renderer_create(VERSION, 8, 8);
            let px = [100u8; 4 * 4 * 4];
            let t = pr_texture_rgba(r, 4, 4, px.as_ptr());
            let mask = [1u8; 16];
            assert_eq!(pr_texture_set_contour(r, t, mask.as_ptr(), 4, 4), 1);
            assert_eq!(
                pr_texture_set_contour(r, t, mask.as_ptr(), 2, 8),
                0,
                "size must match"
            );
            assert_eq!(pr_texture_set_contour(r, 9, mask.as_ptr(), 4, 4), 0);
            assert_eq!(pr_texture_set_contour(r, t, std::ptr::null(), 0, 0), 1);
            assert_eq!(pr_texture_contour_auto(r, t, 1.0), 1);
            assert_eq!(pr_texture_contour_auto(r, t, 0.0), 1);
            assert_eq!(
                pr_texture_set_level_contour(r, t, 1, mask.as_ptr(), 2, 2),
                0,
                "built level"
            );
            let pal = [0u8, 0, 0, 200, 200, 200];
            let lp = [1u8; 4];
            assert_eq!(
                pr_texture_add_level_indexed(r, t, 2, 2, lp.as_ptr(), pal.as_ptr(), 2, -1),
                1
            );
            assert_eq!(
                pr_texture_set_level_contour(r, t, 1, mask.as_ptr(), 2, 2),
                1
            );
            assert_eq!(
                pr_texture_set_level_contour(r, t, 2, mask.as_ptr(), 2, 2),
                0
            );
            assert_eq!(
                pr_texture_set_level_contour(r, t, -1, mask.as_ptr(), 2, 2),
                0
            );
            let mut o = std::mem::zeroed::<PR_Options>();
            pr_default_options(&mut o);
            assert_eq!(o.contour_falloff, 0.0);
            pr_renderer_destroy(r);
        }
    }

    /// PR_Options must match include/pixelraum.h field by field (order, offsets, size); any drift
    /// between the header and this mirror shows here.
    #[test]
    fn options_layout_matches_header() {
        let want = [
            (
                std::mem::offset_of!(PR_Options, struct_size),
                0,
                "struct_size",
            ),
            (
                std::mem::offset_of!(PR_Options, background),
                4,
                "background",
            ),
            (std::mem::offset_of!(PR_Options, lighting), 8, "lighting"),
            (
                std::mem::offset_of!(PR_Options, shadow_size),
                12,
                "shadow_size",
            ),
            (
                std::mem::offset_of!(PR_Options, shadow_res),
                16,
                "shadow_res",
            ),
            (
                std::mem::offset_of!(PR_Options, point_shadow_res),
                20,
                "point_shadow_res",
            ),
            (
                std::mem::offset_of!(PR_Options, shadow_bias),
                24,
                "shadow_bias",
            ),
            (
                std::mem::offset_of!(PR_Options, shadow_soft),
                28,
                "shadow_soft",
            ),
            (
                std::mem::offset_of!(PR_Options, point_shadow_soft),
                32,
                "point_shadow_soft",
            ),
            (
                std::mem::offset_of!(PR_Options, light_levels),
                36,
                "light_levels",
            ),
            (std::mem::offset_of!(PR_Options, sun_dir), 40, "sun_dir"),
            (
                std::mem::offset_of!(PR_Options, sun_strength),
                52,
                "sun_strength",
            ),
            (std::mem::offset_of!(PR_Options, fog_start), 56, "fog_start"),
            (std::mem::offset_of!(PR_Options, fog_end), 60, "fog_end"),
            (std::mem::offset_of!(PR_Options, fog_min), 64, "fog_min"),
            (std::mem::offset_of!(PR_Options, fog_color), 68, "fog_color"),
            (std::mem::offset_of!(PR_Options, quantize), 72, "quantize"),
            (std::mem::offset_of!(PR_Options, dither), 76, "dither"),
            (
                std::mem::offset_of!(PR_Options, dither_strength),
                80,
                "dither_strength",
            ),
            (std::mem::offset_of!(PR_Options, snap), 84, "snap"),
            (std::mem::offset_of!(PR_Options, outlines), 88, "outlines"),
            (
                std::mem::offset_of!(PR_Options, outline_depth),
                92,
                "outline_depth",
            ),
            (
                std::mem::offset_of!(PR_Options, outline_normal),
                96,
                "outline_normal",
            ),
            (
                std::mem::offset_of!(PR_Options, outline_color),
                100,
                "outline_color",
            ),
            (std::mem::offset_of!(PR_Options, pick), 104, "pick"),
            (
                std::mem::offset_of!(PR_Options, draw_distance),
                108,
                "draw_distance",
            ),
            (
                std::mem::offset_of!(PR_Options, texture_lod),
                112,
                "texture_lod",
            ),
            (std::mem::offset_of!(PR_Options, lod_bias), 116, "lod_bias"),
            (
                std::mem::offset_of!(PR_Options, lod_dither),
                120,
                "lod_dither",
            ),
            (std::mem::offset_of!(PR_Options, face_lod), 124, "face_lod"),
            (
                std::mem::offset_of!(PR_Options, contour_falloff),
                128,
                "contour_falloff",
            ),
            (std::mem::offset_of!(PR_Options, threads), 132, "threads"),
            (
                std::mem::offset_of!(PR_Options, shadow_stipple),
                136,
                "shadow_stipple",
            ),
        ];
        for (got, exp, name) in want {
            assert_eq!(got, exp, "offset of {name}");
        }
        assert_eq!(std::mem::size_of::<PR_Options>(), 140);
        assert_eq!(std::mem::align_of::<PR_Options>(), 4);
    }

    #[test]
    fn create_checks_header_version() {
        unsafe {
            for v in [VERSION, VERSION ^ 0x7f] {
                let r = pr_renderer_create(v, 8, 8);
                assert!(!r.is_null(), "patch {v:#x} may differ");
                pr_renderer_destroy(r);
            }
            for v in [VERSION + 0x100, VERSION - 0x100, VERSION + 0x10000, 0] {
                assert!(
                    pr_renderer_create(v, 8, 8).is_null(),
                    "{v:#x} must be rejected"
                );
            }
            assert!(pr_renderer_create(VERSION, 0, 8).is_null());
        }
    }

    #[test]
    fn set_options_checks_struct_size() {
        unsafe {
            let r = pr_renderer_create(VERSION, 8, 8);
            let mut o: PR_Options = std::mem::zeroed();
            pr_default_options(&mut o);
            assert_eq!(o.struct_size as usize, std::mem::size_of::<PR_Options>());
            o.fog_end = 5.0;
            let mut bad = o;
            bad.struct_size -= 4;
            bad.fog_end = 9.0;
            assert_eq!(pr_set_options(r, &bad), 0);
            let mut got: PR_Options = std::mem::zeroed();
            pr_get_options(r, &mut got);
            assert_eq!(
                got.fog_end, 0.0,
                "rejected options leave the renderer unchanged"
            );
            assert_eq!(got.struct_size, o.struct_size);
            assert_eq!(pr_set_options(r, &o), 1);
            pr_get_options(r, &mut got);
            assert_eq!(got.fog_end, 5.0);
            assert_eq!(pr_set_options(std::ptr::null_mut(), &o), 0);
            assert_eq!(pr_set_options(r, std::ptr::null()), 0);
            pr_renderer_destroy(r);
        }
    }

    #[test]
    fn camera_layout_matches_header() {
        assert_eq!(std::mem::offset_of!(PR_Camera, near_clip), 24);
        assert_eq!(std::mem::offset_of!(PR_Camera, shift_x), 28);
        assert_eq!(std::mem::offset_of!(PR_Camera, shift_y), 32);
        assert_eq!(std::mem::size_of::<PR_Camera>(), 36);
    }

    #[test]
    fn wall_pick_hit_layout_matches_header() {
        let want = [
            (std::mem::offset_of!(PR_Wall, texture), 0),
            (std::mem::offset_of!(PR_Wall, open), 4),
            (std::mem::offset_of!(PR_Wall, alpha), 8),
            (std::mem::offset_of!(PR_Wall, back_texture), 12),
            (std::mem::offset_of!(PR_Wall, leaf_texture), 16),
            (std::mem::offset_of!(PR_Wall, door_mode), 20),
            (std::mem::offset_of!(PR_Wall, leaf_u0), 24),
            (std::mem::offset_of!(PR_Wall, leaf_u1), 28),
            (std::mem::offset_of!(PR_Wall, leaf_top), 32),
            (std::mem::offset_of!(PR_Wall, swing_angle), 36),
        ];
        for (k, (got, exp)) in want.iter().enumerate() {
            assert_eq!(got, exp, "PR_Wall field {k}");
        }
        assert_eq!(std::mem::size_of::<PR_Wall>(), 40);
        assert_eq!(std::mem::offset_of!(PR_Cell, walls), 24);
        assert_eq!(std::mem::offset_of!(PR_Cell, floor_corners), 24 + 160);
        assert_eq!(std::mem::size_of::<PR_Cell>(), 24 + 160 + 32);
        assert_eq!(std::mem::offset_of!(PR_Pick, part), 48);
        assert_eq!(std::mem::offset_of!(PR_Pick, level), 52);
        assert_eq!(std::mem::size_of::<PR_Pick>(), 56);
        assert_eq!(std::mem::offset_of!(PR_GridHit, part), 32);
        assert_eq!(std::mem::offset_of!(PR_GridHit, level), 36);
        assert_eq!(std::mem::size_of::<PR_GridHit>(), 40);
    }

    #[test]
    fn backdrop_round_trip() {
        unsafe {
            let r = pr_renderer_create(VERSION, 16, 16);
            let tex = [10u8, 200, 30, 255];
            let t = pr_texture_rgba(r, 1, 1, tex.as_ptr());
            let mut b = PR_Backdrop {
                kind: 0,
                texture: t,
                order: 0,
                flags: 0,
                azimuth: 0.0,
                elevation: -45.0,
                height: 90.0,
                repeats: 1.0,
                size: 0.0,
                blend: 0,
                alpha: 1.0,
            };
            let id = pr_backdrop_create(r, &b);
            assert!(id >= 0);
            pr_begin(r, &cam());
            pr_end(r);
            assert_eq!(
                &std::slice::from_raw_parts(pr_pixels_rgba(r), 4)[..3],
                &[10, 200, 30]
            );
            b.height = 0.0;
            assert_eq!(pr_backdrop_set(r, id, &b), 0, "invalid band");
            assert_eq!(pr_backdrop_create(r, &b), -1);
            b.height = 90.0;
            b.kind = 1;
            assert_eq!(pr_backdrop_set(r, id, &b), 1, "now a spot");
            assert_eq!(pr_backdrop_set(r, id, ptr::null()), 0);
            assert_eq!(pr_backdrop_create(ptr::null_mut(), &b), -1);
            assert_eq!(pr_backdrop_destroy(r, id), 1);
            assert_eq!(pr_backdrop_destroy(r, id), 0);
            assert_eq!(pr_backdrop_set(r, id, &b), 0);
            assert!(pr_backdrop_create(r, &b) > id, "ids are not reused");
            assert_eq!(std::mem::size_of::<PR_Backdrop>(), 44);
            assert_eq!(std::mem::offset_of!(PR_Backdrop, alpha), 40);
            pr_renderer_destroy(r);
        }
    }
}
