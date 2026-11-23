//! Light gaps: sun leaking into closed rooms (volume closing for shadows), torch light on
//! particles and billboards, stippled shadows of see-through casters, scrolling casters and the
//! shadow cache. Golden images as in tests/images.rs (PIXELRAUM_BLESS=1 writes them).
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

fn check(name: &str, rgba: &[u8]) {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("golden-out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(
        out.join(format!("{name}.png")),
        png::encode_rgba(W, H, rgba, 4),
    )
    .unwrap();
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(format!("{name}.ppm"));
    let mut actual = format!("P6\n{W} {H}\n255\n").into_bytes();
    for p in rgba.as_chunks::<4>().0 {
        actual.extend_from_slice(&p[..3]);
    }
    if std::env::var("PIXELRAUM_BLESS").is_ok() {
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

fn sun(d: [f32; 3]) -> Sun {
    Sun {
        dir: Vec3::new(d[0], d[1], d[2]),
        color: 0xffffff,
        strength: 1.0,
        shadows: true,
    }
}

fn lit() -> Options {
    Options {
        lighting: true,
        shadow_size: 8.0,
        ..Options::default()
    }
}

fn wiz() -> Options {
    Options {
        lighting: true,
        shadow_size: 9.0,
        shadow_res: 2048,
        ..Options::default()
    }
}

// ---- 1. Sun leaking into closed rooms ----

/// `map` x `map` cells; the roofed room lo..=hi (ceiling 1, textured) is surrounded by solid cells
/// (ceiling 0), optionally with textured walls all around.
fn room(map: usize, lo: usize, hi: usize, walls: bool) -> common::Scene {
    let mut s = common::scene(W, H);
    let mut g = GridMap::new(map, map, 1.0);
    for z in 0..map {
        for x in 0..map {
            let c = g.cell_mut(x, z).unwrap();
            if !(x >= lo && x <= hi && z >= lo && z <= hi) {
                c.ceiling_height = 0.0;
                continue;
            }
            c.floor_texture = 0;
            c.ceiling_texture = 1;
            c.ceiling_height = 1.0;
        }
    }
    if walls {
        for i in lo..=hi {
            g.cell_mut(i, lo).unwrap().walls[grid::NORTH].texture = 2;
            g.cell_mut(i, hi).unwrap().walls[grid::SOUTH].texture = 2;
            g.cell_mut(lo, i).unwrap().walls[grid::WEST].texture = 2;
            g.cell_mut(hi, i).unwrap().walls[grid::EAST].texture = 2;
        }
    }
    s.grid = g;
    s
}

fn frame(s: &mut common::Scene, o: Options, sun_dir: Option<[f32; 3]>, cam: &Camera) -> Vec<u8> {
    s.renderer.options = o;
    s.renderer.set_sun(sun_dir.map(sun));
    s.renderer.set_ambient(0x8090b0, 0.3);
    s.renderer.begin(cam);
    s.renderer.draw_grid(&s.grid);
    s.renderer.end();
    s.renderer.rgba().to_vec()
}

fn differing(a: &[u8], b: &[u8]) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(p, q)| p[..3] != q[..3])
        .count()
}

const SUNS: [[f32; 3]; 5] = [
    [0.5, -0.45, 1.0],
    [-0.9, -0.5, 0.6],
    [1.0, -0.8, -0.6],
    [0.0, -1.0, 0.3],
    [0.3, -0.2, -1.0],
];

/// Pixels the sun changes inside a room it cannot reach, summed over four views from the middle
/// (looking slightly up at the wall tops and down at the wall feet).
fn sun_leak(map: usize, lo: usize, hi: usize, walls: bool, o: Options, d: [f32; 3]) -> usize {
    let mut total = 0;
    for yaw in [0.0f32, 1.57, 3.14, -1.57] {
        let c = (lo + hi) as f32 * 0.5 + 0.5;
        let cam = Camera {
            position: Vec3::new(c, 0.5, c),
            yaw,
            pitch: 0.25,
            fov_y: 70.0,
            near: 0.05,
            shift_x: 0.0,
            shift_y: 0.0,
        };
        let mut s = room(map, lo, hi, walls);
        let dark = frame(&mut s, o, None, &cam);
        total += differing(&dark, &frame(&mut s, o, Some(d), &cam));
    }
    total
}

/// Closed room with walls: before, the sun showed as a line along the top of the walls (the joint
/// with the ceiling, the Wizardry 7 report), along the wall feet and in the corners (100-280 pixels
/// per sun direction). Only single pixels in the three-way corners are left.
#[test]
fn closed_room_keeps_sun_out() {
    for o in [lit(), wiz()] {
        for d in SUNS {
            let n = sun_leak(9, 2, 6, true, o, d);
            assert!(
                n <= 12,
                "sun {d:?}, res {}: {n} pixels leak into the closed room",
                o.shadow_res
            );
        }
    }
}

/// Room without walls between it and the solid cells (wall texture -1): before, thousands of pixels.
#[test]
fn solid_cells_block_without_walls() {
    for o in [lit(), wiz()] {
        for d in SUNS {
            let n = sun_leak(9, 2, 6, false, o, d);
            assert!(n <= 2, "sun {d:?}: {n} pixels leak past solid cells");
        }
    }
}

/// Roofed room filling the whole map, no walls at the border: before, thousands of pixels.
#[test]
fn roofed_room_closed_at_map_border() {
    for o in [lit(), wiz()] {
        for d in SUNS {
            let n = sun_leak(5, 0, 4, false, o, d);
            assert!(n <= 16, "sun {d:?}: {n} pixels leak in at the map border");
        }
    }
}

/// The view of the Wizardry 7 report: wall top under the ceiling, steep sun from the room side.
#[test]
fn wall_ceiling_joint_golden() {
    let cam = Camera {
        position: Vec3::new(4.5, 0.5, 4.5),
        yaw: 3.14,
        pitch: 0.25,
        fov_y: 70.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    let mut s = room(9, 2, 6, true);
    let dark = frame(&mut s, lit(), None, &cam);
    let img = frame(&mut s, lit(), Some([0.0, -1.0, 0.3]), &cam);
    assert_eq!(differing(&dark, &img), 0, "no sun line along the wall top");
    check("gap_wall_ceiling_joint", &img);
}

/// The half-open door of the test scene leads into a solid cell: no sun comes through its gap.
#[test]
fn open_door_into_rock_stays_dark() {
    let mut s = common::scene(W, H);
    s.renderer.set_sky(-1, 0.0);
    let cam = Camera {
        position: Vec3::new(3.5, 0.5, 2.5),
        yaw: 0.0,
        pitch: -0.2,
        ..common::camera()
    };
    assert!(s.grid.cell(3, 1).unwrap().walls[grid::NORTH].open > 0.0);
    let dark = frame(&mut s, lit(), None, &cam);
    let img = frame(&mut s, lit(), Some([0.0, -1.0, 0.3]), &cam);
    assert_eq!(differing(&dark, &img), 0, "sun through the door gap");
}

/// Open yard with a solid block of height `tall` (floor_height) at x 3..6, z 3: shadow pixels on
/// the yard floor, image.
fn block_yard(tall: f32) -> (usize, Vec<u8>) {
    let mut s = common::scene(W, H);
    let mut g = GridMap::new(9, 9, 1.0);
    for c in g.cells.iter_mut() {
        c.floor_texture = 0;
        c.ceiling_height = 1.0;
    }
    for x in 3..6 {
        let c = g.cell_mut(x, 3).unwrap();
        c.ceiling_height = 0.0;
        c.floor_height = tall;
    }
    s.grid = g;
    let cam = Camera {
        position: Vec3::new(4.5, 1.6, 8.5),
        yaw: 0.0,
        pitch: -0.35,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    let dark = frame(&mut s, lit(), None, &cam);
    let img = frame(&mut s, lit(), Some([0.2, -0.6, 1.0]), &cam);
    let shadowed = dark
        .as_chunks::<4>()
        .0
        .iter()
        .zip(img.as_chunks::<4>().0)
        .filter(|(a, b)| a[..3] == b[..3])
        .count();
    (shadowed, img)
}

/// A solid cell taller than its neighbours' ceiling_height (floor_height 3) throws a longer shadow
/// than one as tall as the neighbours (floor 0: the neighbours' ceiling 1 is its height).
#[test]
fn taller_solid_cell_longer_shadow() {
    let (low, _) = block_yard(0.0);
    let (high, img) = block_yard(3.0);
    assert!(
        low > 100 && high > low + 200,
        "shadow pixels: height 1 {low}, height 3 {high}"
    );
    check("gap_tall_block_shadow", &img);
}

// ---- 4. Scrolling textures as shadow casters ----

/// Open 9 x 9 yard with a two-high wall at z = 4, x 3..6 (both faces textured `wall`).
fn yard(wall: i32) -> common::Scene {
    let mut s = common::scene(W, H);
    let mut g = GridMap::new(9, 9, 1.0);
    for c in g.cells.iter_mut() {
        c.floor_texture = 0;
        c.ceiling_height = 2.0;
    }
    for x in 3..6 {
        g.cell_mut(x, 4).unwrap().walls[grid::NORTH] = Wall {
            texture: wall,
            alpha: 1.0,
            ..Wall::default()
        };
        g.cell_mut(x, 3).unwrap().walls[grid::SOUTH] = Wall {
            texture: wall,
            alpha: 1.0,
            ..Wall::default()
        };
    }
    s.grid = g;
    s
}

fn yard_cam() -> Camera {
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

fn torch(x: f32, y: f32, z: f32) -> PointLight {
    PointLight {
        pos: Vec3::new(x, y, z),
        color: 0xffc080,
        strength: 1.5,
        radius: 4.0,
        flicker: 0.0,
        seed: 7,
        shadows: true,
    }
}

/// Shadow maps rebuilt per frame while a scrolling wall texture moves (sun and one torch).
fn scroll_rebuilds(holes: bool) -> Vec<u32> {
    let mut s = yard(2);
    let tex = if holes {
        // bricks with every second column transparent
        let mut rgba = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                rgba.extend_from_slice(&[150, 80, 60, if (x + y) % 2 == 0 { 255 } else { 0 }]);
            }
        }
        s.renderer
            .add_texture(Texture::from_rgba(8, 8, &rgba).unwrap())
    } else {
        s.renderer.add_texture(common::bricks())
    };
    for x in 3..6 {
        s.grid.cell_mut(x, 4).unwrap().walls[grid::NORTH].texture = tex;
        s.grid.cell_mut(x, 3).unwrap().walls[grid::SOUTH].texture = tex;
    }
    assert!(s.renderer.set_texture_scroll(tex, 0.25, 0.0));
    s.renderer.options = lit();
    s.renderer.set_sun(Some(sun([0.5, -0.45, 1.0])));
    s.renderer.create_light(torch(4.5, 1.0, 3.5));
    (0..4)
        .map(|f| {
            s.renderer.set_time(f as f64 * 0.1);
            s.renderer.begin(&yard_cam());
            s.renderer.draw_grid(&s.grid);
            s.renderer.end();
            s.renderer.stats().shadow_maps_rebuilt
        })
        .collect()
}

/// An opaque scrolling texture cannot change the shadow: no rebuilds after the first frame.
/// With alpha holes the scroll moves the holes, so the maps follow it every frame.
#[test]
fn scrolling_opaque_caster_keeps_shadow_cache() {
    assert_eq!(scroll_rebuilds(false), vec![2, 0, 0, 0]);
    assert_eq!(scroll_rebuilds(true), vec![2, 2, 2, 2]);
}

// ---- 3. Stippled shadows of see-through things ----

/// Renders the yard (glass wall at z = 4 with `wall_alpha`, flat white) with `extra` drawn after
/// the grid; returns the image and per pixel whether it shows the floor in front of the wall
/// (z > 4.65, so not seen through the glass or the see-through sprites).
fn stipple_yard(
    wall_alpha: f32,
    stipple: bool,
    extra: &dyn Fn(&mut Renderer),
) -> (Vec<u8>, Vec<bool>) {
    let mut s = yard(2);
    let white = s
        .renderer
        .add_texture(Texture::from_rgba(1, 1, &[230, 230, 230, 255]).unwrap());
    for x in 3..6 {
        s.grid.cell_mut(x, 4).unwrap().walls[grid::NORTH] = Wall {
            texture: white,
            alpha: wall_alpha,
            ..Wall::default()
        };
        s.grid.cell_mut(x, 3).unwrap().walls[grid::SOUTH] = Wall {
            texture: white,
            alpha: wall_alpha,
            ..Wall::default()
        };
    }
    s.renderer.options = Options {
        shadow_stipple: stipple,
        ..lit()
    };
    s.renderer.set_sun(Some(sun([0.5, -0.45, 1.0])));
    s.renderer.set_ambient(0x8090b0, 0.3);
    let cam = yard_cam();
    s.renderer.begin(&cam);
    s.renderer.draw_grid(&s.grid);
    extra(&mut s.renderer);
    s.renderer.end();
    let view = camera::View::new(&cam, W, H, 0.0);
    let floor = s
        .renderer
        .depth()
        .iter()
        .enumerate()
        .map(|(i, z)| {
            z.is_finite() && {
                let p = renderer::world_pos(&view, i % W, i / W, *z);
                p.y.abs() < 0.01 && p.z > 4.65
            }
        })
        .collect();
    (s.renderer.rgba().to_vec(), floor)
}

/// Share of the floor pixels shadowed by the opaque version (`full`) that `img` darkens too
/// (compared with `none`, the same scene without that shadow).
fn shadow_share(img: &[u8], none: &[u8], full: &[u8], floor: &[bool]) -> f32 {
    let lum = |v: &[u8], i: usize| v[i * 4] as u32 + v[i * 4 + 1] as u32 + v[i * 4 + 2] as u32;
    let region: Vec<usize> = (0..W * H)
        .filter(|&i| floor[i] && lum(full, i) < lum(none, i))
        .collect();
    assert!(
        region.len() > 200,
        "opaque shadow on the floor ({} pixels)",
        region.len()
    );
    region
        .iter()
        .filter(|&&i| lum(img, i) < lum(none, i))
        .count() as f32
        / region.len() as f32
}

fn nothing(_: &mut Renderer) {}

/// Grid glass (wall alpha 0.5) throws a checkered shadow, about half of the opaque one; with
/// `shadow_stipple` off it throws none (as before).
#[test]
fn glass_wall_stippled_shadow() {
    let (none, floor) = stipple_yard(0.0, true, &nothing);
    let (full, _) = stipple_yard(1.0, true, &nothing);
    let (half, _) = stipple_yard(0.5, true, &nothing);
    let (quarter, _) = stipple_yard(0.25, true, &nothing);
    let (off, _) = stipple_yard(0.5, false, &nothing);
    let share = |img: &[u8]| shadow_share(img, &none, &full, &floor);
    assert!(
        (share(&half) - 0.5).abs() < 0.12,
        "alpha 0.5: {}",
        share(&half)
    );
    assert!(
        (share(&quarter) - 0.25).abs() < 0.12,
        "alpha 0.25: {}",
        share(&quarter)
    );
    assert_eq!(share(&off), 0.0, "switched off: no shadow");
    check("stipple_glass_wall", &half);
}

fn ghost(flags: u32, alpha: f32, blend: i32) -> impl Fn(&mut Renderer) {
    move |r: &mut Renderer| {
        let t = r.add_texture(Texture::from_rgba(1, 1, &[200, 220, 255, 255]).unwrap());
        r.draw_sprite(&Sprite {
            position: Vec3::new(4.5, 0.0, 4.6),
            width: 1.2,
            height: 1.6,
            texture: t,
            flags,
            yaw: 0.0,
            light: 1.0,
            tag: 0,
            alpha,
            blend,
        });
    }
}

/// A BLEND_ALPHA billboard throws a stippled shadow; BLEND_ADD and SPRITE_NO_SHADOW throw none.
#[test]
fn see_through_sprite_shadows() {
    const BB: u32 = renderer::SPRITE_BILLBOARD;
    let (none, floor) = stipple_yard(0.0, true, &nothing);
    let (full, _) = stipple_yard(0.0, true, &ghost(BB, 1.0, BLEND_NONE));
    let (half, _) = stipple_yard(0.0, true, &ghost(BB, 0.5, BLEND_ALPHA));
    let (add, _) = stipple_yard(0.0, true, &ghost(BB, 0.5, BLEND_ADD));
    let (hidden, _) = stipple_yard(
        0.0,
        true,
        &ghost(BB | renderer::SPRITE_NO_SHADOW, 0.5, BLEND_ALPHA),
    );
    let share = |img: &[u8]| shadow_share(img, &none, &full, &floor);
    assert!(
        (share(&half) - 0.5).abs() < 0.15,
        "ghost alpha 0.5: {}",
        share(&half)
    );
    assert_eq!(share(&add), 0.0, "additive casts no shadow");
    assert_eq!(share(&hidden), 0.0, "SPRITE_NO_SHADOW");
    check("stipple_ghost_sprite", &half);
}

fn pane(alpha: f32, blend: i32, cast: bool) -> impl Fn(&mut Renderer) {
    // an upright square at z = 4.6, x 3.9..5.1, 0..1.4 high
    move |r: &mut Renderer| {
        let t = r.add_texture(Texture::from_rgba(1, 1, &[200, 220, 255, 255]).unwrap());
        let v = |x: f32, y: f32| Vertex {
            x,
            y,
            z: 4.6,
            u: 0.0,
            v: 0.0,
        };
        let tris = [
            v(3.9, 1.4),
            v(5.1, 1.4),
            v(5.1, 0.0),
            v(3.9, 1.4),
            v(5.1, 0.0),
            v(3.9, 0.0),
        ];
        r.draw_triangles_ex(&tris, t, 1.0, false, cast, 0, alpha, blend);
    }
}

/// BLEND_ALPHA triangle lists cast a stippled shadow unless `cast` is false.
#[test]
fn see_through_mesh_shadow() {
    let (none, floor) = stipple_yard(0.0, true, &nothing);
    let (full, _) = stipple_yard(0.0, true, &pane(1.0, BLEND_NONE, true));
    let (half, _) = stipple_yard(0.0, true, &pane(0.5, BLEND_ALPHA, true));
    let (nocast, _) = stipple_yard(0.0, true, &pane(0.5, BLEND_ALPHA, false));
    let share = |img: &[u8]| shadow_share(img, &none, &full, &floor);
    assert!(
        (share(&half) - 0.5).abs() < 0.12,
        "mesh alpha 0.5: {}",
        share(&half)
    );
    assert_eq!(share(&nocast), 0.0, "cast = false");
}

// ---- 2. Torch light on particles and billboards ----

fn night_cam() -> Camera {
    Camera {
        position: Vec3::new(4.5, 0.8, 8.0),
        yaw: 0.0,
        pitch: -0.1,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    }
}

/// Night yard (ambient 0.15, no sun, no wall), optional torch at (5.3, 0.8, 5.5), with `extra`.
fn night(torch_on: bool, o: Options, extra: &dyn Fn(&mut Renderer)) -> Vec<u8> {
    let mut s = yard(-1);
    s.renderer.options = o;
    s.renderer.set_sun(None);
    s.renderer.set_ambient(0x8090b0, 0.15);
    if torch_on {
        s.renderer.create_light(torch(5.3, 0.8, 5.5));
    }
    s.renderer.begin(&night_cam());
    s.renderer.draw_grid(&s.grid);
    extra(&mut s.renderer);
    s.renderer.end();
    s.renderer.rgba().to_vec()
}

fn smoke(alpha: f32) -> impl Fn(&mut Renderer) {
    move |r: &mut Renderer| {
        let ps: Vec<particles::Particle> = (0..5)
            .map(|k| particles::Particle {
                position: Vec3::new(4.3 + k as f32 * 0.1, 0.9, 5.5),
                size: 0.15,
                color: 0xa0a0a0,
                texture: -1,
                flags: 0,
                alpha,
            })
            .collect();
        r.draw_particles(&ps);
    }
}

/// Pixel at the screen point of world `p` (camera of `night`).
fn px_at(img: &[u8], p: Vec3) -> [u8; 3] {
    let v = camera::View::new(&night_cam(), W, H, 0.0);
    let q = v.to_view(p);
    let x = (v.cx + q.x * v.focal / q.z) as usize;
    let y = (v.cy - q.y * v.focal / q.z) as usize;
    let i = (y * W + x) * 4;
    [img[i], img[i + 1], img[i + 2]]
}

fn lum_at(img: &[u8], p: Vec3) -> u32 {
    px_at(img, p).iter().map(|&c| c as u32).sum()
}

/// Non-emissive particles next to a torch are lit by it (once per particle, no shadow), opaque and
/// blended alike; before they only got ambient and sun.
#[test]
fn torch_lights_particles() {
    let p = Vec3::new(4.7, 0.9, 5.5);
    for alpha in [1.0, 0.5] {
        let dark = night(false, lit(), &smoke(alpha));
        let img = night(true, lit(), &smoke(alpha));
        assert!(
            lum_at(&img, p) > lum_at(&dark, p) + 60,
            "alpha {alpha}: {} vs {}",
            lum_at(&img, p),
            lum_at(&dark, p)
        );
        for t in [1, 3] {
            assert!(
                night(
                    true,
                    Options {
                        threads: t,
                        ..lit()
                    },
                    &smoke(alpha)
                ) == img,
                "threads {t}"
            );
        }
    }
    // light_levels: the particle light is stepped like surface light (strongest channel to 0, 1/2
    // or 1 of the particle colour 160)
    let stepped = night(
        true,
        Options {
            light_levels: 2,
            ..lit()
        },
        &smoke(1.0),
    );
    let px = px_at(&stepped, p);
    let m = px.iter().copied().max().unwrap();
    assert!(m == 80 || m == 160, "stepped particle light {px:?}");
    check("gap_torch_smoke", &night(true, lit(), &smoke(1.0)));
}

fn figure_at(flags: u32, yaw: f32) -> impl Fn(&mut Renderer) {
    move |r: &mut Renderer| {
        let t = r.add_texture(Texture::from_rgba(1, 1, &[200, 200, 200, 255]).unwrap());
        r.draw_sprite(&Sprite {
            position: Vec3::new(4.5, 0.0, 5.5),
            width: 0.6,
            height: 1.4,
            texture: t,
            flags,
            yaw,
            light: 1.0,
            tag: 0,
            alpha: 1.0,
            blend: BLEND_NONE,
        });
    }
}

/// A billboard beside a torch (its face turned to the camera, edge-on to the torch) is lit like an
/// upright sprite facing the torch: point lights treat billboards as facing the light. Before it
/// got next to nothing (n.l about 0) and its own shadow caster darkened half of it.
#[test]
fn torch_lights_billboard_like_a_wall() {
    let bb = figure_at(renderer::SPRITE_BILLBOARD, 0.0);
    // a fixed sprite turned to face +x (the torch)
    let facing = figure_at(0, -std::f32::consts::FRAC_PI_2);
    let p = Vec3::new(4.5, 0.8, 5.5);
    let dark = night(false, lit(), &bb);
    let lit_bb = night(true, lit(), &bb);
    let lit_facing = night(true, lit(), &facing);
    let gain = |img: &[u8]| lum_at(img, p) as i32 - lum_at(&dark, p) as i32;
    assert!(
        gain(&lit_facing) > 100,
        "the torch lights the sprite facing it ({})",
        gain(&lit_facing)
    );
    assert!(
        gain(&lit_bb) * 10 >= gain(&lit_facing) * 9,
        "billboard {} vs facing sprite {}",
        gain(&lit_bb),
        gain(&lit_facing)
    );
    // both halves alike: no self shadow
    let (l, r) = (Vec3::new(4.35, 0.8, 5.5), Vec3::new(4.65, 0.8, 5.5));
    assert!(
        (lum_at(&lit_bb, l) as i32 - lum_at(&lit_bb, r) as i32).abs() < 40,
        "{} vs {}",
        lum_at(&lit_bb, l),
        lum_at(&lit_bb, r)
    );
    check("gap_torch_billboard", &lit_bb);
}

/// A wall between the torch and the billboard still shadows it.
#[test]
fn billboard_behind_wall_stays_dark() {
    let with_wall = |r: &mut Renderer| {
        let t = r.add_texture(Texture::from_rgba(1, 1, &[90, 90, 90, 255]).unwrap());
        // upright square at x = 4.95 between the figure (x 4.5) and the torch (x 5.3)
        let v = |y: f32, z: f32| Vertex {
            x: 4.95,
            y,
            z,
            u: 0.0,
            v: 0.0,
        };
        r.draw_triangles(
            &[
                v(1.6, 5.0),
                v(1.6, 6.0),
                v(0.0, 6.0),
                v(1.6, 5.0),
                v(0.0, 6.0),
                v(0.0, 5.0),
            ],
            t,
            1.0,
            false,
        );
        figure_at(renderer::SPRITE_BILLBOARD, 0.0)(r);
    };
    let dark = night(false, lit(), &with_wall);
    let img = night(true, lit(), &with_wall);
    let p = Vec3::new(4.4, 0.8, 5.5);
    assert_eq!(
        lum_at(&img, p),
        lum_at(&dark, p),
        "shadowed billboard pixel"
    );
}
