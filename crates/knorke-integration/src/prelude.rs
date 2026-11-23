// Ported from Sagax (https://github.com/STARSCrazy/sagax), revision fab00c0, crates/sagax/src/prelude.rs.
//! The names most games need: `use knorke_integration::prelude::*;`.
//!
//! Builders such as `add_consideration` answer `false` for an unknown
//! option, and the compiler warns when that answer is thrown away:
//!
//! ```compile_fail
//! #![deny(unused_must_use)]
//! use knorke_integration::prelude::*;
//! let mut d = Decider::new();
//! let o = d.add_option(ONE);
//! let c = Consideration::new(0, Curve::Linear, 0, 10);
//! d.add_consideration(o, c);
//! ```

pub use crate::goal_guess::observe_flow;
pub use crate::perception::{
    Awareness, Contagion, ContagionConfig, DeedInputs, DeedRule, Eye, Percept, SenseProfile, Stats,
    Stimulus, Target, hear, learn_percepts, look, observe, sight_line,
};
pub use crate::walk::FlowDistance;
pub use knorke_ai::group::Status as SquadStatus;
pub use knorke_ai::group::{
    BuiltInRules, Crew, CrewConfig, CrewEvent, Goal, GoalKeeper, KeeperInputs, Member, MemberState,
    Spot, Squad, SquadConfig, SquadRules, SquadView, Tactic, Walking, tactics,
};
pub use knorke_ai::memory::{
    Areas, Distance, Evict, Fact, GoalGuess, GoalGuessConfig, Knowledge, Message, Metric,
    PointPattern, StepError,
};
pub use knorke_ai::mission::BuiltInRules as MissionBuiltInRules;
pub use knorke_ai::mission::{
    Action, BreakReason, Detour, DetourEnd, Fail, FailCtx, Leash, Leeway, Mission, MissionConfig,
    MissionEvent, MissionRules, Motive, OnLost, Plan, PlanCtx, Step, Where,
};
pub use knorke_ai::{Candidate, Choice, Consideration, Curve, DecideError, Decider, ONE, Pick};
pub use knorke_grid::{Cell, HexLayout, Pos};
pub use knorke_navigation::Budget;
pub use knorke_navigation::grid::{CellMap, DenseGrid, GridRules, Preset};
pub use knorke_navigation::nav::Status as NavStatus;
pub use knorke_navigation::nav::{ANY_LEVEL, Navigator, Output, Report};
pub use knorke_navigation::topology::Topology;
