//! Contract: a steady-state frame of the pixel renderer with the software light pass
//! (`knorke-integration::pixel`, serial) allocates nothing: G-buffer columns, light values,
//! shadow maps, fragment store and frame buffers are reused state (ADR 0013).
//!
//! Own test binary with a single test, so the process-wide allocation counter is not
//! disturbed by other tests running concurrently.

#![allow(unsafe_code)] // counting allocator

use knorke_integration::pixel::prelude::*;
use knorke_render::grid::{NORTH, SOUTH};
use knorke_render::particles::{EMIT_SPARKS, Emitter};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

struct Counting;
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
// SAFETY: forwards to the system allocator unchanged; only counts calls.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}
#[global_allocator]
static GLOBAL: Counting = Counting;

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

/// Lit room with walls, sky, outlines, a sun and shadowed torches, one cut-out and one blended
/// billboard and a spark emitter;
/// the camera sways and the torches flicker, so every frame is lit and shaded anew.
#[test]
fn steady_state_lit_frame_allocates_nothing() {
    let mut r = Renderer::new(160, 100).unwrap();
    let floor = r.add_texture(checker([110, 110, 130], [60, 60, 80]));
    let wall = r.add_texture(checker([160, 70, 50], [90, 40, 30]));
    r.set_sky(wall, 3.0);
    r.options = Options {
        lighting: true,
        shadow_size: 8.0,
        threads: 1,
        outlines: true,
        ..Options::default()
    };
    r.create_emitter(Emitter {
        kind: EMIT_SPARKS,
        position: Vec3::new(3.0, 0.5, 4.0),
        box_size: Vec3::new(0.2, 0.2, 0.2),
        rate: 40.0,
        speed: 0.0,
        size: 0.0,
        color: 0xffa040,
        texture: -1,
        wind: [0.0, 0.0],
        seed: 7,
    });
    r.set_sun(Some(Sun {
        dir: Vec3::new(0.5, -0.5, 1.0),
        color: 0xfff0d0,
        strength: 0.8,
        shadows: true,
    }));
    r.set_ambient(0x8090b0, 0.3);
    for (x, z) in [(1.5, 4.5), (4.5, 1.5)] {
        r.create_light(PointLight {
            pos: Vec3::new(x, 1.0, z),
            color: 0xffc080,
            strength: 1.5,
            radius: 4.0,
            flicker: 0.5,
            seed: 3,
            shadows: true,
        });
    }
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
    let sprite = |alpha: f32, blend: i32, x: f32| Sprite {
        position: Vec3::new(x, 0.0, 4.5),
        width: 0.5,
        height: 1.0,
        texture: floor,
        flags: SPRITE_BILLBOARD,
        yaw: 0.0,
        light: 1.0,
        tag: 0,
        alpha,
        blend,
    };
    let mut frame = |f: u32| {
        let cam = Camera {
            position: Vec3::new(3.5 + 0.02 * (f % 5) as f32, 1.6, 6.5),
            pitch: -0.35,
            ..Camera::default()
        };
        r.set_time(f64::from(f) / 30.0);
        r.begin(&cam);
        r.draw_grid(&g);
        r.draw_sprite(&sprite(1.0, BLEND_NONE, 3.0));
        r.draw_sprite(&sprite(0.5, BLEND_ALPHA, 2.0));
        r.draw_emitters(Some(&g));
        r.end();
    };
    // Warm-up: buffers, shadow maps and caches grow to their frame size.
    for f in 0..10 {
        frame(f);
    }
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    for f in 10..40 {
        frame(f);
    }
    let allocated = ALLOCATIONS.load(Ordering::Relaxed) - before;
    assert_eq!(allocated, 0, "allocations in 30 steady-state frames");
}
