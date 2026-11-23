//! Trace sinks. Same shape as `sagax_core::Sink` (sagax fab00c0, crates/sagax-core/src/lib.rs),
//! so Sagax's traced algorithms can feed Knorke sinks through a thin adapter.

use alloc::vec::Vec;

/// Receives trace events of type `E` from a module.
///
/// Modules offer a traced variant of an operation that takes `&mut impl Sink<E>`; the
/// untraced variant passes [`NoopSink`], and the optimiser removes the event construction.
pub trait Sink<E> {
    /// `false` if events are thrown away anyway; algorithms may skip building them.
    #[inline]
    fn enabled(&self) -> bool {
        true
    }

    /// Takes one event.
    fn emit(&mut self, event: E);
}

/// Discards everything; [`Sink::enabled`] is `false`.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopSink;

impl<E> Sink<E> for NoopSink {
    #[inline]
    fn enabled(&self) -> bool {
        false
    }
    #[inline]
    fn emit(&mut self, _: E) {}
}

impl<E, S: Sink<E> + ?Sized> Sink<E> for &mut S {
    #[inline]
    fn enabled(&self) -> bool {
        (**self).enabled()
    }
    #[inline]
    fn emit(&mut self, event: E) {
        (**self).emit(event);
    }
}

/// Collects events (tests, offline analysis). Grows without bound – use a bounded sink in
/// long-running sessions.
impl<E> Sink<E> for Vec<E> {
    #[inline]
    fn emit(&mut self, event: E) {
        self.push(event);
    }
}

/// Forwards every event to a closure.
pub struct CallbackSink<F>(pub F);

impl<E, F: FnMut(E)> Sink<E> for CallbackSink<F> {
    #[inline]
    fn emit(&mut self, event: E) {
        (self.0)(event);
    }
}

/// Converts events before passing them on, e.g. a module's own trace type into a shared
/// analysis type at an integration boundary.
pub struct MapSink<'a, S: ?Sized, F> {
    inner: &'a mut S,
    map: F,
}

impl<'a, S: ?Sized, F> MapSink<'a, S, F> {
    /// Wraps `inner`, converting each event with `map`.
    pub fn new(inner: &'a mut S, map: F) -> Self {
        Self { inner, map }
    }
}

impl<E, D, S: Sink<D> + ?Sized, F: FnMut(E) -> D> Sink<E> for MapSink<'_, S, F> {
    #[inline]
    fn enabled(&self) -> bool {
        self.inner.enabled()
    }
    #[inline]
    fn emit(&mut self, event: E) {
        self.inner.emit((self.map)(event));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn traced_sum(xs: &[i32], sink: &mut impl Sink<(usize, i32)>) -> i32 {
        let mut total = 0;
        for (i, &x) in xs.iter().enumerate() {
            total += x;
            if sink.enabled() {
                sink.emit((i, total));
            }
        }
        total
    }

    #[test]
    fn sinks_compose() {
        assert_eq!(traced_sum(&[1, 2], &mut NoopSink), 3);
        let mut all = Vec::new();
        traced_sum(&[1, 2], &mut all);
        assert_eq!(all, [(0, 1), (1, 3)]);
        let mut totals: Vec<i32> = Vec::new();
        traced_sum(&[5, 5], &mut MapSink::new(&mut totals, |(_, t)| t));
        assert_eq!(totals, [5, 10]);
        let mut n = 0;
        traced_sum(&[1, 1, 1], &mut CallbackSink(|_| n += 1));
        assert_eq!(n, 3);
        assert!(!MapSink::new(&mut NoopSink, |e: i32| e).enabled());
    }
}
