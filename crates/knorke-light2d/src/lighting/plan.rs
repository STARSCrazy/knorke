// Origin: chiaro @ 9bc5b9b, src/rendering/LightingStage.cpp (LightingStage::Prepare: the split of
// the light list and the fill of the light and mask buffers; LightingStage::Record: the order of
// the draws).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Which lights of a [`LightList`] go where in the light buffer, and the draws over it.
//!
//! Without active shadows every unmasked light goes out in one draw in store order (so the fp16
//! sums match Chiaro's phase 2 bit for bit). With shadows: the unshadowed lights, the shadowed
//! point and spot lights, the shadowed line and rect lights, then the shadowed lights with a
//! caster filter (E53) per class and kind. The masked lights (E115) follow all others, in the
//! same split. Directional lights are drawn per pixel after them.

use crate::gpu_light::{GpuLight, GpuMask};
use crate::light_culling::{Casters, LightList, MaskedLights};

/// Which program of the frame a draw uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DrawProgram {
    /// Unshadowed point/spot (and without shadows line/rect) lights.
    Unshadowed,
    /// Shadowed point/spot lights.
    Shadowed,
    /// Shadowed line/rect lights.
    AreaShadowed,
    /// Masked lights without shadows.
    MaskUnshadowed,
    /// Masked lights with shadows.
    MaskShadowed,
}

/// One instanced draw: `count` lights from light buffer index `offset`, shadowed by `casters`
/// (only the shadow programs read it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightDraw {
    /// The program.
    pub program: DrawProgram,
    /// Index of the first light in the light buffer.
    pub offset: u32,
    /// Number of lights (instances); 0 draws nothing.
    pub count: u32,
    /// Which obstacles cast the shadows.
    pub casters: Casters,
}

/// Number of draws in [`LightDraws::draws`].
pub const LIGHT_DRAW_COUNT: usize = 11;

/// The split of one frame's light list (Chiaro `LightingStage` members `m_firstCount` …).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LightDraws {
    /// Shadows are active: the list is split.
    pub split: bool,
    /// Unshadowed lights (all unmasked ones without shadows), drawn first.
    pub first: u32,
    /// Shadowed point/spot lights.
    pub second: u32,
    /// Shadowed line/rect lights.
    pub third: u32,
    /// Shadowed lights with a caster filter, `[class][0 point/spot, 1 line/rect]`.
    pub filtered: [[u32; 2]; 2],
    /// Light buffer index of the first masked light.
    pub mask_base: u32,
    /// Masked lights without shadows (all masked lights without active shadows).
    pub masked_first: u32,
    /// Masked shadowed lights, all casters.
    pub masked_shadowed: u32,
    /// Masked shadowed lights with a caster filter, per class.
    pub masked_filtered: [u32; 2],
    /// Directional lights drawn per pixel.
    pub directional: u32,
}

fn len(v: &[GpuLight]) -> u32 {
    v.len() as u32
}

impl LightDraws {
    /// The split of `list`. `shadows`: the occlusion stage ran this frame. `per_pixel`:
    /// directional lights are drawn per pixel (shadows, normal mode or a medium map).
    pub fn new(list: &LightList, shadows: bool, per_pixel: bool) -> Self {
        let split = shadows;
        let pick = |n: u32| if split { n } else { 0 };
        let first = if split {
            len(&list.unshadowed)
        } else {
            len(&list.ordered)
        };
        let mut filtered = [[0; 2]; 2];
        for (c, row) in filtered.iter_mut().enumerate() {
            for (a, n) in row.iter_mut().enumerate() {
                *n = pick(len(&list.filtered[c][a]));
            }
        }
        let (second, third) = (pick(len(&list.shadowed)), pick(len(&list.shadowed_area)));
        let unmasked = first + second + third + filtered.iter().flatten().sum::<u32>();
        let masked_first = if split {
            list.masked_unshadowed.len()
        } else {
            list.masked_ordered.len()
        } as u32;
        Self {
            split,
            first,
            second,
            third,
            filtered,
            mask_base: unmasked,
            masked_first,
            masked_shadowed: pick(list.masked_shadowed.len() as u32),
            masked_filtered: [
                pick(list.masked_filtered[0].len() as u32),
                pick(list.masked_filtered[1].len() as u32),
            ],
            directional: if per_pixel {
                list.directional.len() as u32
            } else {
                0
            },
        }
    }

    /// Masked lights in the buffer.
    pub fn masked(&self) -> u32 {
        self.masked_first + self.masked_shadowed + self.masked_filtered[0] + self.masked_filtered[1]
    }

    /// Lights in the light buffer (instanced lights, not the directional ones).
    pub fn count(&self) -> u32 {
        self.mask_base + self.masked()
    }

    /// Whether the frame has a light pass at all.
    pub fn light_pass(&self) -> bool {
        self.count() > 0 || self.directional > 0
    }

    /// Shadowed instanced lights (Chiaro `FrameCounters::shadowLights`).
    pub fn shadow_lights(&self) -> u32 {
        self.second
            + self.third
            + self.filtered.iter().flatten().sum::<u32>()
            + self.masked_shadowed
            + self.masked_filtered[0]
            + self.masked_filtered[1]
    }

    /// Whether a program is needed this frame.
    pub fn needs(&self, program: DrawProgram) -> bool {
        match program {
            DrawProgram::Unshadowed => self.first > 0,
            DrawProgram::Shadowed => self.second + self.filtered[0][0] + self.filtered[1][0] > 0,
            DrawProgram::AreaShadowed => self.third + self.filtered[0][1] + self.filtered[1][1] > 0,
            DrawProgram::MaskUnshadowed => self.masked_first > 0,
            DrawProgram::MaskShadowed => {
                self.masked_shadowed + self.masked_filtered[0] + self.masked_filtered[1] > 0
            }
        }
    }

    /// Whether a draw shadows with a caster filter class (E53).
    pub fn needs_casters(&self, casters: Casters) -> bool {
        casters.filter_index().is_some_and(|c| {
            self.filtered[c][0] + self.filtered[c][1] + self.masked_filtered[c] > 0
        })
    }

    /// The draws in recording order; entries with `count == 0` draw nothing.
    pub fn draws(&self) -> [LightDraw; LIGHT_DRAW_COUNT] {
        let d = |program, offset, count, casters| LightDraw {
            program,
            offset,
            count,
            casters,
        };
        let mut at = self.first + self.second + self.third;
        let mut filtered = [d(DrawProgram::Shadowed, 0, 0, Casters::All); 4];
        for (c, casters) in [Casters::Static, Casters::Dynamic].into_iter().enumerate() {
            for a in 0..2 {
                let program = if a == 1 {
                    DrawProgram::AreaShadowed
                } else {
                    DrawProgram::Shadowed
                };
                filtered[c * 2 + a] = d(program, at, self.filtered[c][a], casters);
                at += self.filtered[c][a];
            }
        }
        let m0 = self.mask_base + self.masked_first;
        let m1 = m0 + self.masked_shadowed;
        let m2 = m1 + self.masked_filtered[0];
        [
            d(DrawProgram::Unshadowed, 0, self.first, Casters::All),
            d(DrawProgram::Shadowed, self.first, self.second, Casters::All),
            d(
                DrawProgram::AreaShadowed,
                self.first + self.second,
                self.third,
                Casters::All,
            ),
            filtered[0],
            filtered[1],
            filtered[2],
            filtered[3],
            d(
                DrawProgram::MaskUnshadowed,
                self.mask_base,
                self.masked_first,
                Casters::All,
            ),
            d(
                DrawProgram::MaskShadowed,
                m0,
                self.masked_shadowed,
                Casters::All,
            ),
            d(
                DrawProgram::MaskShadowed,
                m1,
                self.masked_filtered[0],
                Casters::Static,
            ),
            d(
                DrawProgram::MaskShadowed,
                m2,
                self.masked_filtered[1],
                Casters::Dynamic,
            ),
        ]
    }

    /// The groups of the light buffer in order: unmasked lists, then the masked ones.
    fn groups<'l>(&self, list: &'l LightList) -> [&'l [GpuLight]; 11] {
        let n = |v: &'l [GpuLight], count: u32| &v[..count as usize];
        let masked_first: &MaskedLights = if self.split {
            &list.masked_unshadowed
        } else {
            &list.masked_ordered
        };
        [
            n(
                if self.split {
                    &list.unshadowed
                } else {
                    &list.ordered
                },
                self.first,
            ),
            n(&list.shadowed, self.second),
            n(&list.shadowed_area, self.third),
            n(&list.filtered[0][0], self.filtered[0][0]),
            n(&list.filtered[0][1], self.filtered[0][1]),
            n(&list.filtered[1][0], self.filtered[1][0]),
            n(&list.filtered[1][1], self.filtered[1][1]),
            n(&masked_first.lights, self.masked_first),
            n(&list.masked_shadowed.lights, self.masked_shadowed),
            n(&list.masked_filtered[0].lights, self.masked_filtered[0]),
            n(&list.masked_filtered[1].lights, self.masked_filtered[1]),
        ]
    }

    /// Writes the [`count`](Self::count) lights of `list` into `out` (`GpuLight` records of
    /// 128 bytes, in buffer order). `out` must hold them.
    pub fn write_lights(&self, list: &LightList, out: &mut [u8]) {
        let mut at = 0;
        for group in self.groups(list) {
            for light in group {
                out[at..at + GpuLight::SIZE].copy_from_slice(&light.to_bytes());
                at += GpuLight::SIZE;
            }
        }
    }

    /// Writes the masks of the [`masked`](Self::masked) lights into `out` (`GpuMask` records of
    /// 48 bytes): `out[i]` belongs to light `mask_base + i`.
    pub fn write_masks(&self, list: &LightList, out: &mut [u8]) {
        let masked_first: &MaskedLights = if self.split {
            &list.masked_unshadowed
        } else {
            &list.masked_ordered
        };
        let groups: [(&[GpuMask], u32); 4] = [
            (&masked_first.masks, self.masked_first),
            (&list.masked_shadowed.masks, self.masked_shadowed),
            (&list.masked_filtered[0].masks, self.masked_filtered[0]),
            (&list.masked_filtered[1].masks, self.masked_filtered[1]),
        ];
        let mut at = 0;
        for (masks, count) in groups {
            for m in &masks[..count as usize] {
                out[at..at + GpuMask::SIZE].copy_from_slice(&m.to_bytes());
                at += GpuMask::SIZE;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn light(tag: f32) -> GpuLight {
        GpuLight {
            position_radius: [tag, 0.0, 0.0, 1.0],
            ..Default::default()
        }
    }

    fn list() -> LightList {
        let mut masked_filtered: [MaskedLights; 2] = Default::default();
        masked_filtered[0] = MaskedLights {
            lights: vec![light(6.0)],
            masks: vec![GpuMask {
                n: [7.0; 4],
                ..Default::default()
            }],
        };
        let mut filtered: [[Vec<GpuLight>; 2]; 2] = Default::default();
        filtered[1][0] = vec![light(4.0)];
        LightList {
            ordered: vec![light(1.0), light(2.0), light(3.0)],
            unshadowed: vec![light(1.0)],
            shadowed: vec![light(2.0)],
            shadowed_area: vec![light(3.0)],
            filtered,
            masked_ordered: MaskedLights {
                lights: vec![light(5.0), light(6.0)],
                masks: vec![GpuMask::default(); 2],
            },
            masked_unshadowed: MaskedLights {
                lights: vec![light(5.0)],
                masks: vec![GpuMask::default()],
            },
            masked_filtered,
            ..Default::default()
        }
    }

    #[test]
    fn without_shadows_everything_goes_in_store_order() {
        let d = LightDraws::new(&list(), false, false);
        assert_eq!((d.first, d.second, d.third), (3, 0, 0));
        assert_eq!(d.filtered, [[0; 2]; 2]);
        assert_eq!((d.mask_base, d.masked_first, d.masked()), (3, 2, 2));
        assert_eq!(d.count(), 5);
        assert_eq!(d.shadow_lights(), 0);
        let draws = d.draws();
        assert_eq!(
            draws[0],
            LightDraw {
                program: DrawProgram::Unshadowed,
                offset: 0,
                count: 3,
                casters: Casters::All
            }
        );
        assert_eq!(draws[7].offset, 3);
        assert_eq!(draws[7].count, 2);
        assert_eq!(draws.iter().map(|d| d.count).sum::<u32>(), 5);
    }

    #[test]
    fn shadows_split_the_list_in_chiaro_order() {
        let l = list();
        let d = LightDraws::new(&l, true, true);
        assert_eq!((d.first, d.second, d.third), (1, 1, 1));
        assert_eq!(d.filtered, [[0, 0], [1, 0]]);
        assert_eq!(d.mask_base, 4);
        assert_eq!(
            (d.masked_first, d.masked_shadowed, d.masked_filtered),
            (1, 0, [1, 0])
        );
        assert_eq!(d.shadow_lights(), 4);
        assert!(d.needs(DrawProgram::Shadowed) && d.needs(DrawProgram::MaskShadowed));
        assert!(d.needs_casters(Casters::Static) && d.needs_casters(Casters::Dynamic));
        let draws = d.draws();
        // The dynamic point/spot light comes after the static ones (none) at index 3.
        assert_eq!(draws[5].casters, Casters::Dynamic);
        assert_eq!((draws[5].offset, draws[5].count), (3, 1));
        assert_eq!((draws[9].offset, draws[9].count), (5, 1));
        assert_eq!(draws[9].casters, Casters::Static);
        // Draws tile the buffer without gaps.
        let mut next = 0;
        for draw in draws.iter().filter(|d| d.count > 0) {
            assert_eq!(draw.offset, next);
            next += draw.count;
        }
        assert_eq!(next, d.count());

        let mut bytes = vec![0u8; d.count() as usize * GpuLight::SIZE];
        d.write_lights(&l, &mut bytes);
        let tags: Vec<f32> = bytes
            .chunks(GpuLight::SIZE)
            .map(|c| f32::from_le_bytes(c[0..4].try_into().unwrap()))
            .collect();
        assert_eq!(tags, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        let mut masks = vec![0u8; d.masked() as usize * GpuMask::SIZE];
        d.write_masks(&l, &mut masks);
        assert_eq!(f32::from_le_bytes(masks[48..52].try_into().unwrap()), 7.0);
    }

    #[test]
    fn directional_lights_are_drawn_only_per_pixel() {
        let l = LightList {
            directional: vec![Default::default(); 2],
            ..Default::default()
        };
        assert_eq!(LightDraws::new(&l, false, false).directional, 0);
        let d = LightDraws::new(&l, false, true);
        assert_eq!(d.directional, 2);
        assert!(d.light_pass());
        assert_eq!(d.count(), 0);
        assert!(!LightDraws::new(&LightList::default(), true, true).light_pass());
    }
}
