//! Checks the committed test packs (`tests/data/test.*.kpak`) and, when it was built, the
//! full Chiaro pack (`target/shaders/knorke-chiaro.*.kpak`, from `cargo xtask shaders`).
//!
//! The reflection contracts are ported from Chiaro tests/gpu/ShaderLibraryTests.cpp
//! (revision 9bc5b9b); they run against the full pack and skip when it is absent.

use std::path::PathBuf;

use knorke_gpu::kpak::{Reflection, ShaderFormat, ShaderPack, Stage};

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

#[test]
fn committed_test_packs_are_valid() {
    for format in ShaderFormat::ALL {
        let bytes = std::fs::read(data(&format.pack_file_name("test"))).unwrap();
        let pack = ShaderPack::parse(&bytes).unwrap();
        assert_eq!(pack.format(), format);
        let names: Vec<_> = pack.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            [
                "clear.frag",
                "fullscreen.vert",
                "jfa_seed.frag",
                "solid.frag"
            ]
        );
        let vert = pack.get("fullscreen.vert").unwrap();
        assert_eq!(vert.stage, Stage::Vertex);
        assert_eq!(vert.reflection, Reflection::default());
        assert_eq!(vert.entry_point, format.default_entry_point());
        let seed = pack.get("jfa_seed.frag").unwrap();
        assert_eq!(seed.stage, Stage::Fragment);
        assert_eq!(
            (seed.reflection.samplers, seed.reflection.uniform_buffers),
            (1, 1)
        );
        if format == ShaderFormat::Spirv {
            assert_eq!(
                &vert.code[..4],
                &0x0723_0203u32.to_le_bytes(),
                "SPIR-V magic"
            );
        }
    }
}

fn full_pack(format: ShaderFormat) -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/shaders")
        .join(format.pack_file_name("knorke-chiaro"));
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) => {
            eprintln!(
                "skipped: {} missing (run `cargo xtask shaders --src <chiaro>/shaders/compiled`)",
                path.display()
            );
            None
        }
    }
}

fn r(samplers: u16, storage_buffers: u16, uniform_buffers: u16) -> Reflection {
    Reflection {
        samplers,
        storage_textures: 0,
        storage_buffers,
        uniform_buffers,
    }
}

#[test]
fn reflection_counts_match_the_shader_contracts() {
    let Some(bytes) = full_pack(ShaderFormat::Spirv) else {
        return;
    };
    let pack = ShaderPack::parse(&bytes).unwrap();
    let get = |n: &str| pack.get(n).unwrap();
    let check = |n: &str, stage: Stage, want: Reflection| {
        let p = get(n);
        assert_eq!(p.stage, stage, "{n}");
        assert_eq!(p.reflection, want, "{n}");
    };
    use Stage::{Fragment as F, Vertex as V};
    check("fullscreen.vert", V, r(0, 0, 0));
    // scene, light buffer, glow buffer, material, specular/direction sums, surface buffer
    check("composite.frag", F, r(6, 0, 1));
    check("output.frag", F, r(3, 0, 1)); // HDR, overlay, bloom level 0
    check("light.vert", V, r(0, 1, 1));
    check("light.frag", F, r(1, 0, 0)); // the profile atlas
    // occlusion
    check("occluder.vert", V, r(0, 1, 1));
    check("tile.vert", V, r(0, 0, 1));
    check("tile.frag", F, r(1, 0, 1));
    check("occlusion_mask.frag", F, r(1, 0, 1));
    check("solid.frag", F, r(0, 0, 0));
    // shadows
    check("light_sdf.frag", F, r(3, 0, 1));
    for n in ["jfa_seed.frag", "jfa_step.frag", "jfa_distance.frag"] {
        check(n, F, r(1, 0, 1));
    }
    check("rect.vert", V, r(0, 0, 1));
    check("clear.frag", F, r(0, 0, 0));
    // bloom
    for n in ["bloom_prefilter.frag", "bloom_down.frag", "bloom_up.frag"] {
        check(n, F, r(1, 0, 1));
    }
    // exact and direction material variants
    for (base, samplers) in [
        ("light_surface", 2),
        ("light_sdf_surface", 5),
        ("directional_surface", 4),
    ] {
        let plain = get(&format!("{base}.frag"));
        let exact = get(&format!("{base}_exact.frag"));
        let direction = get(&format!("{base}_direction.frag"));
        assert_eq!(plain.reflection.samplers, samplers, "{base}");
        assert_eq!(exact.reflection.samplers, samplers + 1, "{base}");
        assert_eq!(
            exact.reflection.uniform_buffers,
            plain.reflection.uniform_buffers
        );
        assert_eq!(direction.reflection, plain.reflection, "{base}");
    }
    // ambient zones
    check("ambient_zone.vert", V, r(0, 1, 1));
    for n in ["ambient_zone.frag", "ambient_zone_dual.frag"] {
        check(n, F, r(2, 0, 1));
    }
    for (plain, zones) in [
        ("composite.frag", "composite_zones.frag"),
        ("composite_medium.frag", "composite_medium_zones.frag"),
        (
            "composite_medium_nearest.frag",
            "composite_medium_nearest_zones.frag",
        ),
        ("medium_base.frag", "medium_base_zones.frag"),
    ] {
        let (p, z) = (get(plain), get(zones));
        assert_eq!(z.reflection.samplers, p.reflection.samplers + 1, "{zones}");
        let extra = u16::from(plain == "medium_base.frag");
        assert_eq!(
            z.reflection.uniform_buffers,
            p.reflection.uniform_buffers + extra
        );
        assert_eq!(z.reflection.storage_buffers, 0);
    }
    // pattern twins: +2 samplers, 3 storage buffers, +1 uniform block
    let mut twins = 0;
    for p in pack.iter().filter(|p| p.name.contains("_pattern")) {
        let plain = p.name.replace("_pattern", "");
        let Ok(base) = pack.get(&plain) else {
            continue;
        };
        if p.name.ends_with("_id.frag") {
            continue;
        }
        twins += 1;
        assert_eq!(
            p.reflection.samplers,
            base.reflection.samplers + 2,
            "{}",
            p.name
        );
        assert_eq!(
            p.reflection.uniform_buffers,
            base.reflection.uniform_buffers + 1
        );
        assert_eq!(p.reflection.storage_buffers, 3, "{}", p.name);
        assert!(p.reflection.uniform_buffers <= 4, "SDL_GPU limit per stage");
    }
    assert!(twins > 0);
    check("pattern_id.frag", F, r(0, 0, 0));
    check("tile_pattern_id.frag", F, r(1, 2, 1));
    for p in pack.iter() {
        assert_eq!(p.reflection.storage_textures, 0, "{}", p.name);
    }
}

#[test]
fn msl_blobs_use_the_main0_entry_point() {
    let Some(bytes) = full_pack(ShaderFormat::Msl) else {
        return;
    };
    let pack = ShaderPack::parse(&bytes).unwrap();
    for p in pack.iter() {
        assert_eq!(p.entry_point, "main0");
        let text = std::str::from_utf8(p.code).unwrap();
        assert!(text.contains("main0("), "{}", p.name);
    }
}
