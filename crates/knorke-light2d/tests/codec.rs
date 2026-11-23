//! Origin: chiaro @ 9bc5b9b, tests/unit/PixelCodecTests.cpp and tests/unit/MaterialHelperTests.cpp
//! (zlib license, see `src/codec.rs`).
//!
//! The 8-bit rounding must stay bit-exact with the C++ library: the reference encoder below is
//! the C++ reference of the original test, ported line by line.

// Tests compare against platform `std` maths as an oracle and use `f32::max` in tolerances;
// only library code must be platform independent (ADR 0011).
#![allow(clippy::disallowed_methods, clippy::neg_cmp_op_on_partial_ord)]

use knorke_light2d::codec::*;
use knorke_math::Vec3;

fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

/// The header-only C++ code of Chiaro 0.3 (`ReferenceEncodeNormal`).
fn reference_encode_normal(n: Vec3) -> [u8; 2] {
    let (mut x, mut y, mut z) = (n.x, n.y, n.z);
    let len = (x * x + y * y + z * z).sqrt();
    if !(len > 0.0) || !len.is_finite() {
        return [128, 128];
    }
    x /= len;
    y /= len;
    z = (z / len).max(0.0);
    let sum = x.abs() + y.abs() + z;
    if !(sum > 0.0) {
        return [128, 128];
    }
    let (px, py) = (x / sum, y / sum);
    let byte = |q: f32| ((q * 127.0).round() as i32 + 128).clamp(1, 255) as u8;
    [byte(px + py), byte(px - py)]
}

fn fibonacci(i: usize, count: usize, scale: f32) -> Vec3 {
    let z = 1.0 - 2.0 * i as f32 / (count - 1) as f32;
    let r = (1.0 - z * z).max(0.0).sqrt();
    let a = 2.399_963_2_f32 * i as f32;
    v3(r * a.cos() * scale, r * a.sin() * scale, z * scale)
}

#[test]
fn the_normal_codec_equals_the_reference_bit_for_bit() {
    for i in 0..4000 {
        let n = fibonacci(i, 4000, 3.0);
        assert_eq!(encode_normal(Some(n)), reference_encode_normal(n), "{i}");
    }
    let cases = [
        v3(0., 0., 1.),
        v3(1., 0., 0.),
        v3(0., 1., 0.),
        v3(-1., 0., 0.),
        v3(0., -1., 0.),
        v3(1., 1., 0.),
        v3(1., 1., 1.),
        v3(0., 0., -1.),
        v3(0., 0., 0.),
        v3(1e-30, 0., 0.),
        v3(1e30, 1e30, 0.),
    ];
    for c in cases {
        assert_eq!(encode_normal(Some(c)), reference_encode_normal(c), "{c:?}");
    }
}

#[test]
fn up_is_exactly_128_128_and_decodes_exactly_up() {
    assert_eq!(
        encode_normal(Some(v3(0., 0., 1.))),
        [NORMAL_FLAT, NORMAL_FLAT]
    );
    let n = decode_normal(NORMAL_FLAT, NORMAL_FLAT);
    assert_eq!((n.x, n.y, n.z), (0.0, 0.0, 1.0));
}

#[test]
fn non_finite_zero_and_missing_input_give_the_up_normal() {
    let inf = f32::INFINITY;
    for b in [
        v3(inf, 0., 1.),
        v3(0., f32::NAN, 1.),
        v3(0., 0., -inf),
        v3(0., 0., 0.),
        v3(0., 0., -1.),
    ] {
        assert_eq!(encode_normal(Some(b)), [128, 128], "{b:?}");
    }
    assert_eq!(encode_normal(None), [128, 128]);
}

#[test]
fn decode_round_trips_to_within_a_byte_step() {
    for i in 0..500 {
        let n = fibonacci(i, 500, 1.0);
        if n.z < 0.0 {
            continue;
        }
        let m = decode_normal(encode_normal(Some(n))[0], encode_normal(Some(n))[1]);
        assert!((m - n).length() < 0.05, "{n:?} {m:?}");
    }
}

#[test]
fn surface_pixels() {
    let up = v3(0., 0., 1.);
    let east = v3(2., 0., 0.);
    assert_eq!(surface_pixel(10.4, 2.0, up), [5, 128, 128, 255]);
    let e = reference_encode_normal(east);
    assert_eq!(surface_pixel(1e6, 1.0, east), [255, e[0], e[1], 255]);
    assert_eq!(surface_pixel(-3.0, 1.0, v3(0., 0., 0.)), [0, 128, 128, 255]);
    assert_eq!(surface_pixel(f32::NAN, 1.0, up)[0], 0);
    // Height bytes round half away from zero like the C++ encoder.
    for i in 0..600 {
        let h = i as f32 * 0.37 - 5.0;
        let n = v3(
            (i as f32).cos(),
            (i as f32 * 1.3).sin(),
            0.2 + 0.001 * i as f32,
        );
        let q = (h / 0.5).round();
        let expect = if q > 0.0 { q.min(255.0) as u8 } else { 0 };
        let e = reference_encode_normal(n);
        assert_eq!(surface_pixel(h, 0.5, n), [expect, e[0], e[1], 255]);
    }
}

#[test]
fn material_pixel_and_the_b_byte() {
    let up = Some(v3(0., 0., 1.));
    assert_eq!(
        encode_material_pixel(up, 7, true, 15),
        [128, 128, 0xFF, 255]
    );
    assert_eq!(encode_material_pixel(up, 9, false, -1)[2], 0xE0); // clamped, not masked
    assert_eq!(encode_material_pixel(None, 0, false, 0), [128, 128, 0, 255]);
    for s in 0..8 {
        for m in 0..2 {
            for g in 0..16 {
                assert_eq!(
                    encode_material_pixel(up, s, m != 0, g)[2],
                    material_b(s as u32, m != 0, g as u32)
                );
            }
        }
    }
    assert_eq!(
        material_b(5, true, 9),
        (5 << MATERIAL_SPECULAR_SHIFT) | MATERIAL_METAL_BIT | 9
    );
    assert_eq!(
        (material_b(5, false, 9) & MATERIAL_SPECULAR_MASK) >> MATERIAL_SPECULAR_SHIFT,
        5
    );
    assert_eq!(material_b(5, false, 9) & MATERIAL_GLOSS_MASK, 9);
}

#[test]
fn material_pixel_equals_the_integer_encoder_for_every_quantized_value() {
    for sp in 0..8u32 {
        for m in [false, true] {
            for g in 0..16u32 {
                let mat = Material {
                    specular: sp as f32 / 7.0,
                    gloss: g as f32 / 15.0,
                    metal: m,
                    wet: false,
                };
                assert_eq!(
                    material_pixel(mat, None),
                    encode_material_pixel(None, sp as i32, m, g as i32)
                );
                let n = v3(0.3 * (sp as f32 - 3.0), -0.1 * (g as f32 - 7.0), 0.8);
                assert_eq!(
                    material_pixel(mat, Some(n)),
                    encode_material_pixel(Some(n), sp as i32, m, g as i32)
                );
            }
        }
    }
    for i in 0..2000 {
        let z = 1.0 - 2.0 * (i as f32 + 0.5) / 2000.0;
        let r = (1.0 - z * z).max(0.0).sqrt();
        let a = 2.399_963_2_f32 * i as f32;
        let n = v3(r * a.cos(), r * a.sin(), z);
        assert_eq!(
            material_pixel(Material::MATTE, Some(n)),
            encode_material_pixel(Some(n), 0, false, 0)
        );
    }
}

#[test]
fn material_quantization_boundaries_round_half_up_and_clamp() {
    let b = |specular: f32, gloss: f32| {
        material_pixel(
            Material {
                specular,
                gloss,
                metal: false,
                wet: false,
            },
            None,
        )[2]
    };
    assert_eq!(b(0.5 / 7.0 - 1e-4, 0.0), material_b(0, false, 0));
    assert_eq!(b(0.5 / 7.0 + 1e-4, 0.0), material_b(1, false, 0));
    assert_eq!(b(6.5 / 7.0 - 1e-4, 0.0), material_b(6, false, 0));
    assert_eq!(b(6.5 / 7.0 + 1e-4, 0.0), material_b(7, false, 0));
    assert_eq!(b(0.0, 0.5 / 15.0 - 1e-4), material_b(0, false, 0));
    assert_eq!(b(0.0, 0.5 / 15.0 + 1e-4), material_b(0, false, 1));
    assert_eq!(b(0.0, 14.5 / 15.0 + 1e-4), material_b(0, false, 15));
    assert_eq!(b(-1.0, -1.0), 0);
    assert_eq!(b(2.0, 9.0), material_b(7, false, 15));
    assert_eq!(b(f32::NAN, f32::NAN), 0);
    assert_eq!(b(f32::INFINITY, 0.0), material_b(7, false, 0));
}

#[test]
fn wet_materials_a_224_dry_255_and_the_wet_presets() {
    let mut m = Material {
        specular: 0.5,
        gloss: 0.5,
        metal: false,
        wet: true,
    };
    assert_eq!(material_pixel(m, None)[3], MATERIAL_A_WET);
    assert_eq!(material_color(m)[3], f32::from(MATERIAL_A_WET) / 255.0);
    m.wet = false;
    assert_eq!(material_pixel(m, None)[3], 255);
    assert_eq!(material_color(m)[3], 1.0);
    for p in [MaterialPreset::AsphaltWet, MaterialPreset::Puddle] {
        assert!(Material::preset(p).wet);
    }
    for p in [
        MaterialPreset::Glass,
        MaterialPreset::AsphaltDry,
        MaterialPreset::MetalPolished,
    ] {
        assert!(!Material::preset(p).wet);
    }
    // The normal map converter writes the same A.
    let normal = [128, 128, 255, 255];
    let opts = NormalMapOptions {
        material: Material::preset(MaterialPreset::Puddle),
        ..NormalMapOptions::default()
    };
    let mut out = [0u8; 4];
    convert_normal_map(&normal, 1, 1, 4, &opts, &mut out).unwrap();
    assert_eq!(out[3], MATERIAL_A_WET);
}

#[test]
fn material_presets() {
    let b = |p| material_pixel(Material::preset(p), None)[2];
    // re-tuned for the energy normalization (E49, Q13); 0.3 was (6, 0, 15)
    assert_eq!(b(MaterialPreset::AsphaltWet), material_b(4, false, 15));
    assert_eq!(b(MaterialPreset::Matte), 0);
    assert!(Material::preset(MaterialPreset::MetalPolished).metal);
    assert!(!Material::preset(MaterialPreset::Glass).metal);
    assert_eq!(Material::preset(MaterialPreset::Satin).specular, 0.3);
    assert_eq!(Material::preset(MaterialPreset::Puddle).gloss, 1.0);
    assert_eq!(MaterialPreset::ALL.len(), 9);
    // Every named constant equals its preset (C++ chiaro::material::*).
    assert_eq!(Material::MATTE, Material::preset(MaterialPreset::Matte));
    assert_eq!(Material::GLASS, Material::preset(MaterialPreset::Glass));
    assert_eq!(Material::PUDDLE, Material::preset(MaterialPreset::Puddle));
}

#[test]
fn vertex_colours_carry_the_pixel_bytes() {
    let n = v3(0.3, -0.4, 0.8);
    let m = Material::preset(MaterialPreset::Glass);
    let sp = surface_pixel(12.0, 2.0, n);
    let sc = surface_color(12.0, 2.0, n);
    for i in 0..3 {
        assert_eq!((sc[i] * 255.0).round() as u8, sp[i]);
    }
    assert_eq!(sc[3], 1.0);
    let mp = material_pixel(m, None);
    let mc = material_color(m);
    assert_eq!((mc[0] * 255.0).round() as u8, mp[0]);
    assert_eq!((mc[2] * 255.0).round() as u8, mp[2]);
}

#[test]
fn medium_pixel_layout() {
    assert_eq!(
        encode_medium_pixel(1.0, 0.4, 2.0, 3, 0.4),
        Ok([5, 254, 48, 255])
    );
    assert_eq!(
        encode_medium_pixel(0.5, 0.0, 0.0, 0, 1.0),
        Ok([0, 255, 0, 128])
    );
    assert_eq!(
        encode_medium_pixel(0.0, 0.0, 4.0, 15, 1.0),
        Ok([4, 255, 240, 0])
    );
    // Heights beyond 255 steps clamp; density clamps to [0, 1]; NaN is 0.
    assert_eq!(
        encode_medium_pixel(2.0, 1000.0, 5000.0, 1, 1.0),
        Ok([255, 0, 16, 255])
    );
    assert_eq!(
        encode_medium_pixel(-1.0, -3.0, -3.0, 1, 1.0),
        Ok([0, 255, 16, 0])
    );
    let nan = f32::NAN;
    assert_eq!(
        encode_medium_pixel(nan, nan, nan, 1, 1.0),
        Ok([0, 255, 16, 0])
    );
    let err = encode_medium_pixel(1.0, 0.0, 2.0, 16, 1.0).unwrap_err();
    assert!(err.to_string().contains("medium"));
}

#[test]
fn normal_map_options_defaults() {
    let o = NormalMapOptions::default();
    assert_eq!(o.flags, NormalMapFlags::default());
    assert!(!o.flags.green_down && !o.flags.flip_x && !o.flags.flip_y);
    assert_eq!(o.material, Material::MATTE);
    assert!(o.specular_map.is_none() && o.gloss_map.is_none() && o.metal_map.is_none());
    assert!(o.alpha.is_none());
}

#[test]
fn convert_a_normal_map() {
    // 3x1: transparent, flat, +x; maps make the middle pixel a specular metal.
    let input = [128, 128, 255, 0, 128, 128, 255, 255, 255, 128, 128, 255];
    let spec = [255u8, 255, 0];
    let metal = [0u8, 200, 0];
    let gloss = [0u8, 255, 0];
    let mut o = NormalMapOptions {
        specular_map: Some(&spec),
        metal_map: Some(&metal),
        gloss_map: Some(&gloss),
        ..NormalMapOptions::default()
    };
    let mut out = [0x55u8; 12];
    convert_normal_map(&input, 3, 1, 12, &o, &mut out).unwrap();
    assert_eq!(out[0..4], [0, 0, 0, 0]);
    assert_eq!(out[4..8], [128, 128, material_b(7, true, 15), 255]);
    let e = reference_encode_normal(v3(1., 0., 0.));
    assert_eq!(out[8..12], [e[0], e[1], material_b(0, false, 0), 255]);

    // Default options: matte; flip_x moves +x to the left and negates it.
    convert_normal_map(&input, 3, 1, 12, &NormalMapOptions::default(), &mut out).unwrap();
    assert_eq!(out[6], material_b(0, false, 0));
    o = NormalMapOptions {
        flags: NormalMapFlags {
            flip_x: true,
            ..NormalMapFlags::default()
        },
        ..NormalMapOptions::default()
    };
    convert_normal_map(&input, 3, 1, 12, &o, &mut out).unwrap();
    let n = decode_normal(out[0], out[1]);
    assert!(n.x < -0.99);
    assert_eq!(out[11], 0); // the transparent pixel moved to the right

    // Alpha from another image overrides the normal map alpha.
    let alpha = [0, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 255];
    let o = NormalMapOptions {
        alpha: Some(AlphaSource {
            rgba: &alpha,
            pitch: 12,
        }),
        ..NormalMapOptions::default()
    };
    convert_normal_map(&input, 3, 1, 12, &o, &mut out).unwrap();
    assert_eq!((out[3], out[7], out[11]), (255, 0, 255));
}

#[test]
fn invalid_normal_map_input_fails_writes_nothing_and_says_why() {
    let input = [128u8, 128, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut out = [0x55u8; 16];
    let o = NormalMapOptions::default();
    let message = |w, h, pitch, o: &NormalMapOptions, dst: &mut [u8]| {
        convert_normal_map(&input, w, h, pitch, o, dst)
            .unwrap_err()
            .to_string()
    };
    assert!(message(0, 1, 4, &o, &mut out).contains("0x1"));
    assert!(message(1, 70_000, 4, &o, &mut out).contains("1x70000"));
    assert!(message(2, 1, 4, &o, &mut out).contains("pitch 4"));
    let bad_alpha = NormalMapOptions {
        alpha: Some(AlphaSource {
            rgba: &input,
            pitch: 3,
        }),
        ..NormalMapOptions::default()
    };
    assert!(message(1, 1, 4, &bad_alpha, &mut out).contains("alpha pitch 3"));
    // Buffers that are too short are errors, never out-of-bounds reads or writes.
    assert!(message(1, 1, 4, &o, &mut out[..3]).contains("out"));
    let short_map = NormalMapOptions {
        specular_map: Some(&[]),
        ..NormalMapOptions::default()
    };
    assert!(message(1, 1, 4, &short_map, &mut out).contains("specular_map"));
    assert!(convert_normal_map(&input[..3], 1, 1, 4, &o, &mut out).is_err());
    assert!(out.iter().all(|&b| b == 0x55));
    assert_eq!(
        convert_normal_map(&input, 0, 1, 4, &o, &mut out)
            .unwrap_err()
            .to_diagnostic()
            .code
            .as_str(),
        "knorke.light2d.normal_map.size"
    );
}

#[test]
fn material_sprite_flat_map_encodes_exactly_like_the_up_normal() {
    let input = [128, 128, 255, 255];
    let mut out = [0u8; 4];
    convert_normal_map(&input, 1, 1, 4, &NormalMapOptions::default(), &mut out).unwrap();
    assert_eq!((out[0], out[1], out[3]), (128, 128, 255));
    for variant in 1..8 {
        let o = NormalMapOptions {
            flags: NormalMapFlags {
                green_down: variant & 1 != 0,
                flip_x: variant & 2 != 0,
                flip_y: variant & 4 != 0,
            },
            ..NormalMapOptions::default()
        };
        convert_normal_map(&input, 1, 1, 4, &o, &mut out).unwrap();
        assert_eq!((out[0], out[1]), (128, 128), "{variant}");
    }
}

#[test]
fn material_sprite_conventions() {
    // two pixels: +x normal, then flat; flip_x moves it to the right and negates x
    let input = [255, 128, 128, 255, 128, 128, 255, 255];
    let o = NormalMapOptions {
        flags: NormalMapFlags {
            flip_x: true,
            ..NormalMapFlags::default()
        },
        ..NormalMapOptions::default()
    };
    let mut out = [0u8; 8];
    convert_normal_map(&input, 2, 1, 8, &o, &mut out).unwrap();
    assert!(decode_normal(out[4], out[5]).x < -0.99);
    assert!(decode_normal(out[0], out[1]).x.abs() < 0.02);

    let input = [128, 255, 128, 255];
    let o = NormalMapOptions {
        flags: NormalMapFlags {
            green_down: true,
            ..NormalMapFlags::default()
        },
        ..NormalMapOptions::default()
    };
    let mut out = [0u8; 4];
    convert_normal_map(&input, 1, 1, 4, &o, &mut out).unwrap();
    assert!(decode_normal(out[0], out[1]).y < -0.99);

    // flip_y: rows swap and y is negated
    let input = [128, 255, 128, 255, 128, 128, 255, 255];
    let o = NormalMapOptions {
        flags: NormalMapFlags {
            flip_y: true,
            ..NormalMapFlags::default()
        },
        ..NormalMapOptions::default()
    };
    let mut out = [0u8; 8];
    convert_normal_map(&input, 1, 2, 4, &o, &mut out).unwrap();
    assert!(decode_normal(out[4], out[5]).y < -0.99);

    // +y normal with green_down and flip_y: negated twice, ends up +y again
    let input = [128, 255, 128, 255];
    let o = NormalMapOptions {
        flags: NormalMapFlags {
            green_down: true,
            flip_y: true,
            ..NormalMapFlags::default()
        },
        ..NormalMapOptions::default()
    };
    let mut out = [0u8; 4];
    convert_normal_map(&input, 1, 1, 4, &o, &mut out).unwrap();
    assert!(decode_normal(out[0], out[1]).y > 0.99);
}

#[test]
fn material_sprite_alpha_and_maps() {
    let input = [128, 128, 255, 0, 128, 128, 255, 255, 128, 128, 255, 255];
    let spec = [255u8, 255, 0];
    let metal = [0u8, 127, 128];
    let o = NormalMapOptions {
        specular_map: Some(&spec),
        metal_map: Some(&metal),
        ..NormalMapOptions::default()
    };
    let mut out = [0u8; 12];
    convert_normal_map(&input, 3, 1, 12, &o, &mut out).unwrap();
    assert_eq!(out[0..4], [0, 0, 0, 0]);
    assert_eq!((out[6] >> 5) & 7, 7);
    assert_eq!((out[6] >> 4) & 1, 0);
    assert_eq!((out[10] >> 4) & 1, 1);
    assert_eq!((out[10] >> 5) & 7, 0);
    assert_eq!(out[11], 255);

    // shape alpha overrides the normal map alpha
    let shape = [0, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 255];
    let o2 = NormalMapOptions {
        alpha: Some(AlphaSource {
            rgba: &shape,
            pitch: 12,
        }),
        ..NormalMapOptions::default()
    };
    convert_normal_map(&input, 3, 1, 12, &o2, &mut out).unwrap();
    assert_eq!((out[3], out[7], out[11]), (255, 0, 255));
}

#[test]
fn material_sprite_shape_in_source_orientation_with_flips() {
    // 3x2, asymmetric silhouette: only (0,0), (2,0) and (1,1) are inside
    const W: usize = 3;
    const H: usize = 2;
    let inside = [[1, 0, 1], [0, 1, 0]];
    let (mut nm, mut shape, mut opaque) = (Vec::new(), Vec::new(), Vec::new());
    for row in inside {
        for v in row {
            let a = if v == 1 { 255 } else { 0 };
            nm.extend_from_slice(&[128, 128, 255, a]);
            opaque.extend_from_slice(&[128, 128, 255, 255]);
            shape.extend_from_slice(&[0, 0, 0, a]);
        }
    }
    for mode in 0..3 {
        let flags = NormalMapFlags {
            flip_x: mode != 1,
            flip_y: mode != 0,
            ..NormalMapFlags::default()
        };
        let mut o = NormalMapOptions {
            flags,
            ..NormalMapOptions::default()
        };
        let mut via_map = vec![0u8; W * H * 4];
        convert_normal_map(&nm, W as u32, H as u32, W * 4, &o, &mut via_map).unwrap();
        o.alpha = Some(AlphaSource {
            rgba: &shape,
            pitch: W * 4,
        });
        let mut via_shape = vec![0u8; W * H * 4];
        convert_normal_map(&opaque, W as u32, H as u32, W * 4, &o, &mut via_shape).unwrap();
        assert_eq!(via_map, via_shape);
        for y in 0..H {
            for x in 0..W {
                let sx = if flags.flip_x { W - 1 - x } else { x };
                let sy = if flags.flip_y { H - 1 - y } else { y };
                assert_eq!(via_shape[(y * W + x) * 4 + 3] == 255, inside[sy][sx] == 1);
            }
        }
    }
}

#[test]
fn material_sprite_gloss_map() {
    let gloss = [255u8, 0];
    let o = NormalMapOptions {
        gloss_map: Some(&gloss),
        ..NormalMapOptions::default()
    };
    let input = [128, 128, 255, 255, 128, 128, 255, 255];
    let mut out = [0u8; 8];
    convert_normal_map(&input, 2, 1, 8, &o, &mut out).unwrap();
    assert_eq!(out[2] & 0x0F, 15);
    assert_eq!(out[6] & 0x0F, 0);
}
