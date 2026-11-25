//! Frame timing of the CPU pixel renderer's triangle paths (fill, texture sample, palette lookup).
//! Run: `cargo bench -p knorke-contract-tests --bench render`. Best and mean of 40 frames per scene.

#![allow(clippy::disallowed_methods)] // reads the clock on purpose

use knorke_render::{Camera, Renderer, Texture, Vertex};
use std::time::Instant;

/// `n` x `n` quads (two triangles each) covering the view at depth `z`, uv repeating `rep` times.
fn quads(n: usize, z: f32, rep: f32, out: &mut Vec<Vertex>) {
    let (hw, hh) = (2.2 * z.abs() * 0.58, 1.2 * z.abs() * 0.58);
    for j in 0..n {
        for i in 0..n {
            let p = |a: usize, b: usize| {
                let (fx, fy) = (a as f32 / n as f32, b as f32 / n as f32);
                Vertex {
                    x: -hw + 2.0 * hw * fx,
                    y: -hh + 2.0 * hh * fy,
                    z,
                    u: fx * rep,
                    v: fy * rep,
                }
            };
            out.extend([
                p(i, j),
                p(i + 1, j),
                p(i + 1, j + 1),
                p(i, j),
                p(i + 1, j + 1),
                p(i, j + 1),
            ]);
        }
    }
}

fn main() {
    let noise: Vec<u8> = (0..64 * 64u32)
        .flat_map(|i| {
            let h = i.wrapping_mul(2_654_435_761) >> 24;
            [h as u8, (h * 3) as u8, (h * 7) as u8, 255]
        })
        .collect();
    let palette: Vec<u8> = (0..16u32)
        .flat_map(|i| [(i * 17) as u8, (i * 13) as u8, (255 - i * 15) as u8])
        .collect();
    let cam = Camera::default();
    for (w, h) in [(160, 100), (640, 360)] {
        // small: one fine mesh (~8 px per triangle at 640x360); large: 6 layered full-view quads, back to front
        let mut small = Vec::new();
        quads(if w == 160 { 40 } else { 120 }, -2.0, 2.0, &mut small);
        let mut large = Vec::new();
        for k in 0..6 {
            quads(1, -3.0 + k as f32 * 0.1, 2.0, &mut large);
        }
        let empty = Vec::new();
        for (scene, verts) in [("empty", &empty), ("small", &small), ("large", &large)] {
            for textured in [true, false] {
                for pal in [false, true] {
                    let mut r = Renderer::new(w, h).unwrap();
                    let t = if textured {
                        r.add_texture(Texture::from_rgba(64, 64, &noise).unwrap())
                    } else {
                        r.add_texture(Texture::from_rgba(1, 1, &[200, 180, 90, 255]).unwrap())
                    };
                    if pal {
                        r.set_palette(&palette);
                        r.options.quantize = true;
                    }
                    let mut times = Vec::new();
                    for f in 0..41 {
                        let start = Instant::now();
                        r.begin(&cam);
                        r.draw_triangles(verts, t, 1.0, false);
                        r.end();
                        if f > 0 {
                            times.push(start.elapsed().as_secs_f64() * 1e3);
                        }
                    }
                    let best = times.iter().cloned().fold(f64::INFINITY, f64::min);
                    let mean = times.iter().sum::<f64>() / times.len() as f64;
                    println!(
                        "{w}x{h} {scene:<5} {:<8} {:<7} best {best:8.3} ms  mean {mean:8.3} ms",
                        if textured { "textured" } else { "flat" },
                        if pal { "palette" } else { "rgb" }
                    );
                }
            }
        }
    }
}
