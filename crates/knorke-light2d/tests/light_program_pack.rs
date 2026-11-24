//! The light program family against Chiaro's compiled shaders (port of
//! `tests/unit/LightProgramLayoutTests.cpp`, light part): every program's shaders exist in
//! the pack, their reflection counts match [`light_layout_of`], and the SPIR-V descriptor set
//! and binding of every named resource is the slot the layout gives. Then all 92 pipelines
//! are created on a real device.
//!
//! Needs the full pack `target/shaders/knorke-chiaro.spirv.kpak`
//! (`cargo xtask shaders --src <chiaro>/shaders/compiled --formats spirv`); skips (passes)
//! without it, and the pipeline test also without a Vulkan device.

#![cfg(feature = "gpu")]

use std::collections::BTreeMap;
use std::path::Path;

use knorke_gpu::kpak::{Program, ShaderPack, Stage};
use knorke_gpu::sys::gpu::{SDL_GPU_SHADERFORMAT_SPIRV, SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT};
use knorke_gpu::{Device, Gpu, LiveCounter, ShaderLibrary, init_video};
use knorke_light2d::light_pipelines::{LIGHT_PROGRAM_UNKNOWN, LightPipelines};
use knorke_light2d::light_program::{
    LIGHT_PROGRAMS, LightBase, LightFeature, LightLayout, LightProgram, LightSampler, LightStorage,
    LightUniform, light_layout_of,
};

fn pack_bytes() -> Option<Vec<u8>> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shaders/knorke-chiaro.spirv.kpak");
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!(
                "skipped: no shader pack at {} (cargo xtask shaders)",
                path.display()
            );
            None
        }
    }
}

/// Name -> (descriptor set, binding) of every decorated variable in a SPIR-V module.
fn spirv_bindings(code: &[u8]) -> BTreeMap<String, (u32, u32)> {
    let word = |i: usize| u32::from_le_bytes(code[i * 4..i * 4 + 4].try_into().unwrap());
    let words = code.len() / 4;
    let mut names = BTreeMap::new();
    let mut decorations: BTreeMap<u32, (Option<u32>, Option<u32>)> = BTreeMap::new();
    let mut i = 5;
    while i < words {
        let op = word(i) & 0xffff;
        let n = (word(i) >> 16) as usize;
        if n == 0 {
            break;
        }
        if op == 5 && n > 2 {
            // OpName
            let text = &code[(i + 2) * 4..(i + n) * 4];
            let end = text.iter().position(|&b| b == 0).unwrap_or(text.len());
            names.insert(
                word(i + 1),
                String::from_utf8_lossy(&text[..end]).into_owned(),
            );
        } else if op == 71 && n >= 4 {
            // OpDecorate
            let d = decorations.entry(word(i + 1)).or_default();
            match word(i + 2) {
                34 => d.0 = Some(word(i + 3)), // DescriptorSet
                33 => d.1 = Some(word(i + 3)), // Binding
                _ => {}
            }
        }
        i += n;
    }
    decorations
        .into_iter()
        .filter_map(|(id, (set, binding))| {
            Some((names.get(&id)?.clone(), (set.unwrap_or(u32::MAX), binding?)))
        })
        .collect()
}

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

fn check_layout(frag: &Program<'_>, vert: &Program<'_>, l: &LightLayout) {
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
    let fb = spirv_bindings(frag.code);
    let vb = spirv_bindings(vert.code);
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

#[test]
fn every_light_program_layout_matches_its_shaders() {
    let Some(bytes) = pack_bytes() else { return };
    let pack = ShaderPack::parse(&bytes).unwrap();
    for info in &LIGHT_PROGRAMS {
        let frag = pack.get(info.fragment).unwrap();
        let vert = pack.get(info.vertex).unwrap();
        check_layout(&frag, &vert, &light_layout_of(info.program()));
    }
}

#[test]
fn all_light_pipelines_build_lazily() {
    let Some(bytes) = pack_bytes() else { return };
    init_video(Some(c"vulkan"));
    let device = match Device::create(SDL_GPU_SHADERFORMAT_SPIRV, false) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skipped: no Vulkan device ({})", e.message);
            return;
        }
    };
    let counter = LiveCounter::new();
    let gpu = Gpu::new(&device, &counter);
    let mut shaders = ShaderLibrary::new(gpu, bytes).unwrap();
    let mut pipelines = LightPipelines::new(&shaders).unwrap();
    assert!(pipelines.is_empty(), "lazy: nothing created up front");

    let target = SDL_GPU_TEXTUREFORMAT_R16G16B16A16_FLOAT;
    for info in &LIGHT_PROGRAMS {
        let p = info.program();
        assert!(pipelines.pipeline(p, target).is_none());
        pipelines.prepare(&mut shaders, p, target).unwrap();
        assert!(pipelines.contains(p));
        assert!(pipelines.pipeline(p, target).is_some());
    }
    assert_eq!(pipelines.len(), 92);
    // Preparing again reuses the pipeline.
    pipelines
        .prepare(&mut shaders, LIGHT_PROGRAMS[0].program(), target)
        .unwrap();
    assert_eq!(pipelines.len(), 92);

    let invalid = LightProgram::new(LightBase::Light).with(LightFeature::Exact, true);
    let err = pipelines
        .prepare(&mut shaders, invalid, target)
        .unwrap_err();
    assert_eq!(err.code, LIGHT_PROGRAM_UNKNOWN);
    assert!(!pipelines.contains(invalid));
}
