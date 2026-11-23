//! Post-processing steps run at the end of a frame.

use super::*;

impl Renderer {
    pub(super) fn apply_outlines(&mut self) {
        let o = self.options;
        let (w, h) = (self.width, self.height);
        let g = &self.gbuf;
        let edge = &mut self.edge;
        edge.clear();
        edge.resize(w * h, false);
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let z = g.depth[i];
                if !z.is_finite() {
                    continue;
                }
                let mut hit = false;
                let mut check = |j: usize, later: bool| {
                    let zq = g.depth[j];
                    // Silhouette: the neighbour is much farther away (or empty) -> this pixel is the rim.
                    if !zq.is_finite() || zq > z * (1.0 + o.outline_depth) {
                        hit = true;
                    } else if later
                        && g.normal[i].dot(g.normal[j]) < o.outline_normal
                        && (zq - z).abs() < z * o.outline_depth
                    {
                        // Crease on one side only, so a crease is one pixel wide.
                        hit = true;
                    }
                };
                if x > 0 {
                    check(i - 1, false);
                }
                if x + 1 < w {
                    check(i + 1, true);
                }
                if y > 0 {
                    check(i - w, false);
                }
                if y + 1 < h {
                    check(i + w, true);
                }
                edge[i] = hit;
            }
        }
        let c = o.outline_color;
        let rgb = [
            ((c >> 16) & 255) as u8,
            ((c >> 8) & 255) as u8,
            (c & 255) as u8,
        ];
        let idx = self.palette.as_ref().map(|p| p.nearest(rgb));
        for (i, e) in edge.iter().enumerate() {
            if *e {
                let px = match (&self.palette, idx, o.quantize) {
                    (Some(p), Some(k), true) => p.colors[k as usize],
                    _ => rgb,
                };
                self.rgba[i * 4..i * 4 + 4].copy_from_slice(&[px[0], px[1], px[2], 255]);
                if let Some(k) = idx {
                    self.indexed[i] = k;
                }
            }
        }
    }
}
