//! The light program table the rules generate against Chiaro's (`cmake/ChiaroLightPrograms.cmake`,
//! `tests/unit/LightProgramLayoutTests.cpp`); the check against the shader pack is in
//! `light_program_pack.rs` (feature `gpu`).

use knorke_light2d::light_program::{
    LIGHT_PROGRAM_COUNT, LIGHT_PROGRAMS, LightBase, LightBase as B, LightFeature as F,
    LightFeatures, LightProgram, LightSampler, LightSecondTarget, LightSecondTarget as T,
    LightStorage, LightUniform, find_light_program, light_layout_of, light_program_index,
    light_program_spec,
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

/// Chiaro's generated table (revision 9bc5b9b, `cmake/ChiaroLightPrograms.cmake`), checked in
/// as the expectation for the table the rules in `light_program.rs` generate: base, features,
/// vertex shader, fragment shader, second target, in table order.
#[rustfmt::skip]
const CHIARO_TABLE: [(B, &[F], &str, &str, T); 92] = [
    (B::Light, &[], "light.vert", "light.frag", T::None),
    (B::Light, &[F::Surface], "light_surface.vert", "light_surface.frag", T::None),
    (B::Light, &[F::Surface, F::Exact], "light_surface.vert", "light_surface_exact.frag", T::Specular),
    (B::Light, &[F::Surface, F::Direction], "light_surface.vert", "light_surface_direction.frag", T::Direction),
    (B::Light, &[F::Mask], "light_mask.vert", "light_mask.frag", T::None),
    (B::Light, &[F::Surface, F::Mask], "light_surface_mask.vert", "light_surface_mask.frag", T::None),
    (B::Light, &[F::Surface, F::Exact, F::Mask], "light_surface_mask.vert", "light_surface_exact_mask.frag", T::Specular),
    (B::Light, &[F::Surface, F::Direction, F::Mask], "light_surface_mask.vert", "light_surface_direction_mask.frag", T::Direction),
    (B::Light, &[F::Medium], "light.vert", "light_medium.frag", T::None),
    (B::Light, &[F::Surface, F::Medium], "light_surface.vert", "light_surface_medium.frag", T::None),
    (B::Light, &[F::Surface, F::Exact, F::Medium], "light_surface.vert", "light_surface_exact_medium.frag", T::Specular),
    (B::Light, &[F::Surface, F::Direction, F::Medium], "light_surface.vert", "light_surface_direction_medium.frag", T::Direction),
    (B::Light, &[F::Mask, F::Medium], "light_mask.vert", "light_mask_medium.frag", T::None),
    (B::Light, &[F::Surface, F::Mask, F::Medium], "light_surface_mask.vert", "light_surface_mask_medium.frag", T::None),
    (B::Light, &[F::Surface, F::Exact, F::Mask, F::Medium], "light_surface_mask.vert", "light_surface_exact_mask_medium.frag", T::Specular),
    (B::Light, &[F::Surface, F::Direction, F::Mask, F::Medium], "light_surface_mask.vert", "light_surface_direction_mask_medium.frag", T::Direction),
    (B::LightSdf, &[], "light_point.vert", "light_sdf.frag", T::None),
    (B::LightSdf, &[F::Height], "light_point.vert", "light_sdf_height.frag", T::None),
    (B::LightSdf, &[F::Surface], "light_surface_point.vert", "light_sdf_surface.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Exact], "light_surface_point.vert", "light_sdf_surface_exact.frag", T::Specular),
    (B::LightSdf, &[F::Surface, F::Direction], "light_surface_point.vert", "light_sdf_surface_direction.frag", T::Direction),
    (B::LightSdf, &[F::Area], "light.vert", "light_sdf_area.frag", T::None),
    (B::LightSdf, &[F::Height, F::Area], "light.vert", "light_sdf_height_area.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Area], "light_surface.vert", "light_sdf_surface_area.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Exact, F::Area], "light_surface.vert", "light_sdf_surface_exact_area.frag", T::Specular),
    (B::LightSdf, &[F::Surface, F::Direction, F::Area], "light_surface.vert", "light_sdf_surface_direction_area.frag", T::Direction),
    (B::LightSdf, &[F::Mask], "light_mask.vert", "light_sdf_mask.frag", T::None),
    (B::LightSdf, &[F::Height, F::Mask], "light_mask.vert", "light_sdf_height_mask.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Mask], "light_surface_mask.vert", "light_sdf_surface_mask.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Exact, F::Mask], "light_surface_mask.vert", "light_sdf_surface_exact_mask.frag", T::Specular),
    (B::LightSdf, &[F::Surface, F::Direction, F::Mask], "light_surface_mask.vert", "light_sdf_surface_direction_mask.frag", T::Direction),
    (B::LightSdf, &[F::Pattern], "light_point.vert", "light_sdf_pattern.frag", T::None),
    (B::LightSdf, &[F::Height, F::Pattern], "light_point.vert", "light_sdf_height_pattern.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Pattern], "light_surface_point.vert", "light_sdf_surface_pattern.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Exact, F::Pattern], "light_surface_point.vert", "light_sdf_surface_exact_pattern.frag", T::Specular),
    (B::LightSdf, &[F::Surface, F::Direction, F::Pattern], "light_surface_point.vert", "light_sdf_surface_direction_pattern.frag", T::Direction),
    (B::LightSdf, &[F::Area, F::Pattern], "light.vert", "light_sdf_area_pattern.frag", T::None),
    (B::LightSdf, &[F::Height, F::Area, F::Pattern], "light.vert", "light_sdf_height_area_pattern.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Area, F::Pattern], "light_surface.vert", "light_sdf_surface_area_pattern.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Exact, F::Area, F::Pattern], "light_surface.vert", "light_sdf_surface_exact_area_pattern.frag", T::Specular),
    (B::LightSdf, &[F::Surface, F::Direction, F::Area, F::Pattern], "light_surface.vert", "light_sdf_surface_direction_area_pattern.frag", T::Direction),
    (B::LightSdf, &[F::Mask, F::Pattern], "light_mask.vert", "light_sdf_mask_pattern.frag", T::None),
    (B::LightSdf, &[F::Height, F::Mask, F::Pattern], "light_mask.vert", "light_sdf_height_mask_pattern.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Mask, F::Pattern], "light_surface_mask.vert", "light_sdf_surface_mask_pattern.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Exact, F::Mask, F::Pattern], "light_surface_mask.vert", "light_sdf_surface_exact_mask_pattern.frag", T::Specular),
    (B::LightSdf, &[F::Surface, F::Direction, F::Mask, F::Pattern], "light_surface_mask.vert", "light_sdf_surface_direction_mask_pattern.frag", T::Direction),
    (B::LightSdf, &[F::Medium], "light_point.vert", "light_sdf_medium.frag", T::None),
    (B::LightSdf, &[F::Height, F::Medium], "light_point.vert", "light_sdf_height_medium.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Medium], "light_surface_point.vert", "light_sdf_surface_medium.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Exact, F::Medium], "light_surface_point.vert", "light_sdf_surface_exact_medium.frag", T::Specular),
    (B::LightSdf, &[F::Surface, F::Direction, F::Medium], "light_surface_point.vert", "light_sdf_surface_direction_medium.frag", T::Direction),
    (B::LightSdf, &[F::Area, F::Medium], "light.vert", "light_sdf_area_medium.frag", T::None),
    (B::LightSdf, &[F::Height, F::Area, F::Medium], "light.vert", "light_sdf_height_area_medium.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Area, F::Medium], "light_surface.vert", "light_sdf_surface_area_medium.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Exact, F::Area, F::Medium], "light_surface.vert", "light_sdf_surface_exact_area_medium.frag", T::Specular),
    (B::LightSdf, &[F::Surface, F::Direction, F::Area, F::Medium], "light_surface.vert", "light_sdf_surface_direction_area_medium.frag", T::Direction),
    (B::LightSdf, &[F::Mask, F::Medium], "light_mask.vert", "light_sdf_mask_medium.frag", T::None),
    (B::LightSdf, &[F::Height, F::Mask, F::Medium], "light_mask.vert", "light_sdf_height_mask_medium.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Mask, F::Medium], "light_surface_mask.vert", "light_sdf_surface_mask_medium.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Exact, F::Mask, F::Medium], "light_surface_mask.vert", "light_sdf_surface_exact_mask_medium.frag", T::Specular),
    (B::LightSdf, &[F::Surface, F::Direction, F::Mask, F::Medium], "light_surface_mask.vert", "light_sdf_surface_direction_mask_medium.frag", T::Direction),
    (B::LightSdf, &[F::Pattern, F::Medium], "light_point.vert", "light_sdf_pattern_medium.frag", T::None),
    (B::LightSdf, &[F::Height, F::Pattern, F::Medium], "light_point.vert", "light_sdf_height_pattern_medium.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Pattern, F::Medium], "light_surface_point.vert", "light_sdf_surface_pattern_medium.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Exact, F::Pattern, F::Medium], "light_surface_point.vert", "light_sdf_surface_exact_pattern_medium.frag", T::Specular),
    (B::LightSdf, &[F::Surface, F::Direction, F::Pattern, F::Medium], "light_surface_point.vert", "light_sdf_surface_direction_pattern_medium.frag", T::Direction),
    (B::LightSdf, &[F::Area, F::Pattern, F::Medium], "light.vert", "light_sdf_area_pattern_medium.frag", T::None),
    (B::LightSdf, &[F::Height, F::Area, F::Pattern, F::Medium], "light.vert", "light_sdf_height_area_pattern_medium.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Area, F::Pattern, F::Medium], "light_surface.vert", "light_sdf_surface_area_pattern_medium.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Exact, F::Area, F::Pattern, F::Medium], "light_surface.vert", "light_sdf_surface_exact_area_pattern_medium.frag", T::Specular),
    (B::LightSdf, &[F::Surface, F::Direction, F::Area, F::Pattern, F::Medium], "light_surface.vert", "light_sdf_surface_direction_area_pattern_medium.frag", T::Direction),
    (B::LightSdf, &[F::Mask, F::Pattern, F::Medium], "light_mask.vert", "light_sdf_mask_pattern_medium.frag", T::None),
    (B::LightSdf, &[F::Height, F::Mask, F::Pattern, F::Medium], "light_mask.vert", "light_sdf_height_mask_pattern_medium.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Mask, F::Pattern, F::Medium], "light_surface_mask.vert", "light_sdf_surface_mask_pattern_medium.frag", T::None),
    (B::LightSdf, &[F::Surface, F::Exact, F::Mask, F::Pattern, F::Medium], "light_surface_mask.vert", "light_sdf_surface_exact_mask_pattern_medium.frag", T::Specular),
    (B::LightSdf, &[F::Surface, F::Direction, F::Mask, F::Pattern, F::Medium], "light_surface_mask.vert", "light_sdf_surface_direction_mask_pattern_medium.frag", T::Direction),
    (B::Directional, &[], "fullscreen.vert", "directional.frag", T::None),
    (B::Directional, &[F::Surface], "fullscreen.vert", "directional_surface.frag", T::None),
    (B::Directional, &[F::Surface, F::Exact], "fullscreen.vert", "directional_surface_exact.frag", T::Specular),
    (B::Directional, &[F::Surface, F::Direction], "fullscreen.vert", "directional_surface_direction.frag", T::Direction),
    (B::Directional, &[F::Pattern], "fullscreen.vert", "directional_pattern.frag", T::None),
    (B::Directional, &[F::Surface, F::Pattern], "fullscreen.vert", "directional_surface_pattern.frag", T::None),
    (B::Directional, &[F::Surface, F::Exact, F::Pattern], "fullscreen.vert", "directional_surface_exact_pattern.frag", T::Specular),
    (B::Directional, &[F::Surface, F::Direction, F::Pattern], "fullscreen.vert", "directional_surface_direction_pattern.frag", T::Direction),
    (B::Directional, &[F::Medium], "fullscreen.vert", "directional_medium.frag", T::None),
    (B::Directional, &[F::Surface, F::Medium], "fullscreen.vert", "directional_surface_medium.frag", T::None),
    (B::Directional, &[F::Surface, F::Exact, F::Medium], "fullscreen.vert", "directional_surface_exact_medium.frag", T::Specular),
    (B::Directional, &[F::Surface, F::Direction, F::Medium], "fullscreen.vert", "directional_surface_direction_medium.frag", T::Direction),
    (B::Directional, &[F::Pattern, F::Medium], "fullscreen.vert", "directional_pattern_medium.frag", T::None),
    (B::Directional, &[F::Surface, F::Pattern, F::Medium], "fullscreen.vert", "directional_surface_pattern_medium.frag", T::None),
    (B::Directional, &[F::Surface, F::Exact, F::Pattern, F::Medium], "fullscreen.vert", "directional_surface_exact_pattern_medium.frag", T::Specular),
    (B::Directional, &[F::Surface, F::Direction, F::Pattern, F::Medium], "fullscreen.vert", "directional_surface_direction_pattern_medium.frag", T::Direction),
];

#[test]
fn generated_table_matches_chiaros_in_order() {
    for (i, (info, &(base, features, vertex, fragment, second))) in
        LIGHT_PROGRAMS.iter().zip(&CHIARO_TABLE).enumerate()
    {
        let ctx = format!("entry {i}: {fragment}");
        assert_eq!(info.base, base, "{ctx}");
        assert_eq!(info.features, LightFeatures::of(features), "{ctx}");
        assert_eq!(info.vertex, vertex, "{ctx}");
        assert_eq!(info.fragment, fragment, "{ctx}");
        assert_eq!(info.second, second, "{ctx}");
    }
}

#[test]
fn find_and_index_cover_exactly_the_table() {
    for p in all_combinations() {
        let expected = CHIARO_TABLE.iter().position(|&(base, features, ..)| {
            p.base == base && p.features == LightFeatures::of(features)
        });
        assert_eq!(light_program_index(p), expected, "{p:?}");
        assert_eq!(find_light_program(p).is_some(), expected.is_some(), "{p:?}");
        assert_eq!(light_program_spec(p).is_some(), expected.is_some(), "{p:?}");
        if let Some(info) = find_light_program(p) {
            assert_eq!(info.program(), p);
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
