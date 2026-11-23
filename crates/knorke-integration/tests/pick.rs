//! Picking (what is under a pixel) and grid raycasts.

mod common;

use knorke_integration::pixel::prelude::*;
use knorke_render::grid::{self, GridHit};
use knorke_render::pick::{PICK_GRID, PICK_MESH, PICK_SPRITE, Pick};

const W: usize = 64;
const H: usize = 48;

fn white(r: &mut Renderer) -> i32 {
    r.add_texture(Texture::from_rgba(1, 1, &[200, 200, 200, 255]).unwrap())
}

/// 3 x 3 room, every cell open, walls around the outside, floor and ceiling textured.
fn room(tex: i32) -> GridMap {
    let mut g = GridMap::new(3, 3, 1.0);
    for z in 0..3 {
        for x in 0..3 {
            let c = g.cell_mut(x, z).unwrap();
            c.floor_texture = tex;
            c.ceiling_texture = tex;
            if z == 0 {
                c.walls[grid::NORTH].texture = tex;
            }
            if z == 2 {
                c.walls[grid::SOUTH].texture = tex;
            }
            if x == 0 {
                c.walls[grid::WEST].texture = tex;
            }
            if x == 2 {
                c.walls[grid::EAST].texture = tex;
            }
        }
    }
    g
}

/// Standing in the middle of cell (1, 2), looking north (-z).
fn cam() -> Camera {
    Camera {
        position: Vec3::new(1.5, 0.5, 2.5),
        yaw: 0.0,
        pitch: 0.0,
        fov_y: 60.0,
        near: 0.05,
        shift_x: 0.0,
        shift_y: 0.0,
    }
}

fn setup() -> (Renderer, GridMap, i32) {
    let mut r = Renderer::new(W, H).unwrap();
    let t = white(&mut r);
    r.options.pick = true;
    (r, room(t), t)
}

fn centre(r: &Renderer) -> Pick {
    r.pick(W as i32 / 2, H as i32 / 2)
        .expect("something under the centre")
}

#[test]
fn pick_wall_cell_and_side() {
    let (mut r, g, _) = setup();
    r.begin(&cam());
    r.draw_grid(&g);
    r.end();
    let p = centre(&r);
    assert_eq!(p.kind, PICK_GRID);
    assert_eq!(p.tag, 0);
    assert_eq!((p.cell_x, p.cell_z, p.side), (1, 0, grid::NORTH as i32));
    assert!((p.position.z - 0.0).abs() < 0.02, "{:?}", p.position);
    assert!((p.position.x - 1.5).abs() < 0.1);
    assert!((p.depth - 2.5).abs() < 0.02);
    assert_eq!(p.normal, Vec3::new(0.0, 0.0, 1.0));
    // Bottom row of pixels sees the floor of the camera cell or the one in front.
    let f = r.pick(W as i32 / 2, H as i32 - 1).unwrap();
    assert_eq!((f.kind, f.side), (PICK_GRID, grid::SIDE_FLOOR as i32));
    let c = r.pick(W as i32 / 2, 0).unwrap();
    assert_eq!(c.side, grid::SIDE_CEILING as i32);
}

#[test]
fn pick_sprite_tag() {
    let (mut r, g, t) = setup();
    r.begin(&cam());
    r.draw_grid(&g);
    r.draw_sprite(&Sprite {
        position: Vec3::new(1.5, 0.0, 1.2),
        width: 0.6,
        height: 1.0,
        texture: t,
        flags: renderer::SPRITE_BILLBOARD,
        yaw: 0.0,
        light: 1.0,
        tag: 77,
        alpha: 1.0,
        blend: knorke_render::BLEND_NONE,
    });
    r.draw_sprite(&Sprite {
        position: Vec3::new(0.5, 0.0, 1.2),
        width: 0.4,
        height: 1.0,
        texture: t,
        flags: 0,
        yaw: 0.0,
        light: 1.0,
        tag: 0,
        alpha: 1.0,
        blend: knorke_render::BLEND_NONE,
    });
    r.end();
    let p = centre(&r);
    assert_eq!((p.kind, p.tag), (PICK_SPRITE, 77));
    assert!((p.depth - 1.3).abs() < 0.02);
    // Tag 0 still reports the kind.
    let view_x =
        (W as f32 / 2.0 - 1.0 / 1.3 * (H as f32 / 2.0) / (30f32.to_radians().tan())) as i32;
    let q = r.pick(view_x, H as i32 / 2).unwrap();
    assert_eq!((q.kind, q.tag), (PICK_SPRITE, 0));
}

#[test]
fn pick_mesh_tag() {
    let (mut r, g, t) = setup();
    r.begin(&cam());
    r.draw_grid(&g);
    let v = |x, y| Vertex {
        x,
        y,
        z: 1.0,
        u: 0.0,
        v: 0.0,
    };
    // Clockwise on screen from the camera.
    let tri = [v(1.0, 0.2), v(1.5, 0.9), v(2.0, 0.2)];
    r.draw_triangles_ex(&tri, t, 1.0, false, true, 5, 1.0, knorke_render::BLEND_NONE);
    r.end();
    let p = r.pick(W as i32 / 2, H as i32 / 2 + 4).unwrap();
    assert_eq!((p.kind, p.tag), (PICK_MESH, 5));
    assert!((p.position.z - 1.0).abs() < 0.02);
}

#[test]
fn pick_out_of_range() {
    let (mut r, g, _) = setup();
    r.begin(&cam());
    r.draw_grid(&g);
    r.end();
    assert!(r.pick(-1, 0).is_none());
    assert!(r.pick(W as i32, 0).is_none());
    assert!(r.pick(0, H as i32).is_none());
    // Nothing drawn: no hit.
    r.begin(&cam());
    r.end();
    assert!(r.pick(1, 1).is_none());
    // Picking off: no hit, picture unchanged.
    let (mut a, g2, _) = setup();
    a.begin(&cam());
    a.draw_grid(&g2);
    a.end();
    let with = a.rgba().to_vec();
    a.options.pick = false;
    a.begin(&cam());
    a.draw_grid(&g2);
    a.end();
    assert_eq!(with, a.rgba());
    assert!(a.pick(W as i32 / 2, H as i32 / 2).is_none());
    // (the C interface checks stay in pixelraum: capi.rs is not migrated, ADR 0013)
}

fn ray(g: &GridMap, o: [f32; 3], d: [f32; 3], max: f32) -> Option<GridHit> {
    g.raycast(
        Vec3::new(o[0], o[1], o[2]),
        Vec3::new(d[0], d[1], d[2]),
        max,
    )
}

#[test]
fn raycast_hits_wall() {
    let g = room(0);
    let h = ray(&g, [1.5, 0.5, 2.5], [0.0, 0.0, -1.0], 10.0).unwrap();
    assert_eq!((h.cell_x, h.cell_z, h.side), (1, 0, grid::NORTH as i32));
    assert!((h.distance - 2.5).abs() < 1e-4);
    assert!((h.point.z).abs() < 1e-4);
    let e = ray(&g, [0.5, 0.5, 0.5], [1.0, 0.0, 0.0], 10.0).unwrap();
    assert_eq!((e.cell_x, e.side), (2, grid::EAST as i32));
    assert!((e.distance - 2.5).abs() < 1e-4);
    // (the C interface checks stay in pixelraum: capi.rs is not migrated, ADR 0013)
}

#[test]
fn raycast_through_open_door() {
    let mut g = room(0);
    // A door between (1,1) and (1,0), seen from (1,1): 60 % open.
    g.cell_mut(1, 1).unwrap().walls[grid::NORTH] = Wall {
        texture: 0,
        open: 0.6,
        alpha: 1.0,
        ..Wall::default()
    };
    // Low ray passes under the door and hits the outer north wall.
    let low = ray(&g, [1.5, 0.3, 2.5], [0.0, 0.0, -1.0], 10.0).unwrap();
    assert_eq!(
        (low.cell_x, low.cell_z, low.side),
        (1, 0, grid::NORTH as i32)
    );
    // High ray hits the door.
    let high = ray(&g, [1.5, 0.8, 2.5], [0.0, 0.0, -1.0], 10.0).unwrap();
    assert_eq!(
        (high.cell_x, high.cell_z, high.side),
        (1, 1, grid::NORTH as i32)
    );
    assert!((high.distance - 1.5).abs() < 1e-4);
}

#[test]
fn raycast_hits_floor_looking_down() {
    let mut g = room(0);
    g.cell_mut(1, 1).unwrap().floor_height = 0.25;
    let h = ray(&g, [1.5, 0.5, 2.5], [0.0, -1.0, 0.0], 10.0).unwrap();
    assert_eq!(
        (h.cell_x, h.cell_z, h.side),
        (1, 2, grid::SIDE_FLOOR as i32)
    );
    assert!((h.distance - 0.5).abs() < 1e-4);
    let up = ray(&g, [1.5, 0.5, 2.5], [0.0, 1.0, 0.0], 10.0).unwrap();
    assert_eq!(up.side, grid::SIDE_CEILING as i32);
    // Low ray towards the raised cell hits its step face.
    let s = ray(&g, [1.5, 0.1, 2.5], [0.0, 0.0, -1.0], 10.0).unwrap();
    assert_eq!((s.cell_x, s.cell_z, s.side), (1, 2, grid::SIDE_STEP as i32));
    assert!((s.distance - 0.5).abs() < 1e-4);
    // Slanted ray lands on the raised floor.
    let d = Vec3::new(0.0, -0.25, -1.0).normalized();
    let f = ray(&g, [1.5, 0.5, 2.5], [d.x, d.y, d.z], 10.0).unwrap();
    assert_eq!(
        (f.cell_x, f.cell_z, f.side),
        (1, 1, grid::SIDE_FLOOR as i32)
    );
    assert!((f.point.y - 0.25).abs() < 1e-4);
}

#[test]
fn raycast_max_dist_miss() {
    let g = room(0);
    assert!(ray(&g, [1.5, 0.5, 2.5], [0.0, 0.0, -1.0], 2.0).is_none());
    assert!(ray(&g, [1.5, 0.5, 2.5], [0.0, 0.0, 0.0], 10.0).is_none());
    // Leaving the map through a side without wall: miss.
    let mut open = room(0);
    open.cell_mut(1, 0).unwrap().walls[grid::NORTH].texture = -1;
    assert!(ray(&open, [1.5, 0.5, 2.5], [0.0, 0.0, -1.0], 10.0).is_none());
}

#[test]
fn raycast_hits_wall_from_behind() {
    let mut g = room(0);
    // Wall only on (1,0)'s south side, faces into (1,0); the ray comes from (1,2), behind it.
    g.cell_mut(1, 0).unwrap().walls[grid::SOUTH].texture = 0;
    let h = ray(&g, [1.5, 0.5, 2.5], [0.0, 0.0, -1.0], 10.0).unwrap();
    assert_eq!((h.cell_x, h.cell_z, h.side), (1, 0, grid::SOUTH as i32));
    assert!((h.distance - 1.5).abs() < 1e-4);
    // An open door on that wall lets a low ray through to the outer wall.
    g.cell_mut(1, 0).unwrap().walls[grid::SOUTH].open = 0.6;
    let low = ray(&g, [1.5, 0.3, 2.5], [0.0, 0.0, -1.0], 10.0).unwrap();
    assert_eq!((low.cell_z, low.side), (0, grid::NORTH as i32));
}

/// Ramp in (1,1): floor rises from 0 at the south edge to 0.5 at the north edge.
fn ramp_room() -> GridMap {
    let mut g = room(0);
    g.cell_mut(1, 1).unwrap().floor_corners = [0.5, 0.5, 0.0, 0.0];
    g
}

#[test]
fn raycast_hits_slope() {
    let g = ramp_room();
    // Straight down onto the middle of the ramp: height 0.25.
    let h = ray(&g, [1.5, 0.9, 1.5], [0.0, -1.0, 0.0], 10.0).unwrap();
    assert_eq!(
        (h.cell_x, h.cell_z, h.side),
        (1, 1, grid::SIDE_FLOOR as i32)
    );
    assert!((h.point.y - 0.25).abs() < 1e-4, "{:?}", h);
    // Near the north edge (z = 1.1): height 0.45.
    let n = ray(&g, [1.5, 0.9, 1.1], [0.0, -1.0, 0.0], 10.0).unwrap();
    assert!((n.point.y - 0.45).abs() < 1e-4, "{:?}", n);
    // A level ray at y = 0.2 walking north hits the rising ramp (no step face: the south edge is at 0).
    let w = ray(&g, [1.5, 0.2, 2.5], [0.0, 0.0, -1.0], 10.0).unwrap();
    assert_eq!(
        (w.cell_x, w.cell_z, w.side),
        (1, 1, grid::SIDE_FLOOR as i32)
    );
    assert!((w.point.z - 1.6).abs() < 1e-4, "{:?}", w);
    // Higher ray passes over the ramp and meets the step face at its top edge? No: north neighbour is at 0,
    // so the ray leaves the ramp above its floor and goes on to the outer wall.
    let o = ray(&g, [1.5, 0.6, 2.5], [0.0, 0.0, -1.0], 10.0).unwrap();
    assert_eq!((o.cell_z, o.side), (0, grid::NORTH as i32));
    // Diagonal split NW-SE: corner NE high only; the point (u=0.75, v=0.25) lies in triangle NW,NE,SE.
    let mut d = room(0);
    d.cell_mut(1, 1).unwrap().floor_corners = [0.0, 0.8, 0.0, 0.0];
    let t = ray(&d, [1.75, 0.95, 1.25], [0.0, -1.0, 0.0], 10.0).unwrap();
    // height = 0.8 * (u - v) = 0.4
    assert!((t.point.y - 0.4).abs() < 1e-4, "{:?}", t);
    // rain stops at the sloped floor of an open-sky cell
    let mut r = ramp_room();
    r.cell_mut(1, 1).unwrap().ceiling_texture = -1;
    assert!(
        knorke_render::particles::indoors(&r, Vec3::new(1.5, 0.2, 1.5)),
        "below the ramp (0.25)"
    );
    assert!(
        !knorke_render::particles::indoors(&r, Vec3::new(1.5, 0.3, 1.5)),
        "above the ramp"
    );
    // other triangle stays flat
    let f = ray(&d, [1.25, 0.95, 1.75], [0.0, -1.0, 0.0], 10.0).unwrap();
    assert!(f.point.y.abs() < 1e-4, "{:?}", f);
}

#[test]
fn slope_degenerate_corners() {
    let mut g = room(0);
    {
        let c = g.cell_mut(1, 1).unwrap();
        // NW floor above its ceiling: ceiling clamps to the floor there.
        c.floor_corners = [1.5, 0.0, 0.0, 0.0];
        c.ceiling_corners = [0.0, 0.0, -0.2, 0.0];
    }
    let mut q = Vec::new();
    g.cell_quads(1, 1, 0, &mut q);
    let floor: Vec<_> = q
        .iter()
        .filter(|q| q.side == grid::SIDE_FLOOR as i32)
        .collect();
    let ceil: Vec<_> = q
        .iter()
        .filter(|q| q.side == grid::SIDE_CEILING as i32)
        .collect();
    assert!(!floor.is_empty() && !ceil.is_empty());
    let nw_ceiling = ceil
        .iter()
        .flat_map(|q| q.corners)
        .filter(|p| p.x == 1.0 && p.z == 1.0)
        .map(|p| p.y)
        .fold(f32::NAN, f32::max);
    assert_eq!(nw_ceiling, 1.5, "ceiling clamped up to the floor at NW");
    for f in &floor {
        assert!(f.normal.y > 0.0, "floor faces up: {:?}", f.normal);
    }
    for c in &ceil {
        assert!(c.normal.y < 0.0, "ceiling faces down: {:?}", c.normal);
    }
    // No wall has its bottom above its top.
    for w in q.iter().filter(|q| q.side < 4) {
        assert!(
            w.corners[0].y >= w.corners[3].y && w.corners[1].y >= w.corners[2].y,
            "{:?}",
            w.corners
        );
    }
    // All corners inverted: the cell is solid (no quads, raycasts into it hit a step face).
    let c = g.cell_mut(1, 1).unwrap();
    c.floor_corners = [1.0; 4];
    q.clear();
    g.cell_quads(1, 1, 0, &mut q);
    assert!(q.is_empty());
    let h = ray(&g, [1.5, 0.5, 2.5], [0.0, 0.0, -1.0], 10.0).unwrap();
    assert_eq!((h.cell_z, h.side), (2, grid::SIDE_STEP as i32));
    // (the C interface checks stay in pixelraum: capi.rs is not migrated, ADR 0013)
}

#[test]
fn particles_are_see_through_for_picking() {
    let (mut r, g, _) = setup();
    r.begin(&cam());
    r.draw_grid(&g);
    r.end();
    let wall = centre(&r);
    r.begin(&cam());
    r.draw_grid(&g);
    r.draw_particles(&[knorke_render::particles::Particle {
        position: Vec3::new(1.5, 0.5, 1.5),
        size: 0.5,
        color: 0xff0000,
        texture: -1,
        flags: 1,
        alpha: 1.0,
    }]);
    r.end();
    assert!(
        r.depth()[H / 2 * W + W / 2] < wall.depth,
        "the particle covers the centre"
    );
    assert_eq!(
        centre(&r),
        wall,
        "picking sees the wall behind the particle"
    );
}

#[test]
fn draws_after_end_are_ignored() {
    let (mut r, g, t) = setup();
    r.options.lighting = true;
    let frame = |r: &mut Renderer, after: bool| {
        r.begin(&cam());
        r.draw_grid(&g);
        r.end();
        if after {
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
            r.draw_grid(&g);
            r.draw_sprite(&Sprite {
                position: Vec3::new(1.5, 0.0, 1.5),
                width: 0.6,
                height: 1.0,
                texture: t,
                flags: renderer::SPRITE_BILLBOARD,
                yaw: 0.0,
                light: 1.0,
                tag: 5,
                alpha: 1.0,
                blend: knorke_render::BLEND_NONE,
            });
            r.draw_triangles(&tri, t, 1.0, false);
            r.draw_triangles_ex(
                &tri,
                t,
                1.0,
                false,
                true,
                3,
                0.5,
                knorke_render::BLEND_ALPHA,
            );
            r.draw_particles(&[knorke_render::particles::Particle {
                position: Vec3::new(1.5, 0.5, 1.5),
                size: 0.5,
                color: 0xff0000,
                texture: -1,
                flags: 1,
                alpha: 1.0,
            }]);
        }
        let picks: Vec<Option<Pick>> = (0..W as i32).map(|x| r.pick(x, H as i32 / 2)).collect();
        (
            r.rgba().to_vec(),
            r.depth().to_vec(),
            r.normals().to_vec(),
            picks,
        )
    };
    let a = frame(&mut r, false);
    let b = frame(&mut r, true);
    assert!(a == b, "draw calls after end change nothing");
}

// ---- Doors: raycast and picking ----

/// 3 x 3 room (see `room`) with a door on the north side of cell (1, 1): leaf rect u 0.25..0.75,
/// top 0.75, separate leaf texture 0.
fn door_room(mode: i32, open: f32, angle: f32) -> GridMap {
    let mut g = room(0);
    g.cell_mut(1, 1).unwrap().walls[grid::NORTH] = Wall {
        texture: 0,
        open,
        leaf_texture: 0,
        leaf_u0: 0.25,
        leaf_u1: 0.75,
        leaf_top: 0.75,
        door_mode: mode,
        swing_angle: angle,
        ..Wall::default()
    };
    g
}

type Hit5 = Option<(i32, i32, i32, i32, f32)>;

/// (cell_x, cell_z, side, part, distance) of the first hit.
fn hit5(g: &GridMap, o: [f32; 3], d: [f32; 3]) -> Hit5 {
    ray(g, o, d, 10.0).map(|h| (h.cell_x, h.cell_z, h.side, h.part, h.distance))
}

const N: [f32; 3] = [0.0, 0.0, -1.0];
const LEAF: i32 = grid::PART_LEAF;
const FRAME: i32 = grid::PART_WALL;

fn door_at(part: i32, d: f32) -> Hit5 {
    Some((1, 1, grid::NORTH as i32, part, d))
}
fn outer_wall() -> Hit5 {
    Some((1, 0, grid::NORTH as i32, FRAME, 2.5))
}

#[test]
fn raycast_door_frame_always_blocks() {
    for mode in [
        grid::DOOR_UP,
        grid::DOOR_SLIDE_LEFT,
        grid::DOOR_SLIDE_RIGHT,
        grid::DOOR_SWING_LEFT,
        grid::DOOR_SWING_RIGHT,
    ] {
        for open in [0.0, 0.5, 1.0] {
            let g = door_room(mode, open, 90.0);
            assert_eq!(
                hit5(&g, [1.1, 0.3, 2.5], N),
                door_at(FRAME, 1.5),
                "left strip, mode {mode} open {open}"
            );
            assert_eq!(
                hit5(&g, [1.9, 0.3, 2.5], N),
                door_at(FRAME, 1.5),
                "right strip, mode {mode} open {open}"
            );
            assert_eq!(
                hit5(&g, [1.5, 0.9, 2.5], N),
                door_at(FRAME, 1.5),
                "above the leaf, mode {mode} open {open}"
            );
            if open == 0.0 {
                assert_eq!(
                    hit5(&g, [1.5, 0.3, 2.5], N),
                    door_at(LEAF, 1.5),
                    "closed leaf, mode {mode}"
                );
            }
            if open == 1.0 {
                assert_eq!(
                    hit5(&g, [1.5, 0.3, 2.5], N),
                    outer_wall(),
                    "open leaf, mode {mode}"
                );
            }
        }
    }
}

#[test]
fn raycast_door_up_and_slide() {
    // UP, half open: the leaf (height 0.75) is lifted by 0.375; the part above 0.75 is hidden in the frame
    let g = door_room(grid::DOOR_UP, 0.5, 90.0);
    assert_eq!(hit5(&g, [1.5, 0.3, 2.5], N), outer_wall());
    assert_eq!(hit5(&g, [1.5, 0.5, 2.5], N), door_at(LEAF, 1.5));
    // SLIDE_LEFT, half open: the leaf covers u 0.25..0.5
    let g = door_room(grid::DOOR_SLIDE_LEFT, 0.5, 90.0);
    assert_eq!(hit5(&g, [1.4, 0.3, 2.5], N), door_at(LEAF, 1.5));
    assert_eq!(hit5(&g, [1.6, 0.3, 2.5], N), outer_wall());
    // SLIDE_RIGHT, half open: u 0.5..0.75
    let g = door_room(grid::DOOR_SLIDE_RIGHT, 0.5, 90.0);
    assert_eq!(hit5(&g, [1.4, 0.3, 2.5], N), outer_wall());
    assert_eq!(hit5(&g, [1.6, 0.3, 2.5], N), door_at(LEAF, 1.5));
    // from the other side (cell (1, 0) looking south) the same wall and parts are reported
    let g = door_room(grid::DOOR_SLIDE_LEFT, 0.5, 90.0);
    assert_eq!(
        hit5(&g, [1.4, 0.3, 0.5], [0.0, 0.0, 1.0]),
        door_at(LEAF, 0.5)
    );
    assert_eq!(
        hit5(&g, [1.1, 0.3, 0.5], [0.0, 0.0, 1.0]),
        door_at(FRAME, 0.5)
    );
}

#[test]
fn raycast_door_swing() {
    // SWING_LEFT, fully open at 90 degrees: hinge at x 1.25 on z = 1, leaf along +z (into the cell) to z = 1.5
    let g = door_room(grid::DOOR_SWING_LEFT, 1.0, 90.0);
    let e = [1.0, 0.0, 0.0];
    let h = hit5(&g, [0.6, 0.3, 1.25], e).unwrap();
    assert_eq!((h.0, h.1, h.2, h.3), (1, 1, grid::NORTH as i32, LEAF));
    assert!((h.4 - 0.65).abs() < 1e-5, "{h:?}");
    // above the leaf (heights at the hinge: 0..0.75) and beyond its end the ray passes
    assert_ne!(hit5(&g, [0.6, 0.8, 1.25], e).map(|h| h.3), Some(LEAF));
    assert_ne!(
        hit5(&g, [0.6, 0.3, 1.6], e).map(|h| (h.0, h.1)),
        Some((1, 1))
    );
    // negative angle: into the neighbour cell (1, 0), z 0.5..1; still reported as the wall of (1, 1)
    let g = door_room(grid::DOOR_SWING_LEFT, 1.0, -90.0);
    let h = hit5(&g, [0.6, 0.3, 0.75], e).unwrap();
    assert_eq!((h.0, h.1, h.2, h.3), (1, 1, grid::NORTH as i32, LEAF));
    assert!((h.4 - 0.65).abs() < 1e-5, "{h:?}");
    assert_eq!(
        hit5(&g, [0.6, 0.3, 1.25], e).map(|h| h.0),
        Some(2),
        "nothing inside (1, 1)"
    );
    // half open = 45 degrees: the ray north along x = 1.5 meets the leaf at z = 1.25
    let g = door_room(grid::DOOR_SWING_LEFT, 0.5, 90.0);
    let h = hit5(&g, [1.5, 0.3, 1.9], N).unwrap();
    assert_eq!((h.0, h.1, h.2, h.3), (1, 1, grid::NORTH as i32, LEAF));
    assert!((h.4 - 0.65).abs() < 1e-5, "{h:?}");
    // SWING_RIGHT, fully open: hinge at x 1.75, leaf along +z
    let g = door_room(grid::DOOR_SWING_RIGHT, 1.0, 90.0);
    let h = hit5(&g, [2.4, 0.3, 1.25], [-1.0, 0.0, 0.0]).unwrap();
    assert_eq!((h.0, h.1, h.2, h.3), (1, 1, grid::NORTH as i32, LEAF));
    assert!((h.4 - 0.65).abs() < 1e-5, "{h:?}");
    // 100 degrees: the leaf leans back over the wall line; the opening stays free
    let g = door_room(grid::DOOR_SWING_LEFT, 1.0, 100.0);
    assert_eq!(hit5(&g, [1.5, 0.3, 2.5], N), outer_wall());
    // without a separate leaf the whole wall swings (part WALL): hinge at x = 1, leaf along x = 1, z 1..2
    let mut g = room(0);
    g.cell_mut(1, 1).unwrap().walls[grid::NORTH] = Wall {
        texture: 0,
        open: 1.0,
        door_mode: grid::DOOR_SWING_LEFT,
        ..Wall::default()
    };
    assert_eq!(hit5(&g, [1.5, 0.3, 2.5], N), outer_wall());
    let h = hit5(&g, [0.5, 0.3, 1.5], [1.0, 0.0, 0.0]).unwrap();
    assert_eq!((h.0, h.1, h.2, h.3), (1, 1, grid::NORTH as i32, FRAME));
    assert!((h.4 - 0.5).abs() < 1e-5, "{h:?}");
}

#[test]
fn raycast_two_sided_wall_reports_owner() {
    let mut g = room(0);
    g.cell_mut(1, 1).unwrap().walls[grid::NORTH] = Wall {
        texture: 0,
        back_texture: 0,
        ..Wall::default()
    };
    assert_eq!(
        hit5(&g, [1.5, 0.5, 0.5], [0.0, 0.0, 1.0]),
        Some((1, 1, grid::NORTH as i32, FRAME, 0.5))
    );
}

fn door_cam() -> Camera {
    Camera {
        position: Vec3::new(1.5, 0.4, 2.5),
        ..Camera::default()
    }
}

#[test]
fn pick_leaf_vs_frame_and_back() {
    let mut r = Renderer::new(W, H).unwrap();
    let t = white(&mut r);
    r.options.pick = true;
    let mut g = door_room(grid::DOOR_UP, 0.0, 90.0);
    {
        let w = &mut g.cell_mut(1, 1).unwrap().walls[grid::NORTH];
        w.texture = t;
        w.leaf_texture = t;
        w.back_texture = t;
    }
    r.begin(&door_cam());
    r.draw_grid(&g);
    r.end();
    // the centre column looks at u 0.5, a little below the eye height 0.4: the leaf
    let leaf = r.pick(W as i32 / 2, H as i32 / 2 + 2).unwrap();
    assert_eq!(
        (leaf.kind, leaf.cell_x, leaf.cell_z, leaf.side, leaf.part),
        (PICK_GRID, 1, 1, grid::NORTH as i32, grid::PART_LEAF)
    );
    let frame = r.pick(21, H as i32 / 2).unwrap();
    assert_eq!(
        (frame.cell_x, frame.cell_z, frame.side, frame.part),
        (1, 1, grid::NORTH as i32, grid::PART_WALL)
    );
    // from behind: same wall, back face
    r.begin(&Camera {
        position: Vec3::new(1.5, 0.4, 0.5),
        yaw: std::f32::consts::PI,
        ..Camera::default()
    });
    r.draw_grid(&g);
    r.end();
    let back = r.pick(W as i32 / 2, H as i32 / 2 + 2).unwrap();
    assert_eq!(
        (back.cell_x, back.cell_z, back.side, back.part),
        (1, 1, grid::NORTH as i32, grid::PART_LEAF | grid::PART_BACK)
    );
    assert!(
        (back.normal.z + 1.0).abs() < 1e-6,
        "back face normal points into the neighbour (-z): {:?}",
        back.normal
    );
    // (the C interface checks stay in pixelraum: capi.rs is not migrated, ADR 0013)
}

#[test]
fn default_wall_matches_c_defaults() {
    // pixelraum's C defaults (pr_default_wall) are the Rust defaults; the C side stays there.
    let d = Wall::default();
    assert_eq!(
        (d.texture, d.open, d.alpha, d.door_mode),
        (-1, 0.0, 1.0, grid::DOOR_UP)
    );
    assert_eq!(
        (
            d.back_texture,
            d.leaf_texture,
            d.leaf_u0,
            d.leaf_u1,
            d.leaf_top,
            d.swing_angle
        ),
        (-1, -1, 0.0, 1.0, 1.0, 90.0)
    );
}

// ---- Storeys ----

fn hit_of(h: Option<GridHit>) -> Option<(i32, i32, i32, i32, f32)> {
    h.map(|h| {
        (
            h.cell_x,
            h.cell_z,
            h.level,
            h.side,
            (h.distance * 1000.0).round() / 1000.0,
        )
    })
}

#[test]
fn raycast_through_a_hole_into_the_upper_room() {
    let g = common::hole_rooms();
    let (f, c, step) = (
        grid::SIDE_FLOOR as i32,
        grid::SIDE_CEILING as i32,
        grid::SIDE_STEP as i32,
    );
    // straight up in the middle: through the hole, the stone ceiling of the upper room
    assert_eq!(
        hit_of(ray(&g, [2.5, 0.5, 2.5], [0.0, 1.0, 0.0], 10.0)),
        Some((2, 2, 1, c, 1.75))
    );
    // beside the hole: the lower ceiling
    assert_eq!(
        hit_of(ray(&g, [1.5, 0.5, 1.5], [0.0, 1.0, 0.0], 10.0)),
        Some((1, 1, 0, c, 0.5))
    );
    // down from the upper room: its floor, or through the hole the lower floor
    assert_eq!(
        hit_of(ray(&g, [1.5, 1.5, 1.5], [0.0, -1.0, 0.0], 10.0)),
        Some((1, 1, 1, f, 0.25))
    );
    assert_eq!(
        hit_of(ray(&g, [2.5, 1.5, 2.5], [0.0, -1.0, 0.0], 10.0)),
        Some((2, 2, 0, f, 1.5))
    );
    // slanted up through the hole: in the upper room the ray goes on to its east wall (level 1)
    let h = ray(&g, [2.1, 0.9, 2.5], [1.0, 0.5, 0.0], 10.0).unwrap();
    assert_eq!(
        (h.cell_x, h.cell_z, h.level, h.side),
        (3, 2, 1, grid::EAST as i32)
    );
    assert!(
        (h.point.x - 4.0).abs() < 1e-4 && (h.point.y - 1.85).abs() < 1e-4,
        "{:?}",
        h.point
    );
    // wall hits per level: the same wall position on both storeys
    assert_eq!(
        hit_of(ray(&g, [2.5, 0.5, 2.5], [1.0, 0.0, 0.0], 10.0)),
        Some((3, 2, 0, grid::EAST as i32, 1.5))
    );
    assert_eq!(
        hit_of(ray(&g, [2.5, 1.75, 2.5], [1.0, 0.0, 0.0], 10.0)),
        Some((3, 2, 1, grid::EAST as i32, 1.5))
    );
    // in the shaft (lower span of the middle cell, above the lower ceiling) going west: the slab edge
    assert_eq!(
        hit_of(ray(&g, [2.5, 1.1, 2.5], [-1.0, 0.0, 0.0], 10.0)),
        Some((2, 2, 0, step, 0.5))
    );
}

#[test]
fn raycast_under_and_over_the_bridge() {
    let g = common::bridge();
    let (n, step) = (grid::NORTH as i32, grid::SIDE_STEP as i32);
    // under the bridge and over it: the wall at the north end of the street (level 0)
    assert_eq!(
        hit_of(ray(&g, [4.5, 0.5, 8.5], [0.0, 0.0, -1.0], 20.0)),
        Some((4, 0, 0, n, 8.5))
    );
    assert_eq!(
        hit_of(ray(&g, [4.5, 2.0, 8.5], [0.0, 0.0, -1.0], 20.0)),
        Some((4, 0, 0, n, 8.5))
    );
    // into the slab between: a step on the cell before it
    assert_eq!(
        hit_of(ray(&g, [4.5, 1.1, 8.5], [0.0, 0.0, -1.0], 20.0)),
        Some((4, 5, 0, step, 3.5))
    );
    // from the deck: west onto the bank (same height, no step), along it out of the map
    assert_eq!(
        hit_of(ray(&g, [4.5, 1.5, 4.5], [-1.0, 0.0, 0.0], 20.0)),
        None
    );
    // from the deck straight down: the deck (level 1); from under the bridge up: its underside (level 0)
    assert_eq!(
        hit_of(ray(&g, [4.5, 1.5, 4.5], [0.0, -1.0, 0.0], 20.0)),
        Some((4, 4, 1, grid::SIDE_FLOOR as i32, 0.25))
    );
    assert_eq!(
        hit_of(ray(&g, [4.5, 0.5, 4.5], [0.0, 1.0, 0.0], 20.0)),
        Some((4, 4, 0, grid::SIDE_CEILING as i32, 0.5))
    );
    // from the street onto the deck from the side, slanting down: the deck floor
    let h = ray(&g, [4.5, 2.5, 5.5], [0.0, -1.0, -1.0], 20.0).unwrap();
    assert_eq!((h.cell_z, h.level, h.side), (4, 1, grid::SIDE_FLOOR as i32));
    // (the C interface checks stay in pixelraum: capi.rs is not migrated, ADR 0013)
}

#[test]
fn pick_reports_the_level() {
    let mut r = Renderer::new(W, H).unwrap();
    for _ in 0..5 {
        white(&mut r);
    }
    r.options.pick = true;
    let g = common::hole_rooms();
    // in the upper room looking at its east wall, then in the lower room
    for (y, level) in [(1.75, 1), (0.5, 0)] {
        let cam = Camera {
            position: Vec3::new(2.5, y, 2.5),
            yaw: std::f32::consts::FRAC_PI_2,
            ..cam()
        };
        r.begin(&cam);
        r.draw_grid(&g);
        r.end();
        let p = centre(&r);
        assert_eq!(
            (p.kind, p.cell_x, p.cell_z, p.level, p.side),
            (PICK_GRID, 3, 2, level, grid::EAST as i32),
            "eye {y}"
        );
    }
}

#[test]
fn rain_culled_under_the_bridge_and_in_both_rooms_not_on_the_roof() {
    let at = |g: &GridMap, x: f32, y: f32, z: f32| particles::indoors(g, Vec3::new(x, y, z));
    let b = common::bridge();
    assert!(at(&b, 4.5, 0.5, 4.5), "under the bridge");
    assert!(at(&b, 4.5, 1.1, 4.5), "inside the slab");
    assert!(!at(&b, 4.5, 2.0, 4.5), "over the deck");
    assert!(!at(&b, 4.5, 0.5, 6.5), "in the open street");
    assert!(at(&b, 2.5, 0.5, 4.5), "inside the bank");
    let h = common::hole_rooms();
    assert!(at(&h, 1.5, 0.5, 1.5) && at(&h, 1.5, 1.5, 1.5), "both rooms");
    assert!(
        at(&h, 2.5, 0.5, 2.5) && at(&h, 2.5, 1.1, 2.5),
        "under the hole: the upper ceiling covers it"
    );
    let roofed = common::gable_house(0);
    assert!(at(&roofed, 4.5, 1.0, 3.5), "in the house");
    assert!(at(&roofed, 4.5, 1.7, 3.5), "in the attic under the roof");
    assert!(!at(&roofed, 4.5, 2.6, 3.5), "on the roof");
    assert!(!at(&roofed, 1.5, 1.0, 3.5), "in the street");
}
