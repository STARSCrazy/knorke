//! Textures: RGBA texels, nearest sampling with wrap-around. Palette images are expanded at load time.

#[derive(Clone, Debug)]
/// A texture with its far levels and optional contour mask.
pub struct Texture {
    /// Width in texels.
    pub width: usize,
    /// Height in texels.
    pub height: usize,
    /// RGBA with full 8-bit alpha. Opaque drawing: alpha below 128 is a hole. Blended drawing
    /// (BLEND_ALPHA / BLEND_ADD): alpha / 255 scales the draw alpha.
    pub texels: Vec<[u8; 4]>,
    /// Smaller copies for far surfaces, widest first: built at load time (half, quarter, ...) or given
    /// by the caller (any sizes, e.g. hand-drawn far versions of the same picture).
    pub mips: Vec<Texture>,
    /// The levels came from the caller (`add_level`); the first such call drops the built ones.
    pub custom_levels: bool,
    /// Contour mask, one byte per texel: 0 = normal, 1..255 = importance (higher wins when the
    /// levels are built). On the levels it is the max-reduced mask of the level above (or given by
    /// the caller for a caller-made level).
    pub contour: Option<Vec<u8>>,
    /// The mask of this caller-made level was given by the caller (not reduced from the base mask).
    pub contour_given: bool,
    /// Area-preserving contour reduction for the built levels (`Options::contour_falloff`): 0 = a
    /// masked texel always wins its block, 1 = a joint keeps about its true area share. Set with
    /// `set_contour_falloff`; built levels inherit it.
    pub contour_falloff: f32,
    /// With contour_falloff > 0 on a built level: per texel the true share (0..1) of masked base
    /// texels it stands for. None on the texture itself (taken from the mask).
    pub coverage: Option<Box<Coverage>>,
}

/// Max-reduces a mask to another size (each target texel takes the max of the source texels it covers).
fn reduce_mask(mask: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    let mut out = vec![0u8; nw * nh];
    for y in 0..nh {
        let y0 = y * h / nh;
        let y1 = ((y + 1) * h / nh).max(y0 + 1).min(h);
        for x in 0..nw {
            let x0 = x * w / nw;
            let x1 = ((x + 1) * w / nw).max(x0 + 1).min(w);
            let mut m = 0;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    m = m.max(mask[sy * w + sx]);
                }
            }
            out[y * nw + x] = m;
        }
    }
    out
}

/// One level shrunk by `fx` along u and `fy` along v (each 1 or 2), with the contour rule and the
/// mask (if any) of `src`.
fn shrink(src: &Texture, fx: usize, fy: usize) -> Texture {
    if src.contour_falloff > 0.0 && src.contour.is_some() {
        return shrink_falloff(src, fx, fy);
    }
    let (w, h) = (src.width, src.height);
    let edge = contours(w, h, &src.texels);
    let (nw, nh) = (w / fx, h / fy);
    let mut texels = Vec::with_capacity(nw * nh);
    let mut mask = src.contour.as_ref().map(|_| Vec::with_capacity(nw * nh));
    let mut k = [0usize; 4];
    for y in 0..nh {
        for x in 0..nw {
            let mut n = 0;
            for dy in 0..fy {
                for dx in 0..fx {
                    k[n] = (fy * y + dy) * w + fx * x + dx;
                    n += 1;
                }
            }
            let k = &k[..n];
            let mut block = [[0u8; 4]; 4];
            let mut e = [false; 4];
            for (s, &i) in k.iter().enumerate() {
                block[s] = src.texels[i];
                e[s] = edge[i];
            }
            match (&src.contour, &mut mask) {
                (Some(m), Some(out)) => {
                    let mut mb = [0u8; 4];
                    for (s, &i) in k.iter().enumerate() {
                        mb[s] = m[i];
                    }
                    texels.push(reduce(&block[..n], &e[..n], Some(&mb[..n])));
                    out.push(mb[..n].iter().copied().max().unwrap_or(0));
                }
                _ => texels.push(reduce(&block[..n], &e[..n], None)),
            }
        }
    }
    Texture {
        contour: mask,
        ..Texture::plain(nw, nh, texels)
    }
}

/// Threshold rank (0..8) of level texel (x, y) for the falloff rule: bit-reversed (x + 3y) mod 8, so
/// every row and every column runs through all eight ranks, well spread. A line along a row or a
/// column with coverage c thus keeps about c of its texels, the same ones on every frame.
#[inline]
fn falloff_rank(x: usize, y: usize) -> u32 {
    let k = ((x + 3 * y) % 8) as u32;
    ((k & 1) << 2) | (k & 2) | ((k >> 2) & 1)
}

/// Per texel of a built level with contour_falloff: the true share (0..1) of masked base texels it
/// stands for, and the two colours it could show: the joint (masked) one and the slab one. The
/// shown texel picks one by the share, so a hidden joint can show again on a farther level and
/// the choice on one level never biases the next.
#[derive(Clone, Debug)]
pub struct Coverage {
    /// Per texel: share of masked base texels.
    pub share: Vec<f32>,
    /// Per texel: the joint (masked) colour.
    pub joint: Vec<[u8; 4]>,
    /// Per texel: the slab colour.
    pub slab: Vec<[u8; 4]>,
}

/// `shrink` with area-preserving contour reduction (contour_falloff > 0, mask present). The share
/// of a block is the mean of its children's shares; its joint colour is reduced from the children
/// that hold joint (highest mask, darkest), its slab colour from those that hold slab (texel closest to their average).
/// The block shows the joint colour while its share exceeds a position-dependent threshold
/// falloff * (rank + 0.5) / 8 (or when it holds no slab), else the slab colour; so a joint line
/// keeps about its true area share and is thinned instead of covering every far texel.
fn shrink_falloff(src: &Texture, fx: usize, fy: usize) -> Texture {
    let (w, h) = (src.width, src.height);
    let m = src.contour.as_ref().unwrap();
    let base;
    let cv = match &src.coverage {
        Some(c) => c,
        None => {
            base = Coverage {
                share: m.iter().map(|&v| if v > 0 { 1.0 } else { 0.0 }).collect(),
                joint: src.texels.clone(),
                slab: src.texels.clone(),
            };
            &base
        }
    };
    let (nw, nh) = (w / fx, h / fy);
    let mut texels = Vec::with_capacity(nw * nh);
    let mut mask = Vec::with_capacity(nw * nh);
    let mut out = Coverage {
        share: Vec::with_capacity(nw * nh),
        joint: Vec::with_capacity(nw * nh),
        slab: Vec::with_capacity(nw * nh),
    };
    let f = src.contour_falloff.min(1.0);
    for y in 0..nh {
        for x in 0..nw {
            let mut k = [0usize; 4];
            let mut n = 0;
            for dy in 0..fy {
                for dx in 0..fx {
                    k[n] = (fy * y + dy) * w + fx * x + dx;
                    n += 1;
                }
            }
            let k = &k[..n];
            let c = k.iter().map(|&i| cv.share[i]).sum::<f32>() / n as f32;
            let top = k.iter().map(|&i| m[i]).max().unwrap_or(0);
            // joint colour: children holding joint, highest mask first, darkest wins
            let mut jb = [[0u8; 4]; 4];
            let mut jm = [0u8; 4];
            let mut nj = 0;
            // slab colour: children holding slab, the real texel closest to their average (no dark-wins:
            // unmasked dark spots keep their area too)
            let mut sb = [[0u8; 4]; 4];
            let mut ns = 0;
            for &i in k {
                if cv.share[i] > 0.0 {
                    jb[nj] = cv.joint[i];
                    jm[nj] = m[i];
                    nj += 1;
                }
                if cv.share[i] < 1.0 {
                    sb[ns] = cv.slab[i];
                    ns += 1;
                }
            }
            let joint = if nj > 0 {
                reduce(&jb[..nj], &[true; 4][..nj], Some(&jm[..nj]))
            } else {
                [0; 4]
            };
            let slab = if ns > 0 {
                reduce(&sb[..ns], &[false; 4][..ns], None)
            } else {
                joint
            };
            let joint = if nj > 0 { joint } else { slab };
            let show = ns == 0 || (nj > 0 && c > f * (falloff_rank(x, y) as f32 + 0.5) / 8.0);
            texels.push(if show { joint } else { slab });
            mask.push(top);
            out.share.push(c);
            out.joint.push(joint);
            out.slab.push(slab);
        }
    }
    Texture {
        contour: Some(mask),
        coverage: Some(Box::new(out)),
        contour_falloff: src.contour_falloff,
        ..Texture::plain(nw, nh, texels)
    }
}

fn luma(c: [u8; 4]) -> f32 {
    0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32
}

/// Contour map of a level: a texel is part of a contour (joint, crack, stone outline) when it is
/// clearly darker than the average of its 3x3 neighbourhood (wrapping at the edges).
fn contours(w: usize, h: usize, t: &[[u8; 4]]) -> Vec<bool> {
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let c = t[y * w + x];
            if c[3] < 128 {
                continue;
            }
            let mut sum = 0.0;
            let mut n = 0.0;
            for dy in [h - 1, 0, 1] {
                for dx in [w - 1, 0, 1] {
                    let q = t[((y + dy) % h) * w + (x + dx) % w];
                    if q[3] >= 128 {
                        sum += luma(q);
                        n += 1.0;
                    }
                }
            }
            out[y * w + x] = n > 0.0 && luma(c) < 0.8 * sum / n;
        }
    }
    out
}

/// One block (2x2) of a level to one texel of the next. With a
/// mask, only the opaque texels with the highest mask value take part (all of them when it is 0).
/// Among those, contour texels have the right of way: if the block holds one, the darkest contour
/// texel wins with its own colour, so joints and outlines stay one pixel wide in their real colour
/// instead of fading out. Otherwise the block keeps the real texel closest to its average (no mixed
/// colours: the pixel look of hand-drawn far pictures).
fn reduce(block: &[[u8; 4]], contour: &[bool], mask: Option<&[u8]>) -> [u8; 4] {
    let mut opaque: Vec<usize> = (0..block.len()).filter(|&i| block[i][3] >= 128).collect();
    if opaque.len() * 2 < block.len() {
        return [0, 0, 0, 0];
    }
    if let Some(m) = mask {
        let top = opaque.iter().map(|&i| m[i]).max().unwrap_or(0);
        if top > 0 {
            opaque.retain(|&i| m[i] == top);
        }
    }
    let darkest = |list: &mut dyn Iterator<Item = usize>| {
        list.min_by(|&a, &b| luma(block[a]).total_cmp(&luma(block[b])))
    };
    if let Some(i) = darkest(&mut opaque.iter().copied().filter(|&i| contour[i])) {
        return block[i];
    }
    let n = opaque.len() as f32;
    let avg: [f32; 3] =
        std::array::from_fn(|k| opaque.iter().map(|&i| block[i][k] as f32).sum::<f32>() / n);
    let d = |i: usize| {
        (0..3)
            .map(|k| (block[i][k] as f32 - avg[k]).powi(2))
            .sum::<f32>()
    };
    block[*opaque
        .iter()
        .min_by(|&&a, &&b| d(a).total_cmp(&d(b)))
        .unwrap()]
}

impl Texture {
    /// A texture from RGBA bytes (row by row); None for a zero size or too few bytes.
    pub fn from_rgba(width: usize, height: usize, rgba: &[u8]) -> Option<Texture> {
        if width == 0 || height == 0 || rgba.len() < width * height * 4 {
            return None;
        }
        let texels = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .take(width * height)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect();
        Some(Texture::plain(width, height, texels).with_mips())
    }

    /// `palette` holds RGB triples. `transparent` is a palette index drawn as hole, or None.
    pub fn from_indexed(
        width: usize,
        height: usize,
        pixels: &[u8],
        palette: &[u8],
        transparent: Option<u8>,
    ) -> Option<Texture> {
        if width == 0
            || height == 0
            || pixels.len() < width * height
            || !palette.len().is_multiple_of(3)
        {
            return None;
        }
        let count = palette.len() / 3;
        let mut texels = Vec::with_capacity(width * height);
        for &i in &pixels[..width * height] {
            let k = i as usize;
            if Some(i) == transparent || k >= count {
                texels.push([0, 0, 0, 0]);
            } else {
                texels.push([palette[k * 3], palette[k * 3 + 1], palette[k * 3 + 2], 255]);
            }
        }
        Some(Texture::plain(width, height, texels).with_mips())
    }

    fn plain(width: usize, height: usize, texels: Vec<[u8; 4]>) -> Texture {
        Texture {
            width,
            height,
            texels,
            mips: Vec::new(),
            custom_levels: false,
            contour: None,
            contour_given: false,
            contour_falloff: 0.0,
            coverage: None,
        }
    }

    fn with_mips(mut self) -> Texture {
        self.build_mips();
        self
    }

    /// Builds the smaller levels down to 1 texel on the short side (with the mask, if any).
    fn build_mips(&mut self) {
        let mut mips: Vec<Texture> = Vec::new();
        loop {
            let src = mips.last().unwrap_or(self);
            if src.width < 2 || src.height < 2 {
                break;
            }
            let next = shrink(src, 2, 2);
            mips.push(next);
        }
        self.mips = mips;
    }

    /// Sets the contour mask (one byte per texel, see `contour`) or removes it (None) and rebuilds
    /// the levels. Caller-made levels keep a mask given for them; the others get the base mask
    /// reduced to their size. False if the mask does not match the texture size.
    pub fn set_contour(&mut self, mask: Option<Vec<u8>>) -> bool {
        if mask
            .as_ref()
            .is_some_and(|m| m.len() != self.width * self.height)
        {
            return false;
        }
        self.contour = mask;
        if self.custom_levels {
            let (w, h) = (self.width, self.height);
            for l in self.mips.iter_mut().filter(|l| !l.contour_given) {
                l.contour = self
                    .contour
                    .as_ref()
                    .map(|m| reduce_mask(m, w, h, l.width, l.height));
            }
        } else {
            self.build_mips();
        }
        true
    }

    /// Sets the area-preserving contour reduction (see `contour_falloff`, clamped to 0..1) and
    /// rebuilds the built levels if it changed and a mask is set.
    pub fn set_contour_falloff(&mut self, falloff: f32) {
        let f = if falloff.is_nan() {
            0.0
        } else {
            falloff.clamp(0.0, 1.0)
        };
        if f == self.contour_falloff {
            return;
        }
        self.contour_falloff = f;
        if self.contour.is_some() && !self.custom_levels {
            self.build_mips();
        }
    }

    /// Mask for level `level` (0 = the texture itself, 1.. = caller-made far levels, widest first).
    /// None on a caller-made level: reduced from the base mask again. False for a wrong size, an
    /// unknown level, or a built (not caller-made) level.
    pub fn set_level_contour(&mut self, level: usize, mask: Option<Vec<u8>>) -> bool {
        if level == 0 {
            return self.set_contour(mask);
        }
        if !self.custom_levels || level > self.mips.len() {
            return false;
        }
        let (w, h) = (self.width, self.height);
        let (lw, lh) = (self.mips[level - 1].width, self.mips[level - 1].height);
        if mask.as_ref().is_some_and(|m| m.len() != lw * lh) {
            return false;
        }
        let base = self.contour.as_ref().map(|m| reduce_mask(m, w, h, lw, lh));
        let l = &mut self.mips[level - 1];
        l.contour_given = mask.is_some();
        l.contour = mask.or(base);
        true
    }

    /// Automatic contour mask from the picture. A texel is dark when it is clearly darker than the
    /// mean of its 3x3 neighbourhood (luma < 0.8 x mean, as the old level rule). Dark texels that
    /// belong to a run of at least 3 dark texels along their row or column (wrapping) are lines
    /// (joints, seams, rims) and get contrast x strength x 255; isolated dark spots (pits, specks)
    /// get a quarter of that. At least 1 for any dark texel; strength is clamped to 0..1 (above 1
    /// most lines would saturate at 255 and tie). None for strength <= 0 (no mask).
    pub fn contour_auto(&self, strength: f32) -> Option<Vec<u8>> {
        if !(strength > 0.0) {
            return None;
        }
        let strength = strength.min(1.0);
        let (w, h, t) = (self.width, self.height, &self.texels);
        let mut contrast = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let c = t[y * w + x];
                if c[3] < 128 {
                    continue;
                }
                let (mut sum, mut n) = (0.0, 0.0);
                for dy in [h - 1, 0, 1] {
                    for dx in [w - 1, 0, 1] {
                        let q = t[((y + dy) % h) * w + (x + dx) % w];
                        if q[3] >= 128 {
                            sum += luma(q);
                            n += 1.0;
                        }
                    }
                }
                let mean = sum / n;
                if mean > 0.0 && luma(c) < 0.8 * mean {
                    contrast[y * w + x] = 1.0 - luma(c) / mean;
                }
            }
        }
        let dark = |x: usize, y: usize| contrast[(y % h) * w + x % w] > 0.0;
        // run length of dark texels through (x, y) along one axis, at most 3 needed
        let run = |x: usize, y: usize, dx: usize, dy: usize, len: usize| {
            let mut n = 1;
            for step in [1, len - 1] {
                let (mut cx, mut cy) = (x, y);
                for _ in 0..2.min(len - 1) {
                    cx = (cx + dx * step) % w;
                    cy = (cy + dy * step) % h;
                    if !dark(cx, cy) {
                        break;
                    }
                    n += 1;
                }
            }
            n
        };
        let mut out = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let c = contrast[y * w + x];
                if c <= 0.0 {
                    continue;
                }
                let line =
                    (w >= 3 && run(x, y, 1, 0, w) >= 3) || (h >= 3 && run(x, y, 0, 1, h) >= 3);
                let f = if line { 1.0 } else { 0.25 };
                out[y * w + x] = (c * f * strength * 255.0).round().clamp(1.0, 255.0) as u8;
            }
        }
        Some(out)
    }

    /// The level for a footprint of `lod` = log2(texels per pixel): the smallest level that is still at
    /// least width / 2^floor(lod) wide; the texture itself for lod < 1.
    #[inline]
    pub fn level(&self, lod: f32) -> &Texture {
        if lod < 1.0 || self.mips.is_empty() {
            return self;
        }
        let need = self.width as f32 / (1u64 << (lod as u32).min(30)) as f32;
        let mut best = self;
        for m in &self.mips {
            if (m.width as f32) < need - 0.5 {
                break;
            }
            best = m;
        }
        best
    }

    /// Adds a caller-made far level (any size). The first call drops the built levels.
    pub fn add_level(&mut self, level: Texture) {
        if !self.custom_levels {
            self.mips.clear();
            self.custom_levels = true;
        }
        let contour = self
            .contour
            .as_ref()
            .map(|m| reduce_mask(m, self.width, self.height, level.width, level.height));
        self.mips.push(Texture {
            mips: Vec::new(),
            contour,
            contour_given: false,
            coverage: None,
            ..level
        });
        self.mips.sort_by_key(|m| std::cmp::Reverse(m.width));
    }

    /// Nearest texel; coordinates outside [0, 1) stick to the edge texel. Used for faces that show
    /// exactly one image (walls, floors, sprites), so rounding at the edge never pulls in the opposite side.
    #[inline]
    pub fn sample_clamp(&self, u: f32, v: f32) -> [u8; 4] {
        let x = ((u * self.width as f32).floor() as i64).clamp(0, self.width as i64 - 1) as usize;
        let y = ((v * self.height as f32).floor() as i64).clamp(0, self.height as i64 - 1) as usize;
        self.texels[y * self.width + x]
    }

    /// Nearest texel for texture coordinates in [0, 1) (repeats outside).
    #[inline]
    pub fn sample(&self, u: f32, v: f32) -> [u8; 4] {
        let x = (u * self.width as f32).floor() as i64;
        let y = (v * self.height as f32).floor() as i64;
        // the division only for coordinates outside the first repeat
        let wrap = |i: i64, n: usize| {
            if (0..n as i64).contains(&i) {
                i as usize
            } else {
                i.rem_euclid(n as i64) as usize
            }
        };
        let (x, y) = (wrap(x, self.width), wrap(y, self.height));
        self.texels[y * self.width + x]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 32 x 32 grey noise with a bright (not darker!) one-texel column at x = 5.
    fn bright_line() -> (Texture, Vec<u8>) {
        let (w, h) = (32usize, 32usize);
        let mut rgba = Vec::new();
        let mut mask = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let c = if x == 5 {
                    [250u8, 240, 30]
                } else {
                    let b = 100 + ((x * 7 + y * 13) % 9) as u8 * 4;
                    [b, b, b]
                };
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
                if x == 5 {
                    mask[y * w + x] = 200;
                }
            }
        }
        (Texture::from_rgba(w, h, &rgba).unwrap(), mask)
    }

    #[test]
    fn zero_mask_keeps_old_levels() {
        let (t, _) = bright_line();
        let mut z = t.clone();
        assert!(z.set_contour(Some(vec![0; 32 * 32])));
        for (a, b) in t.mips.iter().zip(&z.mips) {
            assert_eq!(a.texels, b.texels);
        }
        assert!(z.set_contour(None));
        assert!(z.contour.is_none());
    }

    #[test]
    fn masked_line_survives_every_level() {
        let (mut t, mask) = bright_line();
        // without the mask the bright line is lost on some level
        let lost = t
            .mips
            .iter()
            .any(|m| !m.texels.contains(&[250, 240, 30, 255]));
        assert!(lost, "old rule drops the bright line");
        assert!(!t.set_contour(Some(vec![0; 3])), "wrong size refused");
        assert!(t.set_contour(Some(mask)));
        for m in &t.mips {
            let mm = m.contour.as_ref().expect("level mask");
            for y in 0..m.height {
                let row = &m.texels[y * m.width..(y + 1) * m.width];
                assert_eq!(
                    row.iter().filter(|c| **c == [250, 240, 30, 255]).count(),
                    1,
                    "one line texel per row, {}x{}",
                    m.width,
                    m.height
                );
                assert_eq!(
                    mm[y * m.width..(y + 1) * m.width]
                        .iter()
                        .filter(|&&v| v == 200)
                        .count(),
                    1,
                    "mask stays a line"
                );
            }
        }
    }

    #[test]
    fn highest_mask_wins_with_exact_colour() {
        let block = [
            [10, 10, 10, 255],
            [200, 100, 50, 255],
            [90, 90, 90, 255],
            [30, 30, 30, 255],
        ];
        let edge = [true, false, false, true];
        assert_eq!(
            reduce(&block, &edge, Some(&[0, 5, 9, 0])),
            [90, 90, 90, 255]
        );
        assert_eq!(
            reduce(&block, &edge, Some(&[0, 0, 0, 0])),
            reduce(&block, &edge, None)
        );
        // tie between two masked texels: old rule among them (darkest contour texel)
        assert_eq!(
            reduce(&block, &edge, Some(&[7, 7, 0, 7])),
            [10, 10, 10, 255]
        );
    }

    #[test]
    fn auto_contour_finds_dark_joints() {
        let (w, h) = (16usize, 16usize);
        let mut rgba = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                let c = if x % 8 == 7 { 40u8 } else { 160 };
                rgba.extend_from_slice(&[c, c, c, 255]);
            }
        }
        let t = Texture::from_rgba(w, h, &rgba).unwrap();
        let weak = t.contour_auto(0.5).unwrap();
        let strong = t.contour_auto(1.0).unwrap();
        for y in 0..h {
            for x in 0..w {
                let (a, b) = (weak[y * w + x], strong[y * w + x]);
                if x % 8 == 7 {
                    assert!(a > 0 && b > a, "joint weighted by strength: {a} {b}");
                } else {
                    assert_eq!(b, 0);
                }
            }
        }
        assert!(t.contour_auto(0.0).is_none());
    }

    #[test]
    fn auto_contour_prefers_lines_over_pits() {
        // grey 16 x 16, a joint column at x = 7 (luma 60) and an isolated darker pit at (3, 3) (luma 20)
        let mut rgba = Vec::new();
        for y in 0..16 {
            for x in 0..16 {
                let c = if x == 3 && y == 3 {
                    20u8
                } else if x == 7 {
                    60
                } else {
                    160
                };
                rgba.extend_from_slice(&[c, c, c, 255]);
            }
        }
        let t = Texture::from_rgba(16, 16, &rgba).unwrap();
        let m = t.contour_auto(1.0).unwrap();
        assert!(m[3 * 16 + 3] > 0, "pit is marked");
        assert!(
            m[5 * 16 + 7] > m[3 * 16 + 3],
            "joint {} beats pit {}",
            m[5 * 16 + 7],
            m[3 * 16 + 3]
        );
        assert_eq!(
            t.contour_auto(5.0),
            t.contour_auto(1.0),
            "strength clamped to 1"
        );
    }

    #[test]
    fn caller_level_mask_given_or_reduced() {
        let (mut t, mask) = bright_line();
        let far = Texture::from_rgba(8, 8, &[100; 8 * 8 * 4]).unwrap();
        t.add_level(far);
        assert!(t.set_contour(Some(mask)));
        // reduced from the base mask: column 5 / 4 = 1
        let m = t.mips[0].contour.as_ref().unwrap();
        assert_eq!(m[1], 200);
        assert_eq!(m[0], 0);
        assert!(t.set_level_contour(1, Some(vec![3; 64])));
        assert_eq!(t.mips[0].contour.as_ref().unwrap()[0], 3);
        assert!(!t.set_level_contour(1, Some(vec![3; 5])));
        assert!(!t.set_level_contour(2, Some(vec![3; 64])), "no such level");
        // a given level mask survives a new base mask
        assert!(t.set_contour(None));
        assert_eq!(t.mips[0].contour.as_ref().unwrap()[0], 3);
    }

    /// Slabs (grey 150) with 1-texel joints (dark 40) every 8 texels, `n` x `n`, joints masked.
    fn joint_grid(n: usize) -> (Texture, Vec<u8>) {
        let mut rgba = Vec::new();
        let mut mask = vec![0u8; n * n];
        for y in 0..n {
            for x in 0..n {
                let j = x % 8 == 7 || y % 8 == 7;
                let b = if j { 40 } else { 150 };
                rgba.extend_from_slice(&[b, b, b, 255]);
                mask[y * n + x] = if j { 255 } else { 0 };
            }
        }
        (Texture::from_rgba(n, n, &rgba).unwrap(), mask)
    }

    fn joint_share(t: &Texture) -> f32 {
        t.texels.iter().filter(|c| c[0] == 40).count() as f32 / t.texels.len() as f32
    }

    #[test]
    fn falloff_keeps_joint_area_share() {
        let (mut t, mask) = joint_grid(64);
        t.set_contour(Some(mask));
        let base = joint_share(&t);
        assert!(
            joint_share(&t.mips[2]) > 0.9,
            "falloff 0: far level all joints (old rule)"
        );
        let old: Vec<Vec<[u8; 4]>> = t.mips.iter().map(|m| m.texels.clone()).collect();
        t.set_contour_falloff(1.0);
        for (k, m) in t.mips.iter().enumerate().take(4) {
            let s = joint_share(m);
            assert!(
                (s - base).abs() <= 0.25 * base,
                "level {} share {s} base {base}",
                k + 1
            );
        }
        t.set_contour_falloff(0.0);
        let back: Vec<Vec<[u8; 4]>> = t.mips.iter().map(|m| m.texels.clone()).collect();
        assert_eq!(old, back, "falloff 0 = old levels");
    }

    #[test]
    fn falloff_thins_lines_but_keeps_them() {
        // a 1-texel horizontal line in row 5 of a 32 x 32 slab, masked
        let n = 32;
        let mut rgba = Vec::new();
        let mut mask = vec![0u8; n * n];
        for y in 0..n {
            for x in 0..n {
                let b = if y == 5 { 40 } else { 150 };
                rgba.extend_from_slice(&[b, b, b, 255]);
                if y == 5 {
                    mask[y * n + x] = 255;
                }
            }
        }
        let mut t = Texture::from_rgba(n, n, &rgba).unwrap();
        t.set_contour(Some(mask));
        t.set_contour_falloff(1.0);
        // level 1: share 1/2 per texel, level 2: 1/4
        for (k, row) in [(0usize, 2usize), (1, 1)] {
            let m = &t.mips[k];
            let line = (0..m.width)
                .filter(|&x| m.texels[row * m.width + x][0] == 40)
                .count();
            assert!(
                line > 0 && line < m.width,
                "level {}: line thinned, not erased ({line} of {})",
                k + 1,
                m.width
            );
            let other = m.texels.iter().filter(|c| c[0] == 40).count() - line;
            assert_eq!(other, 0, "joint stays on its row");
        }
        let a = t.mips.clone();
        t.set_contour_falloff(0.0);
        t.set_contour_falloff(1.0);
        assert!(
            a.iter().zip(&t.mips).all(|(x, y)| x.texels == y.texels),
            "deterministic"
        );
    }
}
