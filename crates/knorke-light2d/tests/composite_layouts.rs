//! The composite pass (P27) against Chiaro's compiled shaders (port of the composite part of
//! `tests/unit/LightProgramLayoutTests.cpp`, E160), for each of the six composite programs in the
//! SPIR-V pack:
//!
//! - the reflection counts and the descriptor set and binding of every named resource are the
//!   slots [`composite_layout_of`] gives;
//! - the member offsets the compiler gave `cbuffer CompositeParams` (`OpMemberDecorate Offset`)
//!   equal the offsets of [`CompositeParams`], member by member (a field written at the wrong
//!   place gives silent image errors);
//! - all six pipelines build lazily on a real device.
//!
//! Needs the pack from `cargo xtask shaders` in `target/shaders/` (Chiaro 9bc5b9b shaders, zlib
//! license); skips without it, the pipeline test also without a Vulkan device.
#![cfg(feature = "gpu")]

use core::mem::{offset_of, size_of};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use knorke_gpu::kpak::{Program, ShaderPack, Stage};
use knorke_gpu::sys::gpu::SDL_GPU_SHADERFORMAT_SPIRV;
use knorke_gpu::{Device, Gpu, LiveCounter, ShaderLibrary, init_video};
use knorke_light2d::composite::{
    COMPOSITE_PROGRAMS, CompositeParams, CompositeStage, composite_layout_of,
};
use knorke_light2d::light_program::{LightLayout, LightSampler, LightUniform};

static SERIAL: Mutex<()> = Mutex::new(());

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

/// One decoded SPIR-V module: names, decorations and struct member offsets.
struct Module {
    bindings: BTreeMap<String, (u32, u32)>,
    structs: Vec<(String, Vec<(String, u32)>)>,
}

fn words(code: &[u8]) -> Vec<u32> {
    let w: Vec<u32> = code
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_le_bytes(*c))
        .collect();
    assert_eq!(w[0], 0x0723_0203, "not SPIR-V");
    w
}

fn string(ws: &[u32]) -> String {
    let bytes: Vec<u8> = ws.iter().flat_map(|w| w.to_le_bytes()).collect();
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8(bytes[..end].to_vec()).unwrap()
}

fn decode(code: &[u8]) -> Module {
    let words = words(code);
    let mut names = BTreeMap::new();
    let mut members = Vec::new();
    let mut offsets = Vec::new();
    let mut decorations: BTreeMap<u32, (Option<u32>, Option<u32>)> = BTreeMap::new();
    let mut i = 5;
    while i < words.len() {
        let (n, op) = ((words[i] >> 16) as usize, words[i] & 0xFFFF);
        assert!(n > 0, "malformed SPIR-V");
        let a = &words[i + 1..i + n];
        match op {
            5 => {
                names.insert(a[0], string(&a[1..]));
            }
            6 => members.push((a[0], a[1], string(&a[2..]))),
            71 if n >= 4 => {
                let d = decorations.entry(a[0]).or_default();
                match a[1] {
                    34 => d.0 = Some(a[2]), // DescriptorSet
                    33 => d.1 = Some(a[2]), // Binding
                    _ => {}
                }
            }
            72 if a[2] == 35 => offsets.push((a[0], a[1], a[3])),
            _ => {}
        }
        i += n;
    }
    let bindings = decorations
        .into_iter()
        .filter_map(|(id, (set, binding))| {
            Some((names.get(&id)?.clone(), (set.unwrap_or(u32::MAX), binding?)))
        })
        .collect();
    let structs = names
        .iter()
        .map(|(&id, name)| {
            let mut m: Vec<(u32, String, u32)> = offsets
                .iter()
                .filter(|o| o.0 == id)
                .map(|&(_, member, off)| {
                    let mname = members
                        .iter()
                        .find(|x| x.0 == id && x.1 == member)
                        .map_or_else(String::new, |x| x.2.clone());
                    (member, mname, off)
                })
                .collect();
            m.sort_by_key(|x| x.0);
            (
                name.clone(),
                m.into_iter().map(|(_, n, o)| (n, o)).collect(),
            )
        })
        .collect();
    Module { bindings, structs }
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
    // SDL's SPIR-V convention: fragment samplers in set 2, fragment uniforms in set 3.
    let fb = decode(frag.code).bindings;
    for s in LightSampler::ALL {
        check_binding(&fb, s.hlsl_name(), 2, l.sampler(s), &ctx);
    }
    for u in LightUniform::ALL {
        check_binding(&fb, u.hlsl_name(), 3, l.uniform(u), &ctx);
    }
}

#[test]
fn every_composite_program_layout_matches_its_shaders() {
    let Some(bytes) = pack_bytes() else { return };
    let pack = ShaderPack::parse(&bytes).unwrap();
    for program in COMPOSITE_PROGRAMS {
        let frag = pack.get(program.fragment()).unwrap();
        let vert = pack.get(program.vertex()).unwrap();
        check_layout(&frag, &vert, &composite_layout_of(program));
    }
}

/// `(HLSL member, Rust offset, Rust size)` of `CompositeParams`.
const MEMBERS: [(&str, usize, usize); 16] = [
    ("DecodeSrgb", offset_of!(CompositeParams, decode_srgb), 4),
    (
        "UseLightBuffer",
        offset_of!(CompositeParams, use_light_buffer),
        4,
    ),
    ("UseGlow", offset_of!(CompositeParams, use_glow), 4),
    ("DecodeGlow", offset_of!(CompositeParams, decode_glow), 4),
    ("Ambient", offset_of!(CompositeParams, ambient), 16),
    (
        "GlowFromScene",
        offset_of!(CompositeParams, glow_from_scene),
        4,
    ),
    ("GlowScale", offset_of!(CompositeParams, glow_scale), 4),
    ("UseSpecular", offset_of!(CompositeParams, use_specular), 4),
    (
        "UseDirection",
        offset_of!(CompositeParams, use_direction),
        4,
    ),
    (
        "ViewDirSpec",
        offset_of!(CompositeParams, view_dir_spec),
        16,
    ),
    ("ScreenRight", offset_of!(CompositeParams, screen_right), 16),
    ("XYAxes", offset_of!(CompositeParams, xy_axes), 16),
    ("ZAxisScene", offset_of!(CompositeParams, z_axis_scene), 16),
    ("InvAxes", offset_of!(CompositeParams, inv_axes), 16),
    ("OriginStep", offset_of!(CompositeParams, origin_step), 16),
    ("Wet", offset_of!(CompositeParams, wet), 16),
];

#[test]
fn rust_layout_matches_every_composite_shader() {
    let Some(bytes) = pack_bytes() else { return };
    let pack = ShaderPack::parse(&bytes).unwrap();
    assert_eq!(size_of::<CompositeParams>(), 160);
    for program in COMPOSITE_PROGRAMS {
        let shader = program.fragment();
        let module = decode(pack.get(shader).unwrap().code);
        let (_, members) = module
            .structs
            .iter()
            .find(|s| s.0 == "type.CompositeParams")
            .unwrap_or_else(|| panic!("{shader}: no CompositeParams"));
        // The compiler keeps every member, used or not.
        assert_eq!(members.len(), MEMBERS.len(), "{shader}");
        for (member, offset) in members {
            let &(_, rust, len) = MEMBERS
                .iter()
                .find(|m| m.0 == member)
                .unwrap_or_else(|| panic!("{shader}: CompositeParams.{member} has no Rust field"));
            assert_eq!(
                *offset as usize, rust,
                "{shader}: CompositeParams.{member} at {offset}, Rust at {rust}"
            );
            assert!(rust + len <= size_of::<CompositeParams>());
        }
    }
}

#[test]
fn all_composite_pipelines_build_lazily() {
    let Some(bytes) = pack_bytes() else { return };
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init_video(Some(c"vulkan"));
    let device = match Device::create(SDL_GPU_SHADERFORMAT_SPIRV, false) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("skipped: no Vulkan device ({})", e.message);
            return;
        }
    };
    let counter = LiveCounter::new();
    {
        let gpu = Gpu::new(&device, &counter);
        let mut shaders = ShaderLibrary::new(gpu, bytes).unwrap();
        // The stage resolves all six programs up front and creates none of the pipelines.
        let stage = CompositeStage::new(gpu, &shaders).unwrap();
        assert!(stage.hdr_buffer().is_none());
        assert_eq!(stage.pipeline_count(), 0, "lazy: nothing created up front");
        for (n, program) in COMPOSITE_PROGRAMS.iter().enumerate() {
            let mut stage = CompositeStage::new(gpu, &shaders).unwrap();
            stage
                .prepare_pipeline(&mut shaders, *program)
                .unwrap_or_else(|e| panic!("{program:?}: {}", e.message));
            assert_eq!(stage.pipeline_count(), 1, "{program:?} (#{n})");
        }
    }
    assert_eq!(counter.live(), 0, "GPU objects leaked");
}
