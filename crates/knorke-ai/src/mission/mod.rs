// Ported from Sagax (https://github.com/STARSCrazy/sagax), revision fab00c0, crates/sagax-decide/src/mission/mod.rs.
//! Mission planner: a motive, a plan of steps, detours and replanning.

mod rules;
mod runner;
mod types;

pub use rules::{BuiltInRules, FailCtx, MissionRules, PlanCtx};
pub use runner::{
    DEFAULT_PREFERENCE, DIST_CAP, IN_DIST, IN_EXTRA, IN_FEASIBLE, IN_KNOWN, IN_PREF, IN_WIDEN,
    KNOWN_CAP, Mission, Output, WIDEN_PREFERENCE,
};
pub use types::*;

#[cfg(test)]
mod tests;
