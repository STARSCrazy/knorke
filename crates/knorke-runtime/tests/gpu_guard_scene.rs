//! The guard scene drawn through the runtime and the 2D GPU light
//! (`examples/guard_scene_view.rs`), checked from cheap to dear (ADR 0010): what the lamp
//! lights and what the wall shades, that poses blend between ticks, then one frame against
//! its golden image.
//!
//! Skips (passes with a note) without a Vulkan device or the shader pack; with
//! `KNORKE_REQUIRE_GPU=1` a skip is a failure. `KNORKE_SCENE_OUT=<dir>` writes every frame.
//! `KNORKE_BLESS=1` rewrites the golden image.
#![cfg(feature = "gpu")]

// The example is shared with this test; its `pub` items are its public face as an example.
#[allow(dead_code, unreachable_pub, unused_imports)]
#[path = "../examples/guard_scene_view.rs"]
mod view;

use knorke_contract_tests::parity::{ParityBudget, compare_rgba8, load_png_rgba8, save_png_rgba8};
use knorke_gpu::test_support::{vulkan_device_with_pack, workspace_root};
use knorke_gpu::{Gpu, LiveCounter};
use std::path::PathBuf;
use view::{PX, Script, View};

/// The picture of the built-in yard: 16 x 10 cells.
const W: u32 = 16 * PX;
const H: u32 = 10 * PX;

/// Brightness of the middle of cell `(x, y)`.
fn light_at(frame: &[u8], x: u32, y: u32) -> u32 {
    let i = (((y * PX + PX / 2) * W + x * PX + PX / 2) * 4) as usize;
    frame[i..i + 3].iter().map(|&c| u32::from(c)).sum()
}

/// The middle of the intruder's disc in pixels: the pixels that are clearly red.
fn intruder_at(frame: &[u8]) -> (f32, f32) {
    let (mut n, mut sx, mut sy) = (0u32, 0u32, 0u32);
    for (i, px) in frame.as_chunks::<4>().0.iter().enumerate() {
        if px[0] > 30 && u32::from(px[0]) > 2 * u32::from(px[1]) {
            n += 1;
            sx += i as u32 % W;
            sy += i as u32 / W;
        }
    }
    assert!(n > 50, "the intruder's disc is missing ({n} red pixels)");
    (sx as f32 / n as f32, sy as f32 / n as f32)
}

#[test]
fn the_lamp_lights_the_yard_and_poses_blend_between_ticks() {
    let Some((device, pack)) = vulkan_device_with_pack() else {
        return;
    };
    let counter = LiveCounter::new();
    let mut view = View::new(Gpu::new(&device, &counter), pack, &view::yard());
    assert_eq!(view.size(), (W, H));
    let out = std::env::var("KNORKE_SCENE_OUT").ok().map(PathBuf::from);
    let mut frames = Vec::new();
    // Ticks 0..=20 at three frames per tick: the intruder walks in, is spotted, the chase starts.
    view::play(&mut view, view::start(), Script::BOLD, 63, |f, pixels| {
        if let Some(dir) = &out {
            std::fs::create_dir_all(dir).unwrap();
            save_png_rgba8(&dir.join(format!("guard_{f:03}.png")), pixels, W, H);
        }
        frames.push(pixels.to_vec());
    });
    drop(view);
    assert_eq!(counter.live(), 0, "the view leaked GPU objects");

    // Frame 0, guard at its post (3, 5) looking east. Cells of the same floor colour: in the
    // cone, beside the guard out of the cone, and in the cone but behind the wall.
    let (lit, beside, shaded) = (
        light_at(&frames[0], 6, 5),
        light_at(&frames[0], 3, 8),
        light_at(&frames[0], 12, 5),
    );
    assert!(lit > 2 * beside, "cone {lit}, beside it {beside}");
    assert!(lit > 2 * shaded, "cone {lit}, behind the wall {shaded}");
    assert!(
        shaded.abs_diff(beside) < 12,
        "only ambient light: {shaded} and {beside}"
    );

    // The intruder walks one cell per tick. Drawn three times per tick it moves a third of a
    // cell per frame, never a whole cell at once. Frames 12..30 are ticks 4..=9 of its walk.
    for pair in frames[12..30].windows(2) {
        let (a, b) = (intruder_at(&pair[0]), intruder_at(&pair[1]));
        let step = knorke_math::det::max((b.0 - a.0).abs(), (b.1 - a.1).abs());
        let third = PX as f32 / 3.0;
        assert!(
            (step - third).abs() < 1.5,
            "moved {step} px in one frame, expected about {third}"
        );
    }

    // Frame 57 is the first of tick 19: the chase is on and the lamp has turned.
    let golden = workspace_root().join("crates/knorke-runtime/tests/golden/guard_scene_057.png");
    if std::env::var_os("KNORKE_BLESS").is_some() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        save_png_rgba8(&golden, &frames[57], W, H);
    }
    let want = load_png_rgba8(&golden, W, H);
    let report = compare_rgba8(&frames[57], &want, W, H, &[], &ParityBudget::STRICT).unwrap();
    report.print("guard_scene_057", "STRICT");
    assert!(report.within(&ParityBudget::STRICT), "{report:?}");
}
