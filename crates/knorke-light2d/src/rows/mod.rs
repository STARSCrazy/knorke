//! Light rows (ADR 0019): the shadow of a point or spot light from one row per lamp instead of a
//! ray march per pixel; a line or rect light takes it from the rows of [`SUPPORTS`] points on
//! its emitter.
//!
//! A lamp's row holds, for each of [`ROW_ANGLES`] directions around its ground point, the up to
//! four obstacle runs (near, far, base, top) a ray meets on its way out through the occlusion
//! mask. The light looks its shadow up in the row; a lamp with a source radius reads a second
//! row, written by the edge pass, that says where the outlines of those obstacles lie across the
//! ray. In frames with pattern obstacles a third row says where a ray crosses one (the pattern
//! pass); the light reads the pattern there at the height of its own ray. A row depends only on
//! the lamp and the mask around it, so it is kept until one of them changes ([`RowCache`]).
//!
//! This is the transition of plan 17.13: with [`LightSettings::light_rows`] the pass plan draws
//! every light [`light_suits`] accepts in a frame [`frame_suits`] accepts with the row
//! programs, all others stay with Chiaro's programs in the same pass and targets.
//!
//! | Part | Content |
//! |---|---|
//! | here | which lights are row lights, the lamp record and uniform blocks, the row cache |
//! | `gpu` (feature `gpu`) | `RowLights`: row and edge atlas, programs, recording |
//!
//! The shaders (`lightrow_*` in `knorke-integration/shaders`) share the constants below. The row
//! cache is GPU cache state, not simulation state: no snapshot (ADR 0016).

#[cfg(feature = "gpu")]
mod gpu;

#[cfg(feature = "gpu")]
pub use gpu::{ROWS_INPUT_MISSING, RowLights, RowsRecord};

use knorke_math::uniform_block;

use knorke_math::{At, Vec2, Vec3, World2, det};

use crate::area_light::rect_across;
use crate::gpu_light::{GpuLight, GpuMask};
use crate::light::{LightDesc, LightRef, LightType, ShadowFilter};
use crate::obstacle::ObstacleStore;
use crate::projection::Projection;
use crate::settings::LightSettings;
use crate::shadow::{DirtyRectList, MaskAnchor, OcclusionDomain, obstacle_grid_rect};

/// Angles of a row (texel `a` holds the ray at `a + 0.5`).
pub const ROW_ANGLES: u32 = 1024;
/// Step of the row build's march in mask texels.
pub const MARCH_STEP: f32 = 0.5;
/// Longest reach of a row lamp in mask texels: the build marches at most 1200 steps
/// (`kMaxSteps` in `lightrow_build.frag`), and a row stores distances up to 1023.75.
pub const MAX_REACH: f32 = 600.0;
/// Most rows of the atlas, so most shadow-casting row lights of a frame; further ones stay with
/// Chiaro's programs.
pub const MAX_ROWS: u32 = 256;
/// Obstacle tops from here on (world units) are infinitely tall to a row: it stores heights in
/// hundredths as 12-bit numbers (`PackRun` in `lightrow.hlsli`).
pub const MAX_TOP: f32 = 40.9;
/// Texels before the receiver that do not count as occluders (Chiaro's trace: 1).
pub const RECEIVER_GAP: f32 = 1.0;
/// Most angles the edge pass walks to each side (below `kEdgeFar` of the shaders).
pub const EDGE_WIDTH: f32 = 32.0;
/// Distance in mask texels from which on an obstacle's penumbra is whole; nearer obstacles get
/// theirs cut at the walked width.
pub const EDGE_NEAR: f32 = 16.0;
/// A lamp's ground point may drift this many mask texels from where its row was built: float
/// noise of a camera pan, far below the quarter texel a row resolves.
pub const GROUND_TOLERANCE: f32 = 1e-3;

/// Whether a frame can draw row lights at all. It cannot
/// - without a surface buffer (the row light is a surface light),
/// - with a medium world map (the row programs have no `*_medium` variant),
/// - with a finite obstacle top (`max_top`, the highest) of [`MAX_TOP`] or more.
///
/// Tile grids with a pattern exclude only the lamps that reach them ([`RowBlockers`]); partly
/// transparent obstacles, the other pattern obstacles and the application's sprite mask (its
/// heights are in the height texture) are runs of the row. The sprite mask may change anywhere
/// every frame, so its scene rectangle counts as changed every frame and the rows of the lamps
/// that reach it are rebuilt every frame.
pub fn frame_suits(
    settings: &LightSettings,
    surface: bool,
    medium_map: bool,
    max_top: f32,
) -> bool {
    settings.light_rows && surface && !medium_map && max_top < MAX_TOP
}

/// Mask rectangles of the active tile grids with a pattern: the pattern pass finds crossings of
/// boxes, circles and polygons only (`lightrow_pattern.hlsli`), so a shadow-casting lamp whose
/// reach meets such a grid stays with Chiaro's programs. One that meets none is exact as a row
/// light: its row describes the mask within its reach only, and an obstacle beyond can shadow
/// nothing it lights.
///
/// A lamp switching between the two ways sits the other's frames out, so its row is rebuilt
/// when it comes back ([`RowCache::assign`]).
#[derive(Clone, Debug, Default)]
pub struct RowBlockers {
    /// In mask texels of the frame; overflowed: every lamp counts as reaching one.
    rects: DirtyRectList,
    /// World points of [`obstacle_grid_rect`], kept so steady frames do not allocate.
    scratch: Vec<At<World2, Vec2>>,
}

impl RowBlockers {
    /// Collects the frame's blockers (none if the frame draws no row lights).
    pub fn collect(
        &mut self,
        rows: bool,
        obstacles: &ObstacleStore,
        p: &Projection,
        domain: &OcclusionDomain,
    ) {
        self.rects.clear();
        if !rows || !obstacles.any_pattern() {
            return;
        }
        let [sx, sy] = MaskAnchor::compute(p, domain).snapped;
        for (id, s) in obstacles.iter() {
            if s.is_active() && s.patterns.refs > 0 && s.is_tile_grid() {
                let r = obstacle_grid_rect(obstacles, id, p, domain, &mut self.scratch);
                self.rects.add(r.offset(sx, sy));
            }
        }
    }

    /// A lamp with this ground point and reach (`RowLight::reach`) can meet a blocker.
    pub fn meet(&self, ground: [f32; 2], reach: f32) -> bool {
        touches(&self.rects, ground, reach)
    }
}

/// The square of `reach` around `ground` (mask texels) overlaps a rectangle of `list`, or the
/// list overflowed.
fn touches(list: &DirtyRectList, ground: [f32; 2], reach: f32) -> bool {
    let lo = [0, 1].map(|i| (ground[i] - reach).floor() as i32);
    let hi = [0, 1].map(|i| (ground[i] + reach).ceil() as i32);
    list.overflowed()
        || list
            .rects()
            .iter()
            .any(|r| r.x0 < hi[0] && r.x1 > lo[0] && r.y0 < hi[1] && r.y1 > lo[1])
}

/// Rows of a shadowed line or rect light, one per support point.
pub const SUPPORTS: u32 = 4;
/// The support points of a line and of a rect light in emitter coordinates (t along U, s along
/// V): the middles of four equal pieces, so their mean is the midpoint rule over the emitter.
/// `kSupports` in `lightrow_surface.hlsli`.
const SUPPORT_TS: [[[f32; 2]; 4]; 2] = [
    [[-0.75, 0.0], [-0.25, 0.0], [0.25, 0.0], [0.75, 0.0]],
    [[-0.5, -0.5], [0.5, -0.5], [-0.5, 0.5], [0.5, 0.5]],
];

/// Whether a light can be a row light in a frame [`frame_suits`] accepts: a point, spot, line or
/// rect light (masks only point and spot lights have); if it casts shadows also without a caster
/// filter (E53: a row is built from the mask of all casters), not marked `inside_obstacle`
/// (E147: the row skips only the obstacle a lamp's height lies in) and with a reach
/// ([`RowLight::reach`]) of at most [`MAX_REACH`] mask texels.
/// Animation, flicker, a turning cone, a profile and a mask change nothing a row holds.
pub fn light_suits(d: &LightDesc, reach: f32) -> bool {
    d.kind != LightType::Directional
        && (!d.cast_shadows
            || (d.shadow_filter == ShadowFilter::All && !d.inside_obstacle && reach <= MAX_REACH))
}

/// The ground plane as the rows see it: mask texels to world units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowSpace {
    /// `world = (dot(xy, m), dot(zw, m))` for a vector `m` in mask texels.
    pub world_from_mask: [f32; 4],
    /// The fewest world units a mask texel covers in any direction.
    pub shortest: f32,
}

impl RowSpace {
    /// The space of a frame.
    pub fn new(p: &Projection, domain: &OcclusionDomain) -> Self {
        let det = f64::from(p.x_axis.x) * f64::from(p.y_axis.y)
            - f64::from(p.y_axis.x) * f64::from(p.x_axis.y);
        let [tx, ty] = domain.texels_per_scene.map(f64::from);
        let m = [
            f64::from(p.y_axis.y) / det / tx,
            -f64::from(p.y_axis.x) / det / ty,
            -f64::from(p.x_axis.y) / det / tx,
            f64::from(p.x_axis.x) / det / ty,
        ];
        // The smaller singular value of the 2 x 2 matrix.
        let sum: f64 = m.iter().map(|v| v * v).sum();
        let d = m[0] * m[3] - m[1] * m[2];
        // Rounding may take either root's argument just below zero.
        let root = |v: f64| if v > 0.0 { v.sqrt() } else { 0.0 };
        let shortest = root((sum - root(sum * sum - 4.0 * d * d)) * 0.5);
        Self {
            world_from_mask: m.map(|v| v as f32),
            shortest: shortest as f32,
        }
    }

    /// How far a lamp of `radius` reaches in mask texels, in the direction it reaches farthest,
    /// plus one texel.
    pub fn reach(&self, radius: f32) -> f32 {
        radius / self.shortest + 1.0
    }
}

/// A row light of a frame (`LightList::rows`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowLight {
    /// The light as Chiaro's programs get it.
    pub light: GpuLight,
    /// What its row is remembered under ([`row_key`]; a support point's adds its index).
    pub key: u64,
    /// It casts shadows this frame, so it needs a row.
    pub shadowed: bool,
    /// [`RowSpace::reach`] of the distance its rows must reach ([`row_span`] beyond the radius).
    pub reach: f32,
    /// Source radius in world units (0: hard shadows).
    pub size: f32,
    /// A point or spot light's mask (E115): drawn by the `_mask` programs. A window mask's ray
    /// ends at the window, so the obstacles nearer the lamp do not count.
    pub mask: Option<GpuMask>,
    /// A shadowed window mask's rectangle on the ground, in mask texels from the lamp's ground
    /// point: `[0]` xy its middle, zw the half side U, `[1]` xy the half side V; all zero
    /// without. The row is built from where a ray enters it on: what lies nearer never counts,
    /// and an infinitely tall wall there must not end the march.
    pub window: [[f32; 4]; 2],
}

/// The key of a light's row: its slot index in the light store, transient lights counted down
/// from the top so they never share a persistent lamp's row; the low bits number the support
/// points ([`RowLight::support`]).
pub fn row_key(light: LightRef) -> u64 {
    u64::from(match light {
        LightRef::Persistent(id) => id.index(),
        LightRef::Transient(i) => u32::MAX - 1 - i,
    }) << 2
}

/// A rect light's half extent across (V), zero for other lights.
fn across(g: &GpuLight) -> Vec3 {
    let v = |a: [f32; 4]| Vec3::new(a[0], a[1], a[2]);
    if g.direction_type[3] > 2.5 {
        rect_across(v(g.direction_type), v(g.area), g.area[3])
    } else {
        Vec3::ZERO
    }
}

/// How much farther than its radius a support point's row must reach, in world units: a place
/// is lit within the radius of the emitter's nearest point, which may lie the emitter's whole
/// ground extent away from the support point. 0 for point and spot lights.
pub fn row_span(g: &GpuLight) -> f32 {
    if g.direction_type[3] <= 1.5 {
        return 0.0;
    }
    let v = across(g);
    2.0 * (det::hypot(g.area[0], g.area[1]) + det::hypot(v.x, v.y))
}

impl RowLight {
    /// A line or rect light: its shadow is the mean of its support points' rows.
    pub fn is_area(&self) -> bool {
        self.light.direction_type[3] > 1.5
    }

    /// Rows it needs this frame: none without shadow, [`SUPPORTS`] for a line or rect light.
    pub fn rows(&self) -> u32 {
        match (self.shadowed, self.is_area()) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => SUPPORTS,
        }
    }

    /// The lamp whose row is the `k`-th of this light: itself, or for a line or rect light a
    /// hard lamp at support point `k`, reaching [`row_span`] farther (its own key, ground point,
    /// height and radius, so the cache tells its row apart).
    pub fn support(&self, k: u32) -> RowLight {
        if !self.is_area() {
            return *self;
        }
        let mut s = *self;
        let g = &mut s.light;
        let [t, v] = SUPPORT_TS[usize::from(g.direction_type[3] > 2.5)][k as usize];
        let m = g.area_mask;
        g.shadow[2] += t * m[0] + v * m[2];
        g.shadow[3] += t * m[1] + v * m[3];
        g.position_radius[2] += t * g.area[2] + v * across(g).z;
        g.position_radius[3] += row_span(g);
        s.key += u64::from(k);
        s.size = 0.0;
        s
    }
}

uniform_block! {
    /// A lamp as the row programs read it; matches `struct RowLamp` in `lightrow.hlsli`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct RowLamp[224] {
        /// The light.
        pub light: GpuLight,
        /// x the lamp's row in the atlas, -1 without shadow; y source radius in world units.
        /// A line or rect light's `light.profile` holds its support points' rows instead
        /// (Chiaro's area light never reads a profile).
        pub row: [f32; 4],
        /// Its mask (zero without); only the `_mask` programs read it.
        pub mask: GpuMask,
        /// [`RowLight::window`]; only the row build reads it.
        pub window: [f32; 4],
        /// See `window`.
        pub window_v: [f32; 4],
    }
}

uniform_block! {
    /// `cbuffer RowParams` of `lightrow_build.frag`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct RowBuildParams[32] {
        /// [`RowSpace::world_from_mask`].
        pub world_from_mask: [f32; 4],
        /// xy mask size in texels, z [`MARCH_STEP`], w 1 if an obstacle is partly transparent
        /// (the build reads the mask's opacity then).
        pub domain: [f32; 4],
    }
}

uniform_block! {
    /// `cbuffer EdgeParams` of `lightrow_edge.frag`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct RowEdgeParams[32] {
        /// x [`ROW_ANGLES`], y rows of the atlas, z [`EDGE_WIDTH`].
        pub row_size: [f32; 4],
        /// x world units one angle spans one texel from the lamp (a lamp of that source radius
        /// has a penumbra one angle wide behind an obstacle there), y [`EDGE_NEAR`].
        pub near: [f32; 4],
    }
}

uniform_block! {
    /// `cbuffer LookupParams` of `lightrow_surface.hlsli`.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct RowLookupParams[48] {
        /// x [`ROW_ANGLES`], y rows of the atlas, z [`RECEIVER_GAP`], w tolerance below a top
        /// (`height_tolerance`).
        pub row_size: [f32; 4],
        /// xy mask size in texels; w is the reference program's lamp points per axis (tests).
        pub domain: [f32; 4],
        /// [`RowSpace::world_from_mask`].
        pub world_from_mask: [f32; 4],
    }
}

impl RowBuildParams {
    /// The block of a frame; `partial`: an active obstacle is partly transparent.
    pub fn new(space: &RowSpace, mask: [u32; 2], partial: bool) -> Self {
        Self {
            world_from_mask: space.world_from_mask,
            domain: [
                mask[0] as f32,
                mask[1] as f32,
                MARCH_STEP,
                f32::from(u8::from(partial)),
            ],
        }
    }
}

impl RowEdgeParams {
    /// The block of a frame with an atlas of `rows` rows.
    pub fn new(space: &RowSpace, rows: u32) -> Self {
        Self {
            row_size: [ROW_ANGLES as f32, rows as f32, EDGE_WIDTH, 0.0],
            near: [
                space.shortest * core::f32::consts::TAU / ROW_ANGLES as f32,
                EDGE_NEAR,
                0.0,
                0.0,
            ],
        }
    }
}

impl RowLookupParams {
    /// The block of a frame with an atlas of `rows` rows.
    pub fn new(space: &RowSpace, mask: [u32; 2], rows: u32, tolerance: f32) -> Self {
        Self {
            row_size: [ROW_ANGLES as f32, rows as f32, RECEIVER_GAP, tolerance],
            domain: [mask[0] as f32, mask[1] as f32, 0.0, 0.0],
            world_from_mask: space.world_from_mask,
        }
    }
}

/// What the occlusion stage changed in the mask this frame.
#[derive(Clone, Copy, Debug, Default)]
pub enum MaskChange<'a> {
    /// Unknown or everything (also: redrawn completely).
    #[default]
    All,
    /// Nothing.
    None,
    /// These texels (of the mask as it is now) were redrawn; all others hold what they held,
    /// shifted with the anchor.
    Rects(&'a DirtyRectList),
}

/// The occlusion mask of a frame as the row cache needs it.
#[derive(Clone, Copy, Debug, Default)]
pub struct RowMask<'a> {
    /// Mask size in texels.
    pub size: [u32; 2],
    /// `MaskAnchor::snapped` of the frame.
    pub snapped: [i32; 2],
    /// What changed.
    pub change: MaskChange<'a>,
}

/// What a row was built from.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RowSource {
    /// The lamp's ground point in mask texels (anchor remainder subtracted).
    ground: [f32; 2],
    /// Lamp height, radius and source radius in world units.
    lamp: [f32; 3],
    /// [`RowLight::window`]: the row begins there.
    window: [[f32; 4]; 2],
    /// Reach in mask texels.
    reach: f32,
    /// The mask anchor then.
    snapped: [i32; 2],
    /// The whole reach lay inside the mask.
    inside: bool,
}

impl RowSource {
    fn new(light: &RowLight, mask: &RowMask<'_>) -> Self {
        let g = &light.light;
        let ground = [g.shadow[2], g.shadow[3]];
        let inside = (0..2).all(|i| {
            ground[i] - light.reach >= 0.0 && ground[i] + light.reach <= mask.size[i] as f32
        });
        Self {
            ground,
            lamp: [g.position_radius[2], g.position_radius[3], light.size],
            window: light.window,
            reach: light.reach,
            snapped: mask.snapped,
            inside,
        }
    }

    /// A row built from `self` still holds for `now`. A pan by whole texels moves the lamp and
    /// the mask's contents alike, so the row stays as long as its whole reach lay and lies
    /// inside the mask; what the pan exposed at the mask's rim is new to a lamp reaching there.
    fn holds(&self, now: &Self, change: MaskChange<'_>) -> bool {
        let shift = [0, 1].map(|i| i64::from(now.snapped[i]) - i64::from(self.snapped[i]));
        let panned = shift != [0, 0];
        if self.lamp != now.lamp
            || self.window != now.window
            || (panned && !(self.inside && now.inside))
        {
            return false;
        }
        if (0..2)
            .any(|i| (now.ground[i] - self.ground[i] - shift[i] as f32).abs() > GROUND_TOLERANCE)
        {
            return false;
        }
        match change {
            MaskChange::All => false,
            MaskChange::None => true,
            MaskChange::Rects(list) => !touches(list, now.ground, now.reach),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    /// `RowLight::key`, [`FREE`] for a row never used.
    key: u64,
    /// What the row holds; `None`: nothing valid.
    built: Option<RowSource>,
    /// Frame of the last use.
    used: u64,
}

const FREE: u64 = u64::MAX;

/// Which lamp has which row of the atlas and whether the row is still right.
///
/// Per frame: [`begin`](Self::begin), [`assign`](Self::assign) for every shadow-casting row
/// light, and [`commit`](Self::commit) once the frame's builds were recorded. A frame that was
/// begun but never committed built nothing, so the next one rebuilds every row.
#[derive(Clone, Debug)]
pub struct RowCache {
    slots: Vec<Slot>,
    frame: u64,
    committed: bool,
}

impl Default for RowCache {
    fn default() -> Self {
        Self {
            // All the storage it ever needs: steady frames do not allocate.
            slots: Vec::with_capacity(MAX_ROWS as usize),
            frame: 0,
            committed: true,
        }
    }
}

impl RowCache {
    /// Forgets every row (the atlas was recreated, or the mask can no longer be trusted).
    pub fn invalidate(&mut self) {
        for s in &mut self.slots {
            s.built = None;
        }
    }

    /// Rows in use so far (the atlas needs at least this many).
    pub fn rows(&self) -> u32 {
        self.slots.len() as u32
    }

    /// Starts a frame.
    pub fn begin(&mut self) {
        if !self.committed {
            self.invalidate();
        }
        self.committed = false;
        self.frame += 1;
    }

    /// The row of `light` this frame and whether it must be built. A lamp keeps its row from
    /// one frame to the next; a frame it sat out may have changed the mask unseen, so its row
    /// is built anew then. A new lamp takes a row never used, else the one unused for longest.
    /// `None` if all [`MAX_ROWS`] rows are taken by lamps of this frame.
    pub fn assign(&mut self, light: &RowLight, mask: &RowMask<'_>) -> Option<(u32, bool)> {
        let frame = self.frame;
        let at = match self.slots.iter().position(|s| s.key == light.key) {
            Some(at) => at,
            None if self.slots.len() < MAX_ROWS as usize => {
                self.slots.push(Slot {
                    key: FREE,
                    built: None,
                    used: 0,
                });
                self.slots.len() - 1
            }
            None => {
                let (at, oldest) = self.slots.iter().enumerate().min_by_key(|(_, s)| s.used)?;
                if oldest.used == frame {
                    return None;
                }
                at
            }
        };
        let slot = &mut self.slots[at];
        if slot.key != light.key || slot.used + 1 != frame {
            // Another lamp's row, a frame sat out, or asked twice in a frame: nothing to keep.
            slot.built = None;
        }
        let now = RowSource::new(light, mask);
        let keep = slot.built.is_some_and(|b| b.holds(&now, mask.change));
        if !keep {
            slot.built = Some(now);
        }
        slot.key = light.key;
        slot.used = frame;
        Some((at as u32, !keep))
    }

    /// The row [`assign`](Self::assign) gave the lamp `key` this frame.
    pub fn row_of(&self, key: u64) -> Option<u32> {
        let at = self.slots.iter().position(|s| s.key == key)?;
        (self.slots[at].used == self.frame).then_some(at as u32)
    }

    /// The frame's builds were recorded.
    pub fn commit(&mut self) {
        self.committed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shadow::DirtyRect;
    use knorke_math::det;

    fn light(key: u32, ground: [f32; 2]) -> RowLight {
        let key = u64::from(key);
        RowLight {
            light: GpuLight {
                position_radius: [0.0, 0.0, 2.0, 5.0],
                shadow: [0.0, 1.0, ground[0], ground[1]],
                ..GpuLight::default()
            },
            key,
            shadowed: true,
            reach: 20.0,
            size: 0.0,
            mask: None,
            window: [[0.0; 4]; 2],
        }
    }

    fn mask(snapped: [i32; 2], change: MaskChange<'_>) -> RowMask<'_> {
        RowMask {
            size: [200, 100],
            snapped,
            change,
        }
    }

    #[test]
    fn suitability_names_what_rows_cannot_do() {
        let on = LightSettings::default();
        let off = LightSettings {
            light_rows: false,
            ..on
        };
        assert!(frame_suits(&on, true, false, 3.0));
        assert!(!frame_suits(&off, true, false, 3.0));
        assert!(!frame_suits(&on, false, false, 3.0), "no surface buffer");
        assert!(!frame_suits(&on, true, true, 3.0), "medium map");
        assert!(!frame_suits(&on, true, false, MAX_TOP), "tall obstacle");

        let point = LightDesc::new(LightType::Point);
        assert!(light_suits(&point, 100.0));
        assert!(!light_suits(&point, MAX_REACH + 1.0), "reach");
        let spot = LightDesc {
            kind: LightType::Spot,
            flicker: 0.5,
            animation_offset: 1.0,
            ..point.clone()
        };
        assert!(light_suits(&spot, 100.0), "flicker changes brightness only");
        let profiled = LightDesc {
            profile: crate::light::ProfileId::from_bits(1 << 32),
            ..point.clone()
        };
        assert!(
            profiled.profile.is_some() && light_suits(&profiled, 100.0),
            "profile"
        );
        let masked = LightDesc {
            mask: crate::light::MaskId::from_bits(1 << 32),
            ..point.clone()
        };
        assert!(light_suits(&masked, 100.0), "mask");
        let kind = |kind| LightDesc {
            kind,
            ..point.clone()
        };
        assert!(light_suits(&kind(LightType::Line), 1.0));
        assert!(light_suits(&kind(LightType::Rect), 1.0));
        assert!(!light_suits(&kind(LightType::Directional), 1.0));
        let filtered = LightDesc {
            shadow_filter: ShadowFilter::StaticOnly,
            ..point.clone()
        };
        assert!(!light_suits(&filtered, 1.0), "caster filter");
        let inside = LightDesc {
            inside_obstacle: true,
            ..point.clone()
        };
        assert!(!light_suits(&inside, 1.0), "inside_obstacle");
        // Without shadows nothing of the row matters.
        let unshadowed = LightDesc {
            cast_shadows: false,
            inside_obstacle: true,
            shadow_filter: ShadowFilter::StaticOnly,
            ..point
        };
        assert!(light_suits(&unshadowed, MAX_REACH + 1.0));
    }

    #[test]
    fn patterned_tile_grids_block_only_the_lamps_reaching_them() {
        use crate::obstacle::{ShapeDesc, TileGridDesc};
        const SQUARE: [knorke_math::Vec2; 4] = [
            knorke_math::Vec2::new(0.0, 0.0),
            knorke_math::Vec2::new(1.0, 0.0),
            knorke_math::Vec2::new(1.0, 1.0),
            knorke_math::Vec2::new(0.0, 1.0),
        ];
        let at = |x, y| At::new(Vec2::new(x, y));
        let mut store = ObstacleStore::new();
        let opaque = store.create_shape(&ShapeDesc::polygon(&SQUARE)).unwrap();
        store.set_transform(opaque, at(-10.0, 0.0), 0.0).unwrap();
        let mut glass = ShapeDesc::polygon(&SQUARE);
        glass.common.opacity = 0.5;
        let glass = store.create_shape(&glass).unwrap();
        store.set_transform(glass, at(-20.0, 0.0), 0.0).unwrap();
        let fence = store.create_shape(&ShapeDesc::polygon(&SQUARE)).unwrap();
        store.set_transform(fence, at(-30.0, 0.0), 0.0).unwrap();
        let grid = store
            .create_tile_grid(&TileGridDesc::new(1, 1, &[1]))
            .unwrap();
        store.set_transform(grid, at(10.0, 0.0), 0.0).unwrap();
        let patterns = |_| Some(0.5);
        store.set_pattern(fence, 1, &patterns).unwrap();
        store.set_pattern(grid, 1, &patterns).unwrap();

        let p = Projection::iso(at(3.0, 4.0), 48.0, 0.5, 1280, 720);
        let domain = OcclusionDomain::new(1280, 720, 0.5);
        let space = RowSpace::new(&p, &domain);
        let anchor = MaskAnchor::compute(&p, &domain);
        // As `collect_lights` places a lamp's ground point.
        let lamp = |x, radius| {
            let m = domain.scene_to_domain(p.project(at(x, 0.5), 0.0).raw());
            let ground = [m.x - anchor.remainder[0], m.y - anchor.remainder[1]];
            (ground, space.reach(radius))
        };
        let mut blockers = RowBlockers::default();
        blockers.collect(true, &store, &p, &domain);
        let meets = |b: &RowBlockers, x, r| {
            let (g, reach) = lamp(x, r);
            b.meet(g, reach)
        };
        assert!(meets(&blockers, 12.0, 2.0), "beside the grid");
        assert!(meets(&blockers, 0.0, 10.0), "reaching it from afar");
        // The test is conservative: a square of the reach in the mask.
        assert!(!meets(&blockers, 0.0, 3.0), "short of it");
        assert!(!meets(&blockers, -10.0, 2.0), "at the opaque obstacle");
        assert!(
            !meets(&blockers, -20.0, 2.0),
            "a partly transparent one is in the row"
        );
        assert!(
            !meets(&blockers, -30.0, 2.0),
            "a pattern shape is in the row"
        );
        store.set_transform(grid, at(30.0, 0.0), 0.0).unwrap();
        blockers.collect(true, &store, &p, &domain);
        assert!(!meets(&blockers, 12.0, 2.0), "the grid moved away");
        blockers.collect(false, &store, &p, &domain);
        assert!(!meets(&blockers, 30.0, 2.0), "a frame without rows");
    }

    #[test]
    fn a_row_is_kept_until_lamp_or_mask_change() {
        let mut cache = RowCache::default();
        let still = mask([0, 0], MaskChange::None);
        let frame = |cache: &mut RowCache, lights: &[RowLight], m: &RowMask<'_>| {
            cache.begin();
            let out: Vec<_> = lights.iter().map(|l| cache.assign(l, m).unwrap()).collect();
            cache.commit();
            out
        };
        let (a, b) = (light(7, [50.0, 50.0]), light(3, [150.0, 50.0]));
        assert_eq!(frame(&mut cache, &[a, b], &still), [(0, true), (1, true)]);
        assert_eq!(frame(&mut cache, &[a, b], &still), [(0, false), (1, false)]);
        // Order does not move a lamp's row; a lamp that sat a frame out keeps the row but
        // not its contents.
        assert_eq!(frame(&mut cache, &[b], &still), [(1, false)]);
        assert_eq!(frame(&mut cache, &[b, a], &still), [(1, false), (0, true)]);
        assert_eq!(cache.row_of(7), Some(0));
        assert_eq!(cache.row_of(8), None);
        // A moved lamp, a changed height.
        let moved = light(7, [50.5, 50.0]);
        assert_eq!(
            frame(&mut cache, &[moved, b], &still),
            [(0, true), (1, false)]
        );
        let mut raised = b;
        raised.light.position_radius[2] = 3.0;
        assert_eq!(
            frame(&mut cache, &[moved, raised], &still),
            [(0, false), (1, true)]
        );
        // Colour is not in the row; a window it begins at is.
        let mut red = raised;
        red.light.color_falloff = [1.0, 0.0, 0.0, 1.0];
        assert_eq!(
            frame(&mut cache, &[moved, red], &still),
            [(0, false), (1, false)]
        );
        let mut window = red;
        window.window[0] = [0.0, 10.0, 4.0, 0.0];
        assert_eq!(
            frame(&mut cache, &[moved, window], &still),
            [(0, false), (1, true)]
        );
        // A redrawn rectangle within reach of one lamp.
        let mut rects = DirtyRectList::new();
        rects.add(DirtyRect::new(60, 40, 70, 50));
        let changed = mask([0, 0], MaskChange::Rects(&rects));
        assert_eq!(
            frame(&mut cache, &[moved, window], &changed),
            [(0, true), (1, false)]
        );
        assert_eq!(
            frame(&mut cache, &[moved, raised], &mask([0, 0], MaskChange::All)),
            [(0, true), (1, true)]
        );
        cache.invalidate();
        assert_eq!(frame(&mut cache, &[moved], &still), [(0, true)]);
        // A frame that was never recorded built nothing.
        cache.begin();
        assert_eq!(frame(&mut cache, &[moved], &still), [(0, true)]);
    }

    #[test]
    fn an_area_light_has_a_row_per_support_point() {
        let mut rect = light(2, [50.0, 50.0]);
        // A window: 1.2 long along x, 0.5 half height (V upright), facing y; 5 texels a unit.
        rect.light.direction_type = [0.0, 1.0, 0.0, 3.0];
        rect.light.area = [1.2, 0.0, 0.0, 0.5];
        rect.light.area_mask = [6.0, 0.0, 0.0, 0.0];
        assert_eq!(rect.rows(), SUPPORTS);
        // V = normal x U points down: s = 0.5 lies below the middle.
        assert_eq!(row_span(&rect.light), 2.4);
        let s = rect.support(3);
        assert_eq!(s.key, rect.key + 3);
        assert_eq!([s.light.shadow[2], s.light.shadow[3]], [53.0, 50.0]);
        assert!((s.light.position_radius[2] - 1.75).abs() < 1e-6, "{s:?}");
        assert_eq!(s.light.position_radius[3], 5.0 + 2.4);
        assert_eq!(light(1, [0.0, 0.0]).support(0), light(1, [0.0, 0.0]));
        // Every support point keeps its own row.
        let mut cache = RowCache::default();
        let still = mask([0, 0], MaskChange::None);
        cache.begin();
        let got: Vec<_> = (0..SUPPORTS)
            .map(|k| cache.assign(&rect.support(k), &still).unwrap())
            .collect();
        assert_eq!(got, [(0, true), (1, true), (2, true), (3, true)]);
        cache.commit();
    }

    #[test]
    fn a_pan_by_whole_texels_keeps_rows_inside_the_mask() {
        let mut cache = RowCache::default();
        let frame = |cache: &mut RowCache, l: RowLight, m: &RowMask<'_>| {
            cache.begin();
            let out = cache.assign(&l, m).unwrap();
            cache.commit();
            out.1
        };
        let rects = DirtyRectList::new();
        let pan = |x| mask([x, 0], MaskChange::Rects(&rects));
        assert!(frame(&mut cache, light(1, [50.0, 50.0]), &pan(0)));
        // The anchor moved three texels and the lamp with it.
        assert!(!frame(&mut cache, light(1, [53.0, 50.0]), &pan(3)));
        // The lamp did not follow: it moved in the world.
        assert!(frame(&mut cache, light(1, [53.0, 50.0]), &pan(5)));
        // Reaching over the mask's rim after the pan: the exposed strip is new to it.
        assert!(frame(&mut cache, light(1, [15.0, 50.0]), &pan(-33)));
        assert!(frame(&mut cache, light(1, [16.0, 50.0]), &pan(-32)));
        // Without a pan the rim does not matter.
        assert!(!frame(&mut cache, light(1, [16.0, 50.0]), &pan(-32)));
    }

    #[test]
    fn rows_are_reused_oldest_first_and_run_out() {
        let mut cache = RowCache::default();
        let still = mask([0, 0], MaskChange::None);
        cache.begin();
        for key in 0..MAX_ROWS {
            assert_eq!(
                cache.assign(&light(key, [50.0, 50.0]), &still),
                Some((key, true))
            );
        }
        assert_eq!(cache.assign(&light(1000, [50.0, 50.0]), &still), None);
        cache.commit();
        cache.begin();
        for key in 1..MAX_ROWS {
            cache.assign(&light(key, [50.0, 50.0]), &still);
        }
        // Row 0 was not used this frame.
        assert_eq!(
            cache.assign(&light(1000, [50.0, 50.0]), &still),
            Some((0, true))
        );
        cache.commit();
    }

    #[test]
    fn row_space_measures_the_shortest_texel() {
        let camera = knorke_math::At::new(knorke_math::Vec2::new(3.0, 4.0));
        let p = Projection::iso(camera, 48.0, 0.5, 1280, 720);
        let domain = OcclusionDomain::new(1280, 720, 0.5);
        let space = RowSpace::new(&p, &domain);
        let [a, b, c, d] = space.world_from_mask;
        let shortest = (0..3600)
            .map(|k| {
                let angle = (k as f32 * 0.1).to_radians();
                let (s, co) = (det::sin(angle), det::cos(angle));
                det::hypot(a * co + b * s, c * co + d * s)
            })
            .fold(f32::MAX, |m, v| if v < m { v } else { m });
        assert!(
            (space.shortest - shortest).abs() < 1e-5,
            "{space:?} {shortest}"
        );
        assert!(space.reach(10.0) > 10.0 / shortest);
    }
}
