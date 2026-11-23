//! Origin: chiaro @ 9bc5b9b, tests/unit/MaterialMathTests.cpp (zlib license, see `src/material.rs`).

// Tests compare against platform `std` maths as an oracle and use `f32::max` in tolerances;
// only library code must be platform independent (ADR 0011).
#![allow(clippy::disallowed_methods)]

use knorke_light2d::Projection;
use knorke_light2d::codec::{Material, MaterialPreset, material_pixel};
use knorke_light2d::material::*;
use knorke_math::{At, Vec2, Vec3, World2};

const PI: f32 = std::f32::consts::PI;

fn iso() -> Projection {
    Projection::iso(At::<World2, Vec2>::new(Vec2::ZERO), 2.0, 0.5, 640, 360)
}

fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

fn approx(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps * b.abs().max(1.0)
}

fn check_vec(v: Vec3, x: f32, y: f32, z: f32) {
    assert!(
        approx(v.x, x, 1e-5) && approx(v.y, y, 1e-5) && approx(v.z, z, 1e-5),
        "{v:?} != ({x}, {y}, {z})"
    );
}

#[test]
fn view_direction_test() {
    check_vec(view_direction(&Projection::IDENTITY), 0.0, 0.0, 1.0);
    let v = view_direction(&iso());
    assert!(v.z > 0.0);
    assert!(approx(v.x / v.z, 0.5, 1e-5));
    assert!(approx(v.y / v.z, 0.5, 1e-5));
    assert!(approx(v.length(), 1.0, 1e-5));
}

#[test]
fn screen_right_test() {
    check_vec(screen_right(&Projection::IDENTITY), 1.0, 0.0, 0.0);
    let s = 1.0 / 2.0_f32.sqrt();
    check_vec(screen_right(&iso()), s, -s, 0.0);
}

#[test]
fn tangent_frame_identity_top_down() {
    let p = Projection::IDENTITY;
    let f = tangent_frame(v3(0., 0., 1.), screen_right(&p), &p);
    check_vec(f.t, 1.0, 0.0, 0.0);
    check_vec(f.b, 0.0, -1.0, 0.0);
    check_vec(detail_to_world(&f, v3(0., 1., 0.)), 0.0, -1.0, 0.0);
}

#[test]
fn tangent_frame_iso_facade() {
    let p = iso();
    let f = tangent_frame(v3(1., 0., 0.), screen_right(&p), &p);
    assert!(f.t.z.abs() < 1e-6);
    check_vec(f.b, 0.0, 0.0, 1.0);
}

#[test]
fn tangent_frame_sloped_roof_is_continuous() {
    let p = iso();
    let az = 0.7_f32;
    let mut frames = Vec::new();
    for elev in [84.0_f32, 90.0] {
        let e = elev * PI / 180.0;
        let n = v3(e.cos() * az.cos(), e.cos() * az.sin(), e.sin());
        frames.push(tangent_frame(n, screen_right(&p), &p));
    }
    let c = frames[0].t.dot(frames[1].t).clamp(-1.0, 1.0);
    assert!(c.acos() * 180.0 / PI < 7.0);
}

#[test]
fn tangent_frame_degenerate_n_parallel_to_r() {
    let p = iso();
    let r = screen_right(&p);
    let f = tangent_frame(r, r, &p);
    for v in [f.t, f.b] {
        assert!(v.x.is_finite() && v.y.is_finite() && v.z.is_finite());
        assert!(approx(v.length(), 1.0, 1e-5));
    }
    assert!(f.t.dot(f.n).abs() < 1e-5);
    assert!(f.b.dot(f.n).abs() < 1e-5);
    assert!(f.t.dot(f.b).abs() < 1e-5);
}

#[test]
fn material_bits() {
    let m = decode_material_bits(0xFF);
    assert!(approx(m.strength, 1.0, 1e-5) && m.metal && approx(m.exponent, 256.0, 1e-5));
    let m = decode_material_bits(0x00);
    assert!(m.strength == 0.0 && !m.metal && approx(m.exponent, 2.0, 1e-5));
    let m = decode_material_bits(0x2F);
    assert!(approx(m.strength, 1.0 / 7.0, 1e-5) && !m.metal && approx(m.exponent, 256.0, 1e-5));
}

#[test]
fn toksvig() {
    assert!(approx(toksvig_exponent(64.0, 1.0), 64.0, 1e-5));
    assert!(toksvig_exponent(256.0, 0.9) < 20.0);
    assert!(toksvig_exponent(2.0, 0.01) >= 1.0);
    assert!(toksvig_exponent(256.0, 0.0) >= 1.0);
}

#[test]
fn specular_term_test() {
    let up = v3(0., 0., 1.);
    assert!(approx(
        specular_term(up, up, up, 32.0),
        specular_normalization(32.0),
        1e-5
    ));
    assert_eq!(specular_term(up, v3(0., 0., -1.), up, 32.0), 0.0);
    // light at the pixel: no direction, no specular highlight
    assert_eq!(specular_term(up, v3(0., 0., 0.), up, 32.0), 0.0);
}

#[test]
fn wet_specular_stretch() {
    let up = v3(0., 0., 1.);
    let right = v3(1., 0., 0.); // across is (0, 1, 0)
    let v = v3(0.0, 0.5, 0.75_f32.sqrt());
    let e = 256.0;
    let mirror = v3(0.0, -0.5, 0.75_f32.sqrt());
    let wet = |l, stretch| wet_specular_term(up, l, v, e, right, 0.0, 0.0, stretch, 0.0, 1.0);
    assert!(approx(wet(mirror, 1.0), specular_normalization(e), 1e-4));
    assert!(approx(
        wet(mirror, 4.0),
        specular_normalization(e) / 2.0,
        1e-4
    ));
    let dir = |x: f32, y: f32, z: f32| {
        let l = (x * x + y * y + z * z).sqrt();
        v3(x / l, y / l, z / l)
    };
    let off_across = dir(0.0, -0.8, 0.75_f32.sqrt());
    let off_along = dir(0.12, -0.5, 0.75_f32.sqrt());
    let plain = specular_term(up, off_across, v, e);
    assert!((wet(off_across, 1.0) - plain).abs() <= 0.1 * plain);
    assert!(wet(off_across, 8.0) > 3.0 * plain / 8.0_f32.sqrt());
    let along_plain = wet(off_along, 1.0);
    assert!(along_plain > 0.05);
    assert!(wet(off_along, 8.0) < 0.1 * along_plain);
    assert_eq!(wet(v3(0., 0., -1.), 8.0), 0.0);
}

#[test]
fn wet_specular_breakup_is_deterministic_bounded_and_varies() {
    let up = v3(0., 0., 1.);
    let right = v3(1., 0., 0.);
    let v = v3(0.0, 0.5, 0.75_f32.sqrt());
    let mirror = v3(0.0, -0.5, 0.75_f32.sqrt());
    let peak = specular_normalization(32.0) / 6.0_f32.sqrt();
    let (mut lo, mut hi) = (1e9_f32, 0.0_f32);
    for i in 0..200 {
        let (x, y) = (0.37 * i as f32, 0.11 * i as f32);
        let f = wet_specular_term(up, mirror, v, 32.0, right, x, y, 6.0, 1.0, 2.0);
        assert_eq!(
            f,
            wet_specular_term(up, mirror, v, 32.0, right, x, y, 6.0, 1.0, 2.0)
        );
        assert!(f >= 0.0 && f <= 2.0 * peak + 1e-4);
        lo = lo.min(f);
        hi = hi.max(f);
    }
    assert!(lo < 0.3 * peak, "gaps");
    assert!(hi > 1.5 * peak, "bright strands");
    for i in 0..50 {
        let n = wet_noise(0.73 * i as f32, -1.9 * i as f32);
        assert!((0.0..=1.0).contains(&n));
    }
}

/// Sum of a lobe over the hemisphere of light directions, seen along the normal (v = n = up),
/// midpoint rule in the tangent plane of h.
fn lobe_sum(lobe: impl Fn(Vec3) -> f64) -> f64 {
    const N: usize = 600;
    let step = 2.0 / N as f64;
    let mut sum = 0.0;
    for i in 0..N {
        for j in 0..N {
            let tx = -1.0 + (i as f64 + 0.5) * step;
            let ty = -1.0 + (j as f64 + 0.5) * step;
            let r2 = tx * tx + ty * ty;
            if r2 >= 1.0 {
                continue; // theta_h >= 45 degrees: the light is below the horizon
            }
            let hl = (1.0 + r2).sqrt();
            let h = v3((tx / hl) as f32, (ty / hl) as f32, (1.0 / hl) as f32);
            let l = v3(2.0 * h.z * h.x, 2.0 * h.z * h.y, 2.0 * h.z * h.z - 1.0);
            sum += lobe(l) * 4.0 / ((1.0 + r2) * (1.0 + r2)) * step * step;
        }
    }
    sum
}

#[test]
fn energy_normalization_the_lobe_sums_to_pi_for_every_gloss_step() {
    let up = v3(0., 0., 1.);
    for g in 0..=15 {
        let e = (1.0 + g as f32 * 7.0 / 15.0).exp2();
        let sum = lobe_sum(|l| f64::from(specular_term(up, l, up, e)));
        assert!(
            (sum - f64::from(PI)).abs() <= 0.02 * f64::from(PI),
            "e = {e}: {sum}"
        );
    }
}

#[test]
fn energy_normalization_the_wet_streak_is_never_brighter_in_sum() {
    let up = v3(0., 0., 1.);
    let right = v3(1., 0., 0.);
    for e in [32.0_f32, 128.0, 256.0] {
        for stretch in [1.5_f32, 2.0, 4.0, 8.0] {
            let sum = lobe_sum(|l| {
                f64::from(wet_specular_term(
                    up, l, up, e, right, 0.0, 0.0, stretch, 0.0, 1.0,
                ))
            });
            assert!(
                sum <= f64::from(PI) * 1.02,
                "e {e} stretch {stretch}: {sum}"
            );
            // The streak's length across is stretch / sqrt(e) in tan(theta_h); the hemisphere ends at 1.
            if stretch / e.sqrt() <= 0.15 {
                assert!((sum - f64::from(PI)).abs() <= 0.03 * f64::from(PI), "{sum}");
            }
        }
    }
}

#[test]
fn peak_table_of_the_presets_under_intensity_1() {
    // Peak = quantized specular * N(e): the highlight of a light of intensity 1 straight in the
    // mirror direction, specular_scale 1.
    let rows = [
        (MaterialPreset::Matte, 0.0),
        (MaterialPreset::Satin, 0.57),
        (MaterialPreset::Plastic, 2.77),
        (MaterialPreset::Glass, 20.06),
        (MaterialPreset::MetalDull, 2.04),
        (MaterialPreset::MetalPolished, 23.41),
        (MaterialPreset::AsphaltDry, 0.22),
        (MaterialPreset::AsphaltWet, 18.43),
        (MaterialPreset::Puddle, 32.25),
    ];
    for (preset, expected) in rows {
        let b = material_pixel(Material::preset(preset), None)[2];
        let bits = decode_material_bits(b);
        let peak = bits.strength * specular_normalization(bits.exponent);
        assert!(
            (peak - expected).abs() <= 0.005 * expected.max(1.0e-3) + 1e-6,
            "{preset:?}: {peak} vs {expected}"
        );
    }
}
