//! Snapshot helpers shared by the crate's tests.
use knorke_types::{Reader, SnapshotError, SnapshotValue, Writer};

/// The snapshot bytes of `v`.
pub(crate) fn save<T: SnapshotValue>(v: &T) -> Vec<u8> {
    let mut out = Vec::new();
    Writer::new(&mut out).put(v);
    out
}

/// Restores `bytes` into `v` in place; the whole snapshot must be read.
pub(crate) fn load_into<T: SnapshotValue>(v: &mut T, bytes: &[u8]) -> Result<(), SnapshotError> {
    let mut r = Reader::new(bytes);
    v.get_into(&mut r)?;
    assert_eq!(r.remaining(), 0, "the whole snapshot is read");
    Ok(())
}
