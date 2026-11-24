// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/rendering/OutputStage.cpp and
// BloomStage.cpp: the parameter blocks `OutputParams`, `PrefilterParams`, `DownParams`,
// `UpParams` and how they are filled), zlib licence:
//
// Copyright (C) 2026 Manuel Vögele
//
// This software is provided 'as-is', without any express or implied
// warranty.  In no event will the authors be held liable for any damages
// arising from the use of this software.
//
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not
//    claim that you wrote the original software. If you use this software
//    in a product, an acknowledgment in the product documentation would be
//    appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be
//    misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// Altered: rewritten in Rust for the Knorke Engine; the blocks are byte arrays written field by
// field (no `unsafe`, no allocation) and take the Rust settings types.

//! The uniform blocks of the bloom and output shaders, built from settings and frame data.
//!
//! The blocks are plain byte arrays in the std140-like layout of the HLSL `cbuffer`s
//! (`bloom_prefilter.frag.hlsl`, `bloom_down.frag.hlsl`, `bloom_up.frag.hlsl`,
//! `output.frag.hlsl`), little endian. Building them needs no GPU and allocates nothing; the
//! passes of the `gpu` feature push them as fragment uniforms.

use knorke_math::det;

use crate::bloom::BloomPlan;
use crate::color::{exposure_scale, linear_to_srgb};
use crate::crt::{CRT_WARP_SCALE, crt_scanline_fade};
use crate::output_flags::{self as flags, crt_flags, grade_flags, shockwave_flags};
use crate::output_rect::compute_output_rect;
use crate::settings::{BloomSettings, Filter, PostSettings, Tonemap};
use crate::shockwave::{MAX_SHOCKWAVES, Projection, ShockwaveFrame, ground_inverse};

/// Size of the block of `bloom_prefilter.frag`.
pub const PREFILTER_PARAMS_SIZE: usize = 32;
/// Size of the block of `bloom_down.frag`.
pub const DOWN_PARAMS_SIZE: usize = 16;
/// Size of the block of `bloom_up.frag`.
pub const UP_PARAMS_SIZE: usize = 32;
/// Size of the block of `output.frag`: 240 bytes plus 32 per shockwave slot.
pub const OUTPUT_PARAMS_SIZE: usize = 240 + 32 * MAX_SHOCKWAVES;

/// Tonemap knee the shader reads for the curves that have none (the Chiaro default; unused there).
const DEFAULT_KNEE: f32 = 0.8;

/// Writes a uniform block field by field in declaration order.
struct Block<const N: usize> {
    bytes: [u8; N],
    at: usize,
}

impl<const N: usize> Block<N> {
    fn new() -> Self {
        Self {
            bytes: [0; N],
            at: 0,
        }
    }

    fn f(&mut self, v: f32) -> &mut Self {
        self.bytes[self.at..self.at + 4].copy_from_slice(&v.to_le_bytes());
        self.at += 4;
        self
    }

    fn u(&mut self, v: u32) -> &mut Self {
        self.bytes[self.at..self.at + 4].copy_from_slice(&v.to_le_bytes());
        self.at += 4;
        self
    }

    fn fs(&mut self, v: &[f32]) -> &mut Self {
        for &x in v {
            self.f(x);
        }
        self
    }

    /// Leaves `bytes` zero bytes (padding).
    fn skip(&mut self, bytes: usize) -> &mut Self {
        self.at += bytes;
        self
    }

    /// The finished block; every byte up to `N` must have been written or skipped.
    fn finish(&self) -> [u8; N] {
        debug_assert_eq!(self.at, N, "uniform block not filled completely");
        self.bytes
    }
}

/// Parameters of the first bloom pass (`PrefilterParams`): thresholds the exposed scene into
/// level 0. `scene` and `level0` are sizes in texels.
pub fn prefilter_params(
    scene: (u32, u32),
    level0: (u32, u32),
    exposure_ev: f32,
    bloom: &BloomSettings,
) -> [u8; PREFILTER_PARAMS_SIZE] {
    let mut b = Block::<PREFILTER_PARAMS_SIZE>::new();
    b.fs(&[1.0 / scene.0 as f32, 1.0 / scene.1 as f32])
        .fs(&[level0.0 as f32, level0.1 as f32])
        .f(exposure_scale(exposure_ev))
        .f(bloom.threshold)
        .f(bloom.threshold * bloom.knee)
        .f(0.0);
    b.finish()
}

/// Parameters of a downsample pass (`DownParams`) from `src` into the next smaller level `dst`.
pub fn down_params(src: (u32, u32), dst: (u32, u32)) -> [u8; DOWN_PARAMS_SIZE] {
    let mut b = Block::<DOWN_PARAMS_SIZE>::new();
    b.fs(&[
        2.0 * dst.0 as f32 / src.0 as f32,
        2.0 * dst.1 as f32 / src.1 as f32,
    ])
    .fs(&[1.0 / src.0 as f32, 1.0 / src.1 as f32]);
    b.finish()
}

/// Parameters of an upsample pass (`UpParams`) from `src` onto the next larger level `dst`.
/// `weight` is the fade-in weight of the coarsest level, 1 for all others.
pub fn up_params(src: (u32, u32), dst: (u32, u32), weight: f32) -> [u8; UP_PARAMS_SIZE] {
    let mut b = Block::<UP_PARAMS_SIZE>::new();
    b.fs(&[
        dst.0 as f32 / (2.0 * src.0 as f32),
        dst.1 as f32 / (2.0 * src.1 as f32),
    ])
    .fs(&[1.0 / src.0 as f32, 1.0 / src.1 as f32])
    .f(weight)
    .skip(12);
    b.finish()
}

/// What the output pass needs to know about the bloom result of the same frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomInput {
    /// `intensity / (levels - 1 + coarsest_weight)`: the share of the normalized sum added.
    pub scale: f32,
    /// Width of level 0 in texels.
    pub width: u32,
    /// Height of level 0 in texels.
    pub height: u32,
}

impl BloomInput {
    /// The input for level 0 of `size` texels and the plan of the chain.
    pub fn new(intensity: f32, plan: BloomPlan, size: (u32, u32)) -> Self {
        Self {
            scale: intensity / ((plan.levels - 1) as f32 + plan.coarsest_weight),
            width: size.0,
            height: size.1,
        }
    }
}

/// How the overlay of a frame is blended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayBlend {
    /// The overlay samples sRGB-encoded values (8-bit UNORM); blended in encoded space.
    Srgb,
    /// The overlay samples linear values (`*_SRGB` and float formats); blended in linear space.
    Linear,
}

/// The frame data the output block depends on, besides the settings.
#[derive(Clone, Copy, Debug)]
pub struct OutputFrame<'a> {
    /// Scene (HDR buffer) size in texels.
    pub scene: (u32, u32),
    /// Target size in pixels.
    pub target: (u32, u32),
    /// The target stores encoded values (UNORM): the shader encodes to sRGB. False for `*_SRGB`
    /// and float targets, which expect linear values.
    pub encode_srgb: bool,
    /// The target has 8 bits per channel, so the dither applies ([`OutputSettings::dither`] is
    /// ignored otherwise).
    ///
    /// [`OutputSettings::dither`]: crate::settings::OutputSettings::dither
    pub dither_target: bool,
    /// The overlay, if the frame has one.
    pub overlay: Option<OverlayBlend>,
    /// The bloom result, if bloom runs.
    pub bloom: Option<BloomInput>,
    /// Projection and the culled waves of the frame, if any (waves without distortion in view are
    /// ignored).
    pub shockwaves: Option<(&'a Projection, &'a ShockwaveFrame)>,
}

/// The block of `output.frag` (`OutputParams`) for `settings` and `frame`.
///
/// Mirrors `OutputStage::Record` of Chiaro: the scene rectangle from the scale mode, exposure,
/// tonemap, grade, vignette (colour encoded to sRGB, `2 / shorter scene side`), CRT (the scanline
/// fade follows the scale), dither, shockwaves and bloom, with a flag for each step that has an
/// effect.
pub fn output_params(settings: &PostSettings, frame: &OutputFrame<'_>) -> [u8; OUTPUT_PARAMS_SIZE] {
    let o = &settings.output;
    let crt = &settings.crt;
    let rect = compute_output_rect(
        frame.scene.0,
        frame.scene.1,
        frame.target.0,
        frame.target.1,
        o.scale_mode,
    );
    let (scene_w, scene_h) = (frame.scene.0 as f32, frame.scene.1 as f32);
    let (target_w, target_h) = (frame.target.0 as f32, frame.target.1 as f32);
    let (rect_w, rect_h) = (det::max(1.0, rect.w as f32), det::max(1.0, rect.h as f32));
    let nearest = o.filter == Filter::Nearest;

    let mut f = grade_flags(o);
    if frame.encode_srgb {
        f |= flags::ENCODE_SRGB;
    }
    match frame.overlay {
        Some(OverlayBlend::Srgb) => f |= flags::HAS_OVERLAY,
        Some(OverlayBlend::Linear) => f |= flags::HAS_OVERLAY | flags::OVERLAY_LINEAR,
        None => {}
    }
    // Dither only on request and only for 8-bit targets, so everything else stays bit-identical.
    if o.dither && frame.dither_target {
        f |= flags::DITHER;
    }
    let fade = crt_scanline_fade(rect_h, scene_h, crt.scanline_period);
    f |= crt_flags(crt, fade, frame.overlay.is_some());
    let shorter = det::min(rect_w, rect_h);
    let (mode, knee) = match o.tonemap {
        Tonemap::None => (0, DEFAULT_KNEE),
        Tonemap::Soft { knee } => (1, knee),
        Tonemap::Aces => (2, DEFAULT_KNEE),
    };

    let mut b = Block::<OUTPUT_PARAMS_SIZE>::new();
    b.fs(&[rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32]) // sceneRect
        .fs(&[target_w, target_h]) // targetSize
        .f(exposure_scale(o.exposure_ev))
        .u(0) // flags, patched below
        .fs(&[o.letterbox[0], o.letterbox[1], o.letterbox[2], 1.0])
        .u(mode)
        .f(knee);
    // bloomScale, pad, bloomUv.
    match frame.bloom {
        Some(bloom) => {
            f |= flags::BLOOM | if nearest { flags::BLOOM_SNAP } else { 0 };
            b.f(bloom.scale).f(0.0).fs(&[
                scene_w / (2.0 * bloom.width as f32),
                scene_h / (2.0 * bloom.height as f32),
                1.0 / bloom.width as f32,
                1.0 / bloom.height as f32,
            ]);
        }
        None => {
            b.f(0.0).f(0.0).fs(&[0.0; 4]);
        }
    }
    b.fs(&[scene_w, scene_h])
        .skip(8) // sceneSize, pad2
        .fs(&[o.tint[0], o.tint[1], o.tint[2], o.saturation]) // tintSaturation
        .fs(&[
            o.vignette.strength,
            o.vignette.radius,
            o.vignette.softness,
            o.contrast,
        ])
        .fs(&[
            linear_to_srgb(o.vignette.color[0]),
            linear_to_srgb(o.vignette.color[1]),
            linear_to_srgb(o.vignette.color[2]),
            2.0 / det::max(1.0, det::min(scene_w, scene_h)),
        ])
        .fs(&[
            CRT_WARP_SCALE * crt.curvature,
            crt.corner_radius,
            rect_w / shorter,
            rect_h / shorter,
        ]) // crtWarp
        .fs(&[
            crt.scanlines * fade,
            crt.scanline_period,
            crt.mask,
            crt.mask_pitch,
        ]) // crtBeam
        .fs(&[crt.brightness, 0.5 * shorter, 0.0, 0.0]); // crtExtra

    // Shockwaves only with a wave that distorts in view.
    let mut shock_count = 0;
    let mut rings = [[0.0f32; 4]; MAX_SHOCKWAVES];
    let mut strengths = [[0.0f32; 4]; MAX_SHOCKWAVES];
    match frame.shockwaves {
        Some((p, waves)) if waves.distortion_count > 0 => {
            shock_count = waves.distortion_count.min(MAX_SHOCKWAVES);
            f |= shockwave_flags(shock_count, o.filter);
            b.fs(&[p.x_axis.x, p.x_axis.y, p.y_axis.x, p.y_axis.y])
                .fs(&ground_inverse(p))
                .fs(&[p.origin.x, p.origin.y]);
            for (i, w) in waves.distortion[..shock_count].iter().enumerate() {
                rings[i] = w.ring;
                strengths[i] = w.strength;
            }
        }
        _ => {
            b.fs(&[0.0; 10]);
        }
    }
    b.u(shock_count as u32).f(0.0);
    for r in rings {
        b.fs(&r);
    }
    for s in strengths {
        b.fs(&s);
    }
    let mut bytes = b.finish();
    bytes[28..32].copy_from_slice(&f.to_le_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    // The layout of the blocks is the contract with the compiled shaders; the values follow
    // OutputStage::Record and BloomStage::Record of Chiaro 9bc5b9b.
    use super::*;
    use crate::bloom::plan_bloom;
    use crate::output_rect::ScaleMode;
    use crate::settings::{CrtSettings, VignetteSettings};
    use crate::shockwave::{DistortionWave, Shockwave, ShockwaveList, collect_shockwaves};
    use knorke_math::Vec2;

    fn f32_at(b: &[u8], offset: usize) -> f32 {
        f32::from_le_bytes(b[offset..offset + 4].try_into().unwrap())
    }

    fn u32_at(b: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(b[offset..offset + 4].try_into().unwrap())
    }

    fn floats(b: &[u8], offset: usize, n: usize) -> Vec<f32> {
        (0..n).map(|i| f32_at(b, offset + 4 * i)).collect()
    }

    fn frame(scene: (u32, u32), target: (u32, u32)) -> OutputFrame<'static> {
        OutputFrame {
            scene,
            target,
            encode_srgb: true,
            dither_target: true,
            overlay: None,
            bloom: None,
            shockwaves: None,
        }
    }

    #[test]
    fn block_sizes_match_the_shaders() {
        assert_eq!(PREFILTER_PARAMS_SIZE, 32);
        assert_eq!(DOWN_PARAMS_SIZE, 16);
        assert_eq!(UP_PARAMS_SIZE, 32);
        assert_eq!(OUTPUT_PARAMS_SIZE, 752);
    }

    #[test]
    fn defaults_give_the_plain_block() {
        let p = output_params(&PostSettings::default(), &frame((64, 32), (128, 64)));
        assert_eq!(floats(&p, 0, 4), [0.0, 0.0, 128.0, 64.0]); // sceneRect: fit fills exactly
        assert_eq!(floats(&p, 16, 2), [128.0, 64.0]);
        assert_eq!(f32_at(&p, 24), 1.0); // exposure 0 EV
        assert_eq!(u32_at(&p, 28), flags::ENCODE_SRGB);
        assert_eq!(floats(&p, 32, 4), [0.0, 0.0, 0.0, 1.0]); // letterbox
        assert_eq!(u32_at(&p, 48), 0); // tonemap none
        assert_eq!(f32_at(&p, 52), 0.8);
        assert_eq!(floats(&p, 64, 4), [0.0; 4]); // no bloom
        assert_eq!(floats(&p, 80, 2), [64.0, 32.0]); // sceneSize
        assert_eq!(floats(&p, 96, 4), [1.0, 1.0, 1.0, 1.0]); // tint, saturation
        assert_eq!(floats(&p, 112, 4), [0.0, 0.5, 1.5, 1.0]); // vignette off
        assert_eq!(floats(&p, 128, 4), [0.0, 0.0, 0.0, 2.0 / 32.0]);
        assert_eq!(floats(&p, 144, 4), [0.0, 0.0, 2.0, 1.0]); // CRT off: aspect rect / shorter
        assert_eq!(floats(&p, 160, 4), [0.0, 1.0, 0.0, 3.0]);
        assert_eq!(floats(&p, 176, 4), [1.0, 32.0, 0.0, 0.0]);
        assert_eq!(u32_at(&p, 232), 0); // shock count
        assert!(p[240..].iter().all(|&b| b == 0));
    }

    #[test]
    fn rect_letterbox_and_target_encoding() {
        let mut s = PostSettings::default();
        s.output.scale_mode = ScaleMode::Integer;
        s.output.letterbox = [0.1, 0.2, 0.3];
        let mut fr = frame((192, 128), (400, 300));
        fr.encode_srgb = false;
        let p = output_params(&s, &fr);
        // 2x: 384 x 256 centred in 400 x 300.
        assert_eq!(floats(&p, 0, 4), [8.0, 22.0, 384.0, 256.0]);
        assert_eq!(floats(&p, 32, 4), [0.1, 0.2, 0.3, 1.0]);
        assert_eq!(u32_at(&p, 28) & flags::ENCODE_SRGB, 0);
    }

    #[test]
    fn tonemap_mode_and_knee() {
        let mut s = PostSettings::default();
        let fr = frame((8, 8), (8, 8));
        s.output.tonemap = Tonemap::Soft { knee: 0.6 };
        let p = output_params(&s, &fr);
        assert_eq!((u32_at(&p, 48), f32_at(&p, 52)), (1, 0.6));
        s.output.tonemap = Tonemap::Aces;
        let p = output_params(&s, &fr);
        assert_eq!(u32_at(&p, 48), 2);
        s.output.exposure_ev = 1.0;
        assert_eq!(f32_at(&output_params(&s, &fr), 24), 2.0);
    }

    #[test]
    fn grade_vignette_dither_and_overlay() {
        let mut s = PostSettings::default();
        s.output.tint = [1.1, 1.0, 0.9];
        s.output.saturation = 1.3;
        s.output.contrast = 1.15;
        s.output.vignette = VignetteSettings {
            strength: 0.5,
            radius: 0.6,
            softness: 1.2,
            color: [1.0, 0.0, 0.0],
        };
        s.output.filter = Filter::Nearest;
        s.output.dither = true;
        let mut fr = frame((100, 50), (100, 50));
        fr.overlay = Some(OverlayBlend::Linear);
        let p = output_params(&s, &fr);
        let f = u32_at(&p, 28);
        let want = flags::ENCODE_SRGB
            | flags::HAS_OVERLAY
            | flags::OVERLAY_LINEAR
            | flags::GRADE
            | flags::CONTRAST
            | flags::VIGNETTE
            | flags::SCENE_SNAP
            | flags::DITHER;
        assert_eq!(f, want);
        assert_eq!(floats(&p, 96, 4), [1.1, 1.0, 0.9, 1.3]);
        assert_eq!(floats(&p, 112, 4), [0.5, 0.6, 1.2, 1.15]);
        // The colour is mixed in display values: encoded to sRGB, 1 at full red, 0 elsewhere.
        // (`det::pow` is not exactly `powf`: compare with a tolerance of a few ulp.)
        let v = floats(&p, 128, 4);
        assert!((v[0] - 1.0).abs() < 1e-6 && v[1] == 0.0 && v[2] == 0.0);
        assert_eq!(v[3], 2.0 / 50.0);
        // The dither needs an 8-bit target.
        fr.dither_target = false;
        assert_eq!(u32_at(&output_params(&s, &fr), 28) & flags::DITHER, 0);
        // An sRGB-encoded overlay has no linear flag.
        fr.overlay = Some(OverlayBlend::Srgb);
        let f = u32_at(&output_params(&s, &fr), 28);
        assert_ne!(f & flags::HAS_OVERLAY, 0);
        assert_eq!(f & flags::OVERLAY_LINEAR, 0);
    }

    #[test]
    fn crt_block_follows_the_rect() {
        let mut s = PostSettings::default();
        s.output.scale_mode = ScaleMode::Stretch;
        s.crt = CrtSettings {
            overlay: true,
            curvature: 0.3,
            corner_radius: 0.15,
            scanlines: 0.4,
            scanline_period: 1.0,
            mask: 0.25,
            mask_pitch: 3.0,
            brightness: 1.3,
        };
        let mut fr = frame((100, 50), (400, 200));
        fr.overlay = Some(OverlayBlend::Srgb);
        let p = output_params(&s, &fr);
        let f = u32_at(&p, 28);
        for bit in [
            flags::CRT_WARP,
            flags::CRT_OVERLAY,
            flags::CRT_SCANLINES,
            flags::CRT_MASK,
            flags::CRT_BEAM,
        ] {
            assert_ne!(f & bit, 0, "flag {bit}");
        }
        // Rect 400 x 200 over 100 x 50 scene rows: four target rows per scene row, so the
        // scanline fade is 1 (rect_height / scene_height * period - 1 = 3, clamped).
        assert_eq!(floats(&p, 144, 4), [0.25 * 0.3, 0.15, 2.0, 1.0]);
        assert_eq!(floats(&p, 160, 4), [0.4, 1.0, 0.25, 3.0]);
        assert_eq!(floats(&p, 176, 4), [1.3, 100.0, 0.0, 0.0]);
    }

    #[test]
    fn scanlines_fade_out_at_one_to_one() {
        let mut s = PostSettings::default();
        s.crt.scanlines = 0.5;
        // Scene and target equal: one target row per scene row, the lines fade out entirely.
        let p = output_params(&s, &frame((64, 64), (64, 64)));
        assert_eq!(u32_at(&p, 28) & flags::CRT_SCANLINES, 0);
        assert_eq!(f32_at(&p, 160), 0.0);
        // Two target rows per scene row: fade = clamp(2 - 1) = 1, the lines show.
        let p = output_params(&s, &frame((64, 64), (128, 128)));
        assert_ne!(u32_at(&p, 28) & flags::CRT_SCANLINES, 0);
        assert_eq!(f32_at(&p, 160), 0.5);
    }

    #[test]
    fn bloom_block_and_scale() {
        let plan = plan_bloom(24.0, 192, 128);
        let level0 = (96, 64);
        let bloom = BloomInput::new(0.8, plan, level0);
        assert_eq!(
            bloom.scale,
            0.8 / ((plan.levels - 1) as f32 + plan.coarsest_weight)
        );
        let mut fr = frame((192, 128), (192, 128));
        fr.bloom = Some(bloom);
        let mut s = PostSettings::default();
        let p = output_params(&s, &fr);
        assert_ne!(u32_at(&p, 28) & flags::BLOOM, 0);
        assert_eq!(u32_at(&p, 28) & flags::BLOOM_SNAP, 0);
        assert_eq!(f32_at(&p, 56), bloom.scale);
        assert_eq!(floats(&p, 64, 4), [1.0, 1.0, 1.0 / 96.0, 1.0 / 64.0]);
        s.output.filter = Filter::Nearest;
        assert_ne!(u32_at(&output_params(&s, &fr), 28) & flags::BLOOM_SNAP, 0);
    }

    #[test]
    fn shockwaves_fill_the_wave_tables() {
        let p = Projection {
            origin: Vec2::new(10.0, 20.0),
            x_axis: Vec2::new(2.0, 0.0),
            y_axis: Vec2::new(0.0, 1.0),
            z_axis: Vec2::new(0.0, 0.0),
        };
        let mut list = ShockwaveList::default();
        list.set(&[Shockwave {
            x: 40.0,
            y: 30.0,
            radius: 10.0,
            width: 8.0,
            strength: 1.0,
            ..Shockwave::default()
        }])
        .unwrap();
        let mut waves = ShockwaveFrame::default();
        collect_shockwaves(&list, &p, 200, 100, &mut waves);
        assert_eq!(waves.distortion_count, 1);
        let mut fr = frame((200, 100), (200, 100));
        fr.shockwaves = Some((&p, &waves));
        let mut s = PostSettings::default();
        s.output.filter = Filter::Nearest;
        let b = output_params(&s, &fr);
        let f = u32_at(&b, 28);
        assert_eq!(
            f & (flags::SHOCKWAVE | flags::SHOCKWAVE_SNAP),
            flags::SHOCKWAVE | flags::SHOCKWAVE_SNAP
        );
        assert_eq!(floats(&b, 192, 4), [2.0, 0.0, 0.0, 1.0]);
        assert_eq!(floats(&b, 208, 4), ground_inverse(&p).to_vec());
        assert_eq!(floats(&b, 224, 2), [10.0, 20.0]);
        assert_eq!(u32_at(&b, 232), 1);
        let DistortionWave { ring, strength } = waves.distortion[0];
        assert_eq!(floats(&b, 240, 4), ring.to_vec());
        assert_eq!(floats(&b, 496, 4), strength.to_vec());
        // Slot 1 stays zero.
        assert_eq!(floats(&b, 256, 4), [0.0; 4]);

        // Waves with no distortion in view change nothing.
        let none = ShockwaveFrame::default();
        fr.shockwaves = Some((&p, &none));
        let b = output_params(&s, &fr);
        assert_eq!(u32_at(&b, 28) & flags::SHOCKWAVE, 0);
        assert_eq!(u32_at(&b, 232), 0);
        assert_eq!(floats(&b, 192, 4), [0.0; 4]);
    }

    #[test]
    fn bloom_pass_blocks() {
        let b = prefilter_params(
            (192, 128),
            (96, 64),
            1.0,
            &BloomSettings {
                enabled: true,
                threshold: 2.0,
                knee: 0.5,
                intensity: 1.0,
                radius: 16.0,
            },
        );
        assert_eq!(floats(&b, 0, 4), [1.0 / 192.0, 1.0 / 128.0, 96.0, 64.0]);
        assert_eq!(floats(&b, 16, 4), [2.0, 2.0, 1.0, 0.0]);

        let d = down_params((96, 64), (48, 32));
        assert_eq!(floats(&d, 0, 4), [1.0, 1.0, 1.0 / 96.0, 1.0 / 64.0]);
        // Odd sizes: 25 -> 13 scales by 2 * 13 / 25.
        let d = down_params((25, 9), (13, 5));
        assert_eq!(floats(&d, 0, 2), [26.0 / 25.0, 10.0 / 9.0]);

        let u = up_params((48, 32), (96, 64), 0.25);
        assert_eq!(floats(&u, 0, 4), [1.0, 1.0, 1.0 / 48.0, 1.0 / 32.0]);
        assert_eq!(f32_at(&u, 16), 0.25);
        assert_eq!(&u[20..], &[0; 12]);
    }
}
