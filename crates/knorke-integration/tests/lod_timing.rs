//! Release timing of the texture level path on the joint floor (320 x 200):
//! cargo test --release --test lod_timing -- --ignored --nocapture
#![cfg(feature = "pixel")]

mod common;

use knorke_integration::pixel::prelude::*;

/// Best and mean frame time of `frames` frames of the joint floor with the given options change.
#[allow(clippy::disallowed_methods)] // host-side timing test: reads the clock on purpose
pub fn time_floor(setup: impl Fn(&mut Renderer, i32)) -> (f64, f64) {
    let (mut r, g, t, cam) = common::joint_floor(320, 200);
    setup(&mut r, t);
    let mut times = Vec::new();
    for f in 0..61 {
        let start = std::time::Instant::now();
        r.begin(&cam);
        r.draw_grid(&g);
        r.end();
        if f > 0 {
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    (
        times.iter().cloned().fold(f64::INFINITY, f64::min),
        times.iter().sum::<f64>() / times.len() as f64,
    )
}

#[test]
#[ignore]
fn timing_lod_floor() {
    for _ in 0..3 {
        let (best, mean) = time_floor(|_, _| {});
        println!("timing_lod_floor plain: best {best:.3} ms, mean {mean:.3} ms");
    }
}
