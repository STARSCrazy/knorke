//! The caches kept between frames (shadow factors in `knorke-lighting`, backdrop ray angles in
//! `knorke-render`) must never change a pixel.
//!
//! Origin: pixelraum `src/renderer/shade.rs` test `frame_caches_never_change_a_pixel` @ 81927d8.

use knorke_integration::pixel::prelude::*;
use knorke_render::grid::{NORTH, SOUTH};

fn checker(a: [u8; 3], b: [u8; 3]) -> Texture {
    let mut px = Vec::new();
    for y in 0..8 {
        for x in 0..8 {
            let c = if (x / 4 + y / 4) % 2 == 0 { a } else { b };
            px.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    Texture::from_rgba(8, 8, &px).unwrap()
}

fn figure() -> Texture {
    let mut px = Vec::new();
    for y in 0..8 {
        for x in 0..4 {
            let on = (1..3).contains(&x) || y > 2;
            px.extend_from_slice(&[200, 180, 60, if on { 255 } else { 0 }]);
        }
    }
    Texture::from_rgba(4, 8, &px).unwrap()
}

fn torch(x: f32, z: f32, radius: f32) -> PointLight {
    PointLight {
        pos: Vec3::new(x, 1.0, z),
        color: 0xffc080,
        strength: 1.5,
        radius,
        flicker: 0.5,
        seed: 3,
        shadows: true,
    }
}

fn sun(x: f32) -> Option<Sun> {
    Some(Sun {
        dir: Vec3::new(x, -0.5, 1.0),
        color: 0xfff0d0,
        strength: 0.8,
        shadows: true,
    })
}

/// One scripted change per frame number (most frames change something).
fn step(
    f: usize,
    r: &mut Renderer,
    g: &mut GridMap,
    cam: &mut Camera,
    sprite: &mut bool,
    ids: &[u32],
    wall: i32,
) {
    match f {
        4..=6 => {
            r.set_light(ids[0], torch(1.5 + 0.2 * f as f32, 4.5, 4.0));
        }
        7 => {
            r.set_light(ids[1], torch(4.5, 1.5, 4.5));
        }
        8 => r.options.shadow_bias *= 2.0,
        9 => r.options.shadow_soft = 0.2,
        10 => r.options.point_shadow_soft = 0.2,
        13 => cam.position.x += 0.05,
        14 => cam.position.x -= 0.05,
        15 => {
            g.cell_mut(5, 4).unwrap().walls[NORTH] = Wall {
                texture: wall,
                open: 0.0,
                alpha: 1.0,
                ..Wall::default()
            }
        }
        16 => r.set_sun(sun(-0.3)),
        17 => {
            r.destroy_light(ids[0]);
        }
        18 => {
            r.create_light(torch(2.0, 5.0, 3.0));
        }
        19 => *sprite = true,
        23 => r.options.threads = 3,
        24 => r.options.shadow_soft = 0.0,
        25 => r.set_sun(None),
        27 => r.set_sun(sun(-0.3)),
        28 => cam.yaw += 0.01,
        30 => r.options.point_shadow_soft = 0.0,
        32 => {
            r.set_light(
                ids[2],
                PointLight {
                    shadows: false,
                    ..torch(5.0, 5.0, 3.5)
                },
            );
        }
        34 => *sprite = false,
        36 => r.options.lighting = false,
        37 => r.options.lighting = true,
        _ => {}
    }
}

/// A scripted run where something changes between most frames; every frame equals the same frame
/// shaded without the caches (a fresh sky cache for the second renderer every frame).
#[test]
fn frame_caches_never_change_a_pixel() {
    let mk = || {
        let mut r = Renderer::new(96, 64).unwrap();
        let floor = r.add_texture(checker([110, 110, 130], [60, 60, 80]));
        let wall = r.add_texture(checker([160, 70, 50], [90, 40, 30]));
        let fig = r.add_texture(figure());
        r.set_sky(wall, 3.0);
        r.options = Options {
            lighting: true,
            shadow_size: 8.0,
            threads: 1,
            ..Options::default()
        };
        r.set_sun(sun(0.5));
        r.set_ambient(0x8090b0, 0.3);
        let ids = [
            torch(1.5, 4.5, 4.0),
            torch(4.5, 1.5, 3.0),
            torch(5.0, 5.0, 3.5),
        ]
        .map(|l| r.create_light(l));
        (r, floor, wall, fig, ids)
    };
    let (mut a, floor, wall, fig, ids) = mk();
    let (mut b, ..) = mk();
    b.lights_mut().set_shadow_cache(false);
    let mut g = GridMap::new(7, 7, 1.0);
    for z in 0..7 {
        for x in 0..7 {
            let c = g.cell_mut(x, z).unwrap();
            c.floor_texture = floor;
            c.ceiling_texture = -1;
            c.ceiling_height = 2.0;
        }
    }
    for x in 2..5 {
        g.cell_mut(x, 3).unwrap().walls[NORTH] = Wall {
            texture: wall,
            open: 0.0,
            alpha: 1.0,
            ..Wall::default()
        };
        g.cell_mut(x, 2).unwrap().walls[SOUTH] = Wall {
            texture: wall,
            open: 0.0,
            alpha: 1.0,
            ..Wall::default()
        };
    }
    let cam0 = Camera {
        position: Vec3::new(3.5, 1.6, 6.5),
        yaw: 0.0,
        pitch: -0.35,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    let (mut ga, mut gb, mut ca, mut cb, mut sa, mut sb) = (g.clone(), g, cam0, cam0, false, false);
    let mut hits = 0;
    for f in 0..40 {
        for (r, g, cam, sprite) in [
            (&mut a, &mut ga, &mut ca, &mut sa),
            (&mut b, &mut gb, &mut cb, &mut sb),
        ] {
            step(f, r, g, cam, sprite, &ids, wall);
            r.set_time(f as f64 / 30.0);
            r.begin(cam);
            r.draw_grid(g);
            if *sprite {
                r.draw_sprite(&Sprite {
                    position: Vec3::new(3.0, 0.0, 4.5),
                    width: 0.5,
                    height: 1.0,
                    texture: fig,
                    flags: SPRITE_BILLBOARD,
                    yaw: 0.0,
                    light: 1.0,
                    tag: 0,
                    alpha: 1.0,
                    blend: BLEND_NONE,
                });
            }
            r.end();
        }
        b.forget_backdrop_cache();
        assert!(a.rgba() == b.rgba(), "frame {f}");
        assert!(a.backdrop_cache_used(), "the sky is on screen");
        hits += a.lights().shadow_cache_hits();
    }
    assert!(hits > 0);
}
