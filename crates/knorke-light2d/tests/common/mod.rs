//! Shared by the GPU test binaries of knorke-light2d (each binary includes it with
//! `mod common;`, so items one binary does not use are fine).
#![allow(dead_code)]

use std::collections::BTreeMap;

use knorke_gpu::kpak::{Program, Stage};
use knorke_light2d::codec::{AlphaSource, Material, NormalMapOptions, convert_normal_map};
use knorke_light2d::light_program::{LightLayout, LightSampler, LightStorage, LightUniform};
use knorke_math::{At, Vec2, World2};

pub(crate) fn at(x: f32, y: f32) -> At<World2, Vec2> {
    At::new(Vec2::new(x, y))
}

/// `name` must be bound at `(set, slot)` when the layout gives it a slot, and must not exist
/// in the shader otherwise.
fn check_binding(
    bindings: &BTreeMap<String, (u32, u32)>,
    name: &str,
    set: u32,
    slot: Option<u8>,
    ctx: &str,
) {
    match slot {
        None => assert!(!bindings.contains_key(name), "{ctx}: unexpected {name}"),
        Some(s) => assert_eq!(
            bindings.get(name),
            Some(&(set, u32::from(s))),
            "{ctx}: {name}"
        ),
    }
}

/// The reflection counts of a fragment/vertex pair and the descriptor set and binding of every
/// named resource are the ones a [`LightLayout`] gives (port of Chiaro's
/// `LightProgramLayoutTests.cpp`; the composite programs share the layout type).
pub(crate) fn check_layout(frag: &Program<'_>, vert: &Program<'_>, l: &LightLayout) {
    let ctx = format!("{} with {}", frag.name, vert.name);
    assert_eq!(frag.stage, Stage::Fragment, "{ctx}");
    assert_eq!(vert.stage, Stage::Vertex, "{ctx}");
    let (f, v) = (frag.reflection, vert.reflection);
    assert_eq!(f.samplers, u16::from(l.sampler_count), "{ctx}: samplers");
    assert_eq!(
        f.uniform_buffers,
        u16::from(l.uniform_count),
        "{ctx}: uniforms"
    );
    assert_eq!(
        f.storage_buffers,
        u16::from(l.fragment_storage_count),
        "{ctx}: storage"
    );
    assert_eq!(f.storage_textures, 0, "{ctx}");
    assert_eq!(v.samplers, 0, "{ctx}");
    assert_eq!(
        v.storage_buffers,
        u16::from(l.vertex_storage_count),
        "{ctx}: vertex storage"
    );
    assert_eq!(
        v.uniform_buffers,
        u16::from(l.vertex_uniform_count),
        "{ctx}: vertex uniforms"
    );

    // SDL's SPIR-V convention: fragment samplers then storage buffers in set 2, fragment
    // uniforms in set 3, vertex storage buffers in set 0.
    let fb = knorke_gpu::test_support::spirv::decode(frag.code).bindings;
    let vb = knorke_gpu::test_support::spirv::decode(vert.code).bindings;
    for s in LightSampler::ALL {
        check_binding(&fb, s.hlsl_name(), 2, l.sampler(s), &ctx);
    }
    for u in LightUniform::ALL {
        check_binding(&fb, u.hlsl_name(), 3, l.uniform(u), &ctx);
    }
    for b in LightStorage::ALL {
        let slot = l.storage(b);
        if b.is_vertex() {
            check_binding(&vb, b.hlsl_name(), 0, slot, &ctx);
        } else {
            check_binding(
                &fb,
                b.hlsl_name(),
                2,
                slot.map(|s| l.sampler_count + s),
                &ctx,
            );
        }
    }
}

/// The dome normal map of `Material()` in the dumpers `light_ref.cpp` and `composite_ref.cpp`, as
/// material pixels (satin): a half sphere of radius 20 around (48, 24).
pub(crate) fn dome_material_pixels(w: u32, h: u32) -> Vec<u8> {
    let mut normal = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f32 + 0.5 - 48.0) / 20.0;
            let dy = (y as f32 + 0.5 - 24.0) / 20.0;
            let d2 = dx * dx + dy * dy;
            let n = if d2 < 1.0 {
                [dx, -dy, (1.0 - d2).sqrt()]
            } else {
                [0.0, 0.0, 1.0]
            };
            let p = ((y * w + x) * 4) as usize;
            for i in 0..3 {
                normal[p + i] = ((n[i] * 0.5 + 0.5) * 255.0).round() as u8;
            }
            normal[p + 3] = 255;
        }
    }
    let color = vec![255u8; normal.len()];
    let options = NormalMapOptions {
        material: Material::SATIN,
        alpha: Some(AlphaSource {
            rgba: &color,
            pitch: (w * 4) as usize,
        }),
        ..Default::default()
    };
    let mut out = vec![0u8; normal.len()];
    convert_normal_map(&normal, w, h, (w * 4) as usize, &options, &mut out).unwrap();
    out
}
