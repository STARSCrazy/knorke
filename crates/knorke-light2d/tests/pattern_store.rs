//! Origin: chiaro @ 9bc5b9b, tests/unit/PatternStoreTests.cpp (zlib license, see
//! `src/pattern_store.rs`): sheet conversion to alpha, mean and column means, sampling with
//! repeat, validation, all-or-nothing batches, the limits of 4096 patterns and the 4096 x 4096
//! atlas, atlas reuse under churn and the version counter; plus the store as `PatternSource` of
//! the obstacles, snapshots and a replay.

use knorke_light2d::ObstacleStore;
use knorke_light2d::obstacle::{PatternSource, ShapeDesc};
use knorke_light2d::pattern_store::{
    ALPHA_ROWS, ATLAS_SIZE, MAX_PATTERNS, PatternDesc, Sheet, SheetFormat, raw_id,
};
use knorke_light2d::{PatternError, PatternId, PatternStore};
use knorke_types::{Reader, SnapshotBuf, SnapshotError, restore};

/// 4 x 2 RGBA sheet, alpha top row {0, 255, 0, 255}, bottom row {255, 255, 0, 0}.
fn small_sheet() -> Vec<u8> {
    let alpha = [0u8, 255, 0, 255, 255, 255, 0, 0];
    alpha.iter().flat_map(|&a| [10, 20, 30, a]).collect()
}

fn sheet(px: &[u8], width: u32, height: u32) -> Sheet<'_> {
    Sheet {
        width,
        height,
        pitch: width as usize * 4,
        format: SheetFormat::Rgba8,
        pixels: px,
    }
}

fn desc(x: u32, y: u32, w: u32, h: u32, rw: f32, rh: f32) -> PatternDesc {
    PatternDesc {
        repeat_width: rw,
        repeat_height: rh,
        ..PatternDesc::new(x, y, w, h)
    }
}

fn create_one(
    store: &mut PatternStore,
    px: &[u8],
    d: PatternDesc,
) -> Result<PatternId, PatternError> {
    let mut out = Vec::new();
    store.create(&sheet(px, 4, 2), &[d], &mut out)?;
    Ok(out[0])
}

/// One pattern from a sheet of `w x h` opaque pixels.
fn create_opaque(store: &mut PatternStore, w: u32, h: u32) -> Result<PatternId, PatternError> {
    let px = vec![255u8; (w * h * 4) as usize];
    let mut out = Vec::new();
    store.create(&sheet(&px, w, h), &[desc(0, 0, w, h, 1.0, 1.0)], &mut out)?;
    Ok(out[0])
}

#[test]
fn alpha_mean_and_column_means_of_a_4_x_2_sheet() {
    let mut store = PatternStore::new();
    let px = small_sheet();
    let mut out = Vec::new();
    let no_alpha = store
        .create(&sheet(&px, 4, 2), &[desc(0, 0, 4, 2, 2.0, 0.0)], &mut out)
        .unwrap();
    assert!(!no_alpha);
    assert_eq!(store.len(), 1);
    let s = store.get(out[0]).unwrap();
    assert_eq!((s.width, s.height), (4, 2));
    assert_eq!((s.repeat_width, s.repeat_height), (2.0, 0.0));
    assert!((s.mean - 0.5).abs() < 1e-6);
    assert_eq!(s.alpha, &[0, 255, 0, 255, 255, 255, 0, 0]);
    // Round to nearest of the 0..255 average, halves up: (0 + 255) / 2 = 127.5 -> 128.
    assert_eq!(s.column_mean, &[128, 255, 0, 128]);
    assert_ne!(raw_id(out[0]), 0);
}

#[test]
fn a_sub_rectangle_reads_only_its_pixels() {
    let mut store = PatternStore::new();
    let id = create_one(&mut store, &small_sheet(), desc(1, 1, 2, 1, 1.0, 1.0)).unwrap();
    let s = store.get(id).unwrap();
    assert_eq!(s.alpha, &[255, 0]);
    assert_eq!(s.column_mean, &[255, 0]);
    assert!((s.mean - 0.5).abs() < 1e-6);
}

#[test]
fn sampling_repeats_in_u_has_the_top_row_at_v_1_and_column_means_without_height() {
    let mut store = PatternStore::new();
    let id = create_one(&mut store, &small_sheet(), desc(0, 0, 4, 2, 1.0, 1.0)).unwrap();
    let s = store.get(id).unwrap();
    // v 0.9: top row {0, 255, 0, 255}; v 0.1: bottom row {255, 255, 0, 0}.
    assert_eq!(s.sample(0.1, 0.9, true), 0.0);
    assert_eq!(s.sample(0.3, 0.9, true), 1.0);
    assert_eq!(s.sample(0.6, 0.9, true), 0.0);
    assert_eq!(s.sample(0.1, 0.1, true), 1.0);
    assert_eq!(s.sample(0.6, 0.1, true), 0.0);
    assert_eq!(s.sample(0.9, 0.1, true), 0.0);
    // Repeat: 1.1 and -0.9 are 0.1 of the next and the previous repeat.
    assert_eq!(s.sample(1.1, 0.9, true), s.sample(0.1, 0.9, true));
    assert_eq!(s.sample(-0.9, 0.1, true), s.sample(0.1, 0.1, true));
    assert_eq!(s.sample(1.1, 1.1, true), s.sample(0.1, 0.1, true));
    assert_eq!(s.sample(-0.9, -0.1, true), s.sample(0.1, 0.9, true));
    // A tiny negative u wraps to 1 - epsilon: still the last column, never out of range.
    assert_eq!(s.sample(-1e-9, 0.1, true), 0.0);
    // Without height the column mean, whatever v is.
    assert!((s.sample(0.1, 0.9, false) - 128.0 / 255.0).abs() < 1e-6);
    assert_eq!(s.sample(0.3, 0.1, false), 1.0);
    assert_eq!(s.sample(0.6, 0.5, false), 0.0);
    assert!((s.sample(-0.1, 0.0, false) - 128.0 / 255.0).abs() < 1e-6);
}

#[test]
fn non_finite_u_or_v_reads_as_0_and_stays_in_range() {
    let mut store = PatternStore::new();
    let id = create_one(&mut store, &small_sheet(), desc(0, 0, 4, 2, 1.0, 1.0)).unwrap();
    let s = store.get(id).unwrap();
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(s.sample(bad, 0.9, true), s.sample(0.0, 0.9, true));
        assert_eq!(s.sample(0.6, bad, true), s.sample(0.6, 0.0, true));
        assert_eq!(s.sample(bad, bad, true), s.sample(0.0, 0.0, true));
        assert_eq!(s.sample(bad, 0.5, false), s.sample(0.0, 0.5, false));
    }
    let big = s.sample(1e30, -1e30, true);
    assert!((0.0..=1.0).contains(&big));
}

#[test]
fn invalid_descs_name_field_and_value() {
    let mut store = PatternStore::new();
    let px = small_sheet();
    let err = |d| create_one(&mut PatternStore::new(), &px, d).unwrap_err();

    let e = err(desc(2, 0, 3, 2, 1.0, 1.0));
    assert!(matches!(e, PatternError::Rect { index: 0, .. }));
    assert!(e.to_string().contains("descs[0]"));
    assert!(e.to_string().contains("inside the 4 x 2 sheet"));
    assert!(matches!(
        err(desc(0, 0, 0, 2, 1.0, 1.0)),
        PatternError::Rect { .. }
    ));

    let e = err(desc(0, 0, 4, 2, 0.0, 1.0));
    assert!(e.to_string().contains("repeat_width"));
    assert!(e.to_string().contains("got 0"));
    for bad in [f32::NAN, f32::INFINITY] {
        assert!(
            err(desc(0, 0, 4, 2, bad, 1.0))
                .to_string()
                .contains("repeat_width")
        );
    }
    let e = err(desc(0, 0, 4, 2, 1.0, -1.0));
    assert!(e.to_string().contains("repeat_height"));
    assert!(e.to_string().contains("got -1"));
    assert!(
        err(desc(0, 0, 4, 2, 1.0, f32::NAN))
            .to_string()
            .contains("repeat_height")
    );
    assert_eq!(e.code().as_str(), "knorke.light2d.pattern.desc");

    // A region 257 wide.
    let wide = vec![255u8; 257 * 4];
    let mut out = Vec::new();
    let e = store
        .create(
            &sheet(&wide, 257, 1),
            &[desc(0, 0, 257, 1, 1.0, 1.0)],
            &mut out,
        )
        .unwrap_err();
    assert!(e.to_string().contains("descs[0]"));
    assert!(e.to_string().contains("256"));
    assert!(e.to_string().contains("257"));
    assert!(out.is_empty());
    assert!(store.is_empty());
}

#[test]
fn invalid_sheets() {
    let mut store = PatternStore::new();
    let px = small_sheet();
    let d = [desc(0, 0, 4, 2, 1.0, 1.0)];
    let mut out = Vec::new();
    let mut fails = |s: Sheet<'_>| store.create(&s, &d, &mut out).unwrap_err();
    assert!(matches!(
        fails(sheet(&px[..16], 4, 2)),
        PatternError::PixelsTooShort { .. }
    ));
    let short_pitch = Sheet {
        pitch: 8,
        ..sheet(&px, 4, 2)
    };
    assert!(fails(short_pitch).to_string().contains("pitch"));
    assert!(fails(sheet(&px, 0, 2)).to_string().contains("width"));
    assert!(fails(sheet(&px, 4097, 2)).to_string().contains("4096"));
    assert_eq!(
        fails(sheet(&px, 0, 2)).code().as_str(),
        "knorke.light2d.pattern.sheet"
    );
    assert!(out.is_empty());
    assert!(store.is_empty());
}

#[test]
fn a_batch_with_one_invalid_desc_creates_nothing_and_leaves_out_unchanged() {
    let mut store = PatternStore::new();
    let px = small_sheet();
    let d = [
        desc(0, 0, 2, 2, 1.0, 1.0),
        desc(2, 0, 2, 2, 1.0, 1.0),
        desc(0, 0, 4, 2, 1.0, -1.0),
    ];
    let mut out = Vec::new();
    let version = store.version();
    let e = store.create(&sheet(&px, 4, 2), &d, &mut out).unwrap_err();
    assert!(e.to_string().contains("descs[2]"));
    assert!(out.is_empty());
    assert!(store.is_empty());
    assert_eq!(store.version(), version);

    store.create(&sheet(&px, 4, 2), &d[..2], &mut out).unwrap();
    assert_eq!(store.len(), 2);
    assert_ne!(out[0], out[1]);
    let (a, b) = (store.get(out[0]).unwrap(), store.get(out[1]).unwrap());
    assert!(a.atlas_x != b.atlas_x || a.atlas_y != b.atlas_y);
}

#[test]
fn a_sheet_without_alpha_is_opaque_and_reported() {
    let mut store = PatternStore::new();
    let px = vec![17u8; 4 * 2 * 3];
    let s = Sheet {
        width: 4,
        height: 2,
        pitch: 12,
        format: SheetFormat::Rgb8,
        pixels: &px,
    };
    let mut out = Vec::new();
    assert!(
        store
            .create(&s, &[desc(0, 0, 4, 2, 1.0, 1.0)], &mut out)
            .unwrap()
    );
    let p = store.get(out[0]).unwrap();
    assert_eq!(p.alpha, &[255; 8]);
    assert_eq!(p.column_mean, &[255; 4]);
    assert_eq!(p.mean, 1.0);
}

#[test]
fn at_most_4096_live_patterns() {
    let mut store = PatternStore::new();
    let px = vec![255u8; 64 * 64 * 4];
    let d: Vec<_> = (0..MAX_PATTERNS)
        .map(|i| desc(i % 64, i / 64, 1, 1, 1.0, 1.0))
        .collect();
    let mut ids = Vec::new();
    store.create(&sheet(&px, 64, 64), &d, &mut ids).unwrap();
    assert_eq!(store.len(), MAX_PATTERNS as usize);
    let e = create_opaque(&mut store, 1, 1).unwrap_err();
    assert!(e.to_string().contains("too many patterns"));
    assert!(e.to_string().contains("4096"));
    assert_eq!(store.len(), MAX_PATTERNS as usize);
    store.destroy(&ids[..1]).unwrap();
    assert!(create_opaque(&mut store, 1, 1).is_ok());
}

#[test]
fn a_full_atlas_fails_with_the_limit_in_the_message() {
    let mut store = PatternStore::new();
    // 256 x 256 regions: 16 x 15 fill the 4096 x 3840 alpha band.
    let ids: Vec<_> = (0..240)
        .map(|_| create_opaque(&mut store, 256, 256).unwrap())
        .collect();
    let version = store.version();
    let e = create_opaque(&mut store, 256, 256).unwrap_err();
    assert!(e.to_string().contains("atlas"));
    assert!(e.to_string().contains("4096 x 3840"));
    assert!(e.to_string().contains("column means"));
    assert_eq!(e.code().as_str(), "knorke.light2d.pattern.atlas_full");
    assert_eq!(store.len(), 240);
    assert_eq!(store.version(), version);

    // A batch that does not fit as a whole leaves the atlas as it was.
    store.destroy(&ids[..2]).unwrap();
    let px = vec![255u8; 256 * 256 * 4];
    let d = [desc(0, 0, 256, 256, 1.0, 1.0); 3];
    let mut out = Vec::new();
    let e = store
        .create(&sheet(&px, 256, 256), &d, &mut out)
        .unwrap_err();
    assert!(e.to_string().contains("atlas"));
    assert!(out.is_empty());
    assert_eq!(store.len(), 238);
    store
        .create(&sheet(&px, 256, 256), &d[..2], &mut out)
        .unwrap();
    assert_eq!(store.len(), 240);
}

#[test]
fn create_destroy_churn_reuses_atlas_space() {
    let mut store = PatternStore::new();
    let keep = create_opaque(&mut store, 64, 64).unwrap();
    for _ in 0..10000 {
        let id = create_opaque(&mut store, 64, 64).unwrap();
        let s = store.get(id).unwrap();
        assert!(s.atlas_x + s.width <= ATLAS_SIZE);
        assert!(s.atlas_y + s.height <= ALPHA_ROWS);
        store.destroy(&[id]).unwrap();
    }
    assert_eq!(store.len(), 1);
    // Freed space comes back whole: without the kept pattern the atlas takes 240 regions again.
    store.destroy(&[keep]).unwrap();
    let mut fitted = 0;
    while fitted < 241 && create_opaque(&mut store, 256, 256).is_ok() {
        fitted += 1;
    }
    assert_eq!(fitted, 240);
}

#[test]
fn destroy_handles_and_version() {
    let mut store = PatternStore::new();
    let v0 = store.version();
    let a = create_opaque(&mut store, 2, 2).unwrap();
    let v1 = store.version();
    assert_ne!(v1, v0);
    assert!(store.contains(a));

    // Failures change nothing.
    assert!(create_opaque(&mut store, 300, 1).is_err());
    assert_eq!(store.version(), v1);
    let b = create_opaque(&mut store, 2, 2).unwrap();
    store.destroy(&[b]).unwrap();
    let v2 = store.version();
    let e = store.destroy(&[a, b]).unwrap_err();
    assert!(e.to_string().contains("ids[1]"));
    assert_eq!(e.code().as_str(), "knorke.light2d.pattern.invalid_handle");
    assert!(store.contains(a));
    let e = store.destroy(&[a, a]).unwrap_err();
    assert!(e.to_string().contains("twice"));
    assert!(store.contains(a));
    assert_eq!(store.version(), v2);

    store.destroy(&[a]).unwrap();
    assert_ne!(store.version(), v2);
    assert!(store.get(a).is_none());
    assert!(store.is_empty());
    assert!(store.get_raw(raw_id(a)).is_none());
    assert!(store.get_raw(0).is_none());
    assert!(store.get_raw(0x0050_0001).is_none());
    assert!(
        store
            .destroy(&[a])
            .unwrap_err()
            .to_string()
            .contains("ids[0]")
    );
}

#[test]
fn a_store_filled_to_its_limits_fits_a_4096_x_4096_texture() {
    // 3856 patterns 1 x 1 share one 16-row shelf, 224 of 256 x 256 take rows 16 .. 3600 and 16 of
    // 1 x 240 the shelf 3600 .. 3840.
    let mut store = PatternStore::new();
    let mut many = |w: u32, h: u32, count: usize| {
        let px = vec![255u8; (w * h * 4) as usize];
        let d = vec![desc(0, 0, w, h, 1.0, 1.0); count];
        store.create(&sheet(&px, w, h), &d, &mut Vec::new())
    };
    many(1, 1, 3856).unwrap();
    many(256, 256, 224).unwrap();
    assert!(many(256, 256, 1).is_err()); // the alpha band has 240 rows left
    many(1, 240, 16).unwrap();
    assert_eq!(store.len(), MAX_PATTERNS as usize);
    let (rows, alpha_rows) = store.atlas_rows();
    assert_eq!(alpha_rows, ALPHA_ROWS);
    assert_eq!(rows, ATLAS_SIZE);
    for (_, p) in store.iter() {
        assert!(p.atlas_x + p.width <= ATLAS_SIZE);
        assert!(p.atlas_y + p.height <= ALPHA_ROWS);
    }
}

// ---- Knorke additions ----

#[test]
fn the_store_is_the_pattern_source_of_the_obstacles() {
    let mut patterns = PatternStore::new();
    let id = create_one(&mut patterns, &small_sheet(), desc(0, 0, 4, 2, 1.0, 1.0)).unwrap();
    let raw = raw_id(id);
    assert!((patterns.mean_opacity(raw).unwrap() - 0.5).abs() < 1e-6);

    let mut obstacles = ObstacleStore::new();
    let o = obstacles.create_shape(&ShapeDesc::rect(1.0, 1.0)).unwrap();
    obstacles.set_pattern(o, raw, &patterns).unwrap();
    assert!(obstacles.any_pattern());
    assert!((obstacles.mask_opacity(o, 0, &patterns).unwrap() - 0.5).abs() < 1e-6);

    patterns.destroy(&[id]).unwrap();
    obstacles.on_patterns_destroyed(&[raw]);
    assert_eq!(patterns.mean_opacity(raw), None);
    assert_eq!(obstacles.count_destroyed_pattern_users(&patterns, false), 1);
    assert_eq!(obstacles.mask_opacity(o, 0, &patterns), Some(1.0));
}

/// A store after churn: holes in the arena, several shelves, reused slots.
fn churned() -> (PatternStore, Vec<PatternId>) {
    let mut store = PatternStore::new();
    let mut ids = Vec::new();
    for i in 0..12u32 {
        ids.push(create_opaque(&mut store, 1 + i * 7, 1 + i * 11).unwrap());
    }
    store.destroy(&[ids[3], ids[7], ids[8]]).unwrap();
    ids.retain(|&id| store.contains(id));
    ids.push(create_opaque(&mut store, 30, 30).unwrap());
    (store, ids)
}

fn save(store: &PatternStore) -> SnapshotBuf {
    let mut snap = SnapshotBuf::new();
    snap.save(store);
    snap
}

#[test]
fn a_snapshot_round_trip_keeps_handles_texels_and_the_next_placement() {
    let (mut store, ids) = churned();
    let snap = save(&store);
    let mut back = PatternStore::new();
    restore(&mut Reader::new(snap.bytes()), &mut back).unwrap();
    for &id in &ids {
        assert_eq!(back.get(id), store.get(id));
    }
    assert_eq!(save(&back).bytes(), snap.bytes());
    // The packer came along: the next pattern goes to the same place.
    let a = create_opaque(&mut store, 40, 20).unwrap();
    let b = create_opaque(&mut back, 40, 20).unwrap();
    assert_eq!(a, b);
    assert_eq!(store.get(a), back.get(b));
}

#[test]
fn restore_rejects_a_region_too_large_and_leaves_an_empty_store() {
    let (store, _) = churned();
    let mut bytes = save(&store).bytes().to_vec();
    // The first pattern's width, found by its width, height and repeat width (1.0) bytes.
    let first = store.iter().next().unwrap().1;
    let needle = [
        first.width.to_le_bytes(),
        first.height.to_le_bytes(),
        1.0f32.to_le_bytes(),
    ]
    .concat();
    let at = bytes
        .windows(needle.len())
        .position(|w| w == needle.as_slice())
        .unwrap();
    bytes[at..at + 4].copy_from_slice(&300u32.to_le_bytes());
    let mut back = PatternStore::new();
    let e = restore(&mut Reader::new(&bytes), &mut back).unwrap_err();
    assert_eq!(e, SnapshotError::Invalid("pattern region"));
    assert!(back.is_empty());
}

/// Replay: the same commands give the same bytes; a store restored in the middle continues
/// identically.
#[test]
fn replaying_the_commands_gives_the_same_state_and_a_restored_store_continues_identically() {
    let step = |store: &mut PatternStore, ids: &mut Vec<PatternId>, i: u32| {
        if i % 3 == 2 && !ids.is_empty() {
            let id = ids.remove((i as usize * 5) % ids.len());
            store.destroy(&[id]).unwrap();
        } else {
            ids.push(create_opaque(store, 1 + (i * 13) % 200, 1 + (i * 29) % 200).unwrap());
        }
    };
    let (mut a, mut b) = (PatternStore::new(), PatternStore::new());
    let (mut ia, mut ib) = (Vec::new(), Vec::new());
    for i in 0..40 {
        step(&mut a, &mut ia, i);
        step(&mut b, &mut ib, i);
    }
    assert_eq!(save(&a).bytes(), save(&b).bytes());
    let mut c = PatternStore::new();
    restore(&mut Reader::new(save(&a).bytes()), &mut c).unwrap();
    let mut ic = ia.clone();
    for i in 40..80 {
        step(&mut a, &mut ia, i);
        step(&mut c, &mut ic, i);
    }
    assert_eq!(save(&a).bytes(), save(&c).bytes());
    assert_eq!(ia, ic);
}
