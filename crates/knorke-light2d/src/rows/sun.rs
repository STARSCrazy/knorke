//! Hard suns as row-style lights (plan 17.13, L3): a directional light's shadow from a shadow map
//! built once instead of Chiaro's march per pixel through the distance field.
//!
//! The map lies over the occlusion mask turned to the sun: a texel is [`SUN_ALONG`] mask texels
//! long toward the sun and [`SUN_ACROSS`] across. Per texel it holds the receiver heights a ray
//! toward the sun from the texel's middle finds shadowed, merged into two intervals (more are
//! hulled); a pixel reads the nearest texel and moves its height exactly along the light, so only
//! the offset across the light (at most an eighth of a texel) is an error. The map depends only on
//! the sun's direction and slope, the finite obstacles in the mask and the mask's size and anchor,
//! so it is kept until one of them changes ([`SunCache`]). Measured against an exact march:
//! about half of the error of Chiaro's march (plan 17.13, L3).
//!
//! The shaders are `lightrow_sun*` in `knorke-integration/shaders`.

use knorke_math::uniform_block;

use super::{MaskChange, RowMask, Source};
use crate::gpu_light::GpuDirectional;
use crate::light_culling::Casters;

/// Most suns with a map per frame; further ones stay with Chiaro's programs.
pub const MAX_SUNS: usize = 2;
/// Mask texels a map texel spans toward the sun.
pub const SUN_ALONG: f32 = 1.0;
/// Mask texels a map texel spans across the sun's direction.
pub const SUN_ACROSS: f32 = 0.25;
/// Longest side of a map in texels (the map spans the mask's diagonal both ways); larger masks
/// keep their suns with Chiaro's programs. The build marches at most 4096 texels
/// (`kMaxSteps` in `lightrow_sun_build.frag`), more than such a mask's width plus height.
pub const MAX_SUN_MAP: u32 = 8192;
/// Texels before the receiver that do not count as occluders (Chiaro's trace: 1).
const SUN_RECEIVER_GAP: f32 = 1.0;

/// Whether a directional light of [`LightList::directional`](crate::light_culling::LightList)
/// can take its shadow from a map, in a frame [`super::frame_suits`] accepts whose obstacles are
/// all opaque without pattern: it casts shadows from every caster (E53), has no source radius
/// (`dir_slope[3]` is its penumbra), does not stand overhead and does not shine from below the
/// horizon. All others stay with Chiaro's programs.
pub fn sun_suits(g: &GpuDirectional, casters: Casters) -> bool {
    let [x, y, slope, penumbra] = g.dir_slope;
    g.color[3] > 0.5
        && casters == Casters::All
        && penumbra == 0.0
        && (x != 0.0 || y != 0.0)
        && slope > 0.0
}

/// How a sun's map lies over the mask: map texel (i, j)'s middle is at
/// `origin + (i + 0.5) * a + (j + 0.5) * b` in mask texels, `a` toward the sun, `b` across.
/// Its size depends on the mask's size alone, so a turning sun keeps its texture.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SunGrid {
    /// Step of map texel i.
    pub a: [f32; 2],
    /// Step of map texel j.
    pub b: [f32; 2],
    /// The map's corner.
    pub origin: [f32; 2],
    /// Map size in texels.
    pub size: [u32; 2],
}

impl SunGrid {
    /// Map size for a mask of `mask` texels: its diagonal along and across.
    pub fn size_of(mask: [u32; 2]) -> [u32; 2] {
        let [w, h] = mask.map(|v| v as f32);
        let diagonal = (w * w + h * h).sqrt();
        [SUN_ALONG, SUN_ACROSS].map(|step| (diagonal / step).ceil() as u32)
    }

    /// The grid over a mask of `mask` texels for the unit direction `d` toward the sun (mask
    /// texels), centred on the mask.
    pub fn new(mask: [u32; 2], d: [f32; 2]) -> Self {
        let size = Self::size_of(mask);
        let a = [d[0] * SUN_ALONG, d[1] * SUN_ALONG];
        let b = [-d[1] * SUN_ACROSS, d[0] * SUN_ACROSS];
        let (i, j) = (size[0] as f32 * 0.5, size[1] as f32 * 0.5);
        let centre = [mask[0] as f32 * 0.5, mask[1] as f32 * 0.5];
        Self {
            a,
            b,
            origin: [0, 1].map(|k| centre[k] - i * a[k] - j * b[k]),
            size,
        }
    }

    /// The rows that give a mask point p's map texel coordinates: (dot(xy, p - origin),
    /// dot(zw, p - origin)).
    pub fn inverse(&self) -> [f32; 4] {
        let (a, b) = (self.a, self.b);
        let (a2, b2) = (a[0] * a[0] + a[1] * a[1], b[0] * b[0] + b[1] * b[1]);
        [a[0] / a2, a[1] / a2, b[0] / b2, b[1] / b2]
    }
}

uniform_block! {
    /// `SunParams` of `lightrow_sun.hlsli`: the sun's map build and its light.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct SunParams[128] {
        /// The light as Chiaro's directional programs get it.
        pub light: GpuDirectional,
        /// xy mask size in texels, z tolerance below a top, w highest finite top.
        pub domain: [f32; 4],
        /// [`SunGrid::a`] and [`SunGrid::b`].
        pub axes: [f32; 4],
        /// xy [`SunGrid::origin`], zw [`SunGrid::size`].
        pub origin: [f32; 4],
        /// [`SunGrid::inverse`].
        pub inverse: [f32; 4],
        /// x texels before the receiver that do not count (Chiaro: 1), y the lowest receiver
        /// height a map texel stands in for.
        pub build: [f32; 4],
    }
}

impl SunParams {
    /// The block of `light` over a mask of `mask` texels with Chiaro's `tolerance` below a top
    /// and the highest finite top `top`.
    pub fn new(light: &GpuDirectional, mask: [u32; 2], tolerance: f32, top: f32) -> Self {
        let [x, y, slope, _] = light.dir_slope;
        let g = SunGrid::new(mask, [x, y]);
        Self {
            light: *light,
            domain: [mask[0] as f32, mask[1] as f32, tolerance, top],
            axes: [g.a[0], g.a[1], g.b[0], g.b[1]],
            origin: [g.origin[0], g.origin[1], g.size[0] as f32, g.size[1] as f32],
            inverse: g.inverse(),
            // A ground receiver reads a texel whose middle lies up to half a texel farther from
            // the sun, so its height there is down to half a texel's rise below the ground.
            build: [SUN_RECEIVER_GAP, -slope * SUN_ALONG - 0.1, 0.0, 0.0],
        }
    }
}

/// What a sun's map was built from: everything [`SunParams`] holds except the colour and the
/// direction in the world, which only the light reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SunSource {
    /// `dir_slope` xyz.
    aim: [f32; 3],
    /// Mask size.
    size: [u32; 2],
    /// `MaskAnchor::snapped` of the frame.
    snapped: [i32; 2],
    /// Tolerance below a top, highest finite top.
    heights: [f32; 2],
}

impl SunSource {
    /// The source of `params` over `mask`.
    pub fn new(params: &SunParams, mask: &RowMask<'_>) -> Self {
        let [x, y, slope, _] = params.light.dir_slope;
        Self {
            aim: [x, y, slope],
            size: mask.size,
            snapped: mask.snapped,
            heights: [params.domain[2], params.domain[3]],
        }
    }
}

impl Source for SunSource {
    /// A sun's rays cross the whole mask: any redrawn texel and any pan (S1: even one by whole
    /// texels, which the map could follow) rebuild it.
    fn holds(&self, now: &Self, change: MaskChange<'_>) -> bool {
        self == now
            && match change {
                MaskChange::None => true,
                MaskChange::Rects(list) => !list.overflowed() && list.rects().is_empty(),
                MaskChange::All => false,
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::SunCache;
    use crate::shadow::{DirtyRect, DirtyRectList};

    fn sun(x: f32, y: f32, slope: f32) -> GpuDirectional {
        let n = (x * x + y * y).sqrt();
        GpuDirectional {
            dir_slope: [x / n, y / n, slope, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
            world: [0.0, 0.0, 1.0, 0.0],
        }
    }

    #[test]
    fn suits_names_what_a_map_cannot_do() {
        let hard = sun(1.0, 2.0, 0.5);
        assert!(sun_suits(&hard, Casters::All));
        assert!(!sun_suits(&hard, Casters::Static), "caster filter");
        let soft = GpuDirectional {
            dir_slope: [hard.dir_slope[0], hard.dir_slope[1], 0.5, 0.01],
            ..hard
        };
        assert!(!sun_suits(&soft, Casters::All), "source radius");
        let unshadowed = GpuDirectional {
            color: [1.0, 1.0, 1.0, 0.0],
            ..hard
        };
        assert!(!sun_suits(&unshadowed, Casters::All), "no shadows");
        let overhead = GpuDirectional {
            dir_slope: [0.0; 4],
            ..hard
        };
        assert!(!sun_suits(&overhead, Casters::All), "overhead");
        assert!(!sun_suits(&sun(1.0, 0.0, -0.1), Casters::All), "from below");
    }

    #[test]
    fn the_grid_is_turned_to_the_sun_and_covers_the_mask() {
        let mask = [480, 340];
        let d = sun(3.0, -4.0, 1.0).dir_slope;
        let g = SunGrid::new(mask, [d[0], d[1]]);
        assert_eq!(g.size, [589, 2353]);
        assert_eq!(
            SunGrid::size_of(mask),
            g.size,
            "independent of the direction"
        );
        let inv = g.inverse();
        let to_map = |p: [f32; 2]| {
            let r = [p[0] - g.origin[0], p[1] - g.origin[1]];
            [inv[0] * r[0] + inv[1] * r[1], inv[2] * r[0] + inv[3] * r[1]]
        };
        // Every corner of the mask maps into the grid.
        for p in [[0.0, 0.0], [480.0, 0.0], [0.0, 340.0], [480.0, 340.0]] {
            let m = to_map(p);
            assert!(m[0] >= 0.0 && m[0] <= g.size[0] as f32, "{p:?} {m:?}");
            assert!(m[1] >= 0.0 && m[1] <= g.size[1] as f32, "{p:?} {m:?}");
        }
        // A texel middle maps back to itself; one step toward the sun is one texel along.
        let c = [0, 1].map(|k| g.origin[k] + 10.5 * g.a[k] + 20.5 * g.b[k]);
        let m = to_map(c);
        assert!(
            (m[0] - 10.5).abs() < 1e-3 && (m[1] - 20.5).abs() < 1e-3,
            "{m:?}"
        );
        let m = to_map([c[0] + d[0], c[1] + d[1]]);
        assert!(
            (m[0] - 11.5).abs() < 1e-3 && (m[1] - 20.5).abs() < 1e-3,
            "{m:?}"
        );
    }

    #[test]
    fn a_map_is_kept_until_sun_or_mask_change() {
        let mut cache = SunCache::default();
        let none = MaskChange::None;
        let empty = DirtyRectList::new();
        let mut one = DirtyRectList::new();
        one.add(DirtyRect::new(190, 90, 191, 91));
        let mask = |snapped, change| RowMask {
            size: [200, 100],
            snapped,
            change,
        };
        let source = |light: &GpuDirectional, m: &RowMask<'_>| {
            SunSource::new(&SunParams::new(light, m.size, 0.05, 3.0), m)
        };
        let mut frame = |light: &GpuDirectional, m: &RowMask<'_>| {
            cache.begin();
            let out = cache.assign(0, source(light, m), m.change).unwrap();
            cache.commit();
            out
        };
        let a = sun(1.0, 2.0, 0.5);
        let still = mask([0, 0], none);
        assert_eq!(frame(&a, &still), (0, true));
        assert_eq!(frame(&a, &still), (0, false));
        // Colour and the world direction's specular factor are not in the map.
        let red = GpuDirectional {
            color: [1.0, 0.0, 0.0, 1.0],
            ..a
        };
        assert_eq!(frame(&red, &still), (0, false));
        // An empty list of redrawn texels changes nothing, one texel anywhere everything.
        assert_eq!(
            frame(&a, &mask([0, 0], MaskChange::Rects(&empty))),
            (0, false)
        );
        assert_eq!(frame(&a, &mask([0, 0], MaskChange::Rects(&one))), (0, true));
        assert_eq!(frame(&a, &mask([0, 0], MaskChange::All)), (0, true));
        assert_eq!(frame(&a, &still), (0, false));
        // Turned, steeper, panned, a larger mask.
        assert_eq!(frame(&sun(1.0, 2.1, 0.5), &still), (0, true));
        assert_eq!(frame(&sun(1.0, 2.1, 0.6), &still), (0, true));
        assert_eq!(frame(&sun(1.0, 2.1, 0.6), &mask([1, 0], none)), (0, true));
        let wide = RowMask {
            size: [201, 100],
            ..mask([1, 0], none)
        };
        assert_eq!(frame(&sun(1.0, 2.1, 0.6), &wide), (0, true));
    }
}
