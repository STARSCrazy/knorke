//! Profiling probes: where does the time go, and how much work was done?

use core::ops::{Deref, DerefMut};

/// Static description of a timed region. Create with the [`scope!`](crate::scope) macro.
#[derive(Debug)]
pub struct Scope {
    /// Owning module, e.g. `knorke.navigation`.
    pub module: &'static str,
    /// Region name, e.g. `search`.
    pub name: &'static str,
}

impl Scope {
    /// Creates a scope description.
    pub const fn new(module: &'static str, name: &'static str) -> Self {
        Self { module, name }
    }
}

/// Static description of a measured quantity. Create with the [`counter!`](crate::counter) macro.
#[derive(Debug)]
pub struct Counter {
    /// Owning module.
    pub module: &'static str,
    /// Quantity name, e.g. `expanded_nodes`.
    pub name: &'static str,
    /// Unit, e.g. `nodes`, `bytes`, `lights`.
    pub unit: &'static str,
}

impl Counter {
    /// Creates a counter description.
    pub const fn new(module: &'static str, name: &'static str, unit: &'static str) -> Self {
        Self { module, name, unit }
    }
}

/// Receives profiling markers from a module. The host owns the clock.
///
/// Scopes must be properly nested per probe: every `enter` is matched by an `exit` of the
/// same scope, innermost first. [`Probe::scope`] guarantees that.
pub trait Probe {
    /// `false` if markers are ignored anyway.
    #[inline]
    fn enabled(&self) -> bool {
        true
    }

    /// A timed region starts.
    fn enter(&mut self, scope: &'static Scope);

    /// The innermost open region ends.
    fn exit(&mut self, scope: &'static Scope);

    /// Records a value of a counter at this point in time.
    #[inline]
    fn count(&mut self, counter: &'static Counter, value: f64) {
        let _ = (counter, value);
    }

    /// Enters `scope` and exits it when the guard drops. The guard derefs to the probe, so
    /// nested scopes and counters go through it.
    #[inline]
    fn scope(&mut self, scope: &'static Scope) -> ScopeGuard<'_, Self>
    where
        Self: Sized,
    {
        self.enter(scope);
        ScopeGuard { probe: self, scope }
    }
}

/// Ignores everything.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoProbe;

impl Probe for NoProbe {
    #[inline]
    fn enabled(&self) -> bool {
        false
    }
    #[inline]
    fn enter(&mut self, _: &'static Scope) {}
    #[inline]
    fn exit(&mut self, _: &'static Scope) {}
}

impl<P: Probe + ?Sized> Probe for &mut P {
    #[inline]
    fn enabled(&self) -> bool {
        (**self).enabled()
    }
    #[inline]
    fn enter(&mut self, scope: &'static Scope) {
        (**self).enter(scope);
    }
    #[inline]
    fn exit(&mut self, scope: &'static Scope) {
        (**self).exit(scope);
    }
    #[inline]
    fn count(&mut self, counter: &'static Counter, value: f64) {
        (**self).count(counter, value);
    }
}

/// Exits its scope on drop. See [`Probe::scope`].
pub struct ScopeGuard<'p, P: Probe> {
    probe: &'p mut P,
    scope: &'static Scope,
}

impl<P: Probe> Deref for ScopeGuard<'_, P> {
    type Target = P;
    fn deref(&self) -> &P {
        self.probe
    }
}

impl<P: Probe> DerefMut for ScopeGuard<'_, P> {
    fn deref_mut(&mut self) -> &mut P {
        self.probe
    }
}

impl<P: Probe> Drop for ScopeGuard<'_, P> {
    #[inline]
    fn drop(&mut self) {
        self.probe.exit(self.scope);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[derive(Default)]
    struct Log(Vec<(char, &'static str)>);
    impl Probe for Log {
        fn enter(&mut self, s: &'static Scope) {
            self.0.push(('>', s.name));
        }
        fn exit(&mut self, s: &'static Scope) {
            self.0.push(('<', s.name));
        }
        fn count(&mut self, c: &'static Counter, _: f64) {
            self.0.push(('#', c.name));
        }
    }

    #[test]
    fn guards_nest() {
        let mut log = Log::default();
        {
            let mut outer = log.scope(crate::scope!("m", "outer"));
            {
                let mut inner = outer.scope(crate::scope!("m", "inner"));
                inner.count(crate::counter!("m", "n", "items"), 1.0);
            }
        }
        assert_eq!(
            log.0,
            [
                ('>', "outer"),
                ('>', "inner"),
                ('#', "n"),
                ('<', "inner"),
                ('<', "outer")
            ]
        );
    }
}
