//! Test scene shared by the image tests and the demo: procedural textures, a small room with a door,
//! a raised platform, an open courtyard with sky, sprites and a mesh.
#![allow(dead_code)]

use knorke_integration::pixel::prelude::*;

pub(crate) fn checker(w: usize, h: usize, a: [u8; 3], b: [u8; 3], cells: usize) -> Texture {
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let c = if ((x * cells / w) + (y * cells / h)).is_multiple_of(2) {
                a
            } else {
                b
            };
            // A frame line so texture borders are visible in the image.
            let c = if x == 0 || y == 0 {
                [c[0] / 2, c[1] / 2, c[2] / 2]
            } else {
                c
            };
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    Texture::from_rgba(w, h, &rgba).unwrap()
}

/// Brick wall as a palette image: shows that indexed textures work.
pub(crate) fn bricks() -> Texture {
    let pal = [90u8, 40, 30, 160, 70, 50, 200, 190, 170];
    let (w, h) = (16, 16);
    let mut px = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let row = y / 4;
            let off = if row % 2 == 0 { 0 } else { 4 };
            let mortar = y % 4 == 3 || (x + off) % 8 == 7;
            px[y * w + x] = if mortar {
                2
            } else if (x + y) % 5 == 0 {
                0
            } else {
                1
            };
        }
    }
    Texture::from_indexed(w, h, &px, &pal, None).unwrap()
}

/// A little figure with transparent background (palette index 0 = transparent).
pub(crate) fn figure(frame: usize) -> Texture {
    let pal = [0u8, 0, 0, 230, 200, 60, 40, 60, 160, 240, 230, 220];
    let (w, h) = (8, 16);
    let mut px = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let v = if y < 4 {
                (2..6).contains(&x) as u8 * 3 // head
            } else if y < 10 {
                (1..7).contains(&x) as u8 // body
            } else {
                // legs, they swap between frames
                let leg = if frame.is_multiple_of(2) {
                    x == 2 || x == 5
                } else {
                    x == 1 || x == 6
                };
                leg as u8 * 2
            };
            px[y * w + x] = v;
        }
    }
    Texture::from_indexed(w, h, &px, &pal, Some(0)).unwrap()
}

pub(crate) fn sky() -> Texture {
    let (w, h) = (32, 16);
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let star = (x * 7 + y * 13) % 29 == 0;
            let c = if star {
                [250, 250, 200]
            } else {
                [20 + y as u8 * 6, 30 + y as u8 * 5, 90 + y as u8 * 4]
            };
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    Texture::from_rgba(w, h, &rgba).unwrap()
}

/// A 16-colour test palette (VGA-like ramps).
pub(crate) fn palette16() -> Vec<u8> {
    vec![
        0, 0, 0, 40, 30, 30, 90, 40, 30, 160, 70, 50, 200, 190, 170, 255, 255, 255, 60, 60, 80,
        110, 110, 130, 170, 170, 190, 30, 50, 120, 60, 100, 200, 40, 90, 40, 90, 160, 70, 230, 200,
        60, 120, 80, 40, 250, 250, 200,
    ]
}

pub(crate) struct Scene {
    pub renderer: Renderer,
    pub grid: GridMap,
    pub figure: [i32; 2],
    pub stone: i32,
}

pub(crate) fn scene(width: usize, height: usize) -> Scene {
    let mut r = Renderer::new(width, height).unwrap();
    let floor = r.add_texture(checker(16, 16, [110, 110, 130], [60, 60, 80], 2));
    let ceil = r.add_texture(checker(16, 16, [90, 60, 40], [60, 40, 30], 4));
    let wall = r.add_texture(bricks());
    let door = r.add_texture(checker(16, 32, [140, 90, 40], [100, 60, 30], 1));
    let stone = r.add_texture(checker(8, 8, [170, 170, 190], [120, 120, 140], 2));
    let sky = r.add_texture(sky());
    let fig = [r.add_texture(figure(0)), r.add_texture(figure(1))];
    r.set_sky(sky, 4.0);

    // 7 x 7 map, edge cells solid. Rows 1-3 indoor room, rows 4-5 open yard.
    let mut g = GridMap::new(7, 7, 1.0);
    for z in 0..7 {
        for x in 0..7 {
            let c = g.cell_mut(x, z).unwrap();
            let inside = (1..=5).contains(&x) && (1..=5).contains(&z);
            if !inside {
                c.ceiling_height = 0.0;
                continue;
            }
            c.floor_texture = floor;
            c.ceiling_texture = if z <= 3 { ceil } else { -1 };
            c.ceiling_height = if z <= 3 { 1.0 } else { 2.0 };
            c.light = if z <= 3 { 0.8 } else { 1.0 };
            c.side_texture = if z <= 3 { wall } else { -1 };
        }
    }
    // Outer walls facing inward.
    for i in 1..=5 {
        g.cell_mut(i, 1).unwrap().walls[grid::NORTH] = Wall {
            texture: wall,
            open: 0.0,
            alpha: 1.0,
            ..Wall::default()
        };
        g.cell_mut(i, 5).unwrap().walls[grid::SOUTH] = Wall {
            texture: wall,
            open: 0.0,
            alpha: 1.0,
            ..Wall::default()
        };
        g.cell_mut(1, i).unwrap().walls[grid::WEST] = Wall {
            texture: wall,
            open: 0.0,
            alpha: 1.0,
            ..Wall::default()
        };
        g.cell_mut(5, i).unwrap().walls[grid::EAST] = Wall {
            texture: wall,
            open: 0.0,
            alpha: 1.0,
            ..Wall::default()
        };
    }
    // A half-open door in the north wall, a raised platform in the north-east corner.
    g.cell_mut(3, 1).unwrap().walls[grid::NORTH] = Wall {
        texture: door,
        open: 0.4,
        alpha: 1.0,
        ..Wall::default()
    };
    let p = g.cell_mut(5, 1).unwrap();
    p.floor_height = 0.25;
    p.side_texture = stone;

    Scene {
        renderer: r,
        grid: g,
        figure: fig,
        stone,
    }
}

pub(crate) fn camera() -> Camera {
    Camera {
        position: Vec3::new(3.5, 0.5, 5.3),
        yaw: 0.0,
        pitch: 0.0,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    }
}

/// Render the whole scene once.
pub(crate) fn render(s: &mut Scene, cam: &Camera, time: f64) {
    let r = &mut s.renderer;
    r.begin(cam);
    r.draw_grid(&s.grid);
    let f = anim_frame(time, 4.0, 2) as usize;
    r.draw_sprite(&Sprite {
        position: Vec3::new(2.5, 0.0, 3.2),
        width: 0.4,
        height: 0.8,
        texture: s.figure[f],
        flags: renderer::SPRITE_BILLBOARD,
        yaw: 0.0,
        light: 0.8,
        tag: 0,
        alpha: 1.0,
        blend: BLEND_NONE,
    });
    // A sprite partly behind the platform step: tests occlusion.
    r.draw_sprite(&Sprite {
        position: Vec3::new(4.6, 0.0, 1.6),
        width: 0.4,
        height: 0.8,
        texture: s.figure[1 - f],
        flags: renderer::SPRITE_BILLBOARD | renderer::SPRITE_FLIP_X,
        yaw: 0.0,
        light: 0.8,
        tag: 0,
        alpha: 1.0,
        blend: BLEND_NONE,
    });
    // A pyramid mesh in the yard.
    let (cx, cz, b, hgt) = (4.2f32, 4.3f32, 0.3f32, 0.6f32);
    let apex = Vertex {
        x: cx,
        y: hgt,
        z: cz,
        u: 0.5,
        v: 0.0,
    };
    let base = [(-b, -b), (b, -b), (b, b), (-b, b)];
    let mut tris = Vec::new();
    for k in 0..4 {
        let (ax, az) = base[k];
        let (bx, bz) = base[(k + 1) % 4];
        // Order apex, second, first: clockwise on screen when seen from outside.
        tris.push(apex);
        tris.push(Vertex {
            x: cx + bx,
            y: 0.0,
            z: cz + bz,
            u: 1.0,
            v: 1.0,
        });
        tris.push(Vertex {
            x: cx + ax,
            y: 0.0,
            z: cz + az,
            u: 0.0,
            v: 1.0,
        });
    }
    r.draw_triangles(&tris, s.stone, 1.0, true);
    r.end();
}

/// Exact joint mask of `joint_tiles` (255 on the joints), as a caller who knows its picture would give it.
pub(crate) fn joint_mask() -> Vec<u8> {
    (0..32 * 32)
        .map(|i| {
            if i % 32 % 8 == 7 || i / 32 % 8 == 7 {
                255
            } else {
                0
            }
        })
        .collect()
}

/// Stone floor tiles with 1-texel dark joints (every 8 texels in both directions), slight noise and
/// a few pits darker than the joints (they beat the joints under the old dark-texel rule).
pub(crate) fn joint_tiles() -> Texture {
    let (w, h) = (32, 32);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let c = if x % 8 == 7 || y % 8 == 7 {
                [78u8, 66, 58]
            } else if (x * 13 + y * 7) % 11 == 0 {
                [48, 42, 40]
            } else {
                let tile = (x / 8 * 5 + y / 8 * 3) % 4;
                let noise = ((x * 37 + y * 91 + x * y) % 7) as u8;
                let b = 140 + tile as u8 * 9 + noise * 3;
                [b, b - 8, b - 20]
            };
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    Texture::from_rgba(w, h, &rgba).unwrap()
}

/// Joint floor scene: a long open floor of `joint_tiles` (9 x 40 cells), seen flat from low above.
/// Returns the renderer, the grid, the floor texture id and the camera.
pub(crate) fn joint_floor(width: usize, height: usize) -> (Renderer, GridMap, i32, Camera) {
    let mut r = Renderer::new(width, height).unwrap();
    let t = r.add_texture(joint_tiles());
    let mut g = GridMap::new(9, 40, 1.0);
    for z in 0..40 {
        for x in 0..9 {
            let c = g.cell_mut(x, z).unwrap();
            c.floor_texture = t;
            c.ceiling_texture = -1;
            c.ceiling_height = 2.0;
        }
    }
    r.options = Options {
        texture_lod: true,
        background: 0x303848,
        ..Options::default()
    };
    let cam = Camera {
        position: Vec3::new(4.5, 0.45, 39.6),
        yaw: 0.0,
        pitch: -0.1,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    };
    (r, g, t, cam)
}

// ---- Storeys (several spans per column) ----

/// Texture ids of `scene` used by the storey maps.
pub(crate) const T_FLOOR: i32 = 0;
pub(crate) const T_CEIL: i32 = 1;
pub(crate) const T_WALL: i32 = 2;
pub(crate) const T_DOOR: i32 = 3;
pub(crate) const T_STONE: i32 = 4;

fn wall(texture: i32) -> Wall {
    Wall {
        texture,
        ..Wall::default()
    }
}

/// 5 x 5, two levels: a room x, z in 1..=3 (floor 0, ceiling 1, walls of bricks) and a room above it
/// (floor 1.25, ceiling 2.25, door-texture walls, stone ceiling). In the middle cell (2, 2) the lower
/// ceiling (raised to 1.25) and the upper floor are untextured: a hole without stairs; its shaft
/// between 1 and 1.25 shows the upper span's side texture (bricks). The ring around is solid.
pub(crate) fn hole_rooms() -> GridMap {
    let mut g = GridMap::with_levels(5, 5, 2, 1.0);
    for z in 0..5 {
        for x in 0..5 {
            let inside = (1..=3).contains(&x) && (1..=3).contains(&z);
            let lo = g.cell_mut(x, z).unwrap();
            if !inside {
                lo.ceiling_height = 0.0;
                continue;
            }
            *lo = Cell {
                floor_texture: T_FLOOR,
                ceiling_texture: T_CEIL,
                light: 0.8,
                ..Cell::default()
            };
            let hi = g.cell_at_mut(x, z, 1).unwrap();
            *hi = Cell {
                floor_height: 1.25,
                ceiling_height: 2.25,
                floor_texture: T_FLOOR,
                ceiling_texture: T_STONE,
                side_texture: T_WALL,
                ..Cell::default()
            };
            for (level, tex) in [(0, T_WALL), (1, T_DOOR)] {
                let c = g.cell_at_mut(x, z, level).unwrap();
                if z == 1 {
                    c.walls[grid::NORTH] = wall(tex);
                }
                if z == 3 {
                    c.walls[grid::SOUTH] = wall(tex);
                }
                if x == 1 {
                    c.walls[grid::WEST] = wall(tex);
                }
                if x == 3 {
                    c.walls[grid::EAST] = wall(tex);
                }
            }
        }
    }
    let lo = g.cell_mut(2, 2).unwrap();
    lo.ceiling_height = 1.25;
    lo.ceiling_texture = -1;
    g.cell_at_mut(2, 2, 1).unwrap().floor_texture = -1;
    g
}

/// 9 x 9, two levels: a street along x = 4 (floor 0, open sky up to 3, stone) between banks at
/// height 1.25 (floor tiles, bricks below). A bridge crosses at z = 4: the street cell there has a
/// ceiling at 1 (the bridge's underside) and a second span from 1.25 (the deck, floor tiles, door
/// texture on its edge) up to 3. A brick wall closes the street at its north end.
pub(crate) fn bridge() -> GridMap {
    let mut g = GridMap::with_levels(9, 9, 2, 1.0);
    for z in 0..9 {
        for x in 0..9 {
            let c = g.cell_mut(x, z).unwrap();
            *c = if x == 4 {
                Cell {
                    floor_texture: T_STONE,
                    ceiling_height: 3.0,
                    ..Cell::default()
                }
            } else {
                Cell {
                    floor_height: 1.25,
                    ceiling_height: 3.0,
                    floor_texture: T_FLOOR,
                    side_texture: T_WALL,
                    ..Cell::default()
                }
            };
        }
    }
    g.cell_mut(4, 0).unwrap().walls[grid::NORTH] = wall(T_WALL);
    let under = g.cell_mut(4, 4).unwrap();
    under.ceiling_height = 1.0;
    under.ceiling_texture = T_CEIL;
    under.light = 0.6;
    *g.cell_at_mut(4, 4, 1).unwrap() = Cell {
        floor_height: 1.25,
        ceiling_height: 3.0,
        floor_texture: T_FLOOR,
        side_texture: T_DOOR,
        ..Cell::default()
    };
    g
}

/// 9 x 9, two levels: open street (stone, sky up to 4) with a town house on x in 3..=5, z in 3..=4.
/// Level 0 of the house is the room (floor tiles, ceiling 1.5; brick walls facing in, with brick
/// back faces seen from the street; a door with a separate leaf in the south wall of (4, 4)). Level 1
/// is the roof span: floor at 1.5 at the eaves (north and south edges) rising to 2.5 at the ridge
/// (z = 4), tiles `roof`, bricks as side texture: the street cells west and east of the house show
/// the gable triangles as step faces between the room's ceiling and the roof.
pub(crate) fn gable_house(roof: i32) -> GridMap {
    let mut g = GridMap::with_levels(9, 9, 2, 1.0);
    for c in g.cells.iter_mut().take(81) {
        *c = Cell {
            floor_texture: T_STONE,
            ceiling_height: 4.0,
            ..Cell::default()
        };
    }
    for z in 3..=4 {
        for x in 3..=5 {
            let room = g.cell_mut(x, z).unwrap();
            *room = Cell {
                floor_texture: T_FLOOR,
                ceiling_height: 1.5,
                ceiling_texture: T_CEIL,
                light: 0.8,
                ..Cell::default()
            };
            let brick = Wall {
                texture: T_WALL,
                back_texture: T_WALL,
                ..Wall::default()
            };
            if z == 3 {
                room.walls[grid::NORTH] = brick;
            }
            if z == 4 {
                room.walls[grid::SOUTH] = brick;
            }
            if x == 3 {
                room.walls[grid::WEST] = brick;
            }
            if x == 5 {
                room.walls[grid::EAST] = brick;
            }
            // corners NW, NE, SE, SW: the ridge is the edge z = 4
            let up = if z == 3 {
                [0.0, 0.0, 1.0, 1.0]
            } else {
                [1.0, 1.0, 0.0, 0.0]
            };
            *g.cell_at_mut(x, z, 1).unwrap() = Cell {
                floor_height: 1.5,
                ceiling_height: 4.0,
                floor_texture: roof,
                side_texture: T_WALL,
                floor_corners: up,
                ..Cell::default()
            };
        }
    }
    let door = &mut g.cell_mut(4, 4).unwrap().walls[grid::SOUTH];
    door.leaf_texture = T_DOOR;
    door.leaf_u0 = 0.3;
    door.leaf_u1 = 0.7;
    door.leaf_top = 0.7;
    g
}

// ---- Golden images (shared by images.rs and light_gaps.rs) ----

/// Size of every golden image.
pub(crate) const W: usize = 160;
pub(crate) const H: usize = 100;

fn golden_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
}

/// Compares a `W` x `H` RGBA image with `tests/golden/{name}.ppm` (binary PPM, RGB only).
///
/// Side effects: always writes a PNG preview to `CARGO_TARGET_TMPDIR/golden-out/{name}.png`
/// (light_gaps.rs used to write `<crate>/target/golden-out` instead; one place for all now).
/// With `PIXELRAUM_BLESS=1` it (re)writes the reference instead of comparing and creates
/// `tests/golden` if missing (light_gaps.rs used to fail there).
pub(crate) fn check(name: &str, rgba: &[u8]) {
    let out = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("golden-out");
    std::fs::create_dir_all(&out).unwrap();
    let preview = out.join(format!("{name}.png"));
    std::fs::write(&preview, png::encode_rgba(W, H, rgba, 4)).unwrap();
    let path = golden_dir().join(format!("{name}.ppm"));
    let mut actual = format!("P6\n{W} {H}\n255\n").into_bytes();
    for p in rgba.as_chunks::<4>().0 {
        actual.extend_from_slice(&p[..3]);
    }
    if std::env::var("PIXELRAUM_BLESS").is_ok() {
        std::fs::create_dir_all(golden_dir()).unwrap();
        std::fs::write(&path, &actual).unwrap();
        return;
    }
    let expected = std::fs::read(&path)
        .unwrap_or_else(|_| panic!("missing reference {path:?}; run with PIXELRAUM_BLESS=1"));
    if expected != actual {
        let diff = expected.iter().zip(&actual).filter(|(a, b)| a != b).count();
        panic!("{name}: image differs from reference ({diff} bytes); see {preview:?}");
    }
}

/// Shadow-casting sun of full strength.
pub(crate) fn sun(dir: [f32; 3], color: u32) -> light::Sun {
    light::Sun {
        dir: Vec3::new(dir[0], dir[1], dir[2]),
        color,
        strength: 1.0,
        shadows: true,
    }
}
