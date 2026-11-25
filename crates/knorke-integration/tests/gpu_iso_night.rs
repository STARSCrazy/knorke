//! The Chiaro demo scene `iso_night` (examples/iso_night_demo of chiaro @ 9bc5b9b, zlib license)
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
//! `tests/golden/chiaro/iso_night/iso_night_ref.cpp` compiles the demo's `Scene.cpp` unchanged,
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

use demo_scene::{Demo, H, HEIGHT_STEP, OUT, Scene, W};
use knorke_contract_tests::parity::Rect;
use knorke_integration::layers::{Layer, LayerSet};
use knorke_integration::light2d::Light2dSettings;
use knorke_light2d::projection::Projection;
use knorke_light2d::settings::{GlowColor, LightSettings, Quality, QualitySettings, Resolutions};
use knorke_math::{At, Vec2, Vec3};
use knorke_post::output_rect::ScaleMode;
use knorke_post::settings::{CrtPreset, CrtSettings, Filter, PostSettings};

/// `ignore_rects` of every variant in `tests/golden/chiaro/manifest.toml`.
const HUD: Rect = Rect {
    x: 0,
    y: 0,
    w: 372,
    h: 152,
};

/// Renders `variant` and compares it with its golden; `hud_longest` is the length of the HUD's
/// longest line, `layer_suffix` that of the variant's colour, glow and surface fixtures.
fn check(variant: &str, hud_longest: u32, layer_suffix: &str) {
    Demo {
        dir: "iso_night",
        variant,
        golden: &format!("iso_night_{variant}"),
        kinds: LayerSet::of(Layer::Surface).with(Layer::Glow),
        layer_suffix,
        projection: Projection::iso(At::new(Vec2::new(18.0, 14.0)), 10.0, 1.0, W, H),
        time: 5.0,
        overlay: Some(hud_panel(hud_longest)),
        ignore: &[HUD],
        // The renderer and the GPU layers give the same fog here.
        medium_gpu: "medium",
        medium_renderer: None,
    }
    .check(settings);
}

/// The settings of the demo for `scene`: `NightScene::Create` (ambient, height step, glow scale
/// and scene colour, bloom 0.8 with radius 12) and `main.cpp` (integer nearest scaling, custom
/// quality with light 1 and shadow 0.5, the medium keeping the preset's 0.5; haze 0.012 with
/// shafts where the ground fog is drawn, else off; the grade and CRT of `ApplyGrade`/`ApplyCrt`).
fn settings(scene: &Scene) -> Light2dSettings {
    let mut s = Light2dSettings {
        light: LightSettings {
            ambient: Vec3::new(0.03, 0.035, 0.05),
            quality: QualitySettings {
                quality: Quality::Custom(Resolutions {
                    light: 1.0,
                    shadow: 0.5,
                    medium: 0.5,
                }),
                ..QualitySettings::default()
            },
            surface_height_step: HEIGHT_STEP,
            ..LightSettings::default()
        },
        post: PostSettings::default(),
    };
    s.light.glow.scale = 2.5;
    s.light.glow.color = GlowColor::Scene;
    s.light.haze.density = if scene.media.is_empty() { 0.0 } else { 0.012 };
    s.light.haze.color = Vec3::new(0.8, 0.85, 1.0);
    if let Some((o, crt)) = &scene.grade {
        // The tint is Chiaro's `Kelvin(8000)`, which Knorke does not compute.
        let out = &mut s.post.output;
        out.saturation = o[0];
        out.contrast = o[1];
        out.tint = [o[2], o[3], o[4]];
        out.vignette.strength = o[5];
        out.vignette.radius = o[6];
        out.vignette.softness = o[7];
        out.vignette.color = [o[8], o[9], o[10]];
        out.dither = o[11] == 1.0;
        s.post.crt = CrtSettings::preset(CrtPreset::Subtle);
        let c = &s.post.crt;
        let want = [
            0.0,
            c.curvature,
            c.corner_radius,
            c.scanlines,
            c.scanline_period,
            c.mask,
            c.mask_pitch,
            c.brightness,
        ];
        assert_eq!(crt[..], want, "the subtle CRT preset differs from Chiaro's");
    }
    s.post.output.scale_mode = ScaleMode::Integer;
    s.post.output.filter = Filter::Nearest;
    s.post.bloom.enabled = true;
    s.post.bloom.intensity = 0.8;
    s.post.bloom.radius = 12.0;
    s
}

/// The demo's HUD panel (`demo::DrawHudPanel` at scale 1): black at alpha 170 behind 12 lines of
/// 8-pixel characters, `longest` the length of the longest one. Black, so straight alpha is
/// premultiplied already. The text lies inside the HUD rect and is left out.
fn hud_panel(longest: u32) -> Vec<u8> {
    let (w, h) = (12 + 8 * longest, 10 + 11 * 12);
    let mut px = vec![0u8; (OUT.0 * OUT.1 * 4) as usize];
    for y in 6..6 + h {
        for x in 6..6 + w {
            px[((y * OUT.0 + x) * 4 + 3) as usize] = 170;
        }
    }
    px
}

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
