//! Pixel parity with pixelraum @ 81927d8 (ADR 0024): the Wizardry-style scene below, driven
//! through the C API, gives the same bytes (RGBA, palette indices, depth, four picks, three
//! frames) as pixelraum's own C API did. The hash was taken from a scratch build of pixelraum
//! with the identical scene (same calls, same order).
//!
//! Lighting, fog and alpha blending are left out on purpose: since ADR 0013 §6 Knorke computes
//! them in linear light, so those pixels differ from pixelraum by design.
#![allow(unsafe_code, unsafe_op_in_unsafe_fn, missing_docs)]

use knorke_integration::pixel::prelude::Vertex;
use knorke_pixel_c::*;
const PR_VER: u32 = VERSION;

use std::io::Write;
use std::slice;

unsafe fn tex(r: *mut PR_Renderer, w: i32, h: i32, seed: u32, pal: &[u8], transp: i32) -> i32 {
    let px: Vec<u8> = (0..w * h)
        .map(|i| {
            ((i as u32).wrapping_mul(2654435761).wrapping_add(seed) >> 24) as u8 % 16
                + (seed % 8) as u8 * 16
        })
        .collect();
    let id = pr_texture_indexed(r, w, h, px.as_ptr(), pal.as_ptr(), 256, transp);
    let half: Vec<u8> = (0..(w / 2) * (h / 2))
        .map(|i| px[(i * 2) as usize])
        .collect();
    assert_eq!(
        pr_texture_add_level_indexed(
            r,
            id,
            w / 2,
            h / 2,
            half.as_ptr(),
            pal.as_ptr(),
            256,
            transp
        ),
        1
    );
    assert_eq!(
        pr_texture_update_indexed(r, id, w, h, px.as_ptr(), pal.as_ptr(), 256, transp),
        1
    );
    assert_eq!(
        pr_texture_add_level_indexed(
            r,
            id,
            w / 2,
            h / 2,
            half.as_ptr(),
            pal.as_ptr(),
            256,
            transp
        ),
        1
    );
    pr_texture_contour_auto(r, id, 0.5);
    id
}

/// Renders the scene with the switches pixelraum and Knorke still share bit for bit.
fn render() -> Vec<u8> {
    unsafe {
        let (w, h) = (160, 100);
        let r = pr_renderer_create(PR_VER, w, h);
        assert!(!r.is_null());
        let pal: Vec<u8> = (0..256u32)
            .flat_map(|i| [(i * 7 % 256) as u8, (i * 3 % 256) as u8, (255 - i) as u8])
            .collect();
        assert_eq!(pr_set_palette(r, pal.as_ptr(), 256), 1);
        let wall = tex(r, 16, 16, 11, &pal, -1);
        let floor = tex(r, 16, 16, 37, &pal, -1);
        let spr = tex(r, 8, 16, 5, &pal, 0);
        let mut o: PR_Options = std::mem::zeroed();
        pr_default_options(&mut o);
        o.background = 0x102030;
        o.quantize = 1;
        o.draw_distance = 14.0;
        o.fog_start = 1.56;
        o.fog_end = 4.63;
        o.fog_min = 0.1;
        o.fog_color = 0;
        o.texture_lod = 1;
        o.face_lod = 1;
        o.lighting = 1;
        o.shadow_size = 9.0;
        o.shadow_res = 512;
        o.shadow_soft = 0.2;
        o.pick = 1;
        o.threads = 1;
        // light, fog and alpha blending are off: Knorke shades them in linear light (ADR 0013 §6)
        let env = |k: &str| !matches!(k, "LIGHT" | "FOG" | "BLEND");
        if !env("LIGHT") {
            o.lighting = 0;
        }
        if !env("FOG") {
            o.fog_end = 0.0;
        }
        if !env("QUANT") {
            o.quantize = 0;
        }
        if !env("LOD") {
            o.texture_lod = 0;
            o.face_lod = 0;
        }
        if !env("SOFT") {
            o.shadow_soft = 0.0;
        }
        assert_eq!(pr_set_options(r, &o), 1);
        let g = pr_grid_create(6, 6, 1, 1.0);
        let mut c: PR_Cell = std::mem::zeroed();
        pr_default_cell(&mut c);
        c.floor_texture = floor;
        c.ceiling_texture = -1;
        c.ceiling_height = 1.2;
        for z in 0..6 {
            for x in 0..6 {
                assert_eq!(pr_grid_set_cell(g, x, z, 0, &c), 1);
                if x == 0 || z == 0 || (x == 3 && z == 2) {
                    pr_grid_set_wall(g, x, z, 0, (x + z) % 4, wall, 0.0);
                }
            }
        }
        let mut back: PR_Cell = std::mem::zeroed();
        assert_eq!(pr_grid_get_cell(g, 3, 2, 0, &mut back), 1);
        back.walls[1].texture = wall;
        back.walls[1].open = 0.4;
        pr_grid_set_cell(g, 3, 2, 0, &back);
        let sun = PR_Sun {
            dir: [0.4, -1.0, 0.3],
            color: 0xfff0d0,
            strength: if env("SUN") { 0.8 } else { 0.0 },
            shadows: env("SHADOW") as i32,
        };
        pr_set_sun(r, &sun);
        pr_set_ambient(r, 0x8090a0, 0.25);
        let mut l = PR_Light {
            x: 2.5,
            y: 0.8,
            z: 3.5,
            color: 0xffaa55,
            strength: 1.2,
            radius: 4.0,
            flicker: if env("FLICKER") { 0.3 } else { 0.0 },
            seed: 7,
            shadows: env("SHADOW") as i32,
        };
        if !env("TORCH") {
            l.strength = 0.0;
        }
        let torch = pr_light_create(r, &l);
        let mut out = Vec::new();
        for (k, t) in [0.0f64, 0.37, 1.9].iter().enumerate() {
            if k == 2 {
                assert_eq!(pr_renderer_resize(r, w, h), 1);
            }
            pr_set_time(r, *t);
            l.x = 2.5 + k as f32 * 0.3;
            pr_light_set(r, torch, &l);
            let cam = PR_Camera {
                x: 2.5,
                y: 0.6,
                z: 5.2,
                yaw: 0.2 * k as f32,
                pitch: -0.05,
                fov_y: 60.0,
                near_clip: 0.05,
                shift_x: 0.0,
                shift_y: 0.0,
            };
            pr_begin(r, &cam);
            pr_draw_grid(r, g);
            let s = PR_Sprite {
                x: 2.2,
                y: 0.0,
                z: 3.0,
                width: 0.5,
                height: 1.0,
                texture: spr,
                flags: 1,
                yaw: 0.0,
                light: 1.0,
                tag: 9,
                alpha: 1.0,
                blend: 0,
            };
            pr_draw_sprite(r, &s);
            let tri = [
                Vertex {
                    x: 1.0,
                    y: 0.0,
                    z: 2.0,
                    u: 0.0,
                    v: 0.0,
                },
                Vertex {
                    x: 2.0,
                    y: 0.0,
                    z: 2.0,
                    u: 1.0,
                    v: 0.0,
                },
                Vertex {
                    x: 1.5,
                    y: 0.9,
                    z: 2.0,
                    u: 0.5,
                    v: 1.0,
                },
            ];
            pr_draw_triangles(r, tri.as_ptr(), 3, wall, 1.0, 0);
            let tri2 = [
                Vertex {
                    x: 3.0,
                    y: 0.1,
                    z: 3.0,
                    u: 0.0,
                    v: 0.0,
                },
                Vertex {
                    x: 4.0,
                    y: 0.1,
                    z: 3.0,
                    u: 1.0,
                    v: 0.0,
                },
                Vertex {
                    x: 3.5,
                    y: 0.8,
                    z: 3.5,
                    u: 0.5,
                    v: 1.0,
                },
            ];
            if env("BLEND") {
                pr_draw_triangles_ex(r, tri2.as_ptr(), 3, floor, 1.0, 0, 1, 4, 0.6, 1);
            }
            pr_end(r);
            out.write_all(slice::from_raw_parts(
                pr_pixels_rgba(r),
                (w * h * 4) as usize,
            ))
            .unwrap();
            out.write_all(slice::from_raw_parts(
                pr_pixels_indexed(r),
                (w * h) as usize,
            ))
            .unwrap();
            for d in slice::from_raw_parts(pr_depth(r), (w * h) as usize) {
                out.write_all(&d.to_le_bytes()).unwrap();
            }
            for (px, py) in [(80, 50), (10, 90), (40, 60), (120, 30)] {
                let mut pk: PR_Pick = std::mem::zeroed();
                let hit = pr_pick(r, px, py, &mut pk);
                writeln!(
                    out,
                    "{hit} {} {} {} {} {} {:?}",
                    pk.kind,
                    pk.tag,
                    pk.cell_x,
                    pk.cell_z,
                    pk.side,
                    (pk.x, pk.y, pk.z, pk.depth, pk.nx, pk.ny, pk.nz)
                )
                .unwrap();
            }
        }
        out
    }
}

fn fnv1a(d: &[u8]) -> u64 {
    d.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

#[test]
fn same_bytes_as_pixelraum_without_light_fog_and_blending() {
    let out = render();
    assert_eq!(out.len(), 432_784);
    assert_eq!(fnv1a(&out), 0xa713_6e0e_6fff_7175);
}
