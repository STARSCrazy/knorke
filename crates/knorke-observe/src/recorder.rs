// Host-side tool: reading the wall clock is the whole point here (ADR 0003 forbids it only
// in modules, which receive time as `Tick`).
#![allow(clippy::disallowed_methods)]

//! Host-side recording of probes, counters and diagnostics, with export to the Chrome
//! trace event format (opens in <https://ui.perfetto.dev> and `chrome://tracing`).

use crate::json::{diagnostic_to_json, write_f64, write_str};
use crate::{Counter, Probe, Scope, Sink};
use core::fmt::Write as _;
use knorke_types::{Diagnostic, Tick};
use std::format;
use std::io;
use std::string::String;
use std::time::Instant;
use std::vec::Vec;

/// A finished timed region.
#[derive(Clone, Copy, Debug)]
pub struct Span {
    /// The scope.
    pub scope: &'static Scope,
    /// Start, nanoseconds since the recorder's epoch.
    pub start_ns: u64,
    /// Duration in nanoseconds.
    pub duration_ns: u64,
    /// Nesting depth (0 = outermost).
    pub depth: u16,
    /// Simulation tick current when the span started, if the host set one.
    pub tick: Option<Tick>,
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    counter: &'static Counter,
    at_ns: u64,
    value: f64,
}

/// Fixed-capacity ring: keeps the newest `capacity` items, never reallocates after creation.
struct Ring<T> {
    items: Vec<T>,
    capacity: usize,
    next: usize,
    dropped: u64,
}

impl<T: Copy> Ring<T> {
    fn new(capacity: usize) -> Self {
        Self {
            items: Vec::with_capacity(capacity),
            capacity: capacity.max(1),
            next: 0,
            dropped: 0,
        }
    }

    #[inline]
    fn push(&mut self, item: T) {
        if self.items.len() < self.capacity {
            self.items.push(item);
        } else {
            self.items[self.next] = item;
            self.dropped += 1;
        }
        self.next = (self.next + 1) % self.capacity;
    }

    /// Oldest first.
    fn iter(&self) -> impl Iterator<Item = &T> {
        let (newer, older) = if self.items.len() < self.capacity {
            (&self.items[..], &self.items[..0])
        } else {
            (&self.items[..self.next], &self.items[self.next..])
        };
        older.iter().chain(newer)
    }

    fn clear(&mut self) {
        self.items.clear();
        self.next = 0;
        self.dropped = 0;
    }
}

/// Timing statistics of one scope over all retained spans.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeStats {
    /// Module of the scope.
    pub module: &'static str,
    /// Scope name.
    pub name: &'static str,
    /// Number of spans.
    pub count: usize,
    /// Sum of durations.
    pub total_ns: u64,
    /// Shortest.
    pub min_ns: u64,
    /// Median (nearest rank).
    pub p50_ns: u64,
    /// 95th percentile (nearest rank).
    pub p95_ns: u64,
    /// Longest.
    pub max_ns: u64,
}

/// Records what modules report through [`Probe`] and `Sink<Diagnostic>`.
///
/// One recorder per thread (it is `&mut`-driven, no locks); give each a distinct
/// `thread_id` and export them together with [`Recorder::write_chrome_trace`].
/// Memory is bounded by the capacities given at creation; recording allocates nothing
/// after that, except when scopes nest deeper than 64 levels.
pub struct Recorder {
    name: String,
    thread_id: u32,
    epoch: Instant,
    tick: Option<Tick>,
    open: Vec<(&'static Scope, u64)>,
    spans: Ring<Span>,
    samples: Ring<Sample>,
    diagnostics: Vec<(u64, Option<Tick>, Diagnostic)>,
    max_diagnostics: usize,
    dropped_diagnostics: u64,
}

impl Recorder {
    /// A recorder keeping the newest `span_capacity` spans and counter samples and the first
    /// 10 000 diagnostics.
    pub fn new(name: impl Into<String>, thread_id: u32, span_capacity: usize) -> Self {
        Self::with_epoch(name, thread_id, span_capacity, Instant::now())
    }

    /// Like [`Recorder::new`] with an explicit time origin, so recorders of different
    /// threads share one timeline.
    pub fn with_epoch(
        name: impl Into<String>,
        thread_id: u32,
        span_capacity: usize,
        epoch: Instant,
    ) -> Self {
        Self {
            name: name.into(),
            thread_id,
            epoch,
            tick: None,
            open: Vec::with_capacity(64),
            spans: Ring::new(span_capacity),
            samples: Ring::new(span_capacity),
            diagnostics: Vec::new(),
            max_diagnostics: 10_000,
            dropped_diagnostics: 0,
        }
    }

    /// Sets the current simulation tick; spans and diagnostics are tagged with it.
    pub fn set_tick(&mut self, tick: Tick) {
        self.tick = Some(tick);
    }

    #[inline]
    fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    /// Retained spans, oldest first.
    pub fn spans(&self) -> impl Iterator<Item = &Span> {
        self.spans.iter()
    }

    /// Recorded diagnostics with time (ns since epoch) and tick.
    pub fn diagnostics(&self) -> impl Iterator<Item = (u64, Option<Tick>, &Diagnostic)> {
        self.diagnostics.iter().map(|(t, k, d)| (*t, *k, d))
    }

    /// Number of spans, samples and diagnostics lost to the capacity limits.
    pub fn dropped(&self) -> u64 {
        self.spans.dropped + self.samples.dropped + self.dropped_diagnostics
    }

    /// Forgets everything recorded so far (open scopes stay open).
    pub fn clear(&mut self) {
        self.spans.clear();
        self.samples.clear();
        self.diagnostics.clear();
        self.dropped_diagnostics = 0;
    }

    /// Per-scope statistics, sorted by total time, largest first.
    pub fn stats(&self) -> Vec<ScopeStats> {
        let mut by_scope: Vec<(&'static Scope, Vec<u64>)> = Vec::new();
        for s in self.spans.iter() {
            match by_scope
                .iter_mut()
                .find(|(sc, _)| core::ptr::eq(*sc, s.scope))
            {
                Some((_, d)) => d.push(s.duration_ns),
                None => by_scope.push((s.scope, Vec::from([s.duration_ns]))),
            }
        }
        let mut out: Vec<ScopeStats> = by_scope
            .into_iter()
            .map(|(scope, mut d)| {
                d.sort_unstable();
                let rank = |p: usize| d[((d.len() * p).div_ceil(100)).saturating_sub(1)];
                ScopeStats {
                    module: scope.module,
                    name: scope.name,
                    count: d.len(),
                    total_ns: d.iter().sum(),
                    min_ns: d[0],
                    p50_ns: rank(50),
                    p95_ns: rank(95),
                    max_ns: d[d.len() - 1],
                }
            })
            .collect();
        out.sort_by_key(|s| core::cmp::Reverse(s.total_ns));
        out
    }

    /// Human-readable table of [`Recorder::stats`].
    pub fn summary(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "{:<40} {:>8} {:>10} {:>10} {:>10} {:>10}",
            "scope", "count", "total ms", "p50 µs", "p95 µs", "max µs"
        );
        for st in self.stats() {
            let _ = writeln!(
                s,
                "{:<40} {:>8} {:>10.3} {:>10.1} {:>10.1} {:>10.1}",
                format!("{}/{}", st.module, st.name),
                st.count,
                st.total_ns as f64 / 1e6,
                st.p50_ns as f64 / 1e3,
                st.p95_ns as f64 / 1e3,
                st.max_ns as f64 / 1e3,
            );
        }
        if self.dropped() > 0 {
            let _ = writeln!(s, "({} entries dropped by capacity limits)", self.dropped());
        }
        s
    }

    /// Writes one or more recorders as a Chrome trace event JSON document: spans as
    /// complete events, counters as counter tracks, diagnostics as instant events.
    pub fn write_chrome_trace(recorders: &[&Recorder], w: &mut impl io::Write) -> io::Result<()> {
        let mut out = String::from("{\"displayTimeUnit\":\"ns\",\"traceEvents\":[");
        let mut first = true;
        let mut sep = |out: &mut String| {
            if !core::mem::take(&mut first) {
                out.push(',');
            }
            out.push('\n');
        };
        for r in recorders {
            let tid = r.thread_id;
            sep(&mut out);
            out.push_str("{\"ph\":\"M\",\"pid\":1,\"tid\":");
            let _ = write!(out, "{tid},\"name\":\"thread_name\",\"args\":{{\"name\":");
            write_str(&mut out, &r.name);
            out.push_str("}}");
            for s in r.spans.iter() {
                sep(&mut out);
                out.push_str("{\"ph\":\"X\",\"pid\":1,\"tid\":");
                let _ = write!(out, "{tid},\"ts\":");
                write_f64(&mut out, s.start_ns as f64 / 1e3);
                out.push_str(",\"dur\":");
                write_f64(&mut out, s.duration_ns as f64 / 1e3);
                out.push_str(",\"cat\":");
                write_str(&mut out, s.scope.module);
                out.push_str(",\"name\":");
                write_str(&mut out, s.scope.name);
                if let Some(t) = s.tick {
                    let _ = write!(out, ",\"args\":{{\"tick\":{}}}", t.0);
                }
                out.push('}');
            }
            for c in r.samples.iter() {
                sep(&mut out);
                out.push_str("{\"ph\":\"C\",\"pid\":1,\"tid\":");
                let _ = write!(out, "{tid},\"ts\":");
                write_f64(&mut out, c.at_ns as f64 / 1e3);
                out.push_str(",\"cat\":");
                write_str(&mut out, c.counter.module);
                out.push_str(",\"name\":");
                write_str(
                    &mut out,
                    &format!(
                        "{}.{} [{}]",
                        c.counter.module, c.counter.name, c.counter.unit
                    ),
                );
                out.push_str(",\"args\":{\"value\":");
                write_f64(&mut out, c.value);
                out.push_str("}}");
            }
            for (at, tick, d) in &r.diagnostics {
                sep(&mut out);
                out.push_str("{\"ph\":\"i\",\"s\":\"t\",\"pid\":1,\"tid\":");
                let _ = write!(out, "{tid},\"ts\":");
                write_f64(&mut out, *at as f64 / 1e3);
                out.push_str(",\"cat\":\"diagnostic\",\"name\":");
                write_str(&mut out, d.code.as_str());
                out.push_str(",\"args\":{\"diagnostic\":");
                out.push_str(&diagnostic_to_json(d));
                if let Some(t) = tick {
                    let _ = write!(out, ",\"tick\":{}", t.0);
                }
                out.push_str("}}");
            }
        }
        out.push_str("\n]}\n");
        w.write_all(out.as_bytes())
    }

    /// Writes the diagnostics as JSON Lines (one object per line), for agents and CI logs.
    pub fn write_diagnostics_jsonl(&self, w: &mut impl io::Write) -> io::Result<()> {
        for (_, _, d) in &self.diagnostics {
            writeln!(w, "{}", diagnostic_to_json(d))?;
        }
        Ok(())
    }
}

impl Probe for Recorder {
    #[inline]
    fn enter(&mut self, scope: &'static Scope) {
        let now = self.now_ns();
        self.open.push((scope, now));
    }

    #[inline]
    fn exit(&mut self, scope: &'static Scope) {
        let now = self.now_ns();
        let Some((open, start)) = self.open.pop() else {
            debug_assert!(
                false,
                "Probe::exit without enter: {}/{}",
                scope.module, scope.name
            );
            return;
        };
        debug_assert!(
            core::ptr::eq(open, scope),
            "Probe: scopes not nested ({} closed, {} open)",
            scope.name,
            open.name
        );
        self.spans.push(Span {
            scope: open,
            start_ns: start,
            duration_ns: now.saturating_sub(start),
            depth: self.open.len() as u16,
            tick: self.tick,
        });
    }

    #[inline]
    fn count(&mut self, counter: &'static Counter, value: f64) {
        let at_ns = self.now_ns();
        self.samples.push(Sample {
            counter,
            at_ns,
            value,
        });
    }
}

impl Sink<Diagnostic> for Recorder {
    fn emit(&mut self, d: Diagnostic) {
        if self.diagnostics.len() < self.max_diagnostics {
            let at = self.now_ns();
            self.diagnostics.push((at, self.tick, d));
        } else {
            self.dropped_diagnostics += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{counter, scope};
    use knorke_types::DiagnosticCode;

    fn work(p: &mut impl Probe, n: u32) {
        let mut outer = p.scope(scope!("knorke.test", "outer"));
        for _ in 0..n {
            let _inner = outer.scope(scope!("knorke.test", "inner"));
            std::hint::black_box((0..100).sum::<u32>());
        }
        outer.count(counter!("knorke.test", "items", "items"), n as f64);
    }

    #[test]
    fn records_nested_spans_and_stats() {
        let mut r = Recorder::new("main", 1, 1024);
        r.set_tick(Tick(3));
        work(&mut r, 4);
        let spans: Vec<_> = r.spans().collect();
        assert_eq!(spans.len(), 5);
        assert_eq!((spans[0].scope.name, spans[0].depth), ("inner", 1));
        assert_eq!(
            (spans[4].scope.name, spans[4].depth, spans[4].tick),
            ("outer", 0, Some(Tick(3)))
        );
        let stats = r.stats();
        // Sorted by total time. Not "outer first": with a coarse clock the outer span can
        // measure no longer than its rounded-up inner spans.
        assert!(stats.windows(2).all(|w| w[0].total_ns >= w[1].total_ns));
        let inner = stats.iter().find(|s| s.name == "inner").unwrap();
        assert_eq!(inner.count, 4);
        assert!(
            inner.min_ns <= inner.p50_ns
                && inner.p50_ns <= inner.p95_ns
                && inner.p95_ns <= inner.max_ns
        );
        assert!(r.summary().contains("knorke.test/inner"));
    }

    #[test]
    fn ring_keeps_newest_and_counts_drops() {
        let mut r = Recorder::new("main", 1, 3);
        work(&mut r, 4); // 5 spans into capacity 3
        assert_eq!(r.spans().count(), 3);
        assert_eq!(r.spans().last().unwrap().scope.name, "outer");
        assert_eq!(r.dropped(), 2);
    }

    #[test]
    fn no_allocation_growth_while_recording() {
        let mut r = Recorder::new("main", 1, 16);
        work(&mut r, 100);
        assert_eq!(r.spans.items.capacity(), 16);
        assert_eq!(r.open.capacity(), 64);
    }

    #[test]
    fn chrome_trace_is_well_formed() {
        let epoch = Instant::now();
        let mut a = Recorder::with_epoch("sim", 1, 64, epoch);
        let mut b = Recorder::with_epoch("render \"main\"", 2, 64, epoch);
        a.set_tick(Tick(9));
        work(&mut a, 2);
        work(&mut b, 1);
        a.emit(
            Diagnostic::warning(DiagnosticCode::new("knorke.test.slow"), "took long").at("npc/1"),
        );
        let mut buf = Vec::new();
        Recorder::write_chrome_trace(&[&a, &b], &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.starts_with("{\"displayTimeUnit\":\"ns\",\"traceEvents\":["));
        assert!(text.trim_end().ends_with("]}"));
        assert_eq!(text.matches("\"ph\":\"X\"").count(), 5);
        assert_eq!(text.matches("\"ph\":\"C\"").count(), 2);
        assert!(text.contains("\"name\":\"knorke.test.slow\""));
        assert!(text.contains("render \\\"main\\\""));
        assert_eq!(text.matches('{').count(), text.matches('}').count());
        let mut lines = Vec::new();
        a.write_diagnostics_jsonl(&mut lines).unwrap();
        assert_eq!(lines.iter().filter(|&&c| c == b'\n').count(), 1);
    }
}
