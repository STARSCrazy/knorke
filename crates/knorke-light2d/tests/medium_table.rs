//! Origin: chiaro @ 9bc5b9b, tests/unit/MediumTableTests.cpp (zlib license, see
//! `src/medium_table.rs`). The C API test of that file (NULL context, unknown preset value) has no
//! counterpart: presets are an enum, the table is a plain value.

use knorke_light2d::codec::MEDIUM_COUNT;
use knorke_light2d::medium_table::{MediumDesc, MediumError, MediumPreset, MediumTable};
use knorke_math::Vec3;

fn c(r: f32, g: f32, b: f32) -> Vec3 {
    Vec3::new(r, g, b)
}

#[test]
fn a_new_table_holds_the_presets_then_dust() {
    let t = MediumTable::new();
    for (i, preset) in MediumPreset::ALL.into_iter().enumerate() {
        assert_eq!(t.entries()[i], preset.desc(), "entry {i}");
    }
    for i in MediumPreset::ALL.len()..MEDIUM_COUNT as usize {
        assert_eq!(t.entries()[i], MediumPreset::Dust.desc(), "entry {i}");
    }
    assert_eq!(t.entries().len(), MEDIUM_COUNT as usize);
}

#[test]
fn preset_values() {
    let dust = MediumPreset::Dust.desc();
    assert_eq!(dust.color, c(0.55, 0.5, 0.42));
    // Extinctions per metre, tuned on smoke_demo and iso_night_demo.
    assert_eq!(dust.extinction, 0.5);
    assert_eq!(MediumPreset::Smoke.desc().extinction, 1.0);
    assert_eq!(MediumPreset::Soot.desc().extinction, 2.0);
    assert_eq!(MediumPreset::Steam.desc().extinction, 0.6);
    assert_eq!(MediumPreset::Fog.desc().extinction, 0.6);
    assert_eq!(MediumPreset::Smoke.desc().color, c(0.35, 0.35, 0.35));
    assert_eq!(MediumPreset::Soot.desc().color.y, 0.05);
    assert_eq!(MediumPreset::Steam.desc().color.z, 0.95);
    assert_eq!(MediumPreset::Fog.desc().color, c(0.9, 0.9, 0.9));
    // Henyey-Greenstein g.
    assert_eq!(dust.anisotropy, 0.5);
    assert_eq!(MediumPreset::Smoke.desc().anisotropy, 0.3);
    assert_eq!(MediumPreset::Soot.desc().anisotropy, 0.2);
    assert_eq!(MediumPreset::Steam.desc().anisotropy, 0.8);
    assert_eq!(MediumPreset::Fog.desc().anisotropy, 0.7);
}

#[test]
fn set_replaces_an_entry() {
    let mut t = MediumTable::new();
    let green = MediumDesc::new(c(0.2, 0.9, 0.1), 0.5, 0.0);
    t.set(15, green).unwrap();
    assert_eq!(*t.get(15).unwrap(), green);
    assert_eq!(*t.get(14).unwrap(), MediumPreset::Dust.desc());
    // The limits are valid.
    t.set(0, MediumDesc::new(c(0.0, 1.0, 0.0), 0.0, 0.9))
        .unwrap();
    t.set(0, MediumDesc::new(c(0.0, 1.0, 0.0), 0.0, -0.9))
        .unwrap();
}

#[test]
fn invalid_input_names_the_field_and_changes_nothing() {
    let mut t = MediumTable::new();
    let before = *t.get(3).unwrap();
    let fog = MediumPreset::Fog.desc();
    let nan = f32::NAN;

    let e = t.set(MEDIUM_COUNT, fog).unwrap_err();
    assert!(e.to_string().contains("index"), "{e}");
    assert!(matches!(t.get(MEDIUM_COUNT), Err(MediumError::Index(_))));

    for v in [-0.01, nan, f32::INFINITY] {
        let bad = MediumDesc {
            extinction: v,
            ..fog
        };
        let e = t.set(3, bad).unwrap_err();
        assert!(e.to_string().contains("extinction"), "{v}: {e}");
    }
    for v in [-0.01, 1.01, nan] {
        for channel in 0..3 {
            let mut bad = fog;
            match channel {
                0 => bad.color.x = v,
                1 => bad.color.y = v,
                _ => bad.color.z = v,
            }
            let e = t.set(3, bad).unwrap_err();
            assert!(e.to_string().contains("color"), "{v}: {e}");
        }
    }
    for v in [-0.91, 0.91, 1.0, nan] {
        let bad = MediumDesc {
            anisotropy: v,
            ..fog
        };
        let e = t.set(3, bad).unwrap_err();
        assert!(e.to_string().contains("medium.anisotropy"), "{v}: {e}");
    }
    assert_eq!(*t.get(3).unwrap(), before);
}

#[test]
fn errors_carry_stable_codes() {
    let mut t = MediumTable::new();
    let fog = MediumPreset::Fog.desc();
    let code = |e: MediumError| e.to_diagnostic().code.as_str().to_owned();
    let not_finite = MediumDesc {
        extinction: f32::NAN,
        ..fog
    };
    let negative = MediumDesc {
        extinction: -1.0,
        ..fog
    };
    assert_eq!(
        code(t.set(0, not_finite).unwrap_err()),
        "knorke.light2d.medium.not_finite"
    );
    assert_eq!(
        code(t.set(0, negative).unwrap_err()),
        "knorke.light2d.medium.out_of_range"
    );
    assert_eq!(
        code(t.set(99, fog).unwrap_err()),
        "knorke.light2d.medium.out_of_range"
    );
}
