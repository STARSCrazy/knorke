//! The Chiaro demo scene `iso_night` (examples/iso_night_demo of chiaro @ 9bc5b9b plus one fix, e8a149a, in
//! `Scene.cpp` `PatternSheet()`: `closed = x < 2;` instead of `x >= 2`, so the drawn slats block;
//! zlib license)
//! at `--time 5` in its five P3 variants, end to end through [`Layers`] and [`Light2dRenderer`],
//! compared with the goldens `tests/golden/chiaro/iso_night_<variant>.png` under
//! `ParityBudget::STRICT` outside the HUD rect of `manifest.toml`:
//! - `plain` (`--no-heights --no-surface --no-haze --no-fog --no-moon-shadows`): infinitely tall
//!   obstacles, flat ground, no moon shadows, soft shadows;
//! - `hard` (`--hard --no-haze`): real heights, surface heights and normals, hard shadows;
//! - `t5` (no arguments): soft shadows, haze 0.012 with shafts and the ground fog, 150 puffs of
//!   `AddFogPatch` in the medium layer (media table entry 4);
//! - `cutaway` (`--cutaway`): as `t5` with the door hall cut open (roof slab, its ambient zone
//!   lit inside), other colour, glow and surface layers;
//! - `grade_crt` (`--grade look --crt subtle --dither`): as `t5` with the output grade (saturation,
//!   contrast, `Kelvin(8000)` tint, vignette), output dither and the subtle CRT.
//!
//! All have point, spot, line, rect and directional lights, light profiles (hat and LED street
//! lamps), the hall window mask, animations (fire, beacon, flashes, envelope), the car's frame
//! lights, the hall's ambient zone, dynamic obstacles, the two pattern obstacles, glow and bloom.
//! Without haze and clouds Chiaro runs no medium stage (`MediumStage.hpp`: only with a haze
//! density above 0 or a medium layer; `Layers::Fill` leaves out a medium layer nothing drew
//! into), so `plain` and `hard` create no medium layer.
//!
//! `tests/golden/chiaro/iso_night/iso_night_ref.cpp` compiles the demo's `Scene.cpp` (with that fix),
//! renders each variant byte-identical to its golden outside the HUD, and writes the resolved
//! scene as `<variant>.txt` (format: `common/demo_scene.rs`) with the colour and glow layers.
//!
//! The demo's HUD panel reaches past the manifest's HUD rect in `plain` (its longest line, the
//! toggles, is one character longer than in the others): it is drawn as the overlay, without its
//! text, which lies inside the rect.
//!
//! The test plays the host (ADR 0002). It skips without a Vulkan device or the shader pack of
//! `cargo xtask shaders` (a failure with `KNORKE_REQUIRE_GPU=1`). `KNORKE_SCENE_OUT=<dir>` writes
//! the frames and amplified diff images there.
#![cfg(feature = "gpu")]

#[path = "common/gpu.rs"]
mod scene_common;

#[path = "common/demo_scene.rs"]
mod demo_scene;

#[path = "common/iso_night.rs"]
mod iso_night;

use iso_night::check;

#[test]
fn iso_night_plain_matches_chiaro() {
    // The toggles line "M moon shadows off  F soft  P running  Z haze" is the longest.
    check("plain", 45, "");
}

#[test]
fn iso_night_hard_matches_chiaro() {
    // "M moon shadows on  F hard  P running  Z haze": the panel stays inside the HUD rect.
    check("hard", 44, "");
}

#[test]
fn iso_night_t5_matches_chiaro() {
    // Soft instead of hard: the same length.
    check("t5", 44, "");
}

#[test]
fn iso_night_cutaway_matches_chiaro() {
    // The cut-away hall paints other colours, glow and surface.
    check("cutaway", 44, "_cutaway");
}

#[test]
fn iso_night_grade_crt_matches_chiaro() {
    check("grade_crt", 44, "");
}
