//! Ambient zone store limits, rows, handles and fold order (chiaro AmbientZoneStoreTests, ported).

// Origin: chiaro @ 9bc5b9b, tests/unit/AmbientZoneStoreTests.cpp (zlib license, see `src/ambient_zone_store.rs`).
//
// Ported test by test with the same inputs and expected values. Error texts keep the C++ wording
// without the C function names ("CHI_SetAmbientZone: ..."), which the Rust API does not have.
// The handle-0 case is not expressible (a `Handle` is never 0), see the report.

use knorke_light2d::ambient_zone::{AmbientZoneParams, StoredZone, ZoneShape, make_stored_zone};
use knorke_light2d::ambient_zone_store::{AmbientZoneId, AmbientZoneStore, MAX_ZONES};
use knorke_math::Vec2;
use knorke_types::Handle;

fn box_zone(priority: i32) -> StoredZone {
    let p = AmbientZoneParams {
        shape: ZoneShape::Box,
        priority,
        ..AmbientZoneParams::default()
    };
    make_stored_zone(&p).expect("box")
}

fn polygon_zone() -> StoredZone {
    let p = AmbientZoneParams {
        shape: ZoneShape::Polygon,
        vertices: vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(0.0, 1.0),
        ],
        ..AmbientZoneParams::default()
    };
    make_stored_zone(&p).expect("triangle")
}

fn order(store: &AmbientZoneStore) -> Vec<AmbientZoneId> {
    store.iter_ordered().map(|(id, _)| id).collect()
}

#[test]
fn ambient_zone_store_at_most_4096_zones() {
    let mut store = AmbientZoneStore::new();
    for _ in 0..MAX_ZONES {
        store.create(box_zone(0)).expect("below the limit");
    }
    assert_eq!(store.len(), 4096);
    let err = store.create(box_zone(0)).expect_err("limit reached");
    assert!(
        err.to_string()
            .contains("too many ambient zones (at most 4096 alive at once"),
        "{err}"
    );
    assert_eq!(store.len(), 4096);
    assert!(store.can_create(1).is_err());
    assert!(
        store
            .can_create(1)
            .unwrap_err()
            .to_string()
            .contains("too many ambient zones")
    );
    let fresh = AmbientZoneStore::new();
    assert!(fresh.can_create(4096).is_ok());
    assert!(fresh.can_create(4097).is_err());
}

#[test]
fn ambient_zone_store_polygon_rows_stay_low_under_churn() {
    let mut store = AmbientZoneStore::new();
    let keep = store.create(box_zone(0)).expect("keep");
    assert_eq!(store.polygon_row_count(), 0);
    for _ in 0..10000 {
        let id = store.create(polygon_zone()).expect("polygon");
        assert_eq!(store.polygon_row(id), Some(0));
        store.destroy(id).expect("destroy");
    }
    let a = store.create(polygon_zone()).expect("a");
    let b = store.create(polygon_zone()).expect("b");
    assert_eq!(store.polygon_row(a), Some(0));
    assert_eq!(store.polygon_row(b), Some(1));
    assert_eq!(store.polygon_row_count(), 2);
    assert_eq!(store.polygon_row(keep), None);
    store.destroy(a).expect("destroy a");
    assert_eq!(store.polygon_row_count(), 2);
    let c = store.create(polygon_zone()).expect("c");
    assert_eq!(store.polygon_row(c), Some(0)); // the lowest free row
    // A box set to a polygon takes a row, a polygon set to a box gives its row back.
    store.set(keep, polygon_zone()).expect("set polygon");
    assert_eq!(store.polygon_row(keep), Some(2));
    store.set(keep, box_zone(0)).expect("set box");
    assert_eq!(store.polygon_row(keep), None);
    store.destroy(b).expect("destroy b");
    assert_eq!(store.polygon_row_count(), 1);
}

#[test]
fn ambient_zone_store_handle_messages() {
    let mut store = AmbientZoneStore::new();
    let id = store.create(box_zone(0)).expect("box");
    assert!(store.describe(id).is_ok());
    store.destroy(id).expect("destroy");
    assert!(store.find(id).is_none());
    let err = store.destroy(id).expect_err("second destroy");
    let text = err.to_string();
    assert!(text.contains("zone"), "{text}");
    assert!(text.contains("was destroyed"), "{text}");
    assert!(
        text.contains("a handle of another context can look the same"),
        "{text}"
    );
    let err = store.set(id, box_zone(0)).expect_err("set destroyed");
    assert!(err.to_string().contains("was destroyed"), "{}", err);
    let mut other = AmbientZoneStore::new();
    for _ in 0..3 {
        other.create(box_zone(0)).expect("other");
    }
    // generation 1, slot 2: this store never had slot 2
    let foreign: AmbientZoneId = Handle::new(2, std::num::NonZeroU32::new(1).expect("nonzero"));
    let err = store.set(foreign, box_zone(0)).expect_err("foreign");
    assert!(
        err.to_string().contains("was not created by this context"),
        "{err}"
    );
}

#[test]
fn ambient_zone_store_versions_change_only_with_changes() {
    let mut store = AmbientZoneStore::new();
    let v0 = store.version();
    let p0 = store.polygon_version();
    let boxed = store.create(box_zone(0)).expect("box");
    assert!(store.version() > v0);
    assert_eq!(store.polygon_version(), p0);
    let v1 = store.version();
    let _ = order(&store);
    let _ = store.find(boxed);
    assert_eq!(store.version(), v1);
    let poly = store.create(polygon_zone()).expect("polygon");
    assert!(store.polygon_version() > p0);
    let p1 = store.polygon_version();
    store.set(boxed, box_zone(3)).expect("set box");
    assert!(store.version() > v1);
    assert_eq!(store.polygon_version(), p1);
    store.set(poly, polygon_zone()).expect("set polygon");
    assert!(store.polygon_version() > p1);
}

#[test]
fn ambient_zone_store_fold_order_is_priority_then_creation_number_stable() {
    let mut store = AmbientZoneStore::new();
    let a = store.create(box_zone(1)).expect("a");
    let b = store.create(box_zone(0)).expect("b");
    let c = store.create(box_zone(0)).expect("c");
    let d = store.create(box_zone(-5)).expect("d");
    assert_eq!(order(&store), vec![d, b, c, a]);
    assert_eq!(order(&store), vec![d, b, c, a]);
    store.set(b, box_zone(0)).expect("same priority"); // keeps its place before c
    assert_eq!(order(&store), vec![d, b, c, a]);
    store.set(d, box_zone(2)).expect("new priority"); // a new priority moves it, ties go by creation number
    assert_eq!(order(&store), vec![b, c, a, d]);
    store.destroy(c).expect("destroy c");
    let e = store.create(box_zone(0)).expect("e");
    assert_eq!(order(&store), vec![b, e, a, d]);
}
