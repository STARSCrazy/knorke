//! Adapters that join Knorke modules. Domain modules never depend on this crate (ADR 0001).
//!
//! Each adapter lives in its own module so independent profiles stay separate:
//! - [`pixel`]: software renderer (`knorke-render`) + light pass (`knorke-lighting`),
//!   profile `scene.render.lighting` (ADR 0013).

pub mod pixel;
