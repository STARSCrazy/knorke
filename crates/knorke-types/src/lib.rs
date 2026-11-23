//! Shared data contracts of the Knorke Engine.
//!
//! Everything here exists so that independent modules can exchange data **without
//! copying or converting** it and run **in parallel without owning threads**
//! (ADR 0006, ADR 0008):
//!
//! - identities: [`Handle`], with owning storage [`Slots`], per-module side data in
//!   [`HandleMap`] and per-entity flags in [`BitSet`];
//! - borrowed bulk data: [`Column`] / [`ColumnMut`], [`project`] for fields inside host
//!   structs;
//! - communication without a broker: typed [`EventQueue`]s, deferred [`CommandBuffer`]s,
//!   change detection with [`Revision`];
//! - parallelism: the [`Parallel`] executor contract ([`Serial`] here, threaded executors
//!   in `knorke-exec`);
//! - determinism: [`Tick`], [`Seed`], [`StableHasher`];
//! - saving/restoring state and checksums: [`Snapshot`], [`SnapshotBuf`], [`Checksum`]
//!   (ADR 0010);
//! - errors: [`Diagnostic`].
//!
//! The crate has no dependencies, no `unsafe`, no global state and works in `no_std`
//! environments with `alloc`. Coordinates, math and GPU types live elsewhere
//! (`knorke-math`), debugging contracts in `knorke-observe`.

#![no_std]

extern crate alloc;

mod bitset;
mod column;
mod commands;
mod diagnostic;
mod events;
mod handle;
mod hash;
mod parallel;
mod slots;
mod snapshot;
mod time;

pub use bitset::BitSet;
pub use column::{Column, ColumnMut, Project, ProjectMut, project, project_mut};
pub use commands::CommandBuffer;
pub use diagnostic::{Diagnostic, DiagnosticCode, Severity};
pub use events::{EventCursor, EventQueue, Events, Revision};
pub use handle::{Generation, Handle};
pub use hash::{StableHasher, StableState};
pub use parallel::{Parallel, Serial, chunk_count};
pub use slots::{HandleMap, Slots};
pub use snapshot::{
    Checksum, Reader, SectionInfo, Snapshot, SnapshotBuf, SnapshotError, SnapshotValue, Writer,
    restore,
};
pub use time::{Seed, Tick};
