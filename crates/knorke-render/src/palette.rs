//! Output palette with fast nearest-colour lookup and ordered (Bayer) dithering.

use std::collections::BTreeMap;

pub(crate) struct Palette {
    pub(crate) colors: Vec<[u8; 3]>,
    exact: BTreeMap<[u8; 3], u8>,
    /// Nearest index for every colour with 6 bits per channel.
    cache: Vec<u8>,
    /// One bit per 6-bit cell: some palette colour lies in it, so `exact` must be asked.
    has_exact: Vec<u64>,
}

pub(crate) use knorke_math::BAYER4;

impl Palette {
    pub(crate) fn new(rgb: &[u8]) -> Option<Palette> {
        let n = rgb.len() / 3;
        if n == 0 || n > 256 || !rgb.len().is_multiple_of(3) {
            return None;
        }
        let colors: Vec<[u8; 3]> = rgb
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| [c[0], c[1], c[2]])
            .collect();
        let mut exact = BTreeMap::new();
        let mut has_exact = vec![0u64; 64 * 64 * 64 / 64];
        for (i, c) in colors.iter().enumerate() {
            exact.entry(*c).or_insert(i as u8);
            let k = cell(*c);
            has_exact[k / 64] |= 1 << (k % 64);
        }
        let mut cache = vec![0u8; 64 * 64 * 64];
        for r in 0..64u32 {
            for g in 0..64u32 {
                for b in 0..64u32 {
                    let c = [(r * 4 + 2) as i32, (g * 4 + 2) as i32, (b * 4 + 2) as i32];
                    cache[((r * 64 + g) * 64 + b) as usize] = nearest_slow(&colors, c);
                }
            }
        }
        Some(Palette {
            colors,
            exact,
            cache,
            has_exact,
        })
    }

    /// Nearest palette index. Colours that are in the palette map to their first index exactly.
    #[inline]
    pub(crate) fn nearest(&self, c: [u8; 3]) -> u8 {
        let k = cell(c);
        if self.has_exact[k / 64] >> (k % 64) & 1 != 0
            && let Some(&i) = self.exact.get(&c)
        {
            return i;
        }
        self.cache[k]
    }
}

#[inline]
fn cell(c: [u8; 3]) -> usize {
    ((c[0] as usize >> 2) * 64 + (c[1] as usize >> 2)) * 64 + (c[2] as usize >> 2)
}

fn nearest_slow(colors: &[[u8; 3]], c: [i32; 3]) -> u8 {
    let mut best = 0usize;
    let mut best_d = i32::MAX;
    for (i, p) in colors.iter().enumerate() {
        // Weighted distance, roughly matching how the eye sees brightness.
        let dr = p[0] as i32 - c[0];
        let dg = p[1] as i32 - c[1];
        let db = p[2] as i32 - c[2];
        let d = 3 * dr * dr + 4 * dg * dg + 2 * db * db;
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best as u8
}
