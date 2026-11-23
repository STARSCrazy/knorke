//! Image tests: render fixed scenes and compare with reference images in tests/golden (binary PPM).
//! New or changed references: run with PIXELRAUM_BLESS=1, then look at the PNGs in CARGO_TARGET_TMPDIR/golden-out.
#![cfg(feature = "pixel")]
#![allow(
    clippy::too_many_arguments,
    clippy::needless_range_loop,
    clippy::approx_constant
)]
#![allow(clippy::disallowed_methods)] // timing tests read the clock on purpose (host side)
mod common;

use knorke_integration::pixel::prelude::*;
use std::path::PathBuf;

const W: usize = 160;
const H: usize = 100;

/// Orthonormal `(right, up)` for a light direction, as the sun shadow map builds it
/// (copy of `knorke-lighting`'s private `shadow::basis`).
fn basis(fwd: Vec3) -> (Vec3, Vec3) {
    let n = |v: Vec3| {
        let l = v.length();
        if l > 0.0 {
            v * (1.0 / l)
        } else {
            Vec3::new(0.0, 0.0, 0.0)
        }
    };
    let f = n(fwd);
    let reference = if f.y.abs() > 0.99 {
        Vec3::new(0.0, 0.0, 1.0)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    };
    let right = n(reference.cross(f));
    let up = f.cross(right);
    (right, up)
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

fn to_ppm(rgba: &[u8]) -> Vec<u8> {
    let mut v = format!("P6\n{W} {H}\n255\n").into_bytes();
    for p in rgba.as_chunks::<4>().0 {
        v.extend_from_slice(&p[..3]);
    }
    v
}

fn check(name: &str, rgba: &[u8]) {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("golden-out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(
        out.join(format!("{name}.png")),
        png::encode_rgba(W, H, rgba, 4),
    )
    .unwrap();
    let path = golden_dir().join(format!("{name}.ppm"));
    let actual = to_ppm(rgba);
    if std::env::var("PIXELRAUM_BLESS").is_ok() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read(&path)
        .unwrap_or_else(|_| panic!("missing reference {path:?}; run with PIXELRAUM_BLESS=1"));
    if expected != actual {
        let diff = expected.iter().zip(&actual).filter(|(a, b)| a != b).count();
        panic!(
            "{name}: image differs from reference ({diff} bytes); see target/golden-out/{name}.png"
        );
    }
}

fn render(name: &str, opts: Options, palette: bool, cam: Camera, time: f64) -> Vec<u8> {
    let mut s = common::scene(W, H);
    if palette {
        s.renderer.set_palette(&common::palette16());
    }
    s.renderer.options = opts;
    common::render(&mut s, &cam, time);
    let img = s.renderer.rgba().to_vec();
    check(name, &img);
    img
}

#[test]
fn plain_front() {
    render(
        "plain_front",
        Options::default(),
        false,
        common::camera(),
        0.0,
    );
}

#[test]
fn look_up_and_turn() {
    let cam = Camera {
        yaw: 0.8,
        pitch: 0.4,
        position: Vec3::new(2.0, 0.5, 4.5),
        ..common::camera()
    };
    render("look_up_turn", Options::default(), false, cam, 0.0);
}

#[test]
fn second_animation_frame() {
    render(
        "anim_frame1",
        Options::default(),
        false,
        common::camera(),
        0.3,
    );
}

#[test]
fn doom_light() {
    let o = Options {
        fog_start: 1.0,
        fog_end: 6.0,
        fog_min: 0.2,
        light_levels: 8,
        ..Options::default()
    };
    render("doom_light", o, false, common::camera(), 0.0);
}

#[test]
fn palette_dither() {
    let o = Options {
        quantize: true,
        dither: true,
        dither_strength: 32.0,
        sun_dir: [0.4, -0.7, 0.6],
        sun_strength: 0.4,
        ..Options::default()
    };
    render("palette_dither", o, true, common::camera(), 0.0);
}

#[test]
fn outlines() {
    let o = Options {
        outlines: true,
        light_levels: 4,
        ..Options::default()
    };
    render("outlines", o, false, common::camera(), 0.0);
}

#[test]
fn same_input_same_image() {
    let a = render(
        "plain_front",
        Options::default(),
        false,
        common::camera(),
        0.0,
    );
    let b = render(
        "plain_front",
        Options::default(),
        false,
        common::camera(),
        0.0,
    );
    assert_eq!(a, b);
}

/// Without any effect, a texel seen straight on at 1:1 keeps its exact colour, and an indexed
/// output with the texture's own palette gives back the original palette indices.
#[test]
fn original_pixels_survive() {
    let pal = common::palette16();
    let (w, h) = (16usize, 16usize);
    let idx: Vec<u8> = (0..w * h).map(|i| (i % 16) as u8).collect();
    let mut r = Renderer::new(w, h).unwrap();
    r.set_palette(&pal);
    let t = r.add_texture(Texture::from_indexed(w, h, &idx, &pal, None).unwrap());
    // A quad that fills the screen exactly: 1 unit wide at distance d with focal = h/2/tan(fov/2).
    let fov: f32 = 60.0;
    let d = 0.5 / (fov.to_radians() * 0.5).tan();
    let cam = Camera {
        position: Vec3::new(0.5, 0.5, d),
        yaw: 0.0,
        pitch: 0.0,
        fov_y: fov,
        near: 0.01,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    r.begin(&cam);
    let v = |x, y, u, v| Vertex { x, y, z: 0.0, u, v };
    let quad = [
        v(0.0, 1.0, 0.0, 0.0),
        v(1.0, 1.0, 1.0, 0.0),
        v(1.0, 0.0, 1.0, 1.0),
        v(0.0, 1.0, 0.0, 0.0),
        v(1.0, 0.0, 1.0, 1.0),
        v(0.0, 0.0, 0.0, 1.0),
    ];
    r.draw_triangles(&quad, t, 1.0, false);
    r.end();
    assert_eq!(r.indexed(), &idx[..]);
}

#[test]
fn depth_and_normals_exported() {
    let mut s = common::scene(W, H);
    common::render(&mut s, &common::camera(), 0.0);
    let d = s.renderer.depth();
    let n = s.renderer.normals();
    let mid = (H / 2 + 20) * W + W / 2; // floor below the horizon
    assert!(d[mid].is_finite() && d[mid] > 0.0);
    assert!(
        (n[mid * 3 + 1] - 1.0).abs() < 1e-6,
        "floor normal points up"
    );
}

#[test]
fn snapping_moves_in_steps() {
    let mut a = common::scene(W, H);
    let mut b = common::scene(W, H);
    let o = Options {
        snap: 0.05,
        ..Options::default()
    };
    a.renderer.options = o;
    b.renderer.options = o;
    let c1 = common::camera();
    let c2 = Camera {
        position: c1.position + Vec3::new(0.01, 0.0, 0.0),
        ..c1
    };
    common::render(&mut a, &c1, 0.0);
    common::render(&mut b, &c2, 0.0);
    assert_eq!(
        a.renderer.rgba(),
        b.renderer.rgba(),
        "small moves below the snap step change nothing"
    );
}

#[test]
fn anim_helpers() {
    assert_eq!(anim_frame(0.0, 12.0, 4), 0);
    assert_eq!(anim_frame(0.25, 12.0, 4), 3);
    assert_eq!(anim_frame(0.34, 12.0, 4), 0);
    assert!((quantize_time(0.13, 10.0) - 0.1).abs() < 1e-9);
}

/// Inside the roofed room: walls must end exactly at the ceiling.
#[test]
fn inside_room() {
    let cam = Camera {
        position: Vec3::new(3.5, 0.5, 3.4),
        pitch: 0.3,
        ..common::camera()
    };
    render("inside_room", Options::default(), false, cam, 0.0);
}

#[test]
fn lighting_off_by_default() {
    assert!(!Options::default().lighting);
}

#[test]
fn light_ids_stable() {
    let mut r = Renderer::new(W, H).unwrap();
    let l = |x: f32| light::PointLight {
        pos: Vec3::new(x, 1.0, 0.0),
        color: 0xffffff,
        strength: 1.0,
        radius: 5.0,
        flicker: 0.0,
        seed: 0,
        shadows: false,
    };
    let a = r.create_light(l(0.0));
    let b = r.create_light(l(1.0));
    assert_ne!(a, b);
    assert!(r.destroy_light(a));
    let c = r.create_light(l(2.0));
    assert!(c != a && c != b, "ids are not reused");
    assert_eq!(r.light(b).map(|x| x.pos.x), Some(1.0));
    assert!(r.set_light(b, l(3.0)));
    assert_eq!(r.light(b).map(|x| x.pos.x), Some(3.0));
    assert!(!r.set_light(a, l(4.0)), "destroyed id");
    assert!(!r.destroy_light(a), "destroyed twice");
    assert!(r.light(a).is_none());
    // lights stay across frames
    r.begin(&Camera::default());
    r.end();
    assert!(r.light(c).is_some());
}

// ---- Lighting (sun + ambient) ----

fn sun(dir: [f32; 3]) -> light::Sun {
    light::Sun {
        dir: Vec3::new(dir[0], dir[1], dir[2]),
        color: 0xfff0d0,
        strength: 1.0,
        shadows: true,
    }
}

fn lit_options() -> Options {
    Options {
        lighting: true,
        shadow_size: 8.0,
        ..Options::default()
    }
}

/// Open 9 x 9 courtyard (no ceilings), optionally with one two-high wall along z = 4 from x = 3 to 6.
fn courtyard(wall: bool) -> common::Scene {
    let mut s = common::scene(W, H);
    let floor = 0; // first texture of the common scene
    let bricks = 2;
    let mut g = GridMap::new(9, 9, 1.0);
    for z in 0..9 {
        for x in 0..9 {
            let c = g.cell_mut(x, z).unwrap();
            c.floor_texture = floor;
            c.ceiling_texture = -1;
            c.ceiling_height = 2.0;
        }
    }
    if wall {
        for x in 3..6 {
            g.cell_mut(x, 4).unwrap().walls[grid::NORTH] = Wall {
                texture: bricks,
                open: 0.0,
                alpha: 1.0,
                ..Wall::default()
            };
            g.cell_mut(x, 3).unwrap().walls[grid::SOUTH] = Wall {
                texture: bricks,
                open: 0.0,
                alpha: 1.0,
                ..Wall::default()
            };
        }
    }
    s.grid = g;
    s
}

fn yard_camera() -> Camera {
    Camera {
        position: Vec3::new(4.5, 1.6, 8.5),
        yaw: 0.0,
        pitch: -0.35,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    }
}

fn render_lit(
    name: &str,
    s: &mut common::Scene,
    opts: Options,
    sun_dir: Option<[f32; 3]>,
    ambient: f32,
    cam: &Camera,
    sprite: bool,
) -> Vec<u8> {
    s.renderer.options = opts;
    s.renderer.set_sun(sun_dir.map(sun));
    s.renderer.set_ambient(0x8090b0, ambient);
    let r = &mut s.renderer;
    r.begin(cam);
    r.draw_grid(&s.grid);
    if sprite {
        r.draw_sprite(&Sprite {
            position: Vec3::new(4.5, 0.0, 4.0),
            width: 1.0,
            height: 2.0,
            texture: s.figure[0],
            flags: renderer::SPRITE_BILLBOARD,
            yaw: 0.0,
            light: 1.0,
            tag: 0,
            alpha: 1.0,
            blend: BLEND_NONE,
        });
    }
    r.end();
    let img = r.rgba().to_vec();
    check(name, &img);
    img
}

#[test]
fn sun_wall_shadow() {
    let mut s = courtyard(true);
    render_lit(
        "sun_wall_shadow",
        &mut s,
        lit_options(),
        Some([0.5, -0.45, 1.0]),
        0.3,
        &yard_camera(),
        false,
    );
}

#[test]
fn sun_moved() {
    let mut a = courtyard(true);
    let mut b = courtyard(true);
    let img_a = render_lit(
        "sun_wall_shadow",
        &mut a,
        lit_options(),
        Some([0.5, -0.45, 1.0]),
        0.3,
        &yard_camera(),
        false,
    );
    let img_b = render_lit(
        "sun_moved",
        &mut b,
        lit_options(),
        Some([-0.9, -0.5, 0.6]),
        0.3,
        &yard_camera(),
        false,
    );
    assert_ne!(img_a, img_b, "moving the sun moves the shadow");
}

#[test]
fn sprite_silhouette() {
    let mut s = courtyard(false);
    render_lit(
        "sprite_silhouette",
        &mut s,
        lit_options(),
        Some([0.0, -0.8, 1.0]),
        0.3,
        &yard_camera(),
        true,
    );
}

/// Roofed room, slanted sun on, no ambient: everything inside is pure black (also where floor meets
/// wall), even with palette and dithering.
#[test]
fn dungeon_black() {
    let mut s = common::scene(W, H);
    s.renderer.set_palette(&common::palette16());
    s.renderer.set_sky(-1, 0.0);
    // Close the door: its gap opens to the void outside the map, where sun would really shine in.
    s.grid.cell_mut(3, 1).unwrap().walls[grid::NORTH].open = 0.0;
    let o = Options {
        quantize: true,
        dither: true,
        dither_strength: 32.0,
        ..lit_options()
    };
    let cam = Camera {
        position: Vec3::new(3.5, 0.5, 2.5),
        yaw: 0.0,
        pitch: -0.2,
        ..common::camera()
    };
    let img = render_lit(
        "dungeon_black",
        &mut s,
        o,
        Some([0.0, -1.0, 0.3]),
        0.0,
        &cam,
        false,
    );
    assert!(
        img.as_chunks::<4>().0.iter().all(|p| p[..3] == [0, 0, 0]),
        "dungeon must be pure black"
    );
}

#[test]
fn sun_degenerate_directions() {
    let mut a = courtyard(true);
    let mut b = courtyard(true);
    let down = render_lit(
        "sun_degenerate_down",
        &mut a,
        lit_options(),
        Some([0.0, -1.0, 0.0]),
        0.3,
        &yard_camera(),
        true,
    );
    let side = render_lit(
        "sun_degenerate_side",
        &mut b,
        lit_options(),
        Some([1.0, 0.0, 0.0]),
        0.3,
        &yard_camera(),
        true,
    );
    // Floor below the horizon: straight-down sun lights it, a horizontal sun does not.
    let i = ((H - 5) * W + W / 2) * 4;
    assert!(
        down[i] > side[i],
        "straight-down sun is brighter on the floor"
    );
}

// ---- Point lights ----

fn torch(pos: [f32; 3], shadows: bool) -> PointLight {
    PointLight {
        pos: Vec3::new(pos[0], pos[1], pos[2]),
        color: 0xffc080,
        strength: 1.5,
        radius: 4.0,
        flicker: 0.0,
        seed: 7,
        shadows,
    }
}

/// Courtyard render with point lights, optional sun and an optional flame sprite (no shadow) at `flame`.
/// Returns the image and the world position of every pixel (None = sky).
fn render_torch(
    name: Option<&str>,
    s: &mut common::Scene,
    sun_dir: Option<[f32; 3]>,
    ambient: f32,
    lights: &[PointLight],
    flame: Option<[f32; 3]>,
    time: f64,
    cam: &Camera,
) -> (Vec<u8>, Vec<Option<Vec3>>) {
    let opts = lit_options();
    s.renderer.options = opts;
    s.renderer.set_sun(sun_dir.map(sun));
    s.renderer.set_ambient(0x8090b0, ambient);
    s.renderer.set_time(time);
    let r = &mut s.renderer;
    let ids: Vec<_> = lights.iter().map(|l| r.create_light(*l)).collect();
    r.begin(cam);
    r.draw_grid(&s.grid);
    if let Some(p) = flame {
        r.draw_sprite(&Sprite {
            position: Vec3::new(p[0], p[1], p[2]),
            width: 0.2,
            height: 0.3,
            texture: s.figure[0],
            flags: renderer::SPRITE_BILLBOARD | renderer::SPRITE_NO_SHADOW,
            yaw: 0.0,
            light: 1.0,
            tag: 0,
            alpha: 1.0,
            blend: BLEND_NONE,
        });
    }
    r.end();
    for id in ids {
        assert!(r.destroy_light(id));
    }
    let img = r.rgba().to_vec();
    if let Some(n) = name {
        check(n, &img);
    }
    let view = camera::View::new(cam, W, H, opts.snap);
    let pos = r
        .depth()
        .iter()
        .enumerate()
        .map(|(i, z)| {
            z.is_finite()
                .then(|| renderer::world_pos(&view, i % W, i / W, *z))
        })
        .collect();
    (img, pos)
}

fn lum(img: &[u8], i: usize) -> u32 {
    img[i * 4] as u32 + img[i * 4 + 1] as u32 + img[i * 4 + 2] as u32
}

/// Pixel indices whose world position satisfies `f`.
fn pixels(pos: &[Option<Vec3>], f: impl Fn(Vec3) -> bool) -> Vec<usize> {
    (0..pos.len()).filter(|&i| pos[i].is_some_and(&f)).collect()
}

#[test]
fn torch_corridor() {
    // Torch behind the wall (z < 4): the floor right in front of the wall on the camera side stays at
    // ambient, the floor past the wall's end is lit.
    let cam = yard_camera();
    let (dark, _) = render_torch(None, &mut courtyard(true), None, 0.3, &[], None, 0.0, &cam);
    let (img, pos) = render_torch(
        Some("torch_corridor"),
        &mut courtyard(true),
        None,
        0.3,
        &[torch([4.5, 1.0, 3.5], true)],
        None,
        0.0,
        &cam,
    );
    let behind = pixels(&pos, |p| {
        p.y.abs() < 0.02 && p.x > 3.3 && p.x < 5.7 && p.z > 4.1 && p.z < 4.8
    });
    assert!(!behind.is_empty());
    for &i in &behind {
        assert_eq!(
            &img[i * 4..i * 4 + 4],
            &dark[i * 4..i * 4 + 4],
            "floor behind the wall stays at ambient"
        );
    }
    let open = pixels(&pos, |p| {
        p.y.abs() < 0.02 && p.x > 1.5 && p.x < 2.7 && p.z > 3.0 && p.z < 3.95
    });
    assert!(!open.is_empty());
    assert!(
        open.iter().all(|&i| lum(&img, i) > lum(&dark, i)),
        "torch lights the open floor"
    );
}

#[test]
fn torch_in_sun_shadow() {
    let cam = yard_camera();
    let sun_dir = Some([0.5, -0.45, 1.0]);
    let (base, _) = render_torch(
        None,
        &mut courtyard(true),
        sun_dir,
        0.3,
        &[],
        None,
        0.0,
        &cam,
    );
    let (img, pos) = render_torch(
        Some("torch_in_sun_shadow"),
        &mut courtyard(true),
        sun_dir,
        0.3,
        &[torch([4.5, 0.5, 4.8], true)],
        None,
        0.0,
        &cam,
    );
    // shadowed floor right in front of the wall, near the torch
    let near = pixels(&pos, |p| {
        p.y.abs() < 0.02 && p.x > 4.0 && p.x < 5.0 && p.z > 4.2 && p.z < 4.6
    });
    assert!(!near.is_empty());
    assert!(
        near.iter().all(|&i| lum(&img, i) > lum(&base, i)),
        "torch brightens the sun shadow"
    );
    assert!(
        (0..W * H).all(|i| lum(&img, i) >= lum(&base, i)),
        "a light never darkens"
    );
}

#[test]
fn torch_on_wall_no_acne() {
    let cam = yard_camera();
    let t = [4.5, 1.0, 4.1];
    let (dark, _) = render_torch(
        None,
        &mut courtyard(true),
        None,
        0.1,
        &[],
        Some(t),
        0.0,
        &cam,
    );
    let (img, pos) = render_torch(
        Some("torch_on_wall_no_acne"),
        &mut courtyard(true),
        None,
        0.1,
        &[torch(t, true)],
        Some(t),
        0.0,
        &cam,
    );
    // wall face z = 4 within 1.5 of the torch, away from the flame sprite: every pixel lit
    let wall = pixels(&pos, |p| {
        (p.z - 4.0).abs() < 0.02
            && (p.x - t[0]).abs() < 1.5
            && (p.y - t[1]).abs() < 0.9
            && ((p.x - t[0]).abs() > 0.15 || (p.y - t[1]).abs() > 0.2)
    });
    assert!(wall.len() > 50);
    let unlit = wall
        .iter()
        .filter(|&&i| lum(&img, i) <= lum(&dark, i))
        .count();
    assert_eq!(
        unlit, 0,
        "wall near the torch is lit everywhere (no shadow acne)"
    );
}

#[test]
fn flicker_same_time_same_image() {
    let cam = yard_camera();
    let l = PointLight {
        flicker: 0.8,
        ..torch([4.5, 1.0, 5.0], true)
    };
    let (a, _) = render_torch(
        None,
        &mut courtyard(true),
        None,
        0.3,
        &[l],
        None,
        1.25,
        &cam,
    );
    let (b, _) = render_torch(
        None,
        &mut courtyard(true),
        None,
        0.3,
        &[l],
        None,
        1.25,
        &cam,
    );
    let (c, _) = render_torch(
        None,
        &mut courtyard(true),
        None,
        0.3,
        &[l],
        None,
        1.40,
        &cam,
    );
    assert_eq!(a, b, "same time, same image");
    assert_ne!(a, c, "the flame flickers over time");
}

#[test]
fn party_light_no_shadow() {
    let cam = yard_camera();
    let p = cam.position;
    let (dark, _) = render_torch(None, &mut courtyard(true), None, 0.2, &[], None, 0.0, &cam);
    let party = PointLight {
        color: 0xffffff,
        strength: 1.0,
        radius: 6.0,
        ..torch([p.x, p.y, p.z], false)
    };
    let (img, _) = render_torch(
        Some("party_light_no_shadow"),
        &mut courtyard(true),
        None,
        0.2,
        &[party],
        None,
        0.0,
        &cam,
    );
    let i = (H - 5) * W + W / 2;
    assert!(
        lum(&img, i) > lum(&dark, i),
        "party light lights the floor ahead"
    );
}

#[test]
fn degenerate_lights_ignored() {
    let cam = yard_camera();
    let (base, _) = render_torch(None, &mut courtyard(true), None, 0.3, &[], None, 0.0, &cam);
    let bad = [
        PointLight {
            radius: 0.0,
            ..torch([4.5, 1.0, 5.0], true)
        },
        PointLight {
            strength: 0.0,
            ..torch([4.5, 1.0, 5.0], true)
        },
        PointLight {
            radius: -1.0,
            ..torch([4.5, 1.0, 5.0], false)
        },
    ];
    let (img, _) = render_torch(None, &mut courtyard(true), None, 0.3, &bad, None, 0.0, &cam);
    assert_eq!(img, base);
}

// ---- Sky tint, stability, speed ----

#[test]
fn sky_tint_darkens_sky() {
    let cam = Camera {
        pitch: 0.6,
        ..yard_camera()
    };
    let mut a = courtyard(true);
    let (plain, pos) = render_torch(
        None,
        &mut a,
        Some([0.5, -0.45, 1.0]),
        0.3,
        &[],
        None,
        0.0,
        &cam,
    );
    let mut b = courtyard(true);
    b.renderer.set_sky_tint(0x202040, 1.0);
    let (tinted, _) = render_torch(
        None,
        &mut b,
        Some([0.5, -0.45, 1.0]),
        0.3,
        &[],
        None,
        0.0,
        &cam,
    );
    let sky: Vec<usize> = (0..pos.len()).filter(|&i| pos[i].is_none()).collect();
    assert!(sky.len() > 100);
    assert!(
        sky.iter().all(|&i| lum(&tinted, i) < lum(&plain, i)),
        "tinted sky is darker"
    );
    // also the plain background colour (no sky texture)
    let mut c = courtyard(true);
    c.renderer.set_sky(-1, 0.0);
    let o = Options {
        background: 0x8080ff,
        ..lit_options()
    };
    c.renderer.options = o;
    c.renderer.set_sky_tint(0x202040, 1.0);
    c.renderer.begin(&cam);
    c.renderer.end();
    assert_eq!(&c.renderer.rgba()[..3], &[0x10, 0x10, 0x40]);
    // tint without lighting changes nothing
    let mut d = courtyard(true);
    d.renderer.set_sky_tint(0x202040, 1.0);
    common::render(&mut d, &cam, 0.0);
    let mut e = courtyard(true);
    common::render(&mut e, &cam, 0.0);
    assert_eq!(d.renderer.rgba(), e.renderer.rgba());
}

/// Camera glides in small steps (sometimes less than one shadow texel, sometimes more): every floor
/// point must keep its lit/shadow state, i.e. the sun shadow does not crawl.
#[test]
fn sun_shadow_stable_while_gliding() {
    let sun_dir = [0.5, -0.45, 1.0];
    let opts = Options {
        texture_lod: true,
        face_lod: true,
        snap: 0.0,
        ..lit_options()
    };
    let half = opts.shadow_size;
    let texel = 2.0 * half / opts.shadow_res as f32;
    let (right, up) = basis(Vec3::new(sun_dir[0], sun_dir[1], sun_dir[2]));
    let mut state: std::collections::BTreeMap<(i64, i64), bool> = std::collections::BTreeMap::new();
    let (mut mismatches, mut checked, mut shadowed) = (0, 0, 0);
    for step in 0..20 {
        let mut cam = yard_camera();
        cam.position.x += 0.01 * step as f32;
        let shot = |shadows: bool| {
            let mut s = courtyard(true);
            s.renderer.options = opts;
            s.renderer.set_sun(Some(light::Sun {
                shadows,
                ..sun(sun_dir)
            }));
            s.renderer.set_ambient(0x8090b0, 0.3);
            let r = &mut s.renderer;
            r.begin(&cam);
            r.draw_grid(&s.grid);
            r.end();
            let view = camera::View::new(&cam, W, H, 0.0);
            let pos: Vec<Option<Vec3>> = r
                .depth()
                .iter()
                .enumerate()
                .map(|(i, z)| {
                    z.is_finite()
                        .then(|| renderer::world_pos(&view, i % W, i / W, *z))
                })
                .collect();
            (r.rgba().to_vec(), pos)
        };
        let (img, pos) = shot(true);
        let (img_no, _) = shot(false);
        let mut frame: std::collections::BTreeMap<(i64, i64), [bool; 2]> =
            std::collections::BTreeMap::new();
        for i in 0..pos.len() {
            let Some(p) = pos[i] else { continue };
            // skip the contact strip at the wall base: there the depth test, not the texel, decides
            if p.y.abs() > 0.02 || (p.z > 3.9 && p.z < 4.2) {
                continue;
            }
            let (u, v) = (p.dot(right) / texel, p.dot(up) / texel);
            let edge = |t: f32| (t - t.floor()).min(t.ceil() - t) < 0.05;
            if edge(u) || edge(v) {
                continue;
            }
            let dark = lum(&img, i) < lum(&img_no, i);
            shadowed += dark as usize;
            checked += 1;
            let e = frame
                .entry((u.floor() as i64, v.floor() as i64))
                .or_insert([false; 2]);
            e[dark as usize] = true;
        }
        // Texels cut by the edge's depth test (contact with the wall) may be mixed within one frame;
        // compare only texels that are all lit or all dark in a frame.
        for (key, seen) in frame {
            if seen[0] != seen[1] {
                let dark = seen[1];
                if *state.entry(key).or_insert(dark) != dark {
                    mismatches += 1;
                }
            }
        }
    }
    assert!(
        checked > 1000 && shadowed > 100,
        "test sees floor and shadow ({checked}, {shadowed})"
    );
    assert_eq!(mismatches, 0, "shadow edge crawls while the camera glides");
}

/// Courtyard at 320 x 200 with sun and 8 shadowed, flickering torches; `moving` shifts every torch
/// each frame (shadow cache misses), otherwise only the flicker changes (cache hits).
/// Calls `frame` after every finished frame.
fn torch_yard_frames(
    threads: usize,
    frames: usize,
    moving: bool,
    soft: f32,
    frame: impl FnMut(&Renderer),
) {
    torch_yard_run(threads, frames, moving, false, soft, frame)
}

/// `torch_yard_frames`; `pan`: the camera also turns a little every frame (nothing kept between
/// frames can be reused).
fn torch_yard_run(
    threads: usize,
    frames: usize,
    moving: bool,
    pan: bool,
    soft: f32,
    mut frame: impl FnMut(&Renderer),
) {
    let mut s = common::scene(320, 200);
    s.grid = courtyard(true).grid;
    s.renderer.options = Options {
        threads,
        shadow_soft: soft,
        point_shadow_soft: soft,
        ..lit_options()
    };
    s.renderer.set_sun(Some(sun([0.5, -0.45, 1.0])));
    s.renderer.set_ambient(0x8090b0, 0.3);
    let cam = yard_camera();
    let torch_at = |k: usize, f: usize| {
        let dx = if moving {
            (f as f32 * 0.37 + k as f32).sin() * 0.4
        } else {
            0.0
        };
        PointLight {
            flicker: 0.5,
            ..torch([1.0 + k as f32 + dx, 1.0, 2.0 + (k % 3) as f32 * 2.5], true)
        }
    };
    let ids: Vec<u32> = (0..8)
        .map(|k| s.renderer.create_light(torch_at(k, 0)))
        .collect();
    for f in 0..frames {
        let r = &mut s.renderer;
        for (k, &id) in ids.iter().enumerate() {
            r.set_light(id, torch_at(k, f));
        }
        r.set_time(f as f64 / 30.0);
        let cam = if pan {
            Camera {
                yaw: cam.yaw + 0.002 * f as f32,
                ..cam
            }
        } else {
            cam
        };
        r.begin(&cam);
        r.draw_grid(&s.grid);
        r.end();
        frame(r);
    }
}

#[test]
fn same_image_for_any_thread_count() {
    for (moving, soft) in [(false, 0.0), (true, 0.0), (false, 0.2), (true, 0.2)] {
        let mut want: Vec<Vec<u8>> = Vec::new();
        torch_yard_frames(1, 4, moving, soft, |r| want.push(r.rgba().to_vec()));
        assert_ne!(want[1], want[2], "frames differ (flicker / movement)");
        for t in [2, 3, 8, 0] {
            let mut k = 0;
            torch_yard_frames(t, 4, moving, soft, |r| {
                assert!(
                    r.rgba() == want[k].as_slice(),
                    "threads = {t}, moving = {moving}, soft = {soft}, frame {k}"
                );
                k += 1;
            });
        }
    }
}

/// Release timing: cargo test --release -- --ignored timing --nocapture
/// Prints mean and best frame time (best is robust against a busy machine).
#[test]
#[ignore]
fn timing_sun_8_torches() {
    let auto = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    for (moving, threads, soft) in [
        (false, 1, 0.0),
        (true, 1, 0.0),
        (false, 1, 0.2),
        (true, 1, 0.2),
        (false, 0, 0.0),
        (true, 0, 0.0),
        (false, 0, 0.2),
        (true, 0, 0.2),
    ] {
        {
            let mut times = Vec::new();
            let mut last = std::time::Instant::now();
            torch_yard_frames(threads, 41, moving, soft, |_| {
                let now = std::time::Instant::now();
                times.push((now - last).as_secs_f64() * 1000.0);
                last = now;
            });
            times.remove(0); // first frame builds every map
            let mean = times.iter().sum::<f64>() / times.len() as f64;
            let best = times.iter().cloned().fold(f64::INFINITY, f64::min);
            println!(
                "timing_sun_8_torches {} soft {soft}: threads {} -> mean {mean:.2} ms, best {best:.2} ms per frame (320x200)",
                if moving { "moving" } else { "static" },
                if threads == 0 {
                    format!("auto ({auto})")
                } else {
                    threads.to_string()
                }
            );
        }
    }
}

/// Release timing with a turning camera (static torches): every pixel changes, so the light pass
/// cannot reuse anything from the frame before (shadow factors, sky angles).
/// cargo test --release --test images -- --ignored timing_camera_turning --nocapture
#[test]
#[ignore]
fn timing_camera_turning() {
    let auto = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    for (threads, soft) in [(1, 0.0), (1, 0.2), (0, 0.0), (0, 0.2)] {
        let mut times = Vec::new();
        let mut last = std::time::Instant::now();
        torch_yard_run(threads, 41, false, true, soft, |_| {
            let now = std::time::Instant::now();
            times.push((now - last).as_secs_f64() * 1000.0);
            last = now;
        });
        times.remove(0);
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        let best = times.iter().cloned().fold(f64::INFINITY, f64::min);
        println!(
            "timing_camera_turning soft {soft}: threads {} -> mean {mean:.2} ms, best {best:.2} ms per frame (320x200)",
            if threads == 0 {
                format!("auto ({auto})")
            } else {
                threads.to_string()
            }
        );
    }
}

/// Release timing at the Wizardry 7 settings: sun only, 176 x 112, shadow box 9, 2048 texels,
/// camera standing / gliding, shadow_soft 0 and 0.2, one thread.
/// cargo test --release --test images -- --ignored timing_soft --nocapture
#[test]
#[ignore]
fn timing_soft_sun_wiz() {
    for moving in [false, true] {
        for soft in [0.0, 0.2] {
            let mut s = common::scene(176, 112);
            s.grid = courtyard(true).grid;
            s.renderer.options = Options {
                threads: 1,
                shadow_size: 9.0,
                shadow_res: 2048,
                shadow_soft: soft,
                ..lit_options()
            };
            s.renderer.set_sun(Some(sun([0.5, -0.45, 1.0])));
            s.renderer.set_ambient(0x8090b0, 0.3);
            let mut times = Vec::new();
            for f in 0..61 {
                let mut cam = yard_camera();
                if moving {
                    cam.position.x += 0.013 * f as f32;
                }
                let t = std::time::Instant::now();
                s.renderer.begin(&cam);
                s.renderer.draw_grid(&s.grid);
                s.renderer.end();
                if f > 0 {
                    times.push(t.elapsed().as_secs_f64() * 1000.0);
                }
            }
            let mean = times.iter().sum::<f64>() / times.len() as f64;
            let best = times.iter().cloned().fold(f64::INFINITY, f64::min);
            println!(
                "timing_soft_sun_wiz {} soft {soft}: mean {mean:.2} ms, best {best:.2} ms (176x112, 1 thread)",
                if moving { "moving" } else { "static" }
            );
        }
    }
}

/// Torch soft edges are switched on their own: shadow_soft alone leaves torch shadows hard.
#[test]
fn point_soft_is_separate() {
    let render = |name: Option<&str>, sun_soft: f32, point_soft: f32| {
        let mut s = courtyard(true);
        s.renderer.options = Options {
            shadow_soft: sun_soft,
            point_shadow_soft: point_soft,
            ..lit_options()
        };
        s.renderer.set_sun(None);
        s.renderer.set_ambient(0x8090b0, 0.1);
        s.renderer.create_light(torch([4.0, 1.2, 3.0], true));
        s.renderer.begin(&yard_camera());
        s.renderer.draw_grid(&s.grid);
        s.renderer.end();
        let img = s.renderer.rgba().to_vec();
        if let Some(n) = name {
            check(n, &img);
        }
        img
    };
    let hard = render(None, 0.0, 0.0);
    assert!(
        render(None, 0.2, 0.0) == hard,
        "shadow_soft must not touch torch shadows"
    );
    let soft = render(Some("soft_torch_shadow"), 0.0, 0.2);
    assert_ne!(soft, hard, "point_shadow_soft softens torch edges");
}

// ---- Light selection (at most MAX_LIGHTS per frame) ----

fn dim_torch(x: f32, z: f32) -> PointLight {
    PointLight {
        strength: 0.15,
        radius: 3.0,
        ..torch([x, 1.0, z], false)
    }
}

#[test]
fn many_lights_nearest_used() {
    let cam = yard_camera();
    let p = cam.position;
    // 16 lights near the camera plus 8 farther away, all reaching the visible area
    let near: Vec<PointLight> = (0..MAX_LIGHTS)
        .map(|k| dim_torch(p.x - 1.5 + (k % 4) as f32, p.z - 1.0 - (k / 4) as f32 * 0.5))
        .collect();
    let far: Vec<PointLight> = (0..8).map(|k| dim_torch(1.0 + k as f32, 1.0)).collect();
    let mut all = near.clone();
    all.extend(far.iter().copied());
    let (only_near, _) = render_torch(
        None,
        &mut courtyard(true),
        None,
        0.3,
        &near,
        None,
        0.0,
        &cam,
    );
    let (img, _) = render_torch(None, &mut courtyard(true), None, 0.3, &all, None, 0.0, &cam);
    assert_eq!(img, only_near, "only the 16 nearest lights are used");
    // far ones first in creation order: same set, same id order -> same image
    let mut rev = far.clone();
    rev.extend(near.iter().copied());
    let (a, _) = render_torch(None, &mut courtyard(true), None, 0.3, &rev, None, 0.0, &cam);
    let (b, _) = render_torch(None, &mut courtyard(true), None, 0.3, &rev, None, 0.0, &cam);
    assert_eq!(a, b, "same calls, same image");
    let (base, _) = render_torch(None, &mut courtyard(true), None, 0.3, &[], None, 0.0, &cam);
    assert_ne!(img, base);
}

#[test]
fn light_outside_view_takes_no_slot() {
    let cam = yard_camera();
    let p = cam.position;
    // 16 visible lights, plus one very close to the camera but behind it with a tiny radius
    let near: Vec<PointLight> = (0..MAX_LIGHTS)
        .map(|k| dim_torch(p.x - 1.5 + (k % 4) as f32, p.z - 1.0 - (k / 4) as f32 * 0.5))
        .collect();
    let hidden = PointLight {
        radius: 0.3,
        ..dim_torch(p.x, p.z + 1.0)
    };
    let mut all = vec![hidden];
    all.extend(near.iter().copied());
    let (with_hidden, _) =
        render_torch(None, &mut courtyard(true), None, 0.3, &all, None, 0.0, &cam);
    let (only_near, _) = render_torch(
        None,
        &mut courtyard(true),
        None,
        0.3,
        &near,
        None,
        0.0,
        &cam,
    );
    assert_eq!(with_hidden, only_near);
    let mut fewer = near.clone();
    fewer.pop();
    let (fewer_img, _) = render_torch(
        None,
        &mut courtyard(true),
        None,
        0.3,
        &fewer,
        None,
        0.0,
        &cam,
    );
    assert_ne!(
        fewer_img, only_near,
        "the 16th light matters, so the hidden one would have displaced it"
    );
}

// ---- Texture level choice ----

/// Long floor whose texture levels are flat colours (red base, then green, blue, white), seen along
/// its length; returns the level index of every floor pixel (None elsewhere).
fn level_map(lod_dither: bool) -> Vec<Option<u8>> {
    let mut r = Renderer::new(W, H).unwrap();
    let solid = |n: usize, c: [u8; 3]| {
        Texture::from_rgba(n, n, &[c[0], c[1], c[2], 255].repeat(n * n)).unwrap()
    };
    let t = r.add_texture(solid(16, [255, 0, 0]));
    for (n, c) in [(8, [0, 255, 0]), (4, [0, 0, 255]), (2, [255, 255, 255])] {
        assert!(r.add_texture_level(t, solid(n, c)));
    }
    let mut g = GridMap::new(3, 60, 1.0);
    for z in 0..60 {
        for x in 0..3 {
            let c = g.cell_mut(x, z).unwrap();
            c.floor_texture = t;
            c.ceiling_texture = -1;
        }
    }
    r.options = Options {
        texture_lod: true,
        lod_dither,
        ..Options::default()
    };
    r.begin(&Camera {
        position: Vec3::new(1.5, 0.6, 59.5),
        yaw: 0.0,
        pitch: -0.15,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    });
    r.draw_grid(&g);
    r.end();
    r.rgba()
        .chunks(4)
        .zip(r.depth())
        .map(|(p, z)| {
            z.is_finite().then(|| match (p[0], p[1], p[2]) {
                (255, 0, 0) => 0,
                (0, 255, 0) => 1,
                (0, 0, 255) => 2,
                _ => 3,
            })
        })
        .collect()
}

#[test]
fn lod_dither_off_gives_hard_level_boundaries() {
    let soft = level_map(true);
    let hard = level_map(false);
    assert_ne!(soft, hard);
    // Per column from the bottom (near) up (far), the level never goes back to a nearer one when
    // not dithered; with dither it does inside the bands between levels.
    let back_steps = |m: &[Option<u8>]| {
        let mut n = 0;
        for x in 0..W {
            let mut cur = 0u8;
            for y in (0..H).rev() {
                if let Some(l) = m[y * W + x] {
                    if l < cur {
                        n += 1;
                    }
                    cur = cur.max(l);
                }
            }
        }
        n
    };
    let levels: std::collections::BTreeSet<u8> = hard.iter().flatten().copied().collect();
    assert!(levels.len() >= 3, "scene shows several levels: {levels:?}");
    assert_eq!(back_steps(&hard), 0, "no dither pattern between levels");
    assert!(back_steps(&soft) > 50, "dithered bands between levels");
}

// ---- Shadow map cache ----

#[test]
fn shadow_cache_follows_changes() {
    let cam = yard_camera();
    let glass = Texture::from_rgba(1, 1, &[0, 0, 0, 0]).unwrap();
    let setup = |transparent_wall: bool| {
        let mut s = courtyard(true);
        s.renderer.options = lit_options();
        s.renderer.set_sun(Some(sun([0.5, -0.45, 1.0])));
        s.renderer.set_ambient(0x8090b0, 0.3);
        if transparent_wall {
            s.renderer.replace_texture(2, glass.clone());
        }
        let id = s.renderer.create_light(torch([4.5, 1.0, 3.0], true));
        (s, id)
    };
    let frame = |s: &mut common::Scene| {
        s.renderer.begin(&cam);
        s.renderer.draw_grid(&s.grid);
        s.renderer.end();
        s.renderer.rgba().to_vec()
    };
    let (mut s, id) = setup(false);
    let a = frame(&mut s);
    assert_eq!(frame(&mut s), a, "cache hit, same image");
    // move the light and back: rebuilt twice, same image as before
    s.renderer.set_light(id, torch([2.5, 1.0, 6.0], true));
    let moved = frame(&mut s);
    assert_ne!(moved, a);
    let (mut fresh, fid) = setup(false);
    fresh.renderer.set_light(fid, torch([2.5, 1.0, 6.0], true));
    assert_eq!(
        frame(&mut fresh),
        moved,
        "moved light: same as a fresh renderer"
    );
    s.renderer.set_light(id, torch([4.5, 1.0, 3.0], true));
    assert_eq!(frame(&mut s), a);
    // a texture change (alpha test) invalidates the maps
    s.renderer.replace_texture(2, glass.clone());
    let b = frame(&mut s);
    let (mut fresh, _) = setup(true);
    assert_eq!(
        frame(&mut fresh),
        b,
        "texture change: same as a fresh renderer"
    );
    assert_ne!(a, b);
    // caster change: wall removed
    let (mut s, _) = setup(false);
    frame(&mut s);
    for x in 3..6 {
        s.grid.cell_mut(x, 4).unwrap().walls[grid::NORTH] = Wall {
            texture: -1,
            open: 0.0,
            alpha: 1.0,
            ..Wall::default()
        };
        s.grid.cell_mut(x, 3).unwrap().walls[grid::SOUTH] = Wall {
            texture: -1,
            open: 0.0,
            alpha: 1.0,
            ..Wall::default()
        };
    }
    let no_wall = frame(&mut s);
    let mut open = courtyard(false);
    open.renderer.options = lit_options();
    open.renderer.set_sun(Some(sun([0.5, -0.45, 1.0])));
    open.renderer.set_ambient(0x8090b0, 0.3);
    open.renderer.create_light(torch([4.5, 1.0, 3.0], true));
    assert_eq!(
        frame(&mut open),
        no_wall,
        "caster change: same as a fresh renderer"
    );
}

/// Courtyard with the wall textured by `wall` (unlit), rendered at `time`.
fn render_wall_at(s: &mut common::Scene, wall: i32, time: f64) -> Vec<u8> {
    for x in 3..6 {
        s.grid.cell_mut(x, 4).unwrap().walls[grid::NORTH].texture = wall;
        s.grid.cell_mut(x, 3).unwrap().walls[grid::SOUTH].texture = wall;
    }
    s.renderer.set_time(time);
    let r = &mut s.renderer;
    r.begin(&yard_camera());
    r.draw_grid(&s.grid);
    r.end();
    r.rgba().to_vec()
}

#[test]
fn anim_texture_changes_with_time() {
    let mut s = courtyard(true);
    let r = &mut s.renderer;
    assert_eq!(r.add_texture_anim(&[], 2.0), -1);
    assert_eq!(r.add_texture_anim(&[2], 0.0), -1);
    assert_eq!(r.add_texture_anim(&[2, 99], 2.0), -1);
    let anim = r.add_texture_anim(&[2, 4], 2.0);
    assert!(anim >= 0);
    assert_eq!(
        r.add_texture_anim(&[anim, 2], 2.0),
        -1,
        "frames may not be animations"
    );
    let a0 = render_wall_at(&mut s, anim, 0.0);
    let a1 = render_wall_at(&mut s, anim, 0.5);
    let a2 = render_wall_at(&mut s, anim, 1.0);
    assert_ne!(a0, a1, "second frame shows");
    assert_eq!(a0, a2, "animation loops");
    assert_eq!(
        a0,
        render_wall_at(&mut s, 2, 0.0),
        "frame 0 looks like its texture"
    );
    assert_eq!(
        a1,
        render_wall_at(&mut s, 4, 0.0),
        "frame 1 looks like its texture"
    );
}

#[test]
fn scroll_moves_texture() {
    let mut s = courtyard(true);
    let r = &mut s.renderer;
    let wall = r.add_texture(common::bricks());
    assert!(!r.set_texture_scroll(99, 0.1, 0.0));
    assert!(r.set_texture_scroll(wall, 0.25, 0.125));
    let a = render_wall_at(&mut s, wall, 0.0);
    check("scroll_t0", &a);
    let b = render_wall_at(&mut s, wall, 1.0);
    check("scroll_t1", &b);
    assert_ne!(a, b, "scrolling moves the texture");
    assert_eq!(
        a,
        render_wall_at(&mut s, wall, 8.0),
        "scroll offset wraps after a whole repeat"
    );
}

/// Lit courtyard with an animated billboard standing behind the camera: only its shadow is visible.
fn render_anim_shadow(s: &mut common::Scene, anim: i32, time: f64) -> Vec<u8> {
    s.renderer.options = lit_options();
    s.renderer.set_sun(Some(sun([0.0, -0.6, -1.0])));
    s.renderer.set_ambient(0x8090b0, 0.3);
    s.renderer.set_time(time);
    let r = &mut s.renderer;
    r.begin(&yard_camera());
    r.draw_grid(&s.grid);
    r.draw_sprite(&Sprite {
        position: Vec3::new(4.5, 0.0, 8.9),
        width: 1.0,
        height: 2.0,
        texture: anim,
        flags: renderer::SPRITE_BILLBOARD,
        yaw: 0.0,
        light: 1.0,
        tag: 0,
        alpha: 1.0,
        blend: BLEND_NONE,
    });
    r.end();
    r.rgba().to_vec()
}

#[test]
fn anim_texture_shadow_follows_frame() {
    // two frames with different alpha holes in the upper half (whose shadow falls into view)
    let frame = |hole: bool| {
        let mut rgba = Vec::new();
        for y in 0..16 {
            for x in 0..8 {
                let open = y < 8
                    && if hole {
                        (2..6).contains(&x)
                    } else {
                        x == 0 || x == 7
                    };
                rgba.extend_from_slice(&[200, 180, 60, if open { 0 } else { 255 }]);
            }
        }
        Texture::from_rgba(8, 16, &rgba).unwrap()
    };
    let setup = || {
        let mut s = courtyard(false);
        let frames = [
            s.renderer.add_texture(frame(false)),
            s.renderer.add_texture(frame(true)),
        ];
        let anim = s.renderer.add_texture_anim(&frames, 4.0);
        (s, anim)
    };
    let fresh = |time: f64| {
        let (mut s, anim) = setup();
        render_anim_shadow(&mut s, anim, time)
    };
    let (mut s, anim) = setup();
    let a = render_anim_shadow(&mut s, anim, 0.0);
    check("anim_shadow_f0", &a);
    let b = render_anim_shadow(&mut s, anim, 0.25);
    check("anim_shadow_f1", &b);
    assert_ne!(a, b, "the shadow follows the animation frame");
    assert_eq!(b, fresh(0.25), "cached shadow equals a fresh renderer");
    assert_eq!(
        render_anim_shadow(&mut s, anim, 0.5),
        fresh(0.0),
        "cached shadow equals a fresh renderer"
    );
}

#[test]
fn sky_scrolls() {
    let sky_at = |scroll: bool, time: f64| {
        let mut s = courtyard(false);
        if scroll {
            // the common scene's sky is texture 5
            assert!(s.renderer.set_texture_scroll(5, 0.05, 0.1));
        }
        render_wall_at(&mut s, -1, time)
    };
    assert_ne!(sky_at(true, 0.0), sky_at(true, 1.0), "scrolling sky moves");
    assert_eq!(sky_at(true, 0.0), sky_at(false, 0.0), "offset 0 at time 0");
    assert_eq!(
        sky_at(false, 0.0),
        sky_at(false, 1.0),
        "sky without scroll stays"
    );
}

// ---- Transparency ----

fn flat(w: usize, h: usize, rgba: [u8; 4]) -> Texture {
    Texture::from_rgba(w, h, &rgba.repeat(w * h)).unwrap()
}

/// The common scene with glass between the yard and the room, seen from the yard. Alpha 0 = clear
/// glass: the same faces as any alpha (a wall hides the step faces behind it), but nothing drawn.
fn glass_scene(alpha: f32) -> common::Scene {
    let mut s = common::scene(W, H);
    let t = s.renderer.add_texture(flat(4, 4, [150, 210, 230, 255]));
    for x in 1..=5 {
        s.grid.cell_mut(x, 4).unwrap().walls[grid::NORTH] = Wall {
            texture: t,
            open: 0.0,
            alpha,
            ..Wall::default()
        };
        s.grid.cell_mut(x, 3).unwrap().walls[grid::SOUTH] = Wall {
            texture: t,
            open: 0.0,
            alpha,
            ..Wall::default()
        };
    }
    s
}

#[test]
fn glass_wall_shows_room_behind() {
    let mut plain = glass_scene(0.0);
    common::render(&mut plain, &common::camera(), 0.0);
    let old = plain.renderer.rgba().to_vec();
    let old_depth = plain.renderer.depth().to_vec();
    let mut s = glass_scene(0.5);
    common::render(&mut s, &common::camera(), 0.0);
    let img = s.renderer.rgba().to_vec();
    check("glass_wall_shows_room_behind", &img);
    assert!(
        s.renderer.depth() == old_depth.as_slice(),
        "glass writes no depth"
    );
    // every pixel is either untouched (something opaque in front of the glass) or half glass, half room
    let glass = [150.0f32, 210.0, 230.0];
    let mut mixed = 0;
    for i in 0..W * H {
        let (a, b) = (&old[i * 4..i * 4 + 3], &img[i * 4..i * 4 + 3]);
        if a == b {
            continue;
        }
        mixed += 1;
        for k in 0..3 {
            let want = a[k] as f32 * 0.5 + glass[k] * 0.5;
            assert!(
                (b[k] as f32 - want).abs() <= 1.0,
                "pixel {i}: {b:?} vs mix of {a:?}"
            );
        }
    }
    assert!(mixed > W * H / 4, "glass covers much of the view ({mixed})");
    // lit, with a palette: the same image for any thread count
    let lit = |threads: usize| {
        let mut s = glass_scene(0.5);
        s.renderer.set_palette(&common::palette16());
        s.renderer.options = Options {
            lighting: true,
            dither: true,
            threads,
            ..Options::default()
        };
        s.renderer.set_ambient(0xffffff, 0.4);
        s.renderer.create_light(PointLight {
            pos: Vec3::new(3.5, 0.8, 2.5),
            color: 0xffcc88,
            strength: 1.0,
            radius: 4.0,
            flicker: 0.0,
            seed: 1,
            shadows: true,
        });
        common::render(&mut s, &common::camera(), 0.0);
        s.renderer.rgba().to_vec()
    };
    let one = lit(1);
    for t in [2, 3, 8] {
        assert!(lit(t) == one, "threads = {t}");
    }
}

fn glow() -> Texture {
    let n = 16;
    let mut rgba = Vec::new();
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 - 7.5, y as f32 - 7.5);
            let a = (1.0 - (dx * dx + dy * dy).sqrt() / 8.0).max(0.0);
            rgba.extend_from_slice(&[255, 160, 60, (a * 255.0) as u8]);
        }
    }
    Texture::from_rgba(n, n, &rgba).unwrap()
}

fn render_with(s: &mut common::Scene, extra: &[Sprite], first: bool) -> Vec<u8> {
    let cam = common::camera();
    let r = &mut s.renderer;
    r.begin(&cam);
    if first {
        extra.iter().for_each(|e| r.draw_sprite(e));
    }
    r.draw_grid(&s.grid);
    if !first {
        extra.iter().for_each(|e| r.draw_sprite(e));
    }
    r.end();
    r.rgba().to_vec()
}

fn ghost(texture: i32, x: f32, z: f32, alpha: f32, blend: i32) -> Sprite {
    Sprite {
        position: Vec3::new(x, 0.0, z),
        width: 0.8,
        height: 0.8,
        texture,
        flags: renderer::SPRITE_BILLBOARD,
        yaw: 0.0,
        light: 1.0,
        tag: 0,
        alpha,
        blend,
    }
}

#[test]
fn additive_glow_brightens() {
    let mut s = common::scene(W, H);
    let old = render_with(&mut s, &[], false);
    let t = s.renderer.add_texture(glow());
    let img = render_with(&mut s, &[ghost(t, 3.0, 3.0, 1.0, BLEND_ADD)], false);
    check("additive_glow_brightens", &img);
    assert!(
        old.iter().zip(&img).all(|(a, b)| b >= a),
        "adding light never darkens"
    );
    assert!(
        old.iter().zip(&img).filter(|(a, b)| b > a).count() > 100,
        "the glow shows"
    );
}

#[test]
fn transparent_behind_wall_hidden() {
    let mut s = common::scene(W, H);
    let old = render_with(&mut s, &[], false);
    let t = s.renderer.add_texture(flat(1, 1, [255, 0, 0, 255]));
    // behind the solid north wall (x = 1.5 has no door)
    let hidden = ghost(t, 1.5, 0.5, 0.5, BLEND_ALPHA);
    assert!(
        render_with(&mut s, &[hidden], false) == old,
        "drawn after the wall"
    );
    assert!(
        render_with(&mut s, &[hidden], true) == old,
        "drawn before the wall"
    );
    let seen = ghost(t, 1.5, 2.5, 0.5, BLEND_ALPHA);
    assert!(
        render_with(&mut s, &[seen], true) != old,
        "in front of the wall it shows"
    );
}

/// Square of half size 1 at depth z, facing the default camera.
fn square(z: f32) -> Vec<Vertex> {
    let v = |x: f32, y: f32| Vertex {
        x,
        y,
        z,
        u: 0.0,
        v: 0.0,
    };
    vec![
        v(-1.0, 1.0),
        v(1.0, 1.0),
        v(1.0, -1.0),
        v(-1.0, 1.0),
        v(1.0, -1.0),
        v(-1.0, -1.0),
    ]
}

/// Black background, layers drawn in the given order: (texture rgba, depth, alpha).
fn layers(list: &[([u8; 4], f32, f32)]) -> Vec<u8> {
    let mut r = Renderer::new(W, H).unwrap();
    let tex: Vec<i32> = list
        .iter()
        .map(|l| r.add_texture(flat(1, 1, l.0)))
        .collect();
    r.begin(&Camera::default());
    for (l, &t) in list.iter().zip(&tex) {
        r.draw_triangles_ex(&square(-l.1), t, 1.0, false, false, 0, l.2, BLEND_ALPHA);
    }
    r.end();
    r.rgba().to_vec()
}

#[test]
fn two_transparent_sorted() {
    let red = ([255, 0, 0, 255], 2.0, 0.5);
    let blue = ([0, 0, 255, 255], 3.0, 0.5);
    let a = layers(&[blue, red]);
    let b = layers(&[red, blue]);
    assert!(a == b, "draw order does not matter, depth does");
    let c = (H / 2 * W + W / 2) * 4;
    assert_eq!(&a[c..c + 3], &[128, 0, 64], "red over (blue over black)");
    // equal depth: draw order decides, later on top
    let a = layers(&[([255, 0, 0, 255], 2.0, 0.5), ([0, 0, 255, 255], 2.0, 0.5)]);
    assert_eq!(&a[c..c + 3], &[64, 0, 128]);
}

#[test]
fn alpha_steps_of_eighth() {
    use knorke_render::blend::step_alpha;
    for (a, want) in [
        (0.0, 0.0),
        (0.3, 0.25),
        (0.26, 0.25),
        (0.99, 1.0),
        (1.0, 1.0),
        (2.0, 1.0),
        (-1.0, 0.0),
        (f32::NAN, 0.0),
    ] {
        assert_eq!(step_alpha(a), want, "{a}");
    }
    let red = |alpha: f32, ta: u8| layers(&[([255, 0, 0, ta], 2.0, alpha)]);
    assert!(red(0.3, 255) == red(0.26, 255));
    assert!(red(0.3, 255) != red(0.45, 255));
    assert!(red(1.0, 128) == red(0.5, 255), "texel alpha counts");
    let c = (H / 2 * W + W / 2) * 4;
    assert_eq!(&red(0.3, 255)[c..c + 3], &[64, 0, 0]);
}

#[test]
fn opaque_scene_unchanged_with_feature_unused() {
    let mut s = common::scene(W, H);
    let t = s.renderer.add_texture(flat(1, 1, [255, 0, 0, 255]));
    // invisible transparent draws in view leave the image bit-identical
    let img = render_with(
        &mut s,
        &[
            ghost(t, 3.0, 3.0, 0.0, BLEND_ALPHA),
            ghost(t, 3.0, 3.0, 0.0, BLEND_ADD),
        ],
        false,
    );
    let mut s2 = common::scene(W, H);
    assert!(img == render_with(&mut s2, &[], false));
    // the common scene itself still matches its reference
    let mut plain = common::scene(W, H);
    common::render(&mut plain, &common::camera(), 0.0);
    check("plain_front", plain.renderer.rgba());
}

// ---- particles and weather ----

use knorke_render::particles::{
    self, EMIT_RAIN, EMIT_SMOKE, EMIT_SNOW, EMIT_SPARKS, Emitter, Particle,
};

fn emitter(kind: i32, pos: [f32; 3], size: [f32; 3], rate: f32, color: u32) -> Emitter {
    Emitter {
        kind,
        position: Vec3::new(pos[0], pos[1], pos[2]),
        box_size: Vec3::new(size[0], size[1], size[2]),
        rate,
        speed: 0.0,
        size: 0.0,
        color,
        texture: -1,
        wind: [0.0, 0.0],
        seed: 11,
    }
}

/// Courtyard with the wall; rain around the camera, optionally a roof over cells x 1..4, z 5..8.
fn rain_scene(roof_all: bool, roof_part: bool) -> common::Scene {
    let mut s = courtyard(true);
    for z in 0..9 {
        for x in 0..9 {
            let c = s.grid.cell_mut(x, z).unwrap();
            if roof_all || (roof_part && (1..4).contains(&x) && (5..8).contains(&z)) {
                c.ceiling_texture = 1;
            }
        }
    }
    let mut rain = emitter(
        EMIT_RAIN,
        [4.5, 1.6, 6.0],
        [8.0, 4.0, 8.0],
        1500.0,
        0xa0b8d0,
    );
    rain.wind = [0.6, 0.0];
    s.renderer.create_emitter(rain);
    s
}

fn render_particles(
    s: &mut common::Scene,
    cam: &Camera,
    time: f64,
    threads: usize,
    extra: &[Particle],
) -> Vec<u8> {
    s.renderer.options = Options {
        threads,
        ..lit_options()
    };
    s.renderer.set_sun(Some(sun([0.5, -0.8, 0.4])));
    s.renderer.set_ambient(0x8090b0, 0.35);
    s.renderer.set_time(time);
    let r = &mut s.renderer;
    r.begin(cam);
    r.draw_grid(&s.grid);
    r.draw_emitters(Some(&s.grid));
    r.draw_particles(extra);
    r.end();
    r.rgba().to_vec()
}

#[test]
fn particles_same_time_same_image() {
    let cam = yard_camera();
    let mut s = rain_scene(false, false);
    s.renderer.create_emitter(emitter(
        EMIT_SPARKS,
        [4.5, 0.1, 6.0],
        [0.3, 0.0, 0.3],
        60.0,
        0xffa040,
    ));
    let a = render_particles(&mut s, &cam, 3.25, 1, &[]);
    let _ = render_particles(&mut s, &cam, 9.0, 1, &[]);
    let b = render_particles(&mut s, &cam, 3.25, 4, &[]);
    assert!(
        a == b,
        "same time, same picture (any thread count, any history)"
    );
    let c = render_particles(&mut s, &cam, 3.30, 1, &[]);
    assert!(a != c, "time moves the particles");
}

#[test]
fn sparks_rise_over_time() {
    let e = emitter(
        EMIT_SPARKS,
        [0.0, 0.0, 0.0],
        [0.2, 0.0, 0.2],
        40.0,
        0xffa040,
    );
    // particle 20 is born at 0.5 s
    let early = particles::emit_one(&e, 0.55, 20).unwrap();
    let late = particles::emit_one(&e, 0.9, 20).unwrap();
    assert!(
        late.particle.position.y > early.particle.position.y,
        "sparks rise"
    );
    assert!(late.particle.alpha < early.particle.alpha, "and fade");
    assert!(particles::emit_one(&e, 0.45, 20).is_none(), "not born yet");
    assert!(particles::emit_one(&e, 1.6, 20).is_none(), "burnt out");
}

#[test]
fn no_rain_indoors() {
    // camera under a roof over the whole yard, looking down at the floor
    let cam = Camera {
        position: Vec3::new(4.5, 1.2, 6.0),
        yaw: 0.0,
        pitch: -1.2,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    for roof in [true, false] {
        let mut s = rain_scene(roof, false);
        let rainy = render_particles(&mut s, &cam, 2.0, 1, &[]);
        assert!(s.renderer.destroy_emitter(0) && !s.renderer.destroy_emitter(0));
        let dry = render_particles(&mut s, &cam, 2.0, 1, &[]);
        assert_eq!(rainy == dry, roof, "roof = {roof}: rain only in the open");
    }
}

#[test]
fn snow_at_grid_border_no_nan() {
    let mut e = emitter(EMIT_SNOW, [0.0, 1.0, 0.0], [6.0, 3.0, 6.0], 800.0, 0xffffff);
    e.wind = [-0.3, 0.2];
    let mut out = Vec::new();
    particles::emit(&e, 4.0, 10_000, &mut out);
    assert!(!out.is_empty());
    let g = courtyard(false).grid;
    let mut outside = 0;
    for p in &out {
        let q = p.particle.position;
        assert!(
            q.x.is_finite() && q.y.is_finite() && q.z.is_finite() && p.particle.size.is_finite()
        );
        if q.x < 0.0 || q.z < 0.0 {
            assert!(
                !particles::indoors(&g, q),
                "outside the map nothing is culled"
            );
            outside += 1;
        }
    }
    assert!(outside > 0);
    let mut s = courtyard(false);
    s.renderer.create_emitter(e);
    let cam = Camera {
        position: Vec3::new(0.5, 1.0, 2.5),
        yaw: 0.8,
        pitch: -0.2,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    let img = render_particles(&mut s, &cam, 4.0, 1, &[]);
    assert!(s.renderer.depth().iter().all(|d| !d.is_nan()));
    assert_eq!(img, render_particles(&mut s, &cam, 4.0, 2, &[]));
}

#[test]
fn particles_negative_time() {
    for kind in [EMIT_SPARKS, EMIT_SMOKE, EMIT_RAIN, EMIT_SNOW] {
        let e = emitter(kind, [1.0, 1.0, 1.0], [2.0, 2.0, 2.0], 50.0, 0xffffff);
        for t in [-5.0, -0.01, 0.0] {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            particles::emit(&e, t, 10_000, &mut a);
            particles::emit(&e, t, 10_000, &mut b);
            assert!(
                !a.is_empty(),
                "kind {kind} at {t}: steady state also before 0"
            );
            assert_eq!(a, b);
            for p in &a {
                let q = p.particle;
                assert!(
                    q.position.x.is_finite()
                        && q.position.y.is_finite()
                        && q.position.z.is_finite()
                );
                assert!(q.alpha.is_finite() && q.size.is_finite());
            }
        }
    }
}

#[test]
fn particle_budget_drops_by_index() {
    let cam = yard_camera();
    let dot = |x: f32, color: u32| Particle {
        position: Vec3::new(x, 0.5, 6.0),
        size: 0.3,
        color,
        texture: -1,
        flags: particles::PARTICLE_EMISSIVE,
        alpha: 1.0,
    };
    // the first 4096 all sit at one spot, then one red particle elsewhere
    let mut list: Vec<Particle> = (0..particles::PARTICLE_BUDGET)
        .map(|_| dot(4.5, 0x00ff00))
        .collect();
    let mut s = courtyard(false);
    let full = render_particles(&mut s, &cam, 0.0, 1, &list);
    list.push(dot(3.0, 0xff0000));
    assert!(
        render_particles(&mut s, &cam, 0.0, 1, &list) == full,
        "past the budget nothing is drawn"
    );
    let alone = render_particles(&mut s, &cam, 0.0, 1, &list[particles::PARTICLE_BUDGET..]);
    assert!(
        alone != render_particles(&mut s, &cam, 0.0, 1, &[]),
        "the red one is visible on its own"
    );
    // the budget is per frame, not per call
    s.renderer.begin(&cam);
    s.renderer.draw_grid(&s.grid);
    s.renderer.draw_particles(&list[..2000]);
    s.renderer
        .draw_particles(&list[2000..particles::PARTICLE_BUDGET]);
    s.renderer
        .draw_particles(&list[particles::PARTICLE_BUDGET..]);
    s.renderer.end();
    assert!(s.renderer.rgba() == full.as_slice());
}

#[test]
fn rain_town() {
    let mut s = rain_scene(false, true);
    let img = render_particles(&mut s, &yard_camera(), 1.75, 0, &[]);
    check("rain_town", &img);
}

#[test]
fn campfire_sparks_smoke() {
    let mut s = courtyard(true);
    s.renderer.create_emitter(emitter(
        EMIT_SPARKS,
        [4.5, 0.05, 5.5],
        [0.4, 0.0, 0.4],
        50.0,
        0xffa040,
    ));
    let mut smoke = emitter(EMIT_SMOKE, [4.5, 0.3, 5.5], [0.3, 0.0, 0.3], 6.0, 0x707070);
    smoke.wind = [0.15, 0.0];
    s.renderer.create_emitter(smoke);
    s.renderer.create_light(torch([4.5, 0.4, 5.5], false));
    s.renderer.options = lit_options();
    s.renderer.set_sun(None);
    s.renderer.set_ambient(0x8090b0, 0.15);
    s.renderer.set_time(2.4);
    let cam = Camera {
        position: Vec3::new(4.5, 1.0, 8.0),
        yaw: 0.0,
        pitch: -0.15,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    let r = &mut s.renderer;
    r.begin(&cam);
    r.draw_grid(&s.grid);
    r.draw_emitters(None);
    r.end();
    let img = r.rgba().to_vec();
    check("campfire_sparks_smoke", &img);
}

/// A light of exactly -1 (or vertex lights -2 and 0 interpolating through -1) stays dark, as before
/// particles existed: no light value means "pre-lit".
#[test]
fn negative_light_stays_dark() {
    for lighting in [false, true] {
        let mut r = Renderer::new(W, H).unwrap();
        r.options = Options {
            lighting,
            ..Options::default()
        };
        r.set_ambient(0xffffff, 1.0);
        let t = r.add_texture(flat(1, 1, [200, 150, 100, 255]));
        r.begin(&Camera::default());
        r.draw_triangles(&square(-3.0), t, -1.0, false);
        r.end();
        assert!(
            r.rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[..3] == [0, 0, 0]),
            "light -1, lighting = {lighting}"
        );
        // sprite light -1 and a grid cell light -1
        let mut s = common::scene(W, H);
        s.renderer.options = Options {
            lighting,
            ..Options::default()
        };
        s.renderer.set_ambient(0xffffff, 1.0);
        for c in s.grid.cells.iter_mut() {
            c.light = -1.0;
        }
        s.renderer.begin(&common::camera());
        s.renderer.draw_grid(&s.grid);
        s.renderer.end();
        let lit = s
            .renderer
            .rgba()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(s.renderer.depth())
            .filter(|(p, d)| d.is_finite() && p[..3] != [0, 0, 0])
            .count();
        assert_eq!(lit, 0, "grid light -1, lighting = {lighting}");
    }
}

/// The default scene with a ramp in the yard leading up to a raised cell, plus a sloped ceiling indoors.
fn ramp_scene() -> common::Scene {
    let mut s = common::scene(W, H);
    let g = &mut s.grid;
    // (3,3) raised by 0.5; (3,4) in front of it ramps from 0 (south) to 0.5 (north).
    g.cell_mut(3, 3).unwrap().floor_height = 0.5;
    g.cell_mut(3, 4).unwrap().floor_corners = [0.5, 0.5, 0.0, 0.0];
    // (2,4) slopes sideways and diagonally: only its NE corner raised.
    g.cell_mut(2, 4).unwrap().floor_corners = [0.0, 0.5, 0.0, 0.0];
    // Indoor ceiling sloping down towards the west in (1..2, 2..3).
    for z in 2..=3 {
        g.cell_mut(1, z).unwrap().ceiling_corners = [-0.4, -0.2, -0.2, -0.4];
        g.cell_mut(2, z).unwrap().ceiling_corners = [-0.2, 0.0, 0.0, -0.2];
    }
    s
}

#[test]
fn ramp_golden() {
    let mut s = ramp_scene();
    let cam = Camera {
        position: Vec3::new(3.2, 1.6, 5.9),
        yaw: -0.1,
        pitch: -0.45,
        ..common::camera()
    };
    common::render(&mut s, &cam, 0.0);
    check("ramp", s.renderer.rgba());
    // Lit courtyard: a ramp up to a raised block, a diagonal slope beside it, sun shadows on the slopes.
    let mut l = courtyard(false);
    let stone = l.stone;
    let g = &mut l.grid;
    for x in 3..6 {
        let b = g.cell_mut(x, 3).unwrap();
        b.floor_height = 0.6;
        b.side_texture = stone;
    }
    g.cell_mut(4, 4).unwrap().floor_corners = [0.6, 0.6, 0.0, 0.0];
    g.cell_mut(5, 4).unwrap().floor_corners = [0.6, 0.0, 0.0, 0.0];
    for z in 4..6 {
        g.cell_mut(3, z).unwrap().side_texture = stone;
    }
    render_lit(
        "ramp_lit",
        &mut l,
        lit_options(),
        Some([-0.7, -0.5, -0.4]),
        0.3,
        &yard_camera(),
        false,
    );
}

/// Explicit zero corner offsets change nothing: same quads, same image as before slopes existed.
#[test]
fn slope_unsloped_identical() {
    let mut s = common::scene(W, H);
    let before = {
        let mut v = Vec::new();
        for z in 0..s.grid.depth {
            for x in 0..s.grid.width {
                s.grid.cell_quads(x, z, 0, &mut v);
            }
        }
        v.len()
    };
    for c in s.grid.cells.iter_mut() {
        c.floor_corners = [0.0; 4];
        c.ceiling_corners = [-0.0; 4];
    }
    let mut v = Vec::new();
    for z in 0..s.grid.depth {
        for x in 0..s.grid.width {
            s.grid.cell_quads(x, z, 0, &mut v);
        }
    }
    assert_eq!(v.len(), before);
    common::render(&mut s, &common::camera(), 0.0);
    check("plain_front", s.renderer.rgba());
}

#[test]
fn particles_after_end_are_ignored() {
    let cam = yard_camera();
    let mut s = rain_scene(false, false);
    let img = render_particles(&mut s, &cam, 1.0, 1, &[]);
    let depth = s.renderer.depth().to_vec();
    let dot = Particle {
        position: Vec3::new(4.5, 0.5, 6.0),
        size: 0.5,
        color: 0xff0000,
        texture: -1,
        flags: particles::PARTICLE_EMISSIVE,
        alpha: 1.0,
    };
    s.renderer.draw_particles(&[dot]);
    s.renderer.draw_emitters(Some(&s.grid));
    assert!(
        s.renderer.depth() == depth.as_slice(),
        "outside begin/end nothing is rasterised"
    );
    assert!(s.renderer.rgba() == img.as_slice());
}

// ---- Scrolled billboard shadows, light box with transparency, frame stats ----

#[test]
fn scrolled_billboard_shadow_uses_scrolled_uvs() {
    // holes in columns 1..3 (upper half); `shifted` is the same image moved by half its width
    let tex = |shifted: bool| {
        let mut rgba = Vec::new();
        for y in 0..16 {
            for x in 0..8 {
                let sx = if shifted { (x + 4) % 8 } else { x };
                let open = y < 8 && (1..3).contains(&sx);
                rgba.extend_from_slice(&[200, 180, 60, if open { 0 } else { 255 }]);
            }
        }
        Texture::from_rgba(8, 16, &rgba).unwrap()
    };
    let mut s = courtyard(false);
    let a = s.renderer.add_texture(tex(false));
    let still = render_anim_shadow(&mut s, a, 1.0);
    assert!(s.renderer.set_texture_scroll(a, 0.5, 0.0));
    let scrolled = render_anim_shadow(&mut s, a, 1.0);
    let mut t = courtyard(false);
    let b = t.renderer.add_texture(tex(true));
    let want = render_anim_shadow(&mut t, b, 1.0);
    assert!(still != want, "the two textures cast different shadows");
    assert!(
        scrolled == want,
        "the shadow of a scrolled billboard follows the scroll"
    );
}

#[test]
fn torch_lights_glass_beyond_opaque_pixels() {
    // nothing opaque on screen: only a glass pane far away, a torch right next to it
    let render = |torch: bool| {
        let mut r = Renderer::new(W, H).unwrap();
        let t = r.add_texture(flat(1, 1, [200, 200, 200, 255]));
        r.options = Options {
            lighting: true,
            ..Options::default()
        };
        r.set_ambient(0xffffff, 0.2);
        if torch {
            r.create_light(PointLight {
                pos: Vec3::new(0.0, 0.0, -4.5),
                color: 0xffcc88,
                strength: 1.0,
                radius: 2.0,
                flicker: 0.0,
                seed: 1,
                shadows: false,
            });
        }
        r.begin(&Camera::default());
        r.draw_triangles_ex(&square(-5.0), t, 1.0, false, false, 0, 0.5, BLEND_ALPHA);
        r.end();
        r.rgba().to_vec()
    };
    assert!(render(true) != render(false), "the torch lights the glass");
}

#[test]
fn stats_of_last_frame() {
    let mut r = Renderer::new(W, H).unwrap();
    let t = r.add_texture(flat(1, 1, [200, 200, 200, 255]));
    r.options = Options {
        lighting: true,
        ..Options::default()
    };
    let wall = |r: &mut Renderer| r.draw_triangles(&square(-6.0), t, 1.0, false);
    r.create_light(PointLight {
        pos: Vec3::new(0.0, 0.0, -5.0),
        color: 0xffffff,
        strength: 1.0,
        radius: 3.0,
        flicker: 0.0,
        seed: 1,
        shadows: true,
    });
    // covers the whole view
    let big: Vec<Vertex> = square(-2.0)
        .iter()
        .map(|v| Vertex {
            x: v.x * 10.0,
            y: v.y * 10.0,
            ..*v
        })
        .collect();
    let frame = |r: &mut Renderer, layers: usize, particles: usize| {
        r.begin(&Camera::default());
        wall(r);
        for _ in 0..layers {
            r.draw_triangles_ex(&big, t, 1.0, false, false, 0, 0.5, BLEND_ALPHA);
        }
        let dot = Particle {
            position: Vec3::new(0.0, 0.0, -3.0),
            size: 0.1,
            color: 0xffffff,
            texture: -1,
            flags: particles::PARTICLE_EMISSIVE,
            alpha: 1.0,
        };
        r.draw_particles(&vec![dot; particles]);
        r.end();
        r.stats()
    };
    let st = frame(&mut r, 0, 10);
    assert_eq!(
        (st.dropped_fragments, st.dropped_particles, st.lights_used),
        (0, 0, 1)
    );
    assert!(
        st.shadow_maps_rebuilt >= 1,
        "first frame builds the cube map"
    );
    assert_eq!(
        frame(&mut r, 0, 10).shadow_maps_rebuilt,
        0,
        "unchanged scene: cached"
    );
    // a full-screen layer is one fragment per pixel
    let layers = blend::MAX_FRAGS / (W * H) + 2;
    let st = frame(&mut r, layers, particles::PARTICLE_BUDGET + 7);
    assert_eq!(
        st.dropped_fragments as usize,
        layers * W * H - blend::MAX_FRAGS
    );
    assert_eq!(st.dropped_particles, 7);
}

// ---- Contour mask and contour_falloff (joint floor) ----

fn joint_floor_image(mask: bool, threads: usize) -> Vec<u8> {
    joint_floor_with(mask, false, threads, true)
}

fn joint_floor_with(mask: bool, auto: bool, threads: usize, dither: bool) -> Vec<u8> {
    let (mut r, g, t, cam) = common::joint_floor(W, H);
    if mask {
        assert!(r.set_texture_contour(t, Some(&common::joint_mask())));
    }
    if auto {
        assert!(r.texture_contour_auto(t, 1.0));
    }
    r.options.lod_dither = dither;
    r.options.threads = threads;
    r.begin(&cam);
    r.draw_grid(&g);
    r.end();
    r.rgba().to_vec()
}

#[test]
fn contour_floor_goldens() {
    let mut imgs = Vec::new();
    for (name, mask) in [("contour_floor_plain", false), ("contour_floor_mask", true)] {
        let img = joint_floor_image(mask, 1);
        check(name, &img);
        imgs.push(img);
    }
    assert_ne!(imgs[0], imgs[1], "mask variant differs from plain");
    let auto = joint_floor_with(false, true, 1, true);
    check("contour_floor_auto", &auto);
    assert_ne!(
        auto, imgs[0],
        "automatic mask differs from no mask (lines beat pits)"
    );
}

fn falloff_floor(falloff: f32, threads: usize) -> Vec<u8> {
    let (mut r, g, t, cam) = common::joint_floor(W, H);
    assert!(r.texture_contour_auto(t, 1.0));
    r.options.contour_falloff = falloff;
    r.options.threads = threads;
    r.begin(&cam);
    r.draw_grid(&g);
    r.end();
    r.rgba().to_vec()
}

#[test]
fn contour_falloff_floor() {
    let old = joint_floor_with(false, true, 1, true);
    assert!(falloff_floor(0.0, 1) == old, "falloff 0 = old image");
    let full = falloff_floor(1.0, 1);
    assert!(full != old);
    let (f, o) = (far_joint_share(&full, 12), far_joint_share(&old, 12));
    assert!(
        f < 0.7 * o,
        "far joints no longer cover the floor: {f} vs {o}"
    );
    for t in [2, 3, 8, 0] {
        assert!(falloff_floor(1.0, t) == full, "threads {t}");
    }
    let half = falloff_floor(0.5, 1);
    assert!(half != full && half != old);
    // switching back on the same renderer restores the old levels
    let (mut r, g, t, cam) = common::joint_floor(W, H);
    assert!(r.texture_contour_auto(t, 1.0));
    r.options.contour_falloff = 1.0;
    r.begin(&cam);
    r.draw_grid(&g);
    r.end();
    r.options.contour_falloff = 0.0;
    r.begin(&cam);
    r.draw_grid(&g);
    r.end();
    assert!(r.rgba() == old.as_slice());
}

/// Share of joint-coloured pixels among the floor pixels (not background) of the far band: the
/// first `rows` floor rows.
fn far_joint_share(img: &[u8], rows: usize) -> f32 {
    let px = img.as_chunks::<4>().0;
    let floor = |p: &[u8; 4]| p[..3] != [0x30, 0x38, 0x48];
    let first = (0..H)
        .find(|&y| px[y * W..(y + 1) * W].iter().any(floor))
        .unwrap();
    let band: Vec<&[u8; 4]> = px[first * W..(first + rows) * W]
        .iter()
        .filter(|p| floor(p))
        .collect();
    band.iter().filter(|p| p[..3] == [78, 66, 58]).count() as f32 / band.len() as f32
}

#[test]
fn contour_floor_any_thread_count() {
    let want = joint_floor_image(true, 1);
    for t in [2, 3, 8, 0] {
        assert!(joint_floor_image(true, t) == want, "threads {t}");
    }
}

#[test]
fn zero_mask_unchanged() {
    let plain = joint_floor_image(false, 1);
    let (mut r, g, t, cam) = common::joint_floor(W, H);
    assert!(r.set_texture_contour(t, Some(&[0u8; 32 * 32])));
    assert!(!r.set_texture_contour(t, Some(&[0u8; 7])));
    assert!(!r.set_texture_contour(99, None));
    r.begin(&cam);
    r.draw_grid(&g);
    r.end();
    assert!(r.rgba() == plain.as_slice());
}

// ---- Soft shadow edges (PR_Options.shadow_soft) ----

fn soft_options(soft: f32) -> Options {
    Options {
        shadow_soft: soft,
        ..lit_options()
    }
}

#[test]
fn soft_sun_shadow() {
    let hard = render_lit(
        "sun_wall_shadow",
        &mut courtyard(true),
        lit_options(),
        Some([0.5, -0.45, 1.0]),
        0.3,
        &yard_camera(),
        false,
    );
    let soft = render_lit(
        "soft_sun_shadow",
        &mut courtyard(true),
        soft_options(0.2),
        Some([0.5, -0.45, 1.0]),
        0.3,
        &yard_camera(),
        false,
    );
    assert_ne!(hard, soft, "soft edges change the image");
    // more brightness steps on the floor than the hard edge (lit / dark): the stepped penumbra
    let distinct = |img: &[u8]| {
        let mut v: Vec<u32> = (0..W * H).map(|i| lum(img, i)).collect();
        v.sort();
        v.dedup();
        v.len()
    };
    assert!(
        distinct(&soft) > distinct(&hard),
        "penumbra steps ({} vs {})",
        distinct(&soft),
        distinct(&hard)
    );
    // umbra and lit floor stay: the edge is only widened
    let diff = hard
        .as_chunks::<4>()
        .0
        .iter()
        .zip(soft.as_chunks::<4>().0)
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        diff < W * H / 6,
        "only the edge band changes ({diff} pixels)"
    );
}

#[test]
fn soft_off_or_invalid_is_hard() {
    let hard = render_lit(
        "sun_wall_shadow",
        &mut courtyard(true),
        lit_options(),
        Some([0.5, -0.45, 1.0]),
        0.3,
        &yard_camera(),
        false,
    );
    for soft in [0.0, -0.2, -f32::INFINITY, f32::NAN] {
        let mut s = courtyard(true);
        s.renderer.options = soft_options(soft);
        s.renderer.set_sun(Some(sun([0.5, -0.45, 1.0])));
        s.renderer.set_ambient(0x8090b0, 0.3);
        s.renderer.begin(&yard_camera());
        s.renderer.draw_grid(&s.grid);
        s.renderer.end();
        assert!(
            s.renderer.rgba() == hard.as_slice(),
            "shadow_soft {soft} must equal hard shadows"
        );
    }
}

#[test]
fn soft_huge_is_clamped() {
    let render = |soft: f32| {
        let mut s = courtyard(true);
        s.renderer.options = soft_options(soft);
        s.renderer.set_sun(Some(sun([0.5, -0.45, 1.0])));
        s.renderer.set_ambient(0x8090b0, 0.3);
        s.renderer.create_light(torch([4.5, 1.0, 5.0], true));
        s.renderer.begin(&yard_camera());
        s.renderer.draw_grid(&s.grid);
        s.renderer.end();
        s.renderer.rgba().to_vec()
    };
    let a = render(1e30);
    assert!(
        a == render(f32::INFINITY),
        "huge widths clamp to the same image"
    );
    assert!(a == render(1e30), "deterministic");
}

/// Like `sun_shadow_stable_while_gliding` with shadow_soft 0.2: the penumbra must stay on the
/// ground. Each floor pixel gets its shadow step (0..3) from its brightness between the unshadowed
/// and the sunless image; the dither may move a pixel by one step, never more, so per small patch
/// of ground the steps seen over all frames may span at most one step (plus the patch's own slope).
#[test]
fn soft_sun_shadow_stable_while_gliding() {
    let sun_dir = [0.5, -0.45, 1.0];
    let opts = Options {
        texture_lod: true,
        face_lod: true,
        snap: 0.0,
        ..soft_options(0.2)
    };
    let cell = 0.02;
    let mut seen: std::collections::BTreeMap<(i64, i64), (u8, u8)> =
        std::collections::BTreeMap::new();
    let mut penumbra = 0;
    for step in 0..20 {
        let mut cam = yard_camera();
        cam.position.x += 0.01 * step as f32;
        let shot = |sun_on: bool, shadows: bool| {
            let mut s = courtyard(true);
            s.renderer.options = opts;
            s.renderer.set_sun(Some(light::Sun {
                shadows,
                strength: if sun_on { 1.0 } else { 0.0 },
                ..sun(sun_dir)
            }));
            s.renderer.set_ambient(0x8090b0, 0.3);
            let r = &mut s.renderer;
            r.begin(&cam);
            r.draw_grid(&s.grid);
            r.end();
            let view = camera::View::new(&cam, W, H, 0.0);
            let pos: Vec<Option<Vec3>> = r
                .depth()
                .iter()
                .enumerate()
                .map(|(i, z)| {
                    z.is_finite()
                        .then(|| renderer::world_pos(&view, i % W, i / W, *z))
                })
                .collect();
            (r.rgba().to_vec(), pos)
        };
        let (img, pos) = shot(true, true);
        let (lit, _) = shot(true, false);
        let (dark, _) = shot(false, false);
        for (i, p) in pos.iter().enumerate() {
            let Some(p) = *p else { continue };
            if p.y.abs() > 0.02 || (p.z > 3.9 && p.z < 4.2) {
                continue;
            }
            let (l, d, v) = (
                lum(&lit, i) as f32,
                lum(&dark, i) as f32,
                lum(&img, i) as f32,
            );
            if l - d < 60.0 {
                continue;
            }
            let level = ((v - d) / (l - d) * 3.0).round().clamp(0.0, 3.0) as u8;
            penumbra += (level == 1 || level == 2) as usize;
            let e = seen
                .entry(((p.x / cell).floor() as i64, (p.z / cell).floor() as i64))
                .or_insert((level, level));
            e.0 = e.0.min(level);
            e.1 = e.1.max(level);
        }
    }
    let jumps = seen.values().filter(|(lo, hi)| hi - lo > 1).count();
    assert!(
        penumbra > 200 && seen.len() > 1000,
        "test sees floor and penumbra ({penumbra}, {})",
        seen.len()
    );
    println!(
        "glide: {jumps} jumps of {} patches, penumbra px {penumbra}",
        seen.len()
    );
    assert!(
        jumps * 100 <= seen.len(),
        "soft shadow crawls while the camera glides ({jumps} of {} patches)",
        seen.len()
    );
}

// ---- Soft shadows on a sunlit wall (Wizardry 7 report: speckle at the wall foot) ----

const WZ_W: usize = 176;
const WZ_H: usize = 112;

/// North-south corridor x = 3..6, walls one cell high on both sides, open sky.
fn wiz_corridor() -> common::Scene {
    let mut s = common::scene(WZ_W, WZ_H);
    let mut g = GridMap::new(9, 9, 1.0);
    for z in 0..9 {
        for x in 0..9 {
            let c = g.cell_mut(x, z).unwrap();
            c.floor_texture = 0;
            c.ceiling_texture = -1;
            c.ceiling_height = 1.0;
        }
        g.cell_mut(3, z).unwrap().walls[grid::WEST] = Wall {
            texture: 2,
            open: 0.0,
            alpha: 1.0,
            ..Wall::default()
        };
        g.cell_mut(5, z).unwrap().walls[grid::EAST] = Wall {
            texture: 2,
            open: 0.0,
            alpha: 1.0,
            ..Wall::default()
        };
    }
    s.grid = g;
    s
}

/// Renders the corridor; returns the image and, per pixel, whether it shows the east wall's
/// inner face (lit by the sun from the west, nothing between it and the sun).
fn wiz_wall_render(
    sun_soft: f32,
    point_soft: f32,
    torch_light: bool,
    shadows: bool,
) -> (Vec<u8>, Vec<bool>) {
    let mut s = wiz_corridor();
    s.renderer.options = Options {
        shadow_size: 9.0,
        shadow_res: 2048,
        shadow_soft: sun_soft,
        point_shadow_soft: point_soft,
        pick: true,
        ..lit_options()
    };
    if torch_light {
        s.renderer.set_sun(None);
        s.renderer.create_light(torch([3.2, 0.9, 5.0], shadows));
    } else {
        s.renderer.set_sun(Some(light::Sun {
            shadows,
            ..sun([1.0, -0.8, -0.6])
        }));
    }
    s.renderer.set_ambient(0x8090b0, 0.3);
    let cam = Camera {
        position: Vec3::new(4.5, 0.5, 8.5),
        yaw: 0.0,
        pitch: -0.05,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    s.renderer.begin(&cam);
    s.renderer.draw_grid(&s.grid);
    s.renderer.end();
    let mut wall = vec![false; WZ_W * WZ_H];
    for y in 0..WZ_H {
        for x in 0..WZ_W {
            if let Some(p) = s.renderer.pick(x as i32, y as i32) {
                wall[y * WZ_W + x] = p.cell_x == 5 && p.side == grid::EAST as i32;
            }
        }
    }
    (s.renderer.rgba().to_vec(), wall)
}

fn wall_diff(sun_soft: f32, point_soft: f32, torch_light: bool, name: &str) -> (usize, usize) {
    let (hard, wall) = wiz_wall_render(0.0, 0.0, torch_light, true);
    let (soft, _) = wiz_wall_render(sun_soft, point_soft, torch_light, true);
    let (none, _) = wiz_wall_render(0.0, 0.0, torch_light, false);
    let acne = (0..WZ_W * WZ_H)
        .filter(|&i| wall[i] && hard[i * 4..i * 4 + 3] != none[i * 4..i * 4 + 3])
        .count();
    assert!(
        acne < 20,
        "hard shadows keep the wall clean ({acne} pixels)"
    );
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("softfix-demo");
    std::fs::create_dir_all(&out).unwrap();
    let up = |img: &[u8]| {
        let mut v = Vec::with_capacity(WZ_W * WZ_H * 36);
        for y in 0..WZ_H * 3 {
            for x in 0..WZ_W * 3 {
                v.extend_from_slice(&img[((y / 3) * WZ_W + x / 3) * 4..][..4]);
            }
        }
        v
    };
    std::fs::write(
        out.join(format!("{name}_hard.png")),
        png::encode_rgba(WZ_W * 3, WZ_H * 3, &up(&hard), 4),
    )
    .unwrap();
    std::fs::write(
        out.join(format!("{name}_soft.png")),
        png::encode_rgba(WZ_W * 3, WZ_H * 3, &up(&soft), 4),
    )
    .unwrap();
    let n = wall.iter().filter(|&&w| w).count();
    // pixels the hard shadow already darkens (point light acne in the very last row at the foot) are
    // not the soft pattern's doing
    let px = |img: &[u8], i: usize| img[i * 4..i * 4 + 3].to_vec();
    let bad = (0..WZ_W * WZ_H)
        .filter(|&i| wall[i] && px(&hard, i) == px(&none, i) && px(&soft, i) != px(&none, i))
        .count();
    (n, bad)
}

/// A fully sunlit wall standing on a floor stays clean with soft shadows (hard is clean).
#[test]
fn soft_sun_lit_wall_clean() {
    let (n, bad) = wall_diff(0.2, 0.0, false, "wall_sun");
    assert!(n > 1000, "wall visible ({n} pixels)");
    assert_eq!(
        bad, 0,
        "soft taps shadow the sunlit wall ({bad} of {n} pixels)"
    );
}

/// Same for a torch lighting the opposite wall.
#[test]
fn soft_point_lit_wall_clean() {
    let (n, bad) = wall_diff(0.0, 0.2, true, "wall_torch");
    assert!(n > 1000, "wall visible ({n} pixels)");
    assert_eq!(
        bad, 0,
        "soft taps shadow the torch-lit wall ({bad} of {n} pixels)"
    );
}

// ---- Lens shift ----

/// Common scene with lighting, sun shadows, a torch, sky, outlines, billboards and particles.
fn shift_render(w: usize, h: usize, cam: &Camera) -> Vec<u8> {
    let mut s = common::scene(w, h);
    s.renderer.options = Options {
        lighting: true,
        shadow_size: 8.0,
        outlines: true,
        pick: true,
        ..Options::default()
    };
    s.renderer.set_sun(Some(sun([0.5, -0.6, -0.4])));
    s.renderer.set_ambient(0x8090b0, 0.3);
    s.renderer.create_light(torch([2.5, 0.7, 2.5], true));
    s.renderer.create_emitter(emitter(
        particles::EMIT_SPARKS,
        [3.0, 0.1, 3.5],
        [0.3, 0.0, 0.3],
        40.0,
        0xffa040,
    ));
    s.renderer.set_time(1.0);
    let r = &mut s.renderer;
    r.begin(cam);
    r.draw_grid(&s.grid);
    r.draw_sprite(&Sprite {
        position: Vec3::new(2.5, 0.0, 3.2),
        width: 0.4,
        height: 0.8,
        texture: s.figure[0],
        flags: renderer::SPRITE_BILLBOARD,
        yaw: 0.0,
        light: 0.8,
        tag: 7,
        alpha: 1.0,
        blend: BLEND_NONE,
    });
    r.draw_emitters(Some(&s.grid));
    r.end();
    r.rgba().to_vec()
}

fn crop(img: &[u8], w: usize, x0: usize, y0: usize, cw: usize, ch: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(cw * ch * 4);
    for y in y0..y0 + ch {
        out.extend_from_slice(&img[(y * w + x0) * 4..(y * w + x0 + cw) * 4]);
    }
    out
}

/// A horizontally shifted image is the matching crop of a wider one (the focal length depends only on
/// the height, so this is bit-exact).
#[test]
fn lens_shift_x_is_a_crop() {
    let cam = Camera {
        position: Vec3::new(3.0, 0.6, 5.3),
        pitch: -0.1,
        ..common::camera()
    };
    let wide = shift_render(160, 100, &cam);
    let left = shift_render(
        80,
        100,
        &Camera {
            shift_x: 0.5,
            ..cam
        },
    );
    let right = shift_render(
        80,
        100,
        &Camera {
            shift_x: -0.5,
            ..cam
        },
    );
    // the outermost column of each half sees the image border of the crop (outlines look one pixel
    // further), so compare all but the column next to the cut
    let l = crop(&left, 80, 0, 0, 79, 100);
    let r = crop(&right, 80, 1, 0, 79, 100);
    assert_eq!(l, crop(&wide, 160, 0, 0, 79, 100));
    assert_eq!(r, crop(&wide, 160, 81, 0, 79, 100));
}

/// Vertically: shift_y = +0.25 at height 100 (centre at row 25) shows rows 75..175 of a 200-high image whose field of
/// view gives the same focal length. The focal lengths then differ by float rounding only, so
/// nearly every pixel matches.
#[test]
fn lens_shift_y_is_a_crop() {
    let cam = Camera {
        position: Vec3::new(3.0, 0.6, 5.3),
        pitch: -0.1,
        ..common::camera()
    };
    let fov2 = 2.0 * (2.0 * (30f32).to_radians().tan()).atan().to_degrees();
    let tall = shift_render(80, 200, &Camera { fov_y: fov2, ..cam });
    let top = shift_render(
        80,
        100,
        &Camera {
            shift_y: 0.25,
            ..cam
        },
    );
    let bottom = shift_render(
        80,
        100,
        &Camera {
            shift_y: -0.25,
            ..cam
        },
    );
    let same = |a: &[u8], b: &[u8]| a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x == y).count();
    let n = 80 * 98;
    let t = same(
        &crop(&top, 80, 0, 1, 80, 98),
        &crop(&tall, 80, 0, 76, 80, 98),
    );
    let b = same(
        &crop(&bottom, 80, 0, 1, 80, 98),
        &crop(&tall, 80, 0, 26, 80, 98),
    );
    println!("lens shift y: top {t}/{n}, bottom {b}/{n} pixels equal");
    assert!(
        t * 1000 >= n * 995 && b * 1000 >= n * 995,
        "top {t}/{n}, bottom {b}/{n}"
    );
    assert_ne!(
        top,
        shift_render(80, 100, &cam),
        "shift_y changes the image"
    );
}

/// Picking reconstructs world points with the shifted centre: the picked point lies on the ray the
/// grid raycast sends through the same pixel.
#[test]
fn lens_shift_pick_matches_raycast() {
    let cam = Camera {
        shift_x: 0.2,
        shift_y: 0.3,
        ..common::camera()
    };
    let mut s = common::scene(W, H);
    s.renderer.options.pick = true;
    common::render(&mut s, &cam, 0.0);
    let view = camera::View::new(&cam, W, H, 0.0);
    let mut checked = 0;
    for (x, y) in [(5, 5), (80, 10), (150, 90), (40, 70), (120, 40)] {
        let Some(p) = s.renderer.pick(x, y) else {
            continue;
        };
        if p.kind != pick::PICK_GRID {
            continue;
        }
        let d = view.ray(x as usize, y as usize);
        let hit = s
            .grid
            .raycast(cam.position, d, 50.0)
            .expect("ray hits the grid");
        assert!(
            (hit.point - p.position).length() < 0.02,
            "pixel {x},{y}: {:?} vs {:?}",
            hit.point,
            p.position
        );
        checked += 1;
    }
    assert!(checked >= 3);
}

// ---- Doors and two-sided walls ----

/// Frame: grey stone border, transparent opening where the leaf goes (u 0.25..0.75, lower 3/4).
fn frame_texture() -> Texture {
    let (w, h) = (16, 16);
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let hole = (4..12).contains(&x) && y >= 4;
            let c = if hole {
                [0, 0, 0, 0]
            } else if (x + y) % 4 == 0 {
                [90, 90, 100, 255]
            } else {
                [150, 150, 160, 255]
            };
            rgba.extend_from_slice(&c);
        }
    }
    Texture::from_rgba(w, h, &rgba).unwrap()
}

/// Leaf: wooden planks with a yellow knob on the right and a red bar at the top left, so that
/// mirroring and sliding are visible.
fn leaf_texture() -> Texture {
    let (w, h) = (8, 12);
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let c = if x == 6 && y == 6 {
                [240, 220, 40]
            } else if y == 1 && x <= 3 {
                [200, 40, 40]
            } else if x % 3 == 2 {
                [90, 55, 25]
            } else {
                [150, 100, 50]
            };
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    Texture::from_rgba(w, h, &rgba).unwrap()
}

/// 5 x 6 grid; rooms x 1..3, z 1..2 (behind) and z 3..4 (front), joined only at (2, 3) north.
/// Returns the scene and the ids of the frame and leaf textures.
fn door_scene(w: usize, h: usize) -> (common::Scene, i32, i32) {
    let mut s = common::scene(w, h);
    let frame = s.renderer.add_texture(frame_texture());
    let leaf = s.renderer.add_texture(leaf_texture());
    let (floor, ceil, bricks, stone) = (0, 1, 2, 4);
    let mut g = GridMap::new(5, 6, 1.0);
    for z in 0..6 {
        for x in 0..5 {
            let c = g.cell_mut(x, z).unwrap();
            if !(1..=3).contains(&x) || !(1..=4).contains(&z) {
                c.ceiling_height = 0.0;
                continue;
            }
            c.floor_texture = if z <= 2 { stone } else { floor };
            c.ceiling_texture = ceil;
            c.light = if z <= 2 { 0.7 } else { 1.0 };
        }
    }
    let brick = Wall {
        texture: bricks,
        ..Wall::default()
    };
    for i in 1..=3 {
        g.cell_mut(i, 1).unwrap().walls[grid::NORTH] = brick;
        g.cell_mut(i, 4).unwrap().walls[grid::SOUTH] = brick;
        g.cell_mut(i, 3).unwrap().walls[grid::NORTH] = brick;
        g.cell_mut(i, 2).unwrap().walls[grid::SOUTH] = brick;
    }
    for z in 1..=4 {
        g.cell_mut(1, z).unwrap().walls[grid::WEST] = brick;
        g.cell_mut(3, z).unwrap().walls[grid::EAST] = brick;
    }
    g.cell_mut(2, 2).unwrap().walls[grid::SOUTH] = Wall::default();
    s.grid = g;
    (s, frame, leaf)
}

fn door_wall(frame: i32, leaf: i32, mode: i32, open: f32, angle: f32) -> Wall {
    Wall {
        texture: frame,
        leaf_texture: leaf,
        leaf_u0: 0.25,
        leaf_u1: 0.75,
        leaf_top: 0.75,
        door_mode: mode,
        open,
        swing_angle: angle,
        ..Wall::default()
    }
}

fn door_front_cam() -> Camera {
    Camera {
        position: Vec3::new(2.5, 0.5, 4.9),
        ..common::camera()
    }
}

fn door_side_cam() -> Camera {
    Camera {
        position: Vec3::new(1.25, 0.6, 4.5),
        yaw: 0.6,
        ..common::camera()
    }
}

fn render_door(w: usize, h: usize, wall: Wall, cam: &Camera, opts: Options) -> Vec<u8> {
    let (mut s, frame, leaf) = door_scene(w, h);
    let wall = Wall {
        texture: if wall.texture == -2 {
            frame
        } else {
            wall.texture
        },
        leaf_texture: if wall.leaf_texture == -2 {
            leaf
        } else {
            wall.leaf_texture
        },
        ..wall
    };
    s.grid.cell_mut(2, 3).unwrap().walls[grid::NORTH] = wall;
    s.renderer.options = opts;
    let r = &mut s.renderer;
    r.begin(cam);
    r.draw_grid(&s.grid);
    r.end();
    r.rgba().to_vec()
}

/// Panels of `pw` x H side by side with 2-pixel dark gaps, into one W x H image.
fn panels(list: &[Vec<u8>], pw: usize) -> Vec<u8> {
    let mut img = vec![0u8; W * H * 4];
    for p in img.chunks_mut(4) {
        p.copy_from_slice(&[40, 0, 40, 255]);
    }
    for (k, panel) in list.iter().enumerate() {
        let x0 = k * (pw + 2);
        for y in 0..H {
            for x in 0..pw {
                let d = (y * W + x0 + x) * 4;
                img[d..d + 4].copy_from_slice(&panel[(y * pw + x) * 4..(y * pw + x) * 4 + 4]);
            }
        }
    }
    img
}

const DOOR_PW: usize = 52;

fn door_strip(mode: i32, cam: &Camera, angle: f32) -> Vec<u8> {
    let list: Vec<Vec<u8>> = [0.0, 0.5, 1.0]
        .iter()
        .map(|&open| {
            render_door(
                DOOR_PW,
                H,
                door_wall(-2, -2, mode, open, angle),
                cam,
                Options::default(),
            )
        })
        .collect();
    panels(&list, DOOR_PW)
}

#[test]
fn door_modes_goldens() {
    let modes = [
        ("door_up", grid::DOOR_UP),
        ("door_slide_left", grid::DOOR_SLIDE_LEFT),
        ("door_slide_right", grid::DOOR_SLIDE_RIGHT),
        ("door_swing_left", grid::DOOR_SWING_LEFT),
        ("door_swing_right", grid::DOOR_SWING_RIGHT),
    ];
    for (name, mode) in modes {
        check(name, &door_strip(mode, &door_front_cam(), 90.0));
    }
    check(
        "door_swing_left_side",
        &door_strip(grid::DOOR_SWING_LEFT, &door_side_cam(), 90.0),
    );
    check(
        "door_swing_right_side",
        &door_strip(grid::DOOR_SWING_RIGHT, &door_side_cam(), 90.0),
    );
    // 100 degrees, fully open, front and side view
    let pw = 79;
    let list: Vec<Vec<u8>> = [door_front_cam(), door_side_cam()]
        .iter()
        .map(|c| {
            render_door(
                pw,
                H,
                door_wall(-2, -2, grid::DOOR_SWING_LEFT, 1.0, 100.0),
                c,
                Options::default(),
            )
        })
        .collect();
    check("door_swing_100", &panels(&list, pw));
}

/// Open 0 is the same image for every mode; the leaf of a closed door fills the frame's hole.
#[test]
fn closed_doors_look_the_same() {
    let cam = door_front_cam();
    let up = render_door(
        DOOR_PW,
        H,
        door_wall(-2, -2, grid::DOOR_UP, 0.0, 90.0),
        &cam,
        Options::default(),
    );
    for mode in [
        grid::DOOR_SLIDE_LEFT,
        grid::DOOR_SLIDE_RIGHT,
        grid::DOOR_SWING_LEFT,
        grid::DOOR_SWING_RIGHT,
    ] {
        assert_eq!(
            render_door(
                DOOR_PW,
                H,
                door_wall(-2, -2, mode, 0.0, 90.0),
                &cam,
                Options::default()
            ),
            up,
            "mode {mode}"
        );
    }
    // without a separate leaf, UP at any opening is today's door (whole wall slides up)
    for open in [0.0, 0.4, 1.0] {
        let plain = Wall {
            texture: 3,
            open,
            ..Wall::default()
        };
        let a = render_door(DOOR_PW, H, plain, &cam, Options::default());
        let b = render_door(
            DOOR_PW,
            H,
            Wall {
                door_mode: grid::DOOR_UP,
                leaf_u0: 0.2,
                leaf_u1: 0.4,
                leaf_top: 0.5,
                ..plain
            },
            &cam,
            Options::default(),
        );
        assert_eq!(a, b, "leaf rect is ignored without a leaf texture");
    }
    // a zeroed leaf rect (u0 = u1 = 0, top 0) means the whole wall
    let whole = door_wall(-2, -2, grid::DOOR_SLIDE_LEFT, 0.5, 90.0);
    let zeroed = Wall {
        leaf_u0: 0.0,
        leaf_u1: 0.0,
        leaf_top: 0.0,
        ..whole
    };
    let full = Wall {
        leaf_u0: 0.0,
        leaf_u1: 1.0,
        leaf_top: 1.0,
        ..whole
    };
    assert_eq!(
        render_door(DOOR_PW, H, zeroed, &cam, Options::default()),
        render_door(DOOR_PW, H, full, &cam, Options::default())
    );
}

/// A wall with back_texture seen from behind is the same image as a wall set on the neighbour cell.
#[test]
fn two_sided_wall() {
    let back_cam = Camera {
        position: Vec3::new(2.5, 0.5, 1.1),
        yaw: std::f32::consts::PI,
        ..common::camera()
    };
    let leaf = 9; // leaf_texture() in door_scene: asymmetric, shows mirroring
    let two = Wall {
        texture: leaf,
        back_texture: leaf,
        ..Wall::default()
    };
    let front = render_door(79, H, two, &door_front_cam(), Options::default());
    let back = render_door(79, H, two, &back_cam, Options::default());
    check(
        "two_sided_wall",
        &panels(&[front.clone(), back.clone()], 79),
    );
    // reference: the old way, a second wall on the neighbour cell (2, 2) facing south (same light there)
    let (mut s, _, _) = door_scene(79, H);
    s.grid.cell_mut(2, 3).unwrap().walls[grid::NORTH] = Wall {
        texture: leaf,
        ..Wall::default()
    };
    s.grid.cell_mut(2, 2).unwrap().walls[grid::SOUTH] = Wall {
        texture: leaf,
        ..Wall::default()
    };
    s.renderer.begin(&back_cam);
    s.renderer.draw_grid(&s.grid);
    s.renderer.end();
    assert_eq!(
        back,
        s.renderer.rgba(),
        "back face = mirrored wall, image reads the same way"
    );
    assert_eq!(
        front,
        render_door(
            79,
            H,
            Wall {
                texture: leaf,
                ..Wall::default()
            },
            &door_front_cam(),
            Options::default()
        )
    );
    // the neighbour's own wall wins over the back face
    s.grid.cell_mut(2, 3).unwrap().walls[grid::NORTH] = two;
    s.grid.cell_mut(2, 2).unwrap().walls[grid::SOUTH] = Wall {
        texture: 2,
        ..Wall::default()
    };
    s.renderer.begin(&back_cam);
    s.renderer.draw_grid(&s.grid);
    s.renderer.end();
    let with_both = s.renderer.rgba().to_vec();
    s.grid.cell_mut(2, 3).unwrap().walls[grid::NORTH] = Wall {
        texture: leaf,
        ..Wall::default()
    };
    s.renderer.begin(&back_cam);
    s.renderer.draw_grid(&s.grid);
    s.renderer.end();
    assert_eq!(with_both, s.renderer.rgba());
}

/// Lit scene: the shadow of a swung leaf follows `open`; same image for 1, 2 and 8 threads; the
/// shadow map is rebuilt when the leaf moves and reused when it does not.
#[test]
fn swung_leaf_shadow() {
    let cam = door_front_cam();
    let lit = |open: f32, threads: usize| {
        let (mut s, _, _) = door_scene(W, H);
        s.grid.cell_mut(2, 3).unwrap().walls[grid::NORTH] =
            door_wall(8, 9, grid::DOOR_SWING_LEFT, open, 90.0);
        s.renderer.options = Options {
            lighting: true,
            threads,
            ..Options::default()
        };
        s.renderer.set_ambient(0x8090b0, 0.2);
        s.renderer.create_light(torch([3.6, 0.9, 3.5], true));
        let r = &mut s.renderer;
        r.begin(&cam);
        r.draw_grid(&s.grid);
        r.end();
        let first = r.rgba().to_vec();
        // the same frame again reuses the cube map
        r.begin(&cam);
        r.draw_grid(&s.grid);
        r.end();
        assert_eq!(r.stats().shadow_maps_rebuilt, 0);
        assert_eq!(r.rgba(), first.as_slice(), "determinism");
        // move the leaf: rebuilt
        s.grid.cell_mut(2, 3).unwrap().walls[grid::NORTH].open = open + 0.1;
        let r = &mut s.renderer;
        r.begin(&cam);
        r.draw_grid(&s.grid);
        r.end();
        assert_eq!(r.stats().shadow_maps_rebuilt, 1, "cache key follows open");
        first
    };
    let a = lit(0.3, 1);
    let b = lit(0.7, 1);
    assert_ne!(a, b);
    for t in [2, 8] {
        assert_eq!(lit(0.3, t), a, "{t} threads");
    }
    // the shadow moves too, not only the leaf: pixels outside the leaf in both images differ
    let leaf_px = |img_open: f32| -> Vec<bool> {
        let (mut s, _, _) = door_scene(W, H);
        s.grid.cell_mut(2, 3).unwrap().walls[grid::NORTH] =
            door_wall(8, 9, grid::DOOR_SWING_LEFT, img_open, 90.0);
        s.renderer.options.pick = true;
        s.renderer.begin(&cam);
        s.renderer.draw_grid(&s.grid);
        s.renderer.end();
        (0..W * H)
            .map(|i| {
                s.renderer
                    .pick((i % W) as i32, (i / W) as i32)
                    .is_some_and(|p| p.part == grid::PART_LEAF)
            })
            .collect()
    };
    let (la, lb) = (leaf_px(0.3), leaf_px(0.7));
    let outside = (0..W * H)
        .filter(|&i| !la[i] && !lb[i] && a[i * 4..i * 4 + 3] != b[i * 4..i * 4 + 3])
        .count();
    println!("swung leaf shadow: {outside} pixels outside the leaf changed");
    assert!(
        outside > 20,
        "shadow moved on other surfaces: {outside} pixels"
    );
}

/// Doors with lighting, alpha and a sloped cell: frame and leaf follow the trapezoid, alpha applies to both.
#[test]
fn door_alpha_and_slope() {
    let cam = door_front_cam();
    let solid = render_door(
        DOOR_PW,
        H,
        door_wall(-2, -2, grid::DOOR_SLIDE_LEFT, 0.5, 90.0),
        &cam,
        Options::default(),
    );
    let glass = render_door(
        DOOR_PW,
        H,
        Wall {
            alpha: 0.5,
            ..door_wall(-2, -2, grid::DOOR_SLIDE_LEFT, 0.5, 90.0)
        },
        &cam,
        Options::default(),
    );
    assert_ne!(solid, glass);
    let (mut s, _, _) = door_scene(W, H);
    s.grid.cell_mut(2, 3).unwrap().walls[grid::NORTH] = door_wall(8, 9, grid::DOOR_UP, 0.5, 90.0);
    s.grid.cell_mut(2, 3).unwrap().ceiling_corners = [0.5, 0.0, 0.0, 0.0];
    let mut quads = Vec::new();
    s.grid.cell_quads(2, 3, 0, &mut quads);
    let walls: Vec<_> = quads
        .iter()
        .filter(|q| q.side == grid::NORTH as i32)
        .collect();
    // left strip, right strip, top strip, leaf
    assert_eq!(walls.len(), 4);
    for q in &walls {
        for c in q.corners {
            // all corners lie in the wall plane z = 3 and under the sloped ceiling (1.5 at x = 2, 1 at x = 3)
            assert_eq!(c.z, 3.0);
            assert!(c.y <= 1.5 - 0.5 * (c.x - 2.0) + 1e-6, "{c:?}");
        }
    }
}

// ---- Backdrops (background layers) ----

fn rgba_tex(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Texture {
    let mut v = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            v.extend_from_slice(&f(x, y));
        }
    }
    Texture::from_rgba(w, h, &v).unwrap()
}

/// Mountain silhouette: opaque below the ridge, holes above.
fn mountains() -> Texture {
    rgba_tex(64, 16, |x, y| {
        let ridge = 3 + ((x % 32) as i32 - 16).unsigned_abs() as usize / 2 + (x * 5 % 3);
        if y >= ridge.min(15) {
            [90 + (y * 4) as u8, 90 + (y * 3) as u8, 120, 255]
        } else {
            [255, 0, 255, 0]
        }
    })
}

/// Forest band: jagged tree tops, holes above.
fn forest() -> Texture {
    rgba_tex(32, 12, |x, y| {
        let top = [6, 2, 0, 3, 7, 4, 1, 5][x % 8];
        if y >= top {
            [20, 60 + (y * 5) as u8, 30, 255]
        } else {
            [0, 0, 0, 0]
        }
    })
}

/// Round disc with a marked centre texel, holes outside.
fn disc(n: usize, c: [u8; 3], centre: [u8; 3]) -> Texture {
    rgba_tex(n, n, |x, y| {
        let (dx, dy) = (x as i32 * 2 + 1 - n as i32, y as i32 * 2 + 1 - n as i32);
        let r2 = dx * dx + dy * dy;
        if x == n / 2 && y == n / 2 {
            [centre[0], centre[1], centre[2], 255]
        } else if r2 <= (n * n) as i32 {
            [c[0], c[1], c[2], 255]
        } else {
            [0, 0, 0, 0]
        }
    })
}

fn band(
    texture: i32,
    order: i32,
    elevation: f32,
    height: f32,
    repeats: f32,
    flags: u32,
) -> Backdrop {
    Backdrop {
        kind: BACKDROP_BAND,
        texture,
        order,
        elevation,
        height,
        repeats,
        flags,
        ..Backdrop::default()
    }
}

fn spot(texture: i32, order: i32, azimuth: f32, elevation: f32, size: f32) -> Backdrop {
    Backdrop {
        kind: BACKDROP_SPOT,
        texture,
        order,
        azimuth,
        elevation,
        size,
        ..Backdrop::default()
    }
}

/// Sky (legacy set_sky), sun, mountains, moon, forest.
fn add_layers(r: &mut Renderer) {
    let sky = r.add_texture(common::sky());
    r.set_sky(sky, 4.0);
    let m = r.add_texture(mountains());
    let f = r.add_texture(forest());
    let sun = r.add_texture(disc(7, [250, 220, 60], [255, 255, 255]));
    let moon = r.add_texture(disc(5, [200, 200, 220], [120, 120, 140]));
    // the sun sits behind the mountains (order between sky and mountains), the moon in front of them
    r.create_backdrop(spot(sun, 0, 10.0, 6.0, 0.0)).unwrap();
    r.create_backdrop(band(m, 1, -1.0, 14.0, 3.0, BACKDROP_TINT))
        .unwrap();
    r.create_backdrop(spot(moon, 2, -25.0, 18.0, 6.0)).unwrap();
    r.create_backdrop(band(
        f,
        3,
        -12.0,
        13.0,
        6.0,
        BACKDROP_TINT | BACKDROP_CLAMP_BOTTOM,
    ))
    .unwrap();
}

fn render_layers(cam: &Camera, opts: Options) -> Renderer {
    let mut r = Renderer::new(W, H).unwrap();
    r.options = opts;
    add_layers(&mut r);
    r.begin(cam);
    r.end();
    r
}

#[test]
fn backdrop_layers() {
    let cam = Camera::default();
    let r = render_layers(&cam, Options::default());
    check("backdrop_layers", r.rgba());
    let up = render_layers(&Camera { pitch: 0.25, ..cam }, Options::default());
    check("backdrop_layers_pitch_up", up.rgba());
    let down = render_layers(
        &Camera {
            pitch: -0.3,
            yaw: 0.4,
            ..cam
        },
        Options::default(),
    );
    check("backdrop_layers_pitch_down", down.rgba());
    // nothing written into depth, nothing pickable
    let p = render_layers(
        &cam,
        Options {
            pick: true,
            ..Options::default()
        },
    );
    assert!(p.depth().iter().all(|d| !d.is_finite()));
    assert!((0..W as i32).all(|x| p.pick(x, (H / 2) as i32).is_none()));
}

#[test]
fn backdrop_behind_geometry() {
    let mut s = common::scene(W, H);
    s.renderer.options = Options {
        outlines: true,
        ..Options::default()
    };
    add_layers(&mut s.renderer);
    // above the map, looking south over it to the horizon
    let cam = Camera {
        position: Vec3::new(3.5, 2.6, 0.5),
        yaw: std::f32::consts::PI + 0.2,
        pitch: -0.3,
        ..common::camera()
    };
    common::render(&mut s, &cam, 0.0);
    check("backdrop_behind_geometry", s.renderer.rgba());
}

/// Single-colour band: rows of the centre column show it exactly where the ray elevation lies in
/// [elevation, elevation + height).
#[test]
fn band_glued_to_horizon() {
    let (e0, hgt) = (5.0f32, 12.0f32);
    for (pitch, shift_y, fov) in [
        (0.0f32, 0.0f32, 60.0f32),
        (0.4, 0.0, 60.0),
        (-0.5, 0.0, 75.0),
        (0.2, 0.3, 50.0),
        (-0.1, -0.2, 60.0),
    ] {
        let mut r = Renderer::new(W, H).unwrap();
        let red = r.add_texture(rgba_tex(1, 1, |_, _| [255, 0, 0, 255]));
        r.create_backdrop(band(red, 0, e0, hgt, 1.0, 0)).unwrap();
        let cam = Camera {
            pitch,
            shift_y,
            fov_y: fov,
            ..Camera::default()
        };
        r.begin(&cam);
        r.end();
        let focal = H as f64 * 0.5 / (fov as f64 * 0.5).to_radians().tan();
        let cy = H as f64 * 0.5 - shift_y as f64 * H as f64;
        let x = W / 2;
        let mut seen = 0;
        for y in 0..H {
            let ang = (pitch as f64 + ((cy - (y as f64 + 0.5)) / focal).atan()).to_degrees();
            let near_edge =
                (ang - e0 as f64).abs() < 0.05 || (ang - (e0 + hgt) as f64).abs() < 0.05;
            if near_edge {
                continue;
            }
            let want = ang >= e0 as f64 && ang < (e0 + hgt) as f64;
            let got = r.rgba()[(y * W + x) * 4] == 255;
            assert_eq!(
                got, want,
                "pitch {pitch}, shift {shift_y}, row {y}, angle {ang}"
            );
            seen += want as usize;
        }
        assert!(seen > 0, "band visible at pitch {pitch}");
    }
}

#[test]
fn backdrop_lens_shift_is_crop() {
    // same height, wider image: shift_x moves the projection centre, which is a horizontal crop
    let wide = {
        let mut r = Renderer::new(W + 80, H).unwrap();
        add_layers(&mut r);
        r.begin(&Camera {
            pitch: 0.1,
            ..Camera::default()
        });
        r.end();
        r.rgba().to_vec()
    };
    for (sx, ox) in [(0.25f32, 0usize), (-0.25, 80)] {
        let r = render_layers(
            &Camera {
                pitch: 0.1,
                shift_x: sx,
                ..Camera::default()
            },
            Options::default(),
        );
        for y in 0..H {
            let a = &r.rgba()[y * W * 4..(y + 1) * W * 4];
            let b = &wide[(y * (W + 80) + ox) * 4..(y * (W + 80) + ox + W) * 4];
            assert_eq!(a, b, "shift {sx}, row {y}");
        }
    }
    // vertical shift: a taller image with the same focal length (tiny float differences allowed)
    let fov = 60.0f32;
    let fov2 = (2.0 * (fov.to_radians() * 0.5).tan()).atan().to_degrees() * 2.0;
    let tall = {
        let mut r = Renderer::new(W, 2 * H).unwrap();
        add_layers(&mut r);
        r.begin(&Camera {
            fov_y: fov2,
            ..Camera::default()
        });
        r.end();
        r.rgba().to_vec()
    };
    let r = render_layers(
        &Camera {
            shift_y: 0.5,
            ..Camera::default()
        },
        Options::default(),
    );
    // cy = H/2 - H/2 = 0: the top row of the shifted image is the centre row of the tall one
    let b = &tall[H * W * 4..2 * H * W * 4];
    let same = r
        .rgba()
        .chunks(4)
        .zip(b.chunks(4))
        .filter(|(p, q)| p == q)
        .count();
    assert!(
        same * 1000 >= W * H * 995,
        "{same} of {} pixels equal",
        W * H
    );
}

#[test]
fn band_wraps_seamlessly() {
    // 32 columns with their own colour, once around. Looking south shows the atan2 jump (u = +-0.5,
    // columns 15 | 16), looking north the texture wrap (u = 0, columns 31 | 0); both must be seamless.
    for (yaw, a, b) in [(std::f32::consts::PI, 15, 16), (0.0, 31, 0)] {
        let mut r = Renderer::new(W, H).unwrap();
        let t = r.add_texture(rgba_tex(32, 1, |x, _| [x as u8 * 8, 100, 0, 255]));
        r.create_backdrop(band(t, 0, -40.0, 80.0, 1.0, 0)).unwrap();
        r.begin(&Camera {
            yaw,
            fov_y: 90.0,
            ..Camera::default()
        });
        r.end();
        let y = H / 2;
        let cols: Vec<i32> = (0..W)
            .map(|x| r.rgba()[(y * W + x) * 4] as i32 / 8)
            .collect();
        let mut steps = 0;
        for k in 1..W {
            let d = (cols[k] - cols[k - 1]).rem_euclid(32);
            assert!(
                d == 0 || d == 1,
                "yaw {yaw}, column {k}: {} -> {}",
                cols[k - 1],
                cols[k]
            );
            steps += d;
        }
        // about 112 degrees across at 11.25 degrees per texel
        assert!((9..=11).contains(&steps), "{steps} texel steps");
        assert!(
            cols.contains(&a) && cols.contains(&b),
            "the seam is in view"
        );
        // the seam lies at the image centre (pixel columns W/2 - 1 and W/2)
        assert_eq!((cols[W / 2 - 1], cols[W / 2]), (a, b), "yaw {yaw}");
    }
}

#[test]
fn spot_lands_at_direction() {
    for (az, el, size, yaw, pitch) in [
        (30.0f32, 20.0f32, 0.0f32, 0.3f32, 0.2f32),
        (-50.0, 5.0, 4.0, -0.7, 0.0),
        (170.0, 40.0, 9.0, 2.9, 0.5),
    ] {
        let mut r = Renderer::new(W, H).unwrap();
        let t = r.add_texture(disc(5, [250, 220, 60], [10, 250, 250]));
        let id = r.create_backdrop(spot(t, 0, az, el, size)).unwrap();
        let cam = Camera {
            yaw,
            pitch,
            ..Camera::default()
        };
        r.begin(&cam);
        r.end();
        let (a, e) = (az.to_radians(), el.to_radians());
        let d = Vec3::new(e.cos() * a.sin(), e.sin(), -e.cos() * a.cos());
        let view = camera::View::new(&cam, W, H, 0.0);
        let p = view.to_view(cam.position + d * 100.0);
        let px = view.cx + p.x / p.z * view.focal;
        let py = view.cy - p.y / p.z * view.focal;
        let (x, y) = (px.floor() as usize, py.floor() as usize);
        assert!(x < W && y < H, "in view");
        let s = if size > 0.0 {
            ((size.to_radians() * view.focal / 5.0).round() as usize).max(1)
        } else {
            1
        };
        // the centre texel covers the projected pixel (within half a texel of rounding)
        let hit = (0..H)
            .flat_map(|yy| (0..W).map(move |xx| (xx, yy)))
            .filter(|&(xx, yy)| r.rgba()[(yy * W + xx) * 4..][..3] == [10, 250, 250])
            .collect::<Vec<_>>();
        assert_eq!(hit.len(), s * s, "centre texel is s x s pixels, s = {s}");
        assert!(
            hit.iter()
                .any(|&(xx, yy)| xx.abs_diff(x) <= s / 2 + 1 && yy.abs_diff(y) <= s / 2 + 1),
            "centre at ({x}, {y}): {hit:?}"
        );
        // whole-texel scale: the disc is 5 * s pixels wide on its centre row
        let yc = hit[0].1;
        let wide = (0..W)
            .filter(|&k| r.rgba()[(yc * W + k) * 4 + 1] >= 220)
            .count();
        assert_eq!(wide, 5 * s, "width at scale {s}");
        // behind the camera: nothing
        let mut back = r.backdrop(id).unwrap();
        back.azimuth += 180.0;
        back.elevation = -back.elevation;
        assert!(r.set_backdrop(id, back));
        r.begin(&cam);
        r.end();
        assert!(
            r.rgba().chunks(4).all(|p| p[..3] == [0, 0, 0]),
            "spot behind the camera is not drawn"
        );
    }
}

#[test]
fn backdrop_order_and_destroy() {
    let mut r = Renderer::new(W, H).unwrap();
    let red = r.add_texture(rgba_tex(1, 1, |_, _| [255, 0, 0, 255]));
    let blue = r.add_texture(rgba_tex(1, 1, |_, _| [0, 0, 255, 255]));
    let at = |r: &mut Renderer| {
        r.begin(&Camera::default());
        r.end();
        r.rgba()[((H / 2) * W + W / 2) * 4..][..3].to_vec()
    };
    let a = r
        .create_backdrop(band(red, 1, -45.0, 90.0, 1.0, 0))
        .unwrap();
    let b = r
        .create_backdrop(band(blue, 0, -45.0, 90.0, 1.0, 0))
        .unwrap();
    assert_eq!(at(&mut r), [255, 0, 0], "higher order in front");
    let mut ba = r.backdrop(a).unwrap();
    ba.order = 0;
    assert!(r.set_backdrop(a, ba));
    assert_eq!(at(&mut r), [0, 0, 255], "same order: higher id in front");
    ba.order = -1;
    assert!(r.set_backdrop(a, ba));
    assert_eq!(at(&mut r), [0, 0, 255]);
    assert!(r.destroy_backdrop(b));
    assert!(!r.destroy_backdrop(b));
    assert!(!r.set_backdrop(b, ba), "destroyed id");
    assert_eq!(at(&mut r), [255, 0, 0]);
    let c = r
        .create_backdrop(band(blue, 0, -45.0, 90.0, 1.0, 0))
        .unwrap();
    assert!(c > b, "ids are never reused");
    // invalid: unknown texture, bad kind, zero height; a failed set changes nothing
    assert!(r.create_backdrop(band(99, 0, 0.0, 10.0, 1.0, 0)).is_none());
    assert!(
        r.create_backdrop(Backdrop {
            kind: 7,
            texture: red,
            ..Backdrop::default()
        })
        .is_none()
    );
    assert!(r.create_backdrop(band(red, 0, 0.0, 0.0, 1.0, 0)).is_none());
    assert!(!r.set_backdrop(c, band(red, 0, 0.0, f32::NAN, 1.0, 0)));
    assert_eq!(r.backdrop(c).unwrap().texture, blue);
    assert!(r.destroy_backdrop(a) && r.destroy_backdrop(c));
    assert_eq!(at(&mut r), [0, 0, 0], "background colour again");
}

#[test]
fn band_holes_and_clamps() {
    let mut r = Renderer::new(W, H).unwrap();
    r.options.background = 0x000040;
    // two rows: top green, bottom red; column 1 is a hole
    let t = r.add_texture(rgba_tex(2, 2, |x, y| {
        if x == 1 {
            [9, 9, 9, 0]
        } else if y == 0 {
            [0, 255, 0, 255]
        } else {
            [255, 0, 0, 255]
        }
    }));
    let id = r.create_backdrop(band(t, 0, -5.0, 10.0, 1.0, 0)).unwrap();
    let col = |r: &mut Renderer, flags: u32| {
        let b = Backdrop {
            flags,
            ..r.backdrop(id).unwrap()
        };
        r.set_backdrop(id, b);
        r.begin(&Camera {
            fov_y: 90.0,
            ..Camera::default()
        });
        r.end();
        // the centre column looks north: azimuth 0 = texel column 0
        (0..H)
            .map(|y| r.rgba()[(y * W + W / 2) * 4..][..3].to_vec())
            .collect::<Vec<_>>()
    };
    let none = col(&mut r, 0);
    assert_eq!(none[0], [0, 0, 0x40], "above: background");
    assert_eq!(none[H - 1], [0, 0, 0x40], "below: background");
    assert_eq!(none[H / 2 - 1], [0, 255, 0]);
    assert_eq!(none[H / 2], [255, 0, 0]);
    let both = col(&mut r, BACKDROP_CLAMP_TOP | BACKDROP_CLAMP_BOTTOM);
    assert_eq!(both[0], [0, 255, 0], "top row extended upward");
    assert_eq!(both[H - 1], [255, 0, 0], "bottom row extended downward");
    // looking at azimuth 270 degrees (texel column 1): a hole at every height
    r.begin(&Camera {
        yaw: -std::f32::consts::FRAC_PI_2,
        fov_y: 90.0,
        ..Camera::default()
    });
    r.end();
    assert!((0..H).all(|y| r.rgba()[(y * W + W / 2) * 4..][..3] == [0, 0, 0x40]));
}

#[test]
fn backdrop_set_sky_equivalent() {
    // the legacy sky is a full band with both clamps and the tint flag
    for lighting in [false, true] {
        let cam = Camera {
            pitch: 0.3,
            yaw: 2.0,
            ..Camera::default()
        };
        let opts = Options {
            lighting,
            ..Options::default()
        };
        let mut a = Renderer::new(W, H).unwrap();
        a.options = opts;
        let t = a.add_texture(common::sky());
        a.set_sky_tint(0x8090ff, 0.9);
        a.set_sky(t, 4.0);
        a.begin(&cam);
        a.end();
        let mut b = Renderer::new(W, H).unwrap();
        b.options = opts;
        let t = b.add_texture(common::sky());
        b.set_sky_tint(0x8090ff, 0.9);
        b.create_backdrop(band(
            t,
            0,
            0.0,
            90.0,
            4.0,
            BACKDROP_TINT | BACKDROP_CLAMP_TOP | BACKDROP_CLAMP_BOTTOM,
        ))
        .unwrap();
        b.begin(&cam);
        b.end();
        assert_eq!(a.rgba(), b.rgba(), "lighting {lighting}");
    }
}

#[test]
fn backdrop_indexed_colours_exact() {
    let pal = common::palette16();
    let mut r = Renderer::new(W, H).unwrap();
    r.set_palette(&pal);
    r.options.dither = false;
    // indexed band: palette entries 3, 10, 13 and the transparent index 0 (shows background black = 0)
    let px: Vec<u8> = (0..16 * 8)
        .map(|i| [0u8, 3, 10, 13][(i * 7 + i / 16) % 4])
        .collect();
    let t = Texture::from_indexed(16, 8, &px, &pal, Some(0)).unwrap();
    let t = r.add_texture(t);
    let c = |k: usize| [pal[k * 3], pal[k * 3 + 1], pal[k * 3 + 2]];
    let m = r.add_texture(disc(5, c(5), c(12)));
    r.create_backdrop(band(t, 0, -20.0, 40.0, 3.0, BACKDROP_TINT))
        .unwrap();
    r.create_backdrop(spot(m, 1, 0.0, 0.0, 3.0)).unwrap();
    r.begin(&Camera::default());
    r.end();
    let allowed = [0u8, 3, 10, 13, 5, 12];
    let mut seen = std::collections::BTreeSet::new();
    for (i, &k) in r.indexed().iter().enumerate() {
        assert!(allowed.contains(&k), "index {k}");
        assert_eq!(
            &r.rgba()[i * 4..i * 4 + 3],
            &pal[k as usize * 3..k as usize * 3 + 3]
        );
        seen.insert(k);
    }
    assert_eq!(seen.len(), allowed.len(), "every colour shows: {seen:?}");
}

#[test]
fn backdrop_threads_identical() {
    let mut want = None;
    for threads in [1, 2, 8] {
        let mut s = common::scene(W, H);
        s.renderer.options = Options {
            threads,
            outlines: true,
            fog_start: 2.0,
            fog_end: 8.0,
            fog_min: 0.3,
            fog_color: 0x405060,
            ..Options::default()
        };
        add_layers(&mut s.renderer);
        let t = s
            .renderer
            .add_texture(disc(9, [255, 120, 40], [255, 255, 255]));
        s.renderer
            .create_backdrop(Backdrop {
                blend: BLEND_ADD,
                alpha: 0.5,
                flags: BACKDROP_FOG,
                ..spot(t, 9, 5.0, 12.0, 8.0)
            })
            .unwrap();
        common::render(
            &mut s,
            &Camera {
                pitch: 0.4,
                ..common::camera()
            },
            0.3,
        );
        let img = s.renderer.rgba().to_vec();
        match &want {
            None => want = Some(img),
            Some(w) => assert!(w == &img, "threads = {threads}"),
        }
    }
}

#[test]
fn backdrop_blend_fog_anim_scroll() {
    let px = |r: &mut Renderer, time: f64, x: usize, y: usize| {
        r.set_time(time);
        r.begin(&Camera::default());
        r.end();
        r.rgba()[(y * W + x) * 4..][..3].to_vec()
    };
    let (cx, cy) = (W / 2, H / 2);
    let mut r = Renderer::new(W, H).unwrap();
    r.options.background = 0x404040;
    let white = r.add_texture(rgba_tex(3, 3, |_, _| [200, 200, 200, 255]));
    let id = r
        .create_backdrop(Backdrop {
            blend: BLEND_ALPHA,
            alpha: 0.5,
            ..spot(white, 0, 0.0, 0.0, 0.0)
        })
        .unwrap();
    assert_eq!(px(&mut r, 0.0, cx, cy), [132, 132, 132], "alpha 1/2");
    let b = Backdrop {
        alpha: 0.3,
        ..r.backdrop(id).unwrap()
    };
    r.set_backdrop(id, b);
    // 0.3 steps to 2/8: 64 * 0.75 + 200 * 0.25 = 98
    assert_eq!(px(&mut r, 0.0, cx, cy), [98, 98, 98], "alpha in 1/8 steps");
    r.set_backdrop(
        id,
        Backdrop {
            blend: BLEND_ADD,
            alpha: 0.25,
            ..b
        },
    );
    assert_eq!(px(&mut r, 0.0, cx, cy), [114, 114, 114], "additive");
    // fog: the layer gets the fog colour mix at fog_min
    r.set_backdrop(
        id,
        Backdrop {
            blend: BLEND_NONE,
            flags: BACKDROP_FOG,
            ..b
        },
    );
    r.options.fog_start = 1.0;
    r.options.fog_end = 5.0;
    r.options.fog_min = 0.25;
    r.options.fog_color = 0x0000ff;
    assert_eq!(px(&mut r, 0.0, cx, cy), [50, 50, 241], "fog");
    r.set_backdrop(
        id,
        Backdrop {
            blend: BLEND_NONE,
            flags: 0,
            ..b
        },
    );
    assert_eq!(px(&mut r, 0.0, cx, cy), [200, 200, 200], "no fog flag");
    // animated spot
    let red = r.add_texture(rgba_tex(3, 3, |_, _| [255, 0, 0, 255]));
    let anim = r.add_texture_anim(&[white, red], 2.0);
    let b = r.backdrop(id).unwrap();
    r.set_backdrop(id, Backdrop { texture: anim, ..b });
    assert_eq!(px(&mut r, 0.0, cx, cy), [200, 200, 200]);
    assert_eq!(px(&mut r, 0.5, cx, cy), [255, 0, 0], "spot animates");
    assert!(r.destroy_backdrop(id));
    // animated and scrolling band
    let stripes = r.add_texture(rgba_tex(8, 1, |x, _| [x as u8 * 30, 0, 0, 255]));
    let green = r.add_texture(rgba_tex(8, 1, |_, _| [0, 255, 0, 255]));
    let ab = r.add_texture_anim(&[stripes, green], 2.0);
    r.create_backdrop(band(ab, 0, -30.0, 60.0, 1.0, 0)).unwrap();
    assert_eq!(px(&mut r, 0.0, cx, cy), [0, 0, 0], "azimuth 0 = column 0");
    assert_eq!(px(&mut r, 0.5, cx, cy), [0, 255, 0], "band animates");
    assert!(r.set_texture_scroll(ab, 0.125, 0.0));
    assert_eq!(
        px(&mut r, 1.0, cx, cy),
        [30, 0, 0],
        "band scrolls along the horizon"
    );
}

// ---- Storeys ----

fn storey_render(
    name: &str,
    grid: GridMap,
    cam: &Camera,
    sun_dir: Option<[f32; 3]>,
    threads: usize,
) -> Vec<u8> {
    let mut s = common::scene(W, H);
    s.grid = grid;
    let o = Options {
        lighting: sun_dir.is_some(),
        threads,
        shadow_size: 12.0,
        ..Options::default()
    };
    render_lit(name, &mut s, o, sun_dir, 0.45, cam, false)
}

/// From the lower room, looking up through the untextured ceiling of the middle cell into the room
/// above: its door-texture walls and stone ceiling, the brick-lined shaft between the two spans.
#[test]
fn storey_hole_up() {
    let cam = Camera {
        position: Vec3::new(2.5, 0.4, 3.7),
        yaw: 0.0,
        pitch: 0.75,
        fov_y: 70.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    storey_render("storey_hole_up", common::hole_rooms(), &cam, None, 1);
}

/// A bridge over a street: from the street the slab (door texture on its edge, ceiling texture
/// underneath) and the street going on under it; from the deck, the street below.
#[test]
fn storey_bridge() {
    let street = Camera {
        position: Vec3::new(4.5, 0.6, 7.2),
        yaw: 0.0,
        pitch: 0.12,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    storey_render(
        "storey_bridge_street",
        common::bridge(),
        &street,
        Some([0.3, -0.8, -0.4]),
        1,
    );
    let deck = Camera {
        position: Vec3::new(4.5, 1.85, 4.2),
        yaw: std::f32::consts::PI,
        pitch: -0.5,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    storey_render(
        "storey_bridge_deck",
        common::bridge(),
        &deck,
        Some([0.3, -0.8, -0.4]),
        1,
    );
}

/// Town house with a gable roof made of a second span: room below, sloped roof floor above, gable
/// triangles on the west and east sides, sun shadow of the roof on the street. Same image for any
/// thread count.
#[test]
fn storey_gable_house() {
    let mut s = common::scene(W, H);
    let roof = s
        .renderer
        .add_texture(common::checker(16, 16, [170, 60, 40], [120, 40, 30], 4));
    let cam = Camera {
        position: Vec3::new(7.6, 2.4, 7.9),
        yaw: -0.75,
        pitch: -0.3,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    let mut images = Vec::new();
    for threads in [1, 2, 8] {
        s.grid = common::gable_house(roof);
        let o = Options {
            lighting: true,
            threads,
            shadow_size: 12.0,
            ..Options::default()
        };
        images.push(render_lit(
            "storey_gable_house",
            &mut s,
            o,
            Some([0.5, -0.7, -0.35]),
            0.45,
            &cam,
            false,
        ));
    }
    assert!(
        images[0] == images[1] && images[0] == images[2],
        "same image for 1, 2 and 8 threads"
    );
}
