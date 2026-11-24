//! IES and EULUMDAT import (chiaro ProfileImportTests, ported).

// Origin: chiaro @ 9bc5b9b, tests/unit/ProfileImportTests.cpp (zlib license, see `src/profile_import.rs`).
//
// Ported test by test with the same inputs, expected values and tolerances (doctest `Approx`
// semantics: |a - b| < eps * (1 + max(|a|, |b|)), default eps = 100 * f32::EPSILON). The fixtures
// are the files of `tests/data/profiles` (copied from chiaro's tests/unit/data/profiles).
// Error texts keep the C++ wording; the "Chiaro: " prefix of the SDL error is gone.

// Test oracle maths and the `f32::max` of the reference comparison, as in the C++ original.
#![allow(clippy::disallowed_methods)]

use knorke_light2d::profile::{PROFILE_VALUES, sample_profile_table};
use knorke_light2d::profile_import::{ImportError, import_eulumdat, import_ies};

fn approx_eps(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() < eps * (1.0 + a.abs().max(b.abs()))
}

fn approx(a: f32, b: f32) -> bool {
    approx_eps(a, b, 100.0 * f32::EPSILON)
}

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/data/profiles/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn ies(text: &[u8]) -> Vec<f32> {
    let t = import_ies(text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(t.len(), PROFILE_VALUES);
    t
}

fn ldt(text: &[u8]) -> Vec<f32> {
    let t = import_eulumdat(text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(t.len(), PROFILE_VALUES);
    t
}

fn ies_error(text: &[u8]) -> String {
    let err: ImportError = import_ies(text).expect_err("IES must be rejected");
    err.to_string()
}

fn ldt_error(text: &[u8]) -> String {
    let err: ImportError = import_eulumdat(text).expect_err("EULUMDAT must be rejected");
    err.to_string()
}

fn has(s: &str, part: &str) -> bool {
    s.contains(part)
}

fn s(t: &[f32], g: f32, c: f32) -> f32 {
    sample_profile_table(t, g, c)
}

/// A minimal IES body after TILT=NONE with the given angles and candela values (type C).
fn ies_body(counts: &str, angles: &str, candela: &str) -> Vec<u8> {
    ies_body_with_type(counts, angles, candela, 1)
}

/// As [`ies_body`], with the photometric type given.
fn ies_body_with_type(counts: &str, angles: &str, candela: &str, ptype: i32) -> Vec<u8> {
    format!(
        "IESNA:LM-63-2002\n[TEST] x\nTILT=NONE\n1 1000 1 {counts} {ptype} 2 0 0 0\n1 1 10\n{angles}\n{candela}\n"
    )
    .into_bytes()
}

/// A EULUMDAT file with one lamp set; `tail` holds the C angles, gamma angles and intensities.
fn ldt_file(isym: i32, mc: i32, ng: i32, tail: &str) -> Vec<u8> {
    let mut s = format!(
        "test\n1\n{isym}\n{mc}\n{}\n{ng}\n30\n",
        if mc > 0 { 360 / mc } else { 0 }
    );
    s.push_str("r\nname\nnum\nfile\ndate\n");
    for _ in 0..13 {
        s.push_str("0\n");
    }
    s.push_str("1\n1\nLED\n1000\n3000\n80\n10\n");
    for _ in 0..10 {
        s.push_str("0.5\n");
    }
    s.push_str(tail);
    s.into_bytes()
}

#[test]
fn ies_rotationally_symmetric_hat_lamp() {
    let t = ies(&fixture("hat_lamp.ies"));
    let max = t.iter().copied().fold(f32::MIN, f32::max);
    assert!(approx(max, 1.0));
    assert!(approx(s(&t, 70.0, 0.0), 1.0)); // 340 cd is the peak
    assert!(approx(s(&t, 0.0, 0.0), 100.0 / 340.0));
    assert!(approx(s(&t, 75.0, 0.0), 330.0 / 340.0)); // linear between 70 and 80
    assert_eq!(s(&t, 110.0, 0.0), 0.0);
    assert!(approx(s(&t, 60.0, 123.0), s(&t, 60.0, 0.0))); // the same in every plane
}

#[test]
fn ies_quadrant_symmetry_tilt_include_and_commas() {
    let t = ies(&fixture("street_quadrant.ies"));
    // Peak 260 at 60 degrees in C0; C45 and C90 are mirrored into all quadrants.
    assert!(approx(s(&t, 60.0, 0.0), 1.0));
    assert!(approx(s(&t, 60.0, 180.0), 1.0));
    assert!(approx(s(&t, 60.0, 90.0), 60.0 / 260.0));
    assert!(approx(s(&t, 60.0, 270.0), 60.0 / 260.0));
    assert!(approx(s(&t, 60.0, 45.0), 120.0 / 260.0));
    assert!(approx(s(&t, 60.0, 135.0), 120.0 / 260.0));
    assert!(approx(s(&t, 60.0, 315.0), 120.0 / 260.0));
    assert_eq!(s(&t, 95.0, 0.0), 0.0); // measured only down to the horizontal
}

#[test]
fn ies_symmetric_about_the_c0_c180_plane() {
    let t = ies(&fixture("bilateral.ies"));
    // At 45 degrees: C0 10, C45 20, C90 30, C135 40, C180 50; C270 mirrors C90.
    assert!(approx(s(&t, 45.0, 0.0), 0.2));
    assert!(approx(s(&t, 45.0, 180.0), 1.0));
    assert!(approx(s(&t, 45.0, 90.0), 0.6));
    assert!(approx(s(&t, 45.0, 270.0), 0.6));
    assert!(approx(s(&t, 45.0, 315.0), 0.4));
    assert!(approx(s(&t, 0.0, 200.0), 0.2));
}

#[test]
fn ies_full_circle_with_crlf_line_ends() {
    let t = ies(&fixture("full.ies"));
    assert!(approx(s(&t, 90.0, 0.0), 0.25));
    assert!(approx(s(&t, 90.0, 90.0), 0.5));
    assert!(approx(s(&t, 90.0, 180.0), 0.75));
    assert!(approx(s(&t, 90.0, 270.0), 1.0));
    assert!(approx(s(&t, 90.0, 315.0), 0.625)); // between C270 (40) and C360 = C0 (10)
}

#[test]
fn ies_crlf_line_ends_give_the_same_table_as_lf() {
    // The repository stores fixtures with LF (`eol=lf`), so the CRLF of `full.ies` is made here.
    let lf: Vec<u8> = fixture("full.ies")
        .into_iter()
        .filter(|&b| b != b'\r')
        .collect();
    let crlf: Vec<u8> = lf
        .iter()
        .flat_map(|&b| {
            if b == b'\n' {
                vec![b'\r', b'\n']
            } else {
                vec![b]
            }
        })
        .collect();
    assert_eq!(ies(&crlf), ies(&lf));
}

#[test]
fn ies_symmetric_about_the_c90_c270_plane_lf_bom_older_header() {
    let text = b"\xEF\xBB\xBFIESNA91\n[TEST] x\nTILT=NONE\n1 1000 1 2 3 1 2 0 0 0\n1 1 10\n\
                 0 90\n90 180 270\n1 10\n1 20\n1 30\n";
    let t = ies(text);
    assert!(approx(s(&t, 90.0, 90.0), 10.0 / 30.0));
    assert!(approx(s(&t, 90.0, 180.0), 20.0 / 30.0));
    assert!(approx(s(&t, 90.0, 270.0), 1.0));
    assert!(approx(s(&t, 90.0, 0.0), 20.0 / 30.0)); // mirror of C180
    assert!(approx(s(&t, 90.0, 315.0), s(&t, 90.0, 225.0)));
}

#[test]
fn ies_lm_63_1986_without_keywords() {
    let t =
        ies(b"some luminaire\nmore text\nTILT=NONE\n1 1000 1 2 1 1 2 0 0 0 1 1 10 0 90 0 5 10\n");
    assert!(approx(s(&t, 90.0, 0.0), 1.0));
    assert!(approx(s(&t, 0.0, 0.0), 0.5));
}

#[test]
fn ies_errors_name_the_problem() {
    assert!(has(&ies_error(b""), "no data"));
    assert!(has(&ies_error(b"hello\nworld\n"), "no TILT= line"));
    assert!(has(
        &ies_error(b"IESNA:LM-63-2002\nTILT=lamp.tlt\n"),
        "separate tilt file"
    ));
    assert!(has(
        &ies_error(&ies_body_with_type("2 1", "0 90 0", "1 1", 3)),
        "photometric type 3"
    ));
    assert!(has(
        &ies_error(&ies_body("2 1", "0 90 0", "1")),
        "ends after 1 of 2 candela values"
    ));
    assert!(has(
        &ies_error(&ies_body("2 1", "0 x 0", "1 2")),
        "expected vertical angles, got 'x'"
    ));
    assert!(has(
        &ies_error(&ies_body("2 1", "90 0 0", "1 2")),
        "vertical angles must increase"
    ));
    assert!(has(
        &ies_error(&ies_body("2 1", "0 200 0", "1 2")),
        "outside [0, 180]"
    ));
    assert!(has(
        &ies_error(&ies_body("2 2", "0 90 0 45", "1 2 3 4")),
        "not a supported symmetry"
    ));
    assert!(has(
        &ies_error(&ies_body("2 1", "0 90 0", "0 0")),
        "no light"
    ));
    assert!(has(
        &ies_error(&ies_body("2 1", "0 90 0", "1 -2")),
        "must be >= 0"
    ));
    assert!(has(&ies_error(&ies_body("0 1", "0", "")), "0 vertical"));
    assert!(has(
        &ies_error(b"IESNA:LM-63-2002\nTILT=NONE\n1 1000 1 2.5 1 1 2 0 0 0 1 1 1\n"),
        "number of vertical angles (a whole number), got '2.5'"
    ));
    // The line number is the one of the bad token.
    assert!(has(
        &ies_error(
            b"IESNA:LM-63-2002\nTILT=NONE\n1 1000 1 2 1 1 2 0 0 0\n1 1 10\n0 90\n0\n1 oops\n"
        ),
        "IES line 7"
    ));
}

#[test]
fn eulumdat_c0_to_c270_street_lamp() {
    let t = ldt(&fixture("led_street.ldt"));
    assert!(approx(s(&t, 60.0, 0.0), 1.0)); // 350 is the peak
    assert!(approx(s(&t, 60.0, 180.0), 1.0));
    assert!(approx(s(&t, 60.0, 90.0), 120.0 / 350.0));
    assert!(approx(s(&t, 60.0, 270.0), 5.0 / 350.0));
    assert!(approx(s(&t, 60.0, 45.0), 235.0 / 350.0)); // between C0 and C90
    assert!(approx(s(&t, 60.0, 315.0), 177.5 / 350.0)); // between C270 and C360 = C0
    assert_eq!(s(&t, 91.0, 0.0), 0.0);
}

#[test]
fn eulumdat_rotational_symmetry_with_decimal_commas() {
    let t = ldt(&fixture("symmetric.ldt"));
    assert!(approx(s(&t, 0.0, 0.0), 1.0));
    assert!(approx(s(&t, 30.0, 77.0), 400.5 / 500.0));
    assert!(approx(s(&t, 60.0, 300.0), 100.25 / 500.0));
}

#[test]
fn eulumdat_isym_2_3_and_4_fill_the_missing_planes_by_mirroring() {
    // Mc = 4 (C0, C90, C180, C270), Ng = 2 (0, 90 degrees), the value at 90 degrees marks the plane.
    // Isym 2: C0 to C180 stored, C270 mirrors C90
    let t = ldt(&ldt_file(
        2,
        4,
        2,
        "0\n90\n180\n270\n0\n90\n1\n10\n1\n20\n1\n30\n",
    ));
    assert!(approx(s(&t, 90.0, 90.0), 20.0 / 30.0));
    assert!(approx(s(&t, 90.0, 270.0), 20.0 / 30.0));
    assert!(approx(s(&t, 90.0, 180.0), 1.0));

    // Isym 3: C270, C0, C90 stored, C180 mirrors C0
    let t = ldt(&ldt_file(
        3,
        4,
        2,
        "0\n90\n180\n270\n0\n90\n1\n10\n1\n20\n1\n30\n",
    ));
    assert!(approx(s(&t, 90.0, 270.0), 10.0 / 30.0));
    assert!(approx(s(&t, 90.0, 0.0), 20.0 / 30.0));
    assert!(approx(s(&t, 90.0, 90.0), 1.0));
    assert!(approx(s(&t, 90.0, 180.0), 20.0 / 30.0));

    // Isym 4: C0 to C90 stored, mirrored into all quadrants
    let t = ldt(&ldt_file(4, 4, 2, "0\n90\n180\n270\n0\n90\n1\n10\n1\n20\n"));
    assert!(approx(s(&t, 90.0, 0.0), 0.5));
    assert!(approx(s(&t, 90.0, 90.0), 1.0));
    assert!(approx(s(&t, 90.0, 180.0), 0.5));
    assert!(approx(s(&t, 90.0, 270.0), 1.0));
}

#[test]
fn eulumdat_several_intensities_on_one_line() {
    let t = ldt(&ldt_file(1, 1, 3, "0\n0 45 90\n10 20 5\n"));
    assert!(approx(s(&t, 45.0, 0.0), 1.0));
}

#[test]
fn eulumdat_errors_name_the_line_and_the_field() {
    assert!(has(&ldt_error(b""), "no data"));
    assert!(has(
        &ldt_error(b"only\n1\n"),
        "ends before line 3 (symmetry indicator Isym)"
    ));
    assert!(has(
        &ldt_error(b"x\n1\nzwei\n"),
        "line 3 (symmetry indicator Isym): expected a whole number, got 'zwei'"
    ));
    assert!(has(
        &ldt_error(&ldt_file(7, 4, 2, "")),
        "Isym must be 0 to 4 (got 7)"
    ));
    assert!(has(
        &ldt_error(&ldt_file(4, 6, 2, "")),
        "Isym 4 needs Mc divisible by 4"
    ));
    assert!(has(&ldt_error(&ldt_file(0, 0, 2, "")), "Mc = 0"));
    assert!(has(
        &ldt_error(&ldt_file(1, 1, 2, "0\n0\n90\n5\n")),
        "ends after 1 of 2 luminous intensities"
    ));
    assert!(has(
        &ldt_error(&ldt_file(1, 1, 2, "0\n90\n0\n1\n1\n")),
        "gamma angles must increase"
    ));
    assert!(has(
        &ldt_error(&ldt_file(1, 1, 2, "0\n0\n90\n0\n0\n")),
        "no light"
    ));
    assert!(has(
        &ldt_error(&ldt_file(1, 1, 2, "0\n0\n90\nabc\n1\n")),
        "expected a number, got 'abc'"
    ));
}

#[test]
fn truncated_and_corrupted_files_fail_cleanly() {
    for name in [
        "hat_lamp.ies",
        "street_quadrant.ies",
        "led_street.ldt",
        "symmetric.ldt",
    ] {
        let text = fixture(name);
        let is_ies = name.contains(".ies");
        let run = |bytes: &[u8]| -> bool {
            let r = if is_ies {
                import_ies(bytes).ok()
            } else {
                import_eulumdat(bytes).ok()
            };
            // Every success is a full table, every failure carries a message.
            match r {
                Some(t) => {
                    assert_eq!(t.len(), PROFILE_VALUES, "{name}");
                    true
                }
                None => false,
            }
        };
        for n in 0..text.len() {
            run(&text[..n]);
        }
        let mut bad = text.clone();
        let mut i = 0;
        while i < bad.len() {
            bad[i] = ((u32::from(bad[i]) * 31 + 17) & 0xFF) as u8;
            run(&bad);
            i += 7;
        }
    }
}

#[test]
fn numbers_beyond_the_float_range_are_rejected_not_turned_into_infinities() {
    // 1e300 is a finite double but no float: it must not reach the table as inf.
    assert!(has(
        &ies_error(&ies_body("2 1", "0 90 0", "1 1e300")),
        "expected candela values, got '1e300'"
    ));
    assert!(has(
        &ies_error(&ies_body("2 1", "0 1e300 0", "1 1")),
        "expected vertical angles, got '1e300'"
    ));
    assert!(has(
        &ldt_error(&ldt_file(1, 1, 2, "0\n0\n90\n1\n1e300\n")),
        "expected a number, got '1e300'"
    ));
    // Finite floats whose product with the candela multiplier overflows.
    let big = b"IESNA:LM-63-2002\nTILT=NONE\n1 1000 1e30 2 1 1 2 0 0 0\n1 1 10\n0 90\n0\n1e30 1\n";
    assert!(has(&ies_error(big), "too large"));
}
