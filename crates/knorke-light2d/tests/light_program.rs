//! The light program table against its rules (Chiaro `cmake/ChiaroLightPrograms.cmake`,
//! `tests/unit/LightProgramLayoutTests.cpp`); the check against the shader pack is in
//! `light_program_pack.rs` (feature `gpu`).

use knorke_light2d::light_program::{
    LIGHT_PROGRAM_COUNT, LIGHT_PROGRAMS, LightBase, LightFeature as F, LightFeatures, LightProgram,
    LightSampler, LightSecondTarget, LightStorage, LightUniform, find_light_program,
    light_layout_of, light_program_index, light_program_spec,
};

fn all_combinations() -> impl Iterator<Item = LightProgram> {
    LightBase::ALL.into_iter().flat_map(|base| {
        (0..=255u8).map(move |bits| LightProgram {
            base,
            features: LightFeatures::from_bits(bits),
        })
    })
}

#[test]
fn table_has_92_programs() {
    assert_eq!(LIGHT_PROGRAMS.len(), 92);
    assert_eq!(LIGHT_PROGRAM_COUNT, 92);
}

#[test]
fn rules_reproduce_the_table_in_order() {
    let valid: Vec<_> = all_combinations()
        .filter_map(|p| light_program_spec(p).map(|s| (p, s)))
        .collect();
    assert_eq!(valid.len(), LIGHT_PROGRAMS.len());
    for (info, (program, spec)) in LIGHT_PROGRAMS.iter().zip(&valid) {
        assert_eq!(info.program(), *program);
        assert_eq!(info.vertex, spec.vertex, "{}", info.fragment);
        assert_eq!(info.fragment, spec.fragment);
        assert_eq!(info.second, spec.second, "{}", info.fragment);
    }
}

#[test]
fn find_and_index_cover_exactly_the_table() {
    for p in all_combinations() {
        let found = find_light_program(p);
        assert_eq!(found.is_some(), light_program_spec(p).is_some(), "{p:?}");
        if let Some(info) = found {
            assert_eq!(info.program(), p);
            assert_eq!(
                &LIGHT_PROGRAMS[light_program_index(p).unwrap()].program(),
                &p
            );
        }
    }
}

#[test]
fn fragment_names_are_unique() {
    let mut names: Vec<_> = LIGHT_PROGRAMS.iter().map(|i| i.fragment).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 92);
}

#[test]
fn rules_reject_chiaros_invalid_combinations() {
    let sdf = LightProgram::new(LightBase::LightSdf);
    let light = LightProgram::new(LightBase::Light);
    let dir = LightProgram::new(LightBase::Directional);
    assert!(find_light_program(light.with(F::Exact, true)).is_none());
    assert!(
        find_light_program(
            light
                .with(F::Surface, true)
                .with(F::Exact, true)
                .with(F::Direction, true)
        )
        .is_none()
    );
    assert!(find_light_program(light.with(F::Height, true)).is_none());
    assert!(find_light_program(sdf.with(F::Height, true).with(F::Surface, true)).is_none());
    assert!(find_light_program(light.with(F::Area, true)).is_none());
    assert!(find_light_program(sdf.with(F::Area, true).with(F::Mask, true)).is_none());
    assert!(find_light_program(dir.with(F::Mask, true)).is_none());
    assert!(find_light_program(light.with(F::Pattern, true)).is_none());
    assert!(find_light_program(sdf.with(F::Height, true)).is_some());
}

#[test]
fn spec_examples_match_chiaro() {
    let p = LightProgram::new(LightBase::LightSdf)
        .with(F::Surface, true)
        .with(F::Exact, true)
        .with(F::Mask, true);
    let s = light_program_spec(p).unwrap();
    assert_eq!(s.fragment, "light_sdf_surface_exact_mask.frag");
    assert_eq!(s.source, "light_sdf_surface.frag");
    assert_eq!(s.vertex, "light_surface_mask.vert");
    assert_eq!(
        s.defines,
        ["CHI_MATERIAL_EXACT", "CHI_POINT_LIGHTS", "CHI_LIGHT_MASK"]
    );
    assert_eq!(s.second, LightSecondTarget::Specular);
    assert!(s.is_variant());

    let plain = light_program_spec(LightProgram::new(LightBase::Light)).unwrap();
    assert!(!plain.is_variant());
    assert_eq!(plain.vertex, "light.vert");

    let area =
        light_program_spec(LightProgram::new(LightBase::LightSdf).with(F::Area, true)).unwrap();
    assert_eq!(area.vertex, "light.vert");
    assert!(area.defines.is_empty());
    assert!(area.is_variant()); // own name, source light_sdf.frag
}

#[test]
fn second_targets_follow_material_mode() {
    for info in &LIGHT_PROGRAMS {
        let want = if info.features.contains(F::Exact) {
            LightSecondTarget::Specular
        } else if info.features.contains(F::Direction) {
            LightSecondTarget::Direction
        } else {
            LightSecondTarget::None
        };
        assert_eq!(info.second, want);
    }
}

#[test]
fn layouts_number_slots_densely() {
    for info in &LIGHT_PROGRAMS {
        let l = light_layout_of(info.program());
        let mut samplers: Vec<u8> = LightSampler::ALL
            .iter()
            .filter_map(|&s| l.sampler(s))
            .collect();
        samplers.sort_unstable();
        assert_eq!(samplers, (0..l.sampler_count).collect::<Vec<_>>());
        let mut uniforms: Vec<u8> = LightUniform::ALL
            .iter()
            .filter_map(|&u| l.uniform(u))
            .collect();
        uniforms.sort_unstable();
        assert_eq!(uniforms, (0..l.uniform_count).collect::<Vec<_>>());
        let vertex = LightStorage::ALL
            .iter()
            .filter(|b| b.is_vertex() && l.storage(**b).is_some())
            .count();
        assert_eq!(vertex, usize::from(l.vertex_storage_count));
    }
}

#[test]
fn layout_examples() {
    let full = light_layout_of(LightProgram {
        base: LightBase::LightSdf,
        features: LightFeatures::of(&[F::Surface, F::Exact, F::Mask, F::Medium, F::Pattern]),
    });
    assert_eq!(full.sampler_count, 10);
    assert_eq!(full.sampler(LightSampler::Surface), Some(0));
    assert_eq!(full.sampler(LightSampler::OcclusionHeight), Some(3));
    assert_eq!(full.sampler(LightSampler::Material), Some(4));
    assert_eq!(full.sampler(LightSampler::PatternAtlas), Some(9));
    assert_eq!(full.uniform(LightUniform::Pattern), Some(3));
    assert_eq!(full.storage(LightStorage::Masks), Some(1));
    assert_eq!(full.storage(LightStorage::PatternData), Some(2));
    assert_eq!(full.fragment_storage_count, 3);

    let dir = light_layout_of(LightProgram::new(LightBase::Directional).with(F::Surface, true));
    assert_eq!(dir.sampler(LightSampler::Surface), Some(3));
    assert_eq!(dir.sampler(LightSampler::Profile), None);
    assert_eq!(dir.uniform(LightUniform::Surface), Some(1));
    assert_eq!(dir.vertex_uniform_count, 0);
    assert_eq!(dir.storage(LightStorage::Lights), None);

    let plain = light_layout_of(LightProgram::new(LightBase::Light));
    assert_eq!(plain.sampler_count, 1);
    assert_eq!(plain.sampler(LightSampler::Profile), Some(0));
    assert_eq!(plain.vertex_uniform_count, 1);
}
