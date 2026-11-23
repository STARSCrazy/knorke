//! Automatic bug detection: invariants checked at sync points, and stuck detection
//! (ADR 0010, plan section 17.9).
//!
//! Cheapest oracle first: an invariant on simulation state (no NaN positions, nothing inside
//! walls, handles valid) catches most bugs exactly at the tick they happen. Violations are
//! [`Diagnostic`]s, so they reach agents, CI logs and the recorder through the same channel
//! as every other finding – and can trigger a flight-recorder dump.

use crate::Sink;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use knorke_types::{Diagnostic, DiagnosticCode, Handle, HandleMap, Severity, Tick};

/// Collects the violations of one invariant during one check.
pub struct Violations<'a> {
    code: &'a DiagnosticCode,
    severity: Severity,
    sink: &'a mut dyn Sink<Diagnostic>,
    reported: u32,
    limit: u32,
    total: u32,
}

impl Violations<'_> {
    /// Reports one violation. Only the first `limit` per check become diagnostics; the rest
    /// are counted and summarised, so 10 000 broken entities do not flood the log.
    pub fn report(&mut self, message: impl Into<String>, path: Option<&str>) {
        self.total += 1;
        if self.reported < self.limit {
            self.reported += 1;
            let mut d = Diagnostic::new(self.severity, self.code.clone(), message);
            d.path = path.map(String::from);
            self.sink.emit(d);
        }
    }

    /// Number of violations reported so far in this check.
    pub fn count(&self) -> u32 {
        self.total
    }
}

type CheckFn<W> = dyn Fn(&W, Tick, &mut Violations<'_>) + Send + Sync;

struct Check<W: ?Sized> {
    code: DiagnosticCode,
    severity: Severity,
    every: u64,
    enabled: bool,
    violations: u64,
    f: Box<CheckFn<W>>,
}

/// Per-invariant counters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvariantStats {
    /// Code of the invariant.
    pub code: DiagnosticCode,
    /// Total violations since creation.
    pub violations: u64,
}

/// A set of invariants over a world of type `W`, run by the host at sync points.
pub struct Invariants<W: ?Sized> {
    checks: Vec<Check<W>>,
    limit_per_check: u32,
}

impl<W: ?Sized> Default for Invariants<W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: ?Sized> Invariants<W> {
    /// Empty set; at most 8 diagnostics per invariant and check.
    pub fn new() -> Self {
        Self {
            checks: Vec::new(),
            limit_per_check: 8,
        }
    }

    /// Changes how many diagnostics one invariant may emit per check.
    pub fn with_limit(mut self, limit_per_check: u32) -> Self {
        self.limit_per_check = limit_per_check;
        self
    }

    /// Registers an invariant checked every `every` ticks (1 = every tick). Violations are
    /// errors with the given code.
    pub fn add(
        &mut self,
        code: DiagnosticCode,
        every: u64,
        check: impl Fn(&W, Tick, &mut Violations<'_>) + Send + Sync + 'static,
    ) -> &mut Self {
        self.checks.push(Check {
            code,
            severity: Severity::Error,
            every: every.max(1),
            enabled: true,
            violations: 0,
            f: Box::new(check),
        });
        self
    }

    /// Enables or disables all invariants with `code`.
    pub fn set_enabled(&mut self, code: &DiagnosticCode, enabled: bool) {
        self.checks
            .iter_mut()
            .filter(|c| &c.code == code)
            .for_each(|c| c.enabled = enabled);
    }

    /// `true` if no invariant is registered (running costs nothing then).
    pub fn is_empty(&self) -> bool {
        self.checks.is_empty()
    }

    /// Runs the invariants due at `tick`. Returns the number of violations found.
    pub fn run(&mut self, world: &W, tick: Tick, sink: &mut dyn Sink<Diagnostic>) -> u32 {
        let mut found = 0;
        for c in self
            .checks
            .iter_mut()
            .filter(|c| c.enabled && tick.0.is_multiple_of(c.every))
        {
            let mut v = Violations {
                code: &c.code,
                severity: c.severity,
                sink: &mut *sink,
                reported: 0,
                limit: self.limit_per_check,
                total: 0,
            };
            (c.f)(world, tick, &mut v);
            let (total, reported) = (v.total, v.reported);
            if total > reported {
                sink.emit(
                    Diagnostic::new(
                        c.severity,
                        c.code.clone(),
                        alloc::format!("{} further violations suppressed", total - reported),
                    )
                    .at(alloc::format!("tick/{}", tick.0)),
                );
            }
            c.violations += total as u64;
            found += total;
        }
        found
    }

    /// Violation counters per invariant.
    pub fn stats(&self) -> Vec<InvariantStats> {
        self.checks
            .iter()
            .map(|c| InvariantStats {
                code: c.code.clone(),
                violations: c.violations,
            })
            .collect()
    }
}

/// Detects objects that should make progress but do not: a NPC with a goal that does not get
/// closer, a quest step that never completes, a projectile that never arrives.
///
/// Feed it a "remaining work" measure per object each tick (distance to goal, steps left).
/// It reports objects whose best value has not improved by at least `min_progress` for
/// `patience` ticks.
pub struct ProgressWatch<K: ?Sized> {
    state: HandleMap<K, (Tick, f32)>,
    patience: u64,
    min_progress: f32,
}

impl<K: ?Sized> ProgressWatch<K> {
    /// Reports after `patience` ticks without an improvement of at least `min_progress`.
    pub fn new(patience: u64, min_progress: f32) -> Self {
        Self {
            state: HandleMap::new(),
            patience,
            min_progress,
        }
    }

    /// Records the remaining work of `object` at `tick`. Returns the number of ticks without
    /// progress if that exceeds the patience.
    pub fn observe(&mut self, object: Handle<K>, tick: Tick, remaining: f32) -> Option<u64> {
        match self.state.get_mut(object) {
            Some((last, best)) => {
                if remaining <= *best - self.min_progress {
                    *best = remaining;
                    *last = tick;
                    None
                } else {
                    tick.since(*last).filter(|&d| d >= self.patience)
                }
            }
            None => {
                self.state.insert(object, (tick, remaining));
                None
            }
        }
    }

    /// Stops watching `object` (goal reached, despawned, new goal).
    pub fn forget(&mut self, object: Handle<K>) {
        self.state.remove(object);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knorke_types::Generation;

    struct World {
        positions: Vec<f32>,
    }

    const NAN: DiagnosticCode = DiagnosticCode::new("knorke.test.position_nan");
    const RANGE: DiagnosticCode = DiagnosticCode::new("knorke.test.out_of_world");

    fn invariants() -> Invariants<World> {
        let mut inv = Invariants::new().with_limit(2);
        inv.add(NAN, 1, |w: &World, _, v| {
            for (i, p) in w.positions.iter().enumerate() {
                if !p.is_finite() {
                    v.report(
                        "position is not finite",
                        Some(&alloc::format!("entities/{i}/pos")),
                    );
                }
            }
        });
        inv.add(RANGE, 10, |w: &World, _, v| {
            for p in w.positions.iter().filter(|p| p.abs() > 1000.0) {
                v.report(alloc::format!("position {p} outside the world"), None);
            }
        });
        inv
    }

    #[test]
    fn violations_become_limited_diagnostics() {
        let mut inv = invariants();
        let world = World {
            positions: alloc::vec![0.0, f32::NAN, f32::INFINITY, f32::NAN, 5000.0],
        };
        let mut out: Vec<Diagnostic> = Vec::new();
        assert_eq!(
            inv.run(&world, Tick(3), &mut out),
            3,
            "range check not due at tick 3"
        );
        assert_eq!(out.len(), 3, "2 reported + 1 suppression summary");
        assert_eq!(out[0].path.as_deref(), Some("entities/1/pos"));
        assert!(out[2].message.contains("1 further"));
        out.clear();
        assert_eq!(
            inv.run(&world, Tick(10), &mut out),
            5,
            "3 non-finite + INF and 5000 out of range"
        );
        inv.set_enabled(&NAN, false);
        out.clear();
        assert_eq!(inv.run(&world, Tick(20), &mut out), 2);
        assert_eq!(
            inv.stats().iter().map(|s| s.violations).collect::<Vec<_>>(),
            [6, 4]
        );
    }

    #[test]
    fn healthy_world_is_silent() {
        let mut inv = invariants();
        let mut out: Vec<Diagnostic> = Vec::new();
        assert_eq!(
            inv.run(
                &World {
                    positions: alloc::vec![1.0, 2.0]
                },
                Tick(10),
                &mut out
            ),
            0
        );
        assert!(out.is_empty());
    }

    #[test]
    fn progress_watch_flags_stuck_objects() {
        struct Npc;
        let a = Handle::<Npc>::new(0, Generation::MIN);
        let b = Handle::<Npc>::new(1, Generation::MIN);
        let mut w = ProgressWatch::new(30, 0.5);
        let mut stuck = Vec::new();
        for t in 0..100u64 {
            // a walks towards its goal, b jitters in place
            if w.observe(a, Tick(t), 100.0 - t as f32).is_some() {
                stuck.push(("a", t));
            }
            let jitter = if t % 2 == 0 { 50.0 } else { 50.2 };
            if let Some(d) = w.observe(b, Tick(t), jitter) {
                stuck.push(("b", t));
                assert!(d >= 30);
            }
        }
        assert!(stuck.iter().all(|(n, _)| *n == "b"));
        assert_eq!(stuck.first(), Some(&("b", 30)));
        w.forget(b);
        assert_eq!(w.observe(b, Tick(200), 50.0), None);
    }
}
