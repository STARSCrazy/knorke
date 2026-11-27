//! 2D lighting of the Knorke Engine, ported from Chiaro (ADR 0014): the GPU-free part.
//!
//! This is the CPU logic that does not need a graphics device and can be tested headless:
//!
//! | Module | Content | Chiaro package |
//! |---|---|---|
//! | [`projection`] | world to scene-pixel projection (ortho, isometric) | P5 |
//! | [`triangulate`] | ear-clipping triangulation of simple polygons | P5 |
//! | [`codec`] | byte layout of the surface, material and medium buffers, normal-map conversion | P6 |
//! | [`material`] | material shader math as CPU model (tangent frame, specular lobes) | P6 |
//! | [`light_math`] | attenuation, spot cone, flicker, light-quad culling | P7 |
//! | [`light`], [`light_store`] | light description, validation, handle-based store, snapshots | P7 |
//! | [`obstacle`] | obstacle descriptions, handle-based store, dirty tracking for the shadow cache, snapshots | P13 |
//! | [`animation`], [`area_light`], [`light_culling`], [`gpu_light`] | animation curves and store, area-light model, per-frame light lists | P8 |
//! | [`mask`], [`mask_store`], [`light_mask`], [`resources`] | masks (gobo, window): shapes, store, placement; stores as light resources | P10 |
//! | [`medium_table`], [`medium_culling`] | media table with presets; the lights and suns of the medium stage | P12 |
//! | [`medium_map`] | medium world map: uniform blocks, grid, medium program layouts; `MediumMapStage` with `gpu` | P29 |
//! | [`pattern_store`] | obstacle patterns: store with shelf-packed atlas places, sampling | P15 |
//! | [`shadow`] | occlusion domain and anchor, dirty rectangles, incremental mask update plan | P16 |
//! | [`occluder_raster`] | obstacles into the occlusion mask: triangle buffers, tile cell textures | P22 |
//! | [`occlusion`] | occlusion mask cache (GPU stage with `gpu`) | P23 |
//! | [`settings`] | settings and quality presets of the light (replace `CullSettings`) | P19 |
//! | [`frame_warnings`] | debug-check findings of a frame as stable diagnostics | P18 |
//! | [`binding_layout`] | names and slots of the resources the medium and composite programs bind | P24 |
//! | [`lighting`] | light pass: uniform blocks; `LightingStage` with `gpu` | P25 |
//! | [`composite`] | composite pass: programs, binding layouts, uniform block; `CompositeStage` with `gpu` | P27 |
//! | [`pass_plan`] | the fixed stage order of a frame; `PassPlan` with `gpu` | P31 |
//! | [`ring_light`] | ring light of shockwaves: input and uniform blocks; `RingLightPass` with `gpu` | P21 |
//! | [`rows`] | light rows (ADR 0019): every light's shadow from a row per lamp, every sun's from a map, their caches; `RowLights` with `gpu` | – |
//! | [`surface_geometry`] | surface helpers (box, wall, floor, quad as triangles) and surface sprite encoding | P35 |
//!
//! GPU stages come behind the `gpu` feature and take `knorke-gpu` as their only graphics
//! dependency: the occluder raster (P22), the occlusion mask (P23), the atlases and the zone
//! pass (P26), the light pass with the rows (P25, ADR 0019), the composite pass (P27) and the
//! pass plan that runs them in order (P31). The output (tonemap, bloom, target) is
//! `knorke-post`; `knorke-integration` joins both.
//! This crate does not know `knorke-post`, `knorke-lighting`, `knorke-render` or
//! the AI crates (`cargo xtask deps`).
//!
//! What changed against the C++ library is stated in the module docs: the C ABI and its flag
//! bits, raw pointers and SDL error strings are gone; coordinate spaces, handles from
//! [`knorke_types::Slots`], `Column` inputs, typed errors with stable `knorke.light2d.*`
//! diagnostic codes and snapshots take their place. Maths that influences results uses
//! [`knorke_math::det`], so results are identical on every platform.
//!
//! Origin: chiaro @ 9bc5b9b (zlib license, `LICENSE-CHIARO-ZLIB.txt`); every ported file names its
//! source path.

// `!(x > 0.0)` rejects NaN on purpose, as in the C++ original.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod ambient_zone;
pub mod ambient_zone_culling;
pub mod ambient_zone_store;
pub mod animation;
pub mod area_light;
pub mod atlas;
pub mod binding_layout;
pub mod codec;
pub mod composite;
pub mod debug_check;
pub mod frame_warnings;
pub mod gpu_light;
pub mod light;
pub mod light_culling;
pub mod light_mask;
pub mod light_math;
pub mod light_store;
pub mod lighting;
pub mod mask;
pub mod mask_store;
pub mod material;
pub mod medium_culling;
pub mod medium_draw;
pub mod medium_map;
pub mod medium_stage;
pub mod medium_table;
pub mod obstacle;
pub mod occluder_raster;
pub mod occlusion;
pub mod pass_plan;
pub mod pattern_occluders;
pub mod pattern_store;
pub mod profile;
pub mod profile_import;
pub mod profile_store;
pub mod projection;
pub mod resources;
pub mod ring_light;
pub mod rows;
pub mod settings;
pub mod shadow;
pub mod surface_geometry;
pub mod triangulate;
pub mod zone_pass;

pub use animation::{AnimationDesc, AnimationError, AnimationId, AnimationStore};
pub use light::{LightError, LightResources, NoResources};
pub use light_store::LightStore;
pub use mask::MaskError;
pub use mask_store::{MaskId, MaskStore};
pub use obstacle::{ObstacleError, ObstacleId, ObstacleStore};
pub use pattern_store::{PatternError, PatternId, PatternStore};
pub use projection::{Projection, ProjectionError};
pub use resources::StoreResources;
pub use triangulate::{TriangulateError, Triangulator, circle_triangles};

/// `Snapshot::restore` of a section that reads only its current `VERSION`.
pub(crate) fn check_version<S: knorke_types::Snapshot>(
    version: u16,
) -> Result<(), knorke_types::SnapshotError> {
    if version == S::VERSION {
        Ok(())
    } else {
        Err(knorke_types::SnapshotError::UnsupportedVersion { id: S::ID, version })
    }
}

/// Index of the first handle of `ids` equal to an earlier one, `None` without repeats. `sorted`
/// is reused scratch: a sorted copy of the bits tells cheaply whether there is a repeat at all.
pub(crate) fn first_repeat<T>(
    ids: &impl knorke_types::Column<knorke_types::Handle<T>>,
    sorted: &mut Vec<u64>,
) -> Option<usize> {
    sorted.clear();
    sorted.extend(ids.iter().map(|id| id.to_bits()));
    sorted.sort_unstable();
    if !sorted.windows(2).any(|w| w[0] == w[1]) {
        return None;
    }
    Some(
        (0..ids.len())
            .find(|&i| (0..i).any(|j| ids.get(j) == ids.get(i)))
            .unwrap_or(0),
    )
}
