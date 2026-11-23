//! Shared debugging and analysis contracts of the Knorke Engine (ADR 0007).
//!
//! Three channels, each a small trait that a module *accepts* and the host *implements*.
//! Passing the no-op implementation compiles the instrumentation away.
//!
//! | Channel | Trait | Question it answers |
//! |---|---|---|
//! | Trace | [`Sink<E>`] | *Why* did the module do that? (module-specific events: search steps, decisions, light culling) |
//! | Profile | [`Probe`] | *Where* did the time go? (scopes, counters) |
//! | Inspect | [`Inspect`] / [`Visitor`] | *What* is the current state? (structured, no serde) |
//!
//! Errors use [`knorke_types::Diagnostic`], delivered through a `Sink<Diagnostic>`.
//!
//! Automatic bug detection (ADR 0010): [`Invariants`] checked at sync points,
//! [`ProgressWatch`] for objects that stop making progress, and cheap visual glitch
//! heuristics in [`image`].
//!
//! Modules never read a clock or write to a global logger: the host decides whether,
//! where and how events are recorded. With the `std` feature this crate also provides that
//! host side: [`Recorder`] (bounded, allocation-free after warm-up) and exporters for the
//! Chrome/Perfetto trace format and JSON, so traces of all modules end up on one timeline.

#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod image;
mod inspect;
mod invariants;
mod probe;
mod sink;

#[cfg(feature = "std")]
pub mod json;
#[cfg(feature = "std")]
mod recorder;

pub use inspect::{Inspect, Value, Visitor};
pub use invariants::{InvariantStats, Invariants, ProgressWatch, Violations};
pub use probe::{Counter, NoProbe, Probe, Scope, ScopeGuard};
pub use sink::{CallbackSink, MapSink, NoopSink, Sink};

#[cfg(feature = "std")]
pub use recorder::{Recorder, ScopeStats, Span};

/// Declares a `&'static` [`Scope`] in place.
///
/// ```
/// use knorke_observe::{Probe, NoProbe, scope};
/// fn search(probe: &mut impl Probe) {
///     let _s = probe.scope(scope!("knorke.navigation", "search"));
///     // ...
/// }
/// search(&mut NoProbe);
/// ```
#[macro_export]
macro_rules! scope {
    ($module:expr, $name:expr) => {{
        static SCOPE: $crate::Scope = $crate::Scope::new($module, $name);
        &SCOPE
    }};
}

/// Declares a `&'static` [`Counter`] in place.
///
/// ```
/// use knorke_observe::{Probe, NoProbe, counter};
/// let mut probe = NoProbe;
/// probe.count(counter!("knorke.navigation", "expanded_nodes", "nodes"), 42.0);
/// ```
#[macro_export]
macro_rules! counter {
    ($module:expr, $name:expr, $unit:expr) => {{
        static COUNTER: $crate::Counter = $crate::Counter::new($module, $name, $unit);
        &COUNTER
    }};
}
