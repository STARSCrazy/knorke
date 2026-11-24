// Origin: chiaro @ 9bc5b9b, src/lighting/LightStore.hpp and LightStore.cpp (LightParams, StoredLight,
// InitLightDesc, SetLineEndpoints, ValidateLightDesc, Derive) and the field list of CHI_LightDesc in
// include/Chiaro/Chiaro.h.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The description of a light, its validation and the values derived once per change (P7).
//!
//! Redesign against the C++ version:
//!
//! - One [`LightDesc`] replaces the pair `CHI_LightDesc` (public, flag bits) and `LightParams`
//!   (engine, converted at the C boundary). Flag bits are `bool`s and enums
//!   ([`ShadowFilter`] cannot hold the "both filters" combination any more), a height of
//!   `CHI_HEIGHT_AUTO` is [`Height::Auto`], the mask angle's "auto" sentinel is `None`, and
//!   profile, animation and mask are optional typed handles. Defaults are the *public* defaults
//!   of `CHI_InitLightDesc` (shadows on, height automatic).
//! - Unknown light types, unknown flag bits and non-existent handle kinds cannot be expressed.
//! - Errors are [`LightError`] values with stable diagnostic codes `knorke.light2d.light.*`
//!   instead of formatted SDL error strings; the texts keep the field names and received values.
//! - Profiles, animations and masks live in their own stores (animations P8, masks P10, profiles
//!   P9). The light only needs to know whether a handle exists, which the host answers through
//!   [`LightResources`] (see [`StoreResources`](crate::StoreResources)). `AnimationId` and
//!   `MaskId` are re-exported here from their stores; the reference direction comes
//!   from [`crate::profile::profile_reference`].

use crate::light_mask::window_axes_span_plane;
use knorke_math::{At, Vec2, Vec3, World2, det};
use knorke_types::{
    Diagnostic, DiagnosticCode, Handle, Reader, SnapshotError, SnapshotValue, Writer,
};

/// Most transient lights per frame ([`LightStore::set_frame_lights`](crate::LightStore::set_frame_lights)).
pub const MAX_FRAME_LIGHTS: usize = 4096;

/// Largest angular radius of a directional light's `source_radius`, in degrees.
const MAX_DIRECTIONAL_ANGLE: f32 = 10.0;
const DEG_TO_RAD: f32 = core::f32::consts::PI / 180.0;

/// Handle kind of a light profile (stored by a later package; the light only refers to it).
#[derive(Clone, Copy, Debug)]
pub struct ProfileTag;

/// Handle of a light profile.
pub type ProfileId = Handle<ProfileTag>;
pub use crate::animation::AnimationId;
pub use crate::mask_store::MaskId;
/// Handle of a persistent light in a [`LightStore`](crate::LightStore).
pub type LightId = Handle<StoredLight>;

/// Answers whether the stores of profiles, animations and masks know a handle. A light is only
/// accepted if every handle it names exists.
pub trait LightResources {
    /// `true` if `id` names a live profile.
    fn has_profile(&self, id: ProfileId) -> bool;
    /// `true` if `id` names a live animation.
    fn has_animation(&self, id: AnimationId) -> bool;
    /// `true` if `id` names a live mask.
    fn has_mask(&self, id: MaskId) -> bool;
}

/// No profiles, animations or masks exist: every handle is rejected.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoResources;

impl LightResources for NoResources {
    fn has_profile(&self, _: ProfileId) -> bool {
        false
    }
    fn has_animation(&self, _: AnimationId) -> bool {
        false
    }
    fn has_mask(&self, _: MaskId) -> bool {
        false
    }
}

/// Used when restoring a snapshot: the stores are restored separately, so only the rules that
/// do not need a lookup can be checked here.
struct AssumeKnown;

impl LightResources for AssumeKnown {
    fn has_profile(&self, _: ProfileId) -> bool {
        true
    }
    fn has_animation(&self, _: AnimationId) -> bool {
        true
    }
    fn has_mask(&self, _: MaskId) -> bool {
        true
    }
}

/// Kind of light.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LightType {
    /// Radiates in all directions up to `radius`.
    Point,
    /// Point light limited to a cone around `direction`.
    Spot,
    /// Parallel light (sun, moon): lights the whole scene from one direction.
    Directional,
    /// Line light (neon tube): the segment `position` ± `axis`.
    Line,
    /// Area light (window, billboard): the rectangle `position` ± `axis` ± `half_width` across.
    Rect,
}

impl LightType {
    /// `true` for [`Line`](Self::Line) and [`Rect`](Self::Rect).
    pub fn is_area(self) -> bool {
        matches!(self, Self::Line | Self::Rect)
    }

    fn tag(self) -> u8 {
        match self {
            Self::Point => 0,
            Self::Spot => 1,
            Self::Directional => 2,
            Self::Line => 3,
            Self::Rect => 4,
        }
    }

    fn from_tag(t: u8) -> Result<Self, SnapshotError> {
        Ok(match t {
            0 => Self::Point,
            1 => Self::Spot,
            2 => Self::Directional,
            3 => Self::Line,
            4 => Self::Rect,
            _ => return Err(SnapshotError::Invalid("light type")),
        })
    }
}

/// Height of a light above the ground in world units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Height {
    /// Resolved every frame: 0 without a surface or material buffer, 0.4 × radius with one.
    Auto,
    /// A fixed height, finite and `>= 0`.
    At(f32),
}

impl Height {
    /// The fixed height, 0 for [`Auto`](Self::Auto).
    pub fn value_or_zero(self) -> f32 {
        match self {
            Self::Auto => 0.0,
            Self::At(h) => h,
        }
    }
}

/// Which obstacles cast shadows for a light.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShadowFilter {
    /// Every caster.
    #[default]
    All,
    /// Only obstacles marked dynamic.
    DynamicOnly,
    /// Only static shapes, tile grids and the occlusion layer.
    StaticOnly,
}

/// How a mask is placed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MaskMode {
    /// A slide in front of the light, centred on `direction`.
    #[default]
    Projector,
    /// The world rectangle `mask_center` ± `mask_u` ± `mask_v`, a window opening.
    Window,
}

/// A light as the host describes it.
///
/// Lengths are world units, angles degrees, colours linear RGB that may exceed 1 together with
/// `intensity` (HDR).
#[derive(Clone, Debug, PartialEq)]
pub struct LightDesc {
    /// Kind of light.
    pub kind: LightType,
    /// World position on the ground plane.
    pub position: At<World2, Vec2>,
    /// Height above the ground.
    pub height: Height,
    /// Direction of spot, directional and rect lights, and the axis of profiles and projector masks.
    pub direction: Vec3,
    /// Linear RGB, each `>= 0`. Default white.
    pub color: Vec3,
    /// Multiplier, `>= 0`. Default 1.
    pub intensity: f32,
    /// Factor on this light's highlights in `[0, 4]`. Default 1.
    pub specular: f32,
    /// Factor on how strongly this light appears in haze and media, in `[0, 16]`. Default 1.
    pub haze: f32,
    /// Reach in world units, `> 0`; not read by directional lights. Default 100.
    pub radius: f32,
    /// Falloff exponent, `> 0`; not read by directional lights. Default 2.
    pub falloff: f32,
    /// Spot cone: full intensity inside this angle. Default 20.
    pub inner_angle: f32,
    /// Spot cone: zero outside this angle, `< 90`. Default 30.
    pub outer_angle: f32,
    /// Flicker depth in `[0, 1]`. Default 0.
    pub flicker: f32,
    /// Flicker speed, `>= 0`. Default 8.
    pub flicker_speed: f32,
    /// Size of the lamp: world units for local lights, an angle (at most 10) for directional lights.
    pub source_radius: f32,
    /// Rotation of the profile about `direction`, degrees.
    pub profile_rotation: f32,
    /// Light profile (point-like luminaires only).
    pub profile: Option<ProfileId>,
    /// Phase offset of the animation.
    pub animation_offset: f32,
    /// Light animation.
    pub animation: Option<AnimationId>,
    /// Half axis of line and rect lights.
    pub axis: Vec3,
    /// Half width of rect lights, `> 0`.
    pub half_width: f32,
    /// Light mask (point and spot lights).
    pub mask: Option<MaskId>,
    /// Projector or window.
    pub mask_mode: MaskMode,
    /// Projector: degrees from the axis to the side edges in `(0, 90)`; `None` takes the spot's
    /// outer angle (45 for a point light).
    pub mask_angle: Option<f32>,
    /// Projector: degrees about `direction`.
    pub mask_rotation: f32,
    /// Window: world centre.
    pub mask_center: Vec3,
    /// Window: half extent to the right edge.
    pub mask_u: Vec3,
    /// Window: half extent to the top edge.
    pub mask_v: Vec3,
    /// Obstacles shadow this light (default on).
    pub cast_shadows: bool,
    /// Kept but not drawn when `false`.
    pub enabled: bool,
    /// Rect lights: emit from both sides.
    pub two_sided: bool,
    /// Point and spot lights with `source_radius > 0`: sphere-light Lambert term near surfaces.
    pub soft_near_field: bool,
    /// Lamps in a wall or on a mast: the light may stand inside an obstacle.
    pub inside_obstacle: bool,
    /// Which casters shadow this light.
    pub shadow_filter: ShadowFilter,
}

impl LightDesc {
    /// The defaults for a light type (public defaults of `CHI_InitLightDesc`): white, intensity 1,
    /// radius 100, falloff 2, spot angles 20/30, shadows on, height automatic. Directional and
    /// rect lights point down, the others along +x.
    pub fn new(kind: LightType) -> Self {
        let down = matches!(kind, LightType::Directional | LightType::Rect);
        Self {
            kind,
            position: At::new(Vec2::ZERO),
            height: Height::Auto,
            direction: if down {
                Vec3::new(0.0, 0.0, -1.0)
            } else {
                Vec3::new(1.0, 0.0, 0.0)
            },
            color: Vec3::ONE,
            intensity: 1.0,
            specular: 1.0,
            haze: 1.0,
            radius: 100.0,
            falloff: 2.0,
            inner_angle: 20.0,
            outer_angle: 30.0,
            flicker: 0.0,
            flicker_speed: 8.0,
            source_radius: 0.0,
            profile_rotation: 0.0,
            profile: None,
            animation_offset: 0.0,
            animation: None,
            axis: Vec3::new(1.0, 0.0, 0.0),
            half_width: 0.5,
            mask: None,
            mask_mode: MaskMode::Projector,
            mask_angle: None,
            mask_rotation: 0.0,
            mask_center: Vec3::new(0.0, 0.0, 1.0),
            mask_u: Vec3::new(0.5, 0.0, 0.0),
            mask_v: Vec3::new(0.0, 0.0, 0.5),
            cast_shadows: true,
            enabled: true,
            two_sided: false,
            soft_near_field: false,
            inside_obstacle: false,
            shadow_filter: ShadowFilter::All,
        }
    }

    /// `position = (a + b) / 2`, `axis = (b - a) / 2`; `x`, `y` and `z` of the endpoints are
    /// world coordinates with `z` the height.
    pub fn set_line_endpoints(&mut self, a: Vec3, b: Vec3) {
        let mid = (a + b) * 0.5;
        self.position = At::new(Vec2::new(mid.x, mid.y));
        self.height = Height::At(mid.z);
        self.axis = (b - a) * 0.5;
    }

    /// Checks every value the light model cannot use. Needs no other store; whether the named
    /// profile, animation and mask exist is checked by [`check_resources`](Self::check_resources).
    ///
    /// # Errors
    /// The first violated rule, as a [`LightError`].
    pub fn validate(&self) -> Result<(), LightError> {
        let height = match self.height {
            Height::Auto => 0.0,
            Height::At(h) => h,
        };
        let pos = self.position.raw();
        let arrays: [(&'static str, [f32; 3]); 7] = [
            ("x/y/height", [pos.x, pos.y, height]),
            ("direction", self.direction.to_array()),
            ("color", self.color.to_array()),
            ("axis", self.axis.to_array()),
            ("mask_center", self.mask_center.to_array()),
            ("mask_u", self.mask_u.to_array()),
            ("mask_v", self.mask_v.to_array()),
        ];
        for (field, values) in arrays {
            if values.iter().any(|v| !v.is_finite()) {
                return Err(LightError::NotFinite {
                    field,
                    values,
                    count: 3,
                });
            }
        }
        let scalars: [(&'static str, f32); 15] = [
            ("intensity", self.intensity),
            ("specular", self.specular),
            ("haze", self.haze),
            ("radius", self.radius),
            ("falloff", self.falloff),
            ("inner_angle", self.inner_angle),
            ("outer_angle", self.outer_angle),
            ("flicker", self.flicker),
            ("flicker_speed", self.flicker_speed),
            ("source_radius", self.source_radius),
            ("profile_rotation", self.profile_rotation),
            ("animation_offset", self.animation_offset),
            ("half_width", self.half_width),
            ("mask_angle", self.mask_angle.unwrap_or(1.0)),
            ("mask_rotation", self.mask_rotation),
        ];
        for (field, v) in scalars {
            if !v.is_finite() {
                return Err(LightError::NotFinite {
                    field,
                    values: [v, 0.0, 0.0],
                    count: 1,
                });
            }
        }
        if let Height::At(h) = self.height
            && h < 0.0
        {
            return Err(range("height", "must be >= 0", h));
        }
        let c = self.color;
        if let Some(v) = [c.x, c.y, c.z].into_iter().find(|v| *v < 0.0) {
            return Err(range("color", "must be >= 0", v));
        }
        if self.intensity < 0.0 {
            return Err(range("intensity", "must be >= 0", self.intensity));
        }
        if !(0.0..=4.0).contains(&self.specular) {
            return Err(range("specular", "must be in [0, 4]", self.specular));
        }
        if !(0.0..=16.0).contains(&self.haze) {
            return Err(range("haze", "must be in [0, 16]", self.haze));
        }
        if self.source_radius < 0.0 {
            return Err(range("source_radius", "must be >= 0", self.source_radius));
        }
        if self.kind == LightType::Directional && self.source_radius > MAX_DIRECTIONAL_ANGLE {
            return Err(range(
                "source_radius",
                "of a directional light is an angle of at most 10 degrees",
                self.source_radius,
            ));
        }
        if !(0.0..=1.0).contains(&self.flicker) {
            return Err(range("flicker", "must be in [0, 1]", self.flicker));
        }
        if self.flicker_speed < 0.0 {
            return Err(range("flicker_speed", "must be >= 0", self.flicker_speed));
        }
        if self.kind != LightType::Directional {
            if self.radius <= 0.0 {
                return Err(range("radius", "must be > 0", self.radius));
            }
            if self.falloff <= 0.0 {
                return Err(range("falloff", "must be > 0", self.falloff));
            }
        }
        // Unsupported combinations are errors that name the alternative (E52).
        if self.shadow_filter != ShadowFilter::All && !self.cast_shadows {
            return Err(LightError::Unsupported(
                "a shadow_filter other than All with cast_shadows = false is not supported; \
                 remove one of them",
            ));
        }
        if self.inside_obstacle && !self.cast_shadows {
            return Err(LightError::Unsupported(
                "inside_obstacle with cast_shadows = false is not supported (a light without \
                 shadows already ignores every obstacle); remove one of them",
            ));
        }
        if self.inside_obstacle && self.kind == LightType::Directional {
            return Err(LightError::Unsupported(
                "inside_obstacle is not supported on directional lights (they have no \
                 position); use it on point, spot, line and rect lights",
            ));
        }
        if self.two_sided && self.kind != LightType::Rect {
            return Err(LightError::Unsupported(
                "two_sided is only supported on rect lights (a point or line light already \
                 emits in every direction; use a rect light for a panel)",
            ));
        }
        if self.soft_near_field && !matches!(self.kind, LightType::Point | LightType::Spot) {
            return Err(LightError::Unsupported(
                "soft_near_field is only supported on point and spot lights (line and rect \
                 lights are soft by their extent; directional lights use source_radius)",
            ));
        }
        if self.soft_near_field && !(self.source_radius > 0.0) {
            return Err(range(
                "source_radius",
                "must be > 0 with soft_near_field",
                self.source_radius,
            ));
        }
        if let Some(a) = self.mask_angle
            && !(a > 0.0 && a < 90.0)
        {
            return Err(range("mask_angle", "must be in (0, 90) or None (auto)", a));
        }
        if self.kind == LightType::Spot
            && (self.inner_angle < 0.0
                || self.outer_angle < self.inner_angle
                || self.outer_angle >= 90.0)
        {
            return Err(LightError::SpotAngles {
                inner: self.inner_angle,
                outer: self.outer_angle,
            });
        }
        if !matches!(self.kind, LightType::Point | LightType::Line) && !non_zero(self.direction) {
            return Err(LightError::ZeroVector { field: "direction" });
        }
        if self.kind.is_area() {
            if !non_zero(self.axis) {
                return Err(LightError::ZeroVector { field: "axis" });
            }
            if self.kind == LightType::Rect {
                if self.half_width <= 0.0 {
                    return Err(range("half_width", "must be > 0", self.half_width));
                }
                let (a, n) = (self.axis, self.direction);
                if a.cross(n).length_squared() <= 1e-6 * a.length_squared() * n.length_squared() {
                    return Err(LightError::Unsupported(
                        "a rect light's axis must not be parallel to its direction",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Checks that the profile, animation and mask of the light exist (through `resources`) and
    /// fit its kind. Run after [`validate`](Self::validate).
    ///
    /// # Errors
    /// The first violated rule, as a [`LightError`].
    pub fn check_resources(&self, resources: &impl LightResources) -> Result<(), LightError> {
        if let Some(profile) = self.profile {
            if self.kind == LightType::Directional {
                return Err(LightError::Unsupported(
                    "a directional light cannot have a profile (profile must be None)",
                ));
            }
            if self.kind.is_area() {
                return Err(LightError::Unsupported(
                    "a line or rect light cannot have a profile (profile must be None); profiles \
                     describe point-like luminaires; use a spot light with the profile for a \
                     shaped beam",
                ));
            }
            if !resources.has_profile(profile) {
                return Err(LightError::UnknownResource {
                    kind: "profile",
                    handle: profile.to_bits(),
                });
            }
            if !non_zero(self.direction) {
                return Err(LightError::Unsupported(
                    "a light with a profile needs a non-zero direction (the profile axis)",
                ));
            }
        }
        if let Some(animation) = self.animation
            && !resources.has_animation(animation)
        {
            return Err(LightError::UnknownResource {
                kind: "animation",
                handle: animation.to_bits(),
            });
        }
        if let Some(mask) = self.mask {
            // Unsupported combinations name the alternative (E52).
            if self.kind == LightType::Directional {
                return Err(LightError::Unsupported(
                    "a directional light cannot have a mask (mask must be None); for moonlight \
                     through a window use a point or spot light far out along the moon's \
                     direction with mask_mode Window",
                ));
            }
            if self.kind.is_area() {
                return Err(LightError::Unsupported(
                    "a line or rect light cannot have a mask (mask must be None); use a point \
                     or spot light behind the window with mask_mode Window",
                ));
            }
            if !resources.has_mask(mask) {
                return Err(LightError::UnknownResource {
                    kind: "mask",
                    handle: mask.to_bits(),
                });
            }
            if self.mask_mode == MaskMode::Projector && !non_zero(self.direction) {
                return Err(LightError::Unsupported(
                    "a light with a projector mask needs a non-zero direction (the mask's axis)",
                ));
            }
            if self.mask_mode == MaskMode::Window {
                if !non_zero(self.mask_u) || !non_zero(self.mask_v) {
                    return Err(LightError::ZeroVector {
                        field: "mask_u/mask_v",
                    });
                }
                if !window_axes_span_plane(self.mask_u, self.mask_v) {
                    return Err(LightError::Unsupported(
                        "a window mask's mask_u and mask_v must not be parallel",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn range(field: &'static str, rule: &'static str, value: f32) -> LightError {
    LightError::OutOfRange { field, rule, value }
}

fn non_zero(v: Vec3) -> bool {
    v.length_squared() > 1e-12
}

/// Reference direction `C0` of a profile or projector mask, see [`profile::profile_reference`].
fn profile_reference(axis: Vec3, rotation_deg: f32) -> Vec3 {
    Vec3::from(crate::profile::profile_reference(axis.into(), rotation_deg))
}

/// Why a light or a light operation was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum LightError {
    /// A value is NaN or infinite.
    NotFinite {
        /// Field name of the description.
        field: &'static str,
        /// The offending values (only the first `count` are meaningful).
        values: [f32; 3],
        /// How many values the field has.
        count: u8,
    },
    /// A value is outside its allowed range.
    OutOfRange {
        /// Field name of the description.
        field: &'static str,
        /// The rule, e.g. `must be > 0`.
        rule: &'static str,
        /// The received value.
        value: f32,
    },
    /// The spot angles do not satisfy `0 <= inner <= outer < 90`.
    SpotAngles {
        /// Inner angle.
        inner: f32,
        /// Outer angle.
        outer: f32,
    },
    /// A vector that must point somewhere is zero.
    ZeroVector {
        /// Field name of the description.
        field: &'static str,
    },
    /// A combination of settings the light model does not support; the text names the alternative.
    Unsupported(&'static str),
    /// A profile, animation or mask handle that no store knows.
    UnknownResource {
        /// `profile`, `animation` or `mask`.
        kind: &'static str,
        /// The handle's bits.
        handle: u64,
    },
    /// A light handle that is stale or was never issued.
    InvalidHandle(u64),
    /// More transient lights than [`MAX_FRAME_LIGHTS`].
    TooManyFrameLights {
        /// Requested.
        count: usize,
        /// Allowed.
        max: usize,
    },
    /// Two parallel inputs of a batch differ in length.
    LengthMismatch {
        /// Which input.
        what: &'static str,
        /// Expected length.
        expected: usize,
        /// Received length.
        got: usize,
    },
    /// One element of a batch was refused; nothing was changed.
    Batch {
        /// Which input, e.g. `descs`.
        field: &'static str,
        /// Index of the element.
        index: usize,
        /// Why.
        source: Box<LightError>,
    },
}

impl LightError {
    /// The stable diagnostic code (`knorke.light2d.light.*`); a batch error has the code of its cause.
    pub fn code(&self) -> DiagnosticCode {
        match self {
            Self::NotFinite { .. } => DiagnosticCode::new("knorke.light2d.light.not_finite"),
            Self::OutOfRange { .. } | Self::SpotAngles { .. } => {
                DiagnosticCode::new("knorke.light2d.light.out_of_range")
            }
            Self::ZeroVector { .. } => DiagnosticCode::new("knorke.light2d.light.zero_vector"),
            Self::Unsupported(_) => DiagnosticCode::new("knorke.light2d.light.unsupported"),
            Self::UnknownResource { .. } => {
                DiagnosticCode::new("knorke.light2d.light.unknown_resource")
            }
            Self::InvalidHandle(_) => DiagnosticCode::new("knorke.light2d.light.invalid_handle"),
            Self::TooManyFrameLights { .. } => {
                DiagnosticCode::new("knorke.light2d.light.too_many_frame_lights")
            }
            Self::LengthMismatch { .. } => {
                DiagnosticCode::new("knorke.light2d.light.length_mismatch")
            }
            Self::Batch { source, .. } => source.code(),
        }
    }

    /// As a diagnostic; a batch error carries the element as its path (`descs[2]`).
    pub fn to_diagnostic(&self) -> Diagnostic {
        let d = Diagnostic::error(self.code(), self.to_string());
        match self {
            Self::Batch { field, index, .. } => d.at(format!("{field}[{index}]")),
            _ => d,
        }
    }
}

impl core::fmt::Display for LightError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotFinite {
                field,
                values,
                count,
            } => {
                write!(f, "light desc.{field} must be finite (got ")?;
                for (i, v) in values.iter().take(usize::from(*count)).enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_str(")")
            }
            Self::OutOfRange { field, rule, value } => {
                write!(f, "light desc.{field} {rule} (got {value})")
            }
            Self::SpotAngles { inner, outer } => write!(
                f,
                "spot angles must satisfy 0 <= desc.inner_angle <= desc.outer_angle < 90 \
                 (got {inner}, {outer})"
            ),
            Self::ZeroVector { field } => write!(f, "light desc.{field} must not be zero"),
            Self::Unsupported(text) => f.write_str(text),
            Self::UnknownResource { kind, handle } => write!(
                f,
                "light desc.{kind} is an invalid or destroyed {kind} handle 0x{handle:016x}"
            ),
            Self::InvalidHandle(bits) => {
                write!(f, "invalid or destroyed light handle 0x{bits:016x}")
            }
            Self::TooManyFrameLights { count, max } => write!(
                f,
                "{count} frame lights, at most {max} per frame (use persistent lights for more)"
            ),
            Self::LengthMismatch {
                what,
                expected,
                got,
            } => write!(f, "{what} has {got} elements, expected {expected}"),
            Self::Batch {
                field,
                index,
                source,
            } => write!(f, "{field}[{index}]: {source}"),
        }
    }
}

impl std::error::Error for LightError {}

/// A light as stored: the caller's description plus values derived once per change.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredLight {
    /// The description as given.
    pub desc: LightDesc,
    /// `desc.direction` normalized; zero if it is zero.
    pub direction: Vec3,
    /// `cos(inner_angle)`.
    pub cos_inner: f32,
    /// `cos(outer_angle)`.
    pub cos_outer: f32,
    /// `C0` of the profile; only meaningful with a profile.
    pub reference: Vec3,
    /// Line and rect lights: `desc.axis`, for a rect made perpendicular to `direction`.
    pub axis: Vec3,
    /// Projector mask: `C0` turned by `mask_rotation`; only meaningful with a mask.
    pub mask_reference: Vec3,
}

impl StoredLight {
    /// Derives the cached values of a validated description.
    pub fn derive(desc: &LightDesc) -> Self {
        let length = desc.direction.length();
        let direction = if length > 0.0 {
            desc.direction / length
        } else {
            Vec3::ZERO
        };
        let (reference, mask_reference) = if length > 0.0 {
            (
                profile_reference(direction, desc.profile_rotation),
                profile_reference(direction, desc.mask_rotation),
            )
        } else {
            (Vec3::ZERO, Vec3::ZERO)
        };
        let mut axis = desc.axis;
        if desc.kind == LightType::Rect {
            // Gram-Schmidt: the length axis in the emitter plane, its length kept.
            let along = axis.dot(direction);
            let before = desc.axis.length();
            axis -= direction * along;
            // `after > 0`: validation rejects an axis parallel to the direction.
            axis = axis * (before / axis.length());
        }
        Self {
            desc: desc.clone(),
            direction,
            cos_inner: det::cos(desc.inner_angle * DEG_TO_RAD),
            cos_outer: det::cos(desc.outer_angle * DEG_TO_RAD),
            reference,
            axis,
            mask_reference,
        }
    }
}

/// Where a light of a frame came from: the persistent store or the transient list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightRef {
    /// A persistent light.
    Persistent(LightId),
    /// A transient light of this frame, by its index in the list.
    Transient(u32),
}

impl LightRef {
    /// The slot index that seeds flicker: the handle's index for persistent lights, the list
    /// index for transient ones (so a transient light flickers like a persistent one in the
    /// same slot of a fresh store).
    pub fn seed_index(self) -> u32 {
        match self {
            Self::Persistent(id) => id.index(),
            Self::Transient(i) => i,
        }
    }
}

/// A position update: ground position and height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightPosition {
    /// World position on the ground plane.
    pub xy: At<World2, Vec2>,
    /// Height above the ground.
    pub height: Height,
}

impl LightPosition {
    /// A position.
    pub fn new(xy: At<World2, Vec2>, height: Height) -> Self {
        Self { xy, height }
    }

    /// Checks that `xy` is finite and the height finite and `>= 0`.
    pub(crate) fn validate(&self) -> Result<(), LightError> {
        let p = self.xy.raw();
        let h = self.height.value_or_zero();
        if !(p.x.is_finite() && p.y.is_finite() && h.is_finite()) {
            return Err(LightError::NotFinite {
                field: "x/y/height",
                values: [p.x, p.y, h],
                count: 3,
            });
        }
        if h < 0.0 {
            return Err(range("height", "must be >= 0", h));
        }
        Ok(())
    }
}

// ---- snapshots (ADR 0010) ----

impl SnapshotValue for LightDesc {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.kind.tag());
        w.put(&self.position);
        match self.height {
            Height::Auto => w.put(&0u8),
            Height::At(h) => {
                w.put(&1u8);
                w.put(&h);
            }
        }
        w.put(&self.direction);
        w.put(&self.color);
        for v in [
            self.intensity,
            self.specular,
            self.haze,
            self.radius,
            self.falloff,
            self.inner_angle,
            self.outer_angle,
            self.flicker,
            self.flicker_speed,
            self.source_radius,
            self.profile_rotation,
        ] {
            w.put(&v);
        }
        w.put(&self.profile);
        w.put(&self.animation_offset);
        w.put(&self.animation);
        w.put(&self.axis);
        w.put(&self.half_width);
        w.put(&self.mask);
        w.put(&(self.mask_mode == MaskMode::Window));
        w.put(&self.mask_angle);
        w.put(&self.mask_rotation);
        w.put(&self.mask_center);
        w.put(&self.mask_u);
        w.put(&self.mask_v);
        w.put(&self.cast_shadows);
        w.put(&self.enabled);
        w.put(&self.two_sided);
        w.put(&self.soft_near_field);
        w.put(&self.inside_obstacle);
        w.put(&match self.shadow_filter {
            ShadowFilter::All => 0u8,
            ShadowFilter::DynamicOnly => 1,
            ShadowFilter::StaticOnly => 2,
        });
    }

    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let kind = LightType::from_tag(r.get()?)?;
        let position = r.get()?;
        let height = match r.get::<u8>()? {
            0 => Height::Auto,
            1 => Height::At(r.get()?),
            _ => return Err(SnapshotError::Invalid("light height")),
        };
        let mut d = LightDesc::new(kind);
        d.position = position;
        d.height = height;
        d.direction = r.get()?;
        d.color = r.get()?;
        d.intensity = r.get()?;
        d.specular = r.get()?;
        d.haze = r.get()?;
        d.radius = r.get()?;
        d.falloff = r.get()?;
        d.inner_angle = r.get()?;
        d.outer_angle = r.get()?;
        d.flicker = r.get()?;
        d.flicker_speed = r.get()?;
        d.source_radius = r.get()?;
        d.profile_rotation = r.get()?;
        d.profile = r.get()?;
        d.animation_offset = r.get()?;
        d.animation = r.get()?;
        d.axis = r.get()?;
        d.half_width = r.get()?;
        d.mask = r.get()?;
        d.mask_mode = if r.get::<bool>()? {
            MaskMode::Window
        } else {
            MaskMode::Projector
        };
        d.mask_angle = r.get()?;
        d.mask_rotation = r.get()?;
        d.mask_center = r.get()?;
        d.mask_u = r.get()?;
        d.mask_v = r.get()?;
        d.cast_shadows = r.get()?;
        d.enabled = r.get()?;
        d.two_sided = r.get()?;
        d.soft_near_field = r.get()?;
        d.inside_obstacle = r.get()?;
        d.shadow_filter = match r.get::<u8>()? {
            0 => ShadowFilter::All,
            1 => ShadowFilter::DynamicOnly,
            2 => ShadowFilter::StaticOnly,
            _ => return Err(SnapshotError::Invalid("shadow filter")),
        };
        Ok(d)
    }
}

/// Only the description is saved; the derived values are recomputed on restore, so a snapshot
/// cannot carry values that disagree with it. A description that validation would reject is
/// [`SnapshotError::Invalid`].
impl SnapshotValue for StoredLight {
    fn put(&self, w: &mut Writer<'_>) {
        self.desc.put(w);
    }

    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let desc = LightDesc::get(r)?;
        desc.validate()
            .and_then(|()| desc.check_resources(&AssumeKnown))
            .map_err(|_| SnapshotError::Invalid("light desc violates the light rules"))?;
        Ok(Self::derive(&desc))
    }
}
