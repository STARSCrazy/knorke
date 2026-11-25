// AI adapters ported from Sagax (https://github.com/STARSCrazy/sagax), revision fab00c0, crates/sagax/src/lib.rs.
//! Adapters that join Knorke modules. Domain modules never depend on this crate (ADR 0001).
//!
//! Each profile is a feature, so a host pulls in only the modules it joins:
//! - feature `ai`: navigation, perception and AI (ADR 0012), see below.
//! - feature `stealth` (not in `default`): [`stealth`], a yard with guards and an intruder
//!   built from a `knorke-scene` scene and run by `knorke-runtime` (ADR 0018).
//! - feature `pixel`: [`pixel`], software renderer (`knorke-render`) + light pass
//!   (`knorke-lighting`), profile `scene.render.lighting` (ADR 0013).
//! - feature `gpu` (not in `default`): [`light2d`], the pass plan of `knorke-light2d` followed
//!   by the post pass of `knorke-post` on the host's GPU device (ADR 0014, ADR 0017).
//!
//! ## AI (feature `ai`)
//!
//! `knorke-navigation`, `knorke-perception` and `knorke-ai` do not know each other. This
//! crate holds the glue that needs two of them: percepts feed memory
//! ([`perception`]), flow fields feed goal guesses ([`goal_guess`]) and walking distances
//! ([`walk`]), navigation costs feed squad rules ([`rules`]). Domain modules never depend
//! on this crate (`cargo xtask deps`).
//!
//! Origin: the `sagax` facade crate of Sagax (revision fab00c0).
//!
//! `use knorke_integration::prelude::*;` brings the types most games need. The parts are
//! also reachable as [`nav`] and [`decide`]; [`perception`] holds sight
//! and hearing with their nav and memory helpers; [`goal_guess`] feeds a
//! goal guess from flow fields; [`walk`] gives the squad walking distances
//! from flow fields; [`debug`] holds
//! everything trace related.

#[cfg(feature = "gpu")]
pub mod layers;
#[cfg(feature = "gpu")]
pub mod light2d;
#[cfg(feature = "pixel")]
pub mod pixel;
#[cfg(feature = "gpu")]
pub mod sprites;
#[cfg(feature = "gpu")]
pub mod surface;

#[cfg(feature = "ai")]
pub use knorke_ai as decide;
#[cfg(feature = "ai")]
pub use knorke_grid::{Cell, HexLayout, Pos};
#[cfg(feature = "ai")]
pub use knorke_navigation as nav;

#[cfg(feature = "ai")]
pub mod goal_guess;
#[cfg(feature = "ai")]
pub mod perception;
#[cfg(feature = "ai")]
pub mod prelude;
#[cfg(feature = "ai")]
pub mod rules;
#[cfg(feature = "stealth")]
pub mod stealth;
#[cfg(feature = "stealth-view")]
pub mod stealth_view;
#[cfg(feature = "ai")]
pub mod walk;

#[cfg(feature = "ai")]
pub mod debug {
    //! Everything for tracing, in one place: `use knorke_integration::debug::*;`.
    //!
    //! A trace is a [`Sink`] parameter: every operation that decides something
    //! has an `op_traced(.., sink)` form next to the plain `op(..)`, which is
    //! the traced one with [`NoopSink`] and costs nothing. A [`Collector`] keeps
    //! the last (or first) records of a call; a facade hands the records of the
    //! module it drives on, wrapped, through a [`MapSink`].
    //!
    //! ```
    //! use knorke_integration::debug::*;
    //! use knorke_integration::prelude::*;
    //!
    //! let mut map = DenseGrid::new(10, 10);
    //! let mut nav = Navigator::new(Topology::Square(GridRules::preset(Preset::Octile)), 100);
    //! let mut log = Collector::new(256, Overflow::DropOldest);
    //! let goal = [Cell::new(5, 5, 0)];
    //! nav.find_path_traced(&mut map, Cell::new(0, 0, 0), &goal, Budget::UNLIMITED, &mut log);
    //! assert!(matches!(
    //!     log.records().last().unwrap().event,
    //!     NavTrace::Search(SearchTrace::Finished { .. })
    //! ));
    //! ```

    pub use knorke_ai::group::{CrewTrace, KeeperTrace, SpotTrace, SquadTrace};
    pub use knorke_ai::memory::{
        AreasTrace, GoalGuessTrace, KnowledgeTrace, MessageTrace, TimersTrace,
    };
    pub use knorke_ai::{DecideTrace, mission::MissionTrace, state::StateTrace};
    pub use knorke_navigation::{
        formation::FormationTrace, nav::NavTrace, search::SearchTrace, smooth::SmoothTrace,
    };
    pub use knorke_observe::{CallbackSink, MapSink, NoopSink, Sink};
    pub use knorke_observe::{Collector, Overflow, Record};
    pub use knorke_perception::{ContagionTrace, PerceptionTrace};
}
