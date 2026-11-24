//! Saving and restoring the open list, shared by [`Search`](crate::search::Search) and
//! [`DStarLite`](crate::dstar::DStarLite) (ADR 0010).
//!
//! Both write the heap in its internal order and read it back unchanged, so a restored
//! search pops the same nodes in the same order and saves the same bytes again. The order
//! is the one thing that must not be "fixed up" on restore, hence it lives in one place.

use std::collections::BinaryHeap;

use knorke_types::{Reader, SnapshotError, SnapshotValue, Writer};

/// Writes the entries in heap order, as a length followed by the entries.
pub(crate) fn put<T: Ord + SnapshotValue>(heap: &BinaryHeap<T>, w: &mut Writer<'_>) {
    w.put_len(heap.len());
    for entry in heap.iter() {
        w.put(entry);
    }
}

/// Reads entries written by [`put`] into `heap`, reusing its buffer.
///
/// `BinaryHeap::from` leaves an already valid heap as it is, so the saved order survives.
/// Corrupt bytes can break the heap order, which is memory safe but would make later pops
/// wrong, so the heap is emptied on error; callers reject the restore anyway.
pub(crate) fn get_into<T: Ord + SnapshotValue>(
    heap: &mut BinaryHeap<T>,
    r: &mut Reader<'_>,
) -> Result<(), SnapshotError> {
    let mut entries = std::mem::take(heap).into_vec();
    let read = r.get_into_vec(&mut entries);
    if read.is_err() {
        entries.clear();
    }
    *heap = BinaryHeap::from(entries);
    read
}
