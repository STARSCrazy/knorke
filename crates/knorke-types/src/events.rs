//! Typed events and change propagation without a message broker (ADR 0008).
//!
//! - [`Revision`]: a counter a data owner bumps on every change. Equal revision → nothing to do.
//! - [`EventQueue`]: one queue per event type. The producer appends, tagged with the tick.
//!   Every reader keeps its own [`EventCursor`] and gets the unread events as borrowed
//!   slices. No subscription, no callbacks, no dynamic dispatch: the schedule decides when
//!   a reader runs, so ordering is explicit and deterministic.

use crate::Tick;
use alloc::vec::Vec;

/// Monotonic change counter of some data. Equal revisions mean equal content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(pub u64);

impl Revision {
    /// The revision of freshly created data.
    pub const ZERO: Revision = Revision(0);

    /// Advances to the next revision and returns it.
    #[inline]
    pub fn bump(&mut self) -> Revision {
        self.0 += 1;
        *self
    }
}

/// A reader's position in an [`EventQueue`]. [`EventCursor::default`] has seen nothing and
/// gets [`Events::Resync`] on its first read; [`EventQueue::cursor_at_end`] starts after
/// everything currently queued.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventCursor {
    /// Absolute index of the next unread entry; `None` before the first read.
    next: Option<u64>,
}

/// Result of [`EventQueue::read`].
#[derive(Debug, PartialEq)]
pub enum Events<'a, E> {
    /// The events the reader has not seen, oldest first.
    Since {
        /// Tick of each event.
        ticks: &'a [Tick],
        /// The events.
        events: &'a [E],
    },
    /// The reader is new, fell behind events that were already retired, or is ahead of the
    /// queue (the queue was restored to an older state without the cursor). It must rebuild
    /// its view from the owner's current data; the cursor now points past the end.
    Resync,
}

impl<'a, E> Events<'a, E> {
    /// The unread events, or `None` if a resync is required.
    pub fn since(self) -> Option<&'a [E]> {
        match self {
            Events::Since { events, .. } => Some(events),
            Events::Resync => None,
        }
    }
}

/// Typed, append-only event queue read by any number of readers with their own cursors.
///
/// Storage is struct-of-arrays, so a reader gets `&[E]` directly. Retire old events with
/// [`EventQueue::retire_before`] or [`EventQueue::keep_ticks`] once readers had their chance;
/// a reader that was too slow gets [`Events::Resync`] instead of silently missing events.
/// Capacity is kept across retirement, so a queue in steady state does not allocate.
#[derive(Clone, Debug, PartialEq)]
pub struct EventQueue<E> {
    ticks: Vec<Tick>,
    events: Vec<E>,
    /// Absolute index of `events[0]`.
    base: u64,
}

impl<E> Default for EventQueue<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E> EventQueue<E> {
    /// Empty queue.
    pub const fn new() -> Self {
        Self {
            ticks: Vec::new(),
            events: Vec::new(),
            base: 0,
        }
    }

    /// Empty queue with room for `capacity` events.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            ticks: Vec::with_capacity(capacity),
            events: Vec::with_capacity(capacity),
            base: 0,
        }
    }

    /// Appends an event that happened at `tick`. Ticks must not decrease.
    #[inline]
    pub fn push(&mut self, tick: Tick, event: E) {
        debug_assert!(
            self.ticks.last().is_none_or(|&last| last <= tick),
            "EventQueue: tick went backwards"
        );
        self.ticks.push(tick);
        self.events.push(event);
    }

    /// Appends all events of `other` (e.g. a per-chunk buffer) at `tick`, in order, and
    /// leaves `other` empty with its capacity.
    pub fn append(&mut self, tick: Tick, other: &mut alloc::vec::Vec<E>) {
        debug_assert!(
            self.ticks.last().is_none_or(|&last| last <= tick),
            "EventQueue: tick went backwards"
        );
        self.ticks.extend(core::iter::repeat_n(tick, other.len()));
        self.events.append(other);
    }

    /// Number of retained events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// `true` if no events are retained.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// A cursor that skips everything currently queued.
    pub fn cursor_at_end(&self) -> EventCursor {
        EventCursor {
            next: Some(self.base + self.events.len() as u64),
        }
    }

    /// Returns what `cursor` has not seen yet and advances it to the end.
    #[inline]
    pub fn read(&self, cursor: &mut EventCursor) -> Events<'_, E> {
        let end = self.base + self.events.len() as u64;
        match cursor.next.replace(end) {
            // `n > end`: the cursor is ahead of the queue. That happens when the queue was
            // restored from an older snapshot (rollback, seek) but the cursor was not, or
            // when a cursor of another queue is used. The reader's view is then stale.
            Some(n) if n >= self.base && n <= end => {
                let from = (n - self.base) as usize;
                Events::Since {
                    ticks: &self.ticks[from..],
                    events: &self.events[from..],
                }
            }
            _ => Events::Resync,
        }
    }

    /// Like [`EventQueue::read`] but without advancing the cursor.
    pub fn peek(&self, cursor: &EventCursor) -> Events<'_, E> {
        self.read(&mut cursor.clone())
    }

    /// Drops all events from before `tick`.
    pub fn retire_before(&mut self, tick: Tick) {
        let n = self.ticks.partition_point(|&t| t < tick);
        self.retire_first(n);
    }

    /// Keeps the events of the last `ticks` ticks up to and including `now` (Bevy-style
    /// double buffering is `keep_ticks(now, 2)`); `ticks == 0` drops everything up to `now`.
    pub fn keep_ticks(&mut self, now: Tick, ticks: u64) {
        match ticks.checked_sub(1) {
            Some(back) => self.retire_before(Tick(now.0.saturating_sub(back))),
            None => {
                let n = self.ticks.partition_point(|&t| t <= now);
                self.retire_first(n);
            }
        }
    }

    fn retire_first(&mut self, n: usize) {
        self.ticks.drain(..n);
        self.events.drain(..n);
        self.base += n as u64;
    }

    /// Drops all events.
    pub fn clear(&mut self) {
        self.base += self.events.len() as u64;
        self.ticks.clear();
        self.events.clear();
    }
}

/// A cursor is part of its reader's simulation state: save it next to the reader's data so
/// that a restored reader continues exactly where the saved one stopped.
impl crate::SnapshotValue for EventCursor {
    fn put(&self, w: &mut crate::snapshot::Writer<'_>) {
        w.put(&self.next);
    }
    fn get(r: &mut crate::snapshot::Reader<'_>) -> Result<Self, crate::SnapshotError> {
        Ok(EventCursor { next: r.get()? })
    }
}

impl<E: crate::SnapshotValue> EventQueue<E> {
    pub(crate) fn snapshot_put(&self, w: &mut crate::snapshot::Writer<'_>) {
        w.put(&self.base);
        w.put_slice(&self.ticks);
        w.put_slice(&self.events);
    }

    pub(crate) fn snapshot_get_into(
        &mut self,
        r: &mut crate::snapshot::Reader<'_>,
    ) -> Result<(), crate::SnapshotError> {
        self.base = r.get()?;
        r.get_into_vec(&mut self.ticks)?;
        r.get_into_vec(&mut self.events)?;
        if self.ticks.len() != self.events.len() {
            return Err(crate::SnapshotError::Invalid(
                "event queue: tick and event counts differ",
            ));
        }
        // `retire_before` binary-searches the ticks and `read` adds `base + len`.
        if !self.ticks.is_sorted() {
            return Err(crate::SnapshotError::Invalid(
                "event queue: ticks are not ascending",
            ));
        }
        if self.base.checked_add(self.events.len() as u64).is_none() {
            return Err(crate::SnapshotError::Invalid(
                "event queue: base index overflows",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_bumps() {
        let mut r = Revision::ZERO;
        assert_eq!(r.bump(), Revision(1));
        assert!(r > Revision::ZERO);
    }

    #[test]
    fn new_reader_resyncs_then_reads_increments() {
        let mut q = EventQueue::new();
        q.push(Tick(1), 'a');
        let mut c = EventCursor::default();
        assert_eq!(q.read(&mut c), Events::Resync);
        assert_eq!(
            q.read(&mut c),
            Events::Since {
                ticks: &[],
                events: &[]
            }
        );
        q.push(Tick(2), 'b');
        q.push(Tick(2), 'c');
        assert_eq!(
            q.read(&mut c),
            Events::Since {
                ticks: &[Tick(2), Tick(2)],
                events: &['b', 'c']
            }
        );
        let mut late = q.cursor_at_end();
        assert_eq!(q.read(&mut late).since(), Some(&[][..]));
    }

    #[test]
    fn independent_readers() {
        let mut q = EventQueue::new();
        let (mut fast, mut slow) = (q.cursor_at_end(), q.cursor_at_end());
        q.push(Tick(1), 1);
        assert_eq!(q.read(&mut fast).since(), Some(&[1][..]));
        q.push(Tick(2), 2);
        assert_eq!(q.read(&mut fast).since(), Some(&[2][..]));
        assert_eq!(q.peek(&slow).since(), Some(&[1, 2][..]));
        assert_eq!(q.read(&mut slow).since(), Some(&[1, 2][..]));
    }

    #[test]
    fn retired_events_force_resync_for_late_readers() {
        let mut q = EventQueue::new();
        let (mut fast, mut late) = (q.cursor_at_end(), q.cursor_at_end());
        q.push(Tick(1), 1);
        q.push(Tick(2), 2);
        q.read(&mut fast);
        q.keep_ticks(Tick(2), 1);
        assert_eq!(q.len(), 1);
        assert_eq!(q.read(&mut late), Events::Resync);
        q.push(Tick(3), 3);
        assert_eq!(q.read(&mut fast).since(), Some(&[3][..]));
        assert_eq!(q.read(&mut late).since(), Some(&[3][..]));
    }

    #[test]
    fn cursor_ahead_of_a_restored_queue_resyncs() {
        use crate::SnapshotValue;
        use crate::snapshot::{Reader, Writer};
        let mut q = EventQueue::new();
        q.push(Tick(1), 1u32);
        let mut saved = Vec::new();
        q.put(&mut Writer::new(&mut saved));
        q.push(Tick(2), 2);
        q.push(Tick(3), 3);
        let mut reader = q.cursor_at_end();
        // Rollback of the queue only: the cursor now points behind the end.
        q.get_into(&mut Reader::new(&saved)).unwrap();
        assert_eq!(q.peek(&reader), Events::Resync);
        assert_eq!(q.read(&mut reader), Events::Resync);
        q.push(Tick(2), 20);
        assert_eq!(q.read(&mut reader).since(), Some(&[20][..]));
        // A cursor saved with its reader continues without a resync.
        let mut c = q.cursor_at_end();
        let mut bytes = Vec::new();
        c.put(&mut Writer::new(&mut bytes));
        q.push(Tick(4), 40);
        let mut restored = EventCursor::get(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(restored, c);
        assert_eq!(q.read(&mut restored), q.read(&mut c));
        let mut fresh = Vec::new();
        EventCursor::default().put(&mut Writer::new(&mut fresh));
        assert_eq!(
            EventCursor::get(&mut Reader::new(&fresh)).unwrap(),
            EventCursor::default()
        );
    }

    #[test]
    fn keep_ticks_at_the_last_tick_does_not_overflow() {
        let mut q = EventQueue::new();
        q.push(Tick(u64::MAX - 1), 1);
        q.push(Tick(u64::MAX), 2);
        q.keep_ticks(Tick(u64::MAX), 1);
        assert_eq!(q.len(), 1);
        q.push(Tick(u64::MAX), 3);
        q.keep_ticks(Tick(u64::MAX), 0);
        assert!(q.is_empty());
        q.push(Tick(u64::MAX), 4);
        q.clear();
        assert!(q.is_empty());
        let mut c = EventCursor::default();
        assert_eq!(q.read(&mut c), Events::Resync);
        assert_eq!(q.read(&mut c).since(), Some(&[][..]));
    }

    #[test]
    fn restore_rejects_inconsistent_queues() {
        use crate::SnapshotValue;
        use crate::snapshot::{Reader, Writer};
        let encode = |base: u64, ticks: &[Tick], events: &[u8]| {
            let mut b = Vec::new();
            let mut w = Writer::new(&mut b);
            w.put(&base);
            w.put_slice(ticks);
            w.put_slice(events);
            b
        };
        let get = |b: &[u8]| EventQueue::<u8>::get(&mut Reader::new(b));
        assert!(get(&encode(0, &[Tick(1), Tick(2)], &[1, 2])).is_ok());
        assert!(get(&encode(0, &[Tick(2), Tick(1)], &[1, 2])).is_err());
        assert!(get(&encode(0, &[Tick(1)], &[1, 2])).is_err());
        assert!(get(&encode(u64::MAX, &[Tick(1)], &[1])).is_err());
    }

    #[test]
    fn append_moves_chunk_buffers_in_order() {
        let mut q = EventQueue::new();
        let mut c = q.cursor_at_end();
        let mut chunk = alloc::vec![1, 2];
        q.append(Tick(4), &mut chunk);
        assert!(chunk.is_empty() && chunk.capacity() >= 2);
        assert_eq!(
            q.read(&mut c),
            Events::Since {
                ticks: &[Tick(4), Tick(4)],
                events: &[1, 2]
            }
        );
    }

    #[test]
    fn events_are_borrowed_not_copied() {
        let mut q = EventQueue::new();
        let mut c = q.cursor_at_end();
        q.push(Tick(0), [0u8; 64]);
        let events = q.read(&mut c).since().unwrap();
        assert!(core::ptr::eq(events.as_ptr(), q.events.as_ptr()));
    }
}
