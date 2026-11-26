//! The Chiaro demo scene `iso_night` as the tests set it up (`gpu_iso_night.rs` describes the
//! variants): the demo of a variant and its settings. Included with `#[path]` next to
//! `demo_scene.rs`.
#![allow(dead_code)]

use crate::demo_scene::{Demo, H, HEIGHT_STEP, OUT, Scene, W};
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

/// Hands `body` the demo of `variant`; `hud_longest` is the length of the HUD's longest line,
/// `layer_suffix` that of the variant's colour, glow and surface fixtures.
pub(crate) fn with_demo(
    variant: &str,
    hud_longest: u32,
    layer_suffix: &str,
    body: impl FnOnce(Demo<'_>),
) {
    body(Demo {
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
    });
}

/// Renders `variant` and compares it with its golden.
pub(crate) fn check(variant: &str, hud_longest: u32, layer_suffix: &str) {
    with_demo(variant, hud_longest, layer_suffix, |demo| {
        demo.check(settings)
    });
}

/// The settings of the demo for `scene`: `NightScene::Create` (ambient, height step, glow scale
/// and scene colour, bloom 0.8 with radius 12) and `main.cpp` (integer nearest scaling, custom
/// quality with light 1 and shadow 0.5, the medium keeping the preset's 0.5; haze 0.012 with
/// shafts where the ground fog is drawn, else off; the grade and CRT of `ApplyGrade`/`ApplyCrt`).
pub(crate) fn settings(scene: &Scene) -> Light2dSettings {
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
            // Chiaro's light: the scene is compared with Chiaro's pictures.
            light_rows: false,
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
