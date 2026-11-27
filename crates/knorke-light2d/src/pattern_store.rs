// Origin: chiaro @ 9bc5b9b, src/shadows/PatternStore.hpp and PatternStore.cpp (with
// core/SlotMap.hpp replaced by knorke_types::Slots), the pattern part of
// src/core/FrameWarnings.cpp (the no-alpha note) and CHI_PatternDesc of include/Chiaro/Chiaro.h.
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! The patterns of one scene (P15): repeating opacity sheets that let light through slat fences,
//! lattices and grilles.
//!
//! A pattern is the alpha of a rectangle of a sheet (at most [`MAX_REGION`] per side), its mean
//! (the obstacle's mask value is `mean * opacity`, see
//! [`ObstacleStore::mask_opacity`](crate::ObstacleStore::mask_opacity)), its column means (read
//! without a finite ray height) and the world size of one repeat. Each live pattern owns a region
//! of the alpha band (the first [`ALPHA_ROWS`] rows) of the [`ATLAS_SIZE`]² R8 atlas, placed by a
//! shelf packer: shelves span the atlas width, their height is the region's rounded up to a
//! multiple of 16, freed regions go back to their shelf and an emptied shelf gives its rows back,
//! so create/destroy churn does not fill the atlas. The place does not depend on the handle.
//!
//! Redesign against the C++ version:
//!
//! - [`PatternId`] is a generation-checked handle of [`Slots`]. Obstacles keep patterns as raw
//!   32-bit ids (P14, as in Chiaro): [`raw_id`] and the [`PatternSource`] implementation of the
//!   store convert. The raw id holds the slot index in 12 bits and the generation in 20, so a
//!   slot can be reused about a million times; after that its create fails with
//!   [`PatternError::IdsExhausted`] (the C++ slot map retired a slot after 4095 generations).
//!   "Destroyed" and "never created" are both [`PatternError::InvalidHandle`].
//! - The sheet is a byte slice in one of the [`SheetFormat`]s instead of any SDL format, and
//!   errors are typed with stable codes `knorke.light2d.pattern.*` instead of SDL strings. A
//!   `u32` rectangle cannot be negative, so that check is gone.
//! - The free spans of a shelf and of the alpha rows are bit sets (a set bit is free) instead of
//!   sorted, merged span lists. First fit over maximal free runs takes the same place as first fit
//!   over merged spans, so placements equal Chiaro's.
//! - Alpha and column means of all patterns live in one byte arena, the packer in fixed-size
//!   shelves, so a [`Snapshot`] restore into a store of the same size allocates nothing.
//!   Patterns are loaded assets, but obstacles name them by id and the packer decides where the
//!   next one goes, so the whole state is saved (`knorke.light2d.patterns`).

use crate::obstacle::PatternSource;
use knorke_types::{
    Diagnostic, DiagnosticCode, Generation, Handle, Reader, Slots, Snapshot, SnapshotError,
    SnapshotValue, Writer,
};

/// Most patterns alive at once.
pub const MAX_PATTERNS: u32 = 4096;
/// Largest region side in pixels.
pub const MAX_REGION: u32 = 256;
/// Atlas side in texels (R8, 16 MiB); also the largest sheet side.
pub const ATLAS_SIZE: u32 = 4096;
/// Rows at the bottom of the atlas for the column means: one row of [`MAX_REGION`] texels per
/// pattern, `ATLAS_SIZE / MAX_REGION` side by side. The texture so never exceeds
/// `ATLAS_SIZE x ATLAS_SIZE` (Vulkan guarantees only 4096 per side).
pub const MEAN_ROWS: u32 = MAX_PATTERNS / (ATLAS_SIZE / MAX_REGION);
/// Rows the alpha regions may use.
pub const ALPHA_ROWS: u32 = ATLAS_SIZE - MEAN_ROWS;

/// Column-mean rows: patterns per atlas row.
pub const MEANS_PER_ROW: u32 = ATLAS_SIZE / MAX_REGION;

const INDEX_BITS: u32 = 12;
const INDEX_MASK: u32 = (1 << INDEX_BITS) - 1;
const MAX_GENERATION: u32 = (1 << (32 - INDEX_BITS)) - 1;
const SHELF_STEP: u32 = 16;
const ROW_WORDS: usize = (ALPHA_ROWS / 64) as usize;
const COLUMN_WORDS: usize = (ATLAS_SIZE / 64) as usize;

/// A live pattern: its atlas region, repeat and where its texels lie in the store's arena.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pattern {
    atlas_x: u32,
    atlas_y: u32,
    width: u32,
    height: u32,
    repeat_width: f32,
    repeat_height: f32,
    mean: f32,
    /// First arena byte: `width * height` alpha (top row first), then `width` column means.
    start: u32,
}

impl Pattern {
    fn bytes(&self) -> usize {
        (self.width * (self.height + 1)) as usize
    }
}

/// Handle of a pattern in a [`PatternStore`].
pub type PatternId = Handle<Pattern>;

/// The 32-bit id obstacles store for a pattern (`0` is none, never returned).
pub fn raw_id(id: PatternId) -> u32 {
    (id.generation().get() << INDEX_BITS) | id.index()
}

fn from_raw(raw: u32) -> Option<PatternId> {
    Generation::new(raw >> INDEX_BITS).map(|g| PatternId::new(raw & INDEX_MASK, g))
}

/// Pixel layout of a sheet: bytes per pixel and where alpha is. `Rgb8` has no alpha: every texel
/// is opaque, and [`PatternStore::create`] reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetFormat {
    /// R, G, B, A bytes (SDL's `RGBA32`).
    Rgba8,
    /// B, G, R, A bytes (SDL's `BGRA32`).
    Bgra8,
    /// R, G, B bytes, no alpha.
    Rgb8,
    /// One alpha byte.
    A8,
}

impl SheetFormat {
    fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8 | Self::Bgra8 => 4,
            Self::Rgb8 => 3,
            Self::A8 => 1,
        }
    }

    fn alpha_offset(self) -> Option<usize> {
        match self {
            Self::Rgba8 | Self::Bgra8 => Some(3),
            Self::Rgb8 => None,
            Self::A8 => Some(0),
        }
    }
}

/// A sheet of pixels the patterns are cut from.
#[derive(Clone, Copy, Debug)]
pub struct Sheet<'a> {
    /// Width in pixels, `[1, ATLAS_SIZE]`.
    pub width: u32,
    /// Height in pixels, `[1, ATLAS_SIZE]`.
    pub height: u32,
    /// Bytes from one row to the next, at least `width * bytes per pixel`.
    pub pitch: usize,
    /// Pixel layout.
    pub format: SheetFormat,
    /// The pixels, row 0 the top.
    pub pixels: &'a [u8],
}

/// One pattern of a [`PatternStore::create`] batch (`CHI_PatternDesc`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PatternDesc {
    /// Left edge of the region in the sheet.
    pub x: u32,
    /// Top edge of the region in the sheet.
    pub y: u32,
    /// Region width, `[1, MAX_REGION]`.
    pub width: u32,
    /// Region height, `[1, MAX_REGION]`.
    pub height: u32,
    /// World length of one repeat along the outline, finite and `> 0`. Default 1.
    pub repeat_width: f32,
    /// World height of one repeat, finite and `>= 0`; `0` (default) stretches over the
    /// obstacle's height span.
    pub repeat_height: f32,
}

impl PatternDesc {
    /// The region `(x, y, width, height)` with the defaults of `CHI_InitPatternDesc`.
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            repeat_width: 1.0,
            repeat_height: 0.0,
        }
    }
}

/// A live pattern with its texels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PatternView<'a> {
    /// Left edge of the region in the atlas.
    pub atlas_x: u32,
    /// Top edge of the region in the atlas.
    pub atlas_y: u32,
    /// Region width.
    pub width: u32,
    /// Region height.
    pub height: u32,
    /// World length of one repeat.
    pub repeat_width: f32,
    /// World height of one repeat; 0 stretches over the height span.
    pub repeat_height: f32,
    /// Mean alpha over all texels, `[0, 1]`.
    pub mean: f32,
    /// `width * height` alpha bytes, row-major, top row first.
    pub alpha: &'a [u8],
    /// Per column the round-to-nearest (halves up) of its 0..255 average.
    pub column_mean: &'a [u8],
}

impl PatternView<'_> {
    /// The alpha at `(u, v)`, both in repeats: the fraction of each (also of negative values)
    /// picks the nearest texel, `v` 0 is the bottom row and `v -> 1` the top. Without height the
    /// column mean at `u`, whatever `v` is. Non-finite `u` or `v` read as 0. Returns `[0, 1]`.
    ///
    /// The same steps run in HLSL (Chiaro `shaders/src/pattern.hlsli`, `SamplePatternAtlas`).
    pub fn sample(&self, u: f32, v: f32, has_height: bool) -> f32 {
        // The texel index is clamped on both ends: the fraction may round up to 1.0 for a tiny
        // negative value.
        let texel = |t: f32, n: u32| {
            let f = if t.is_finite() { t - t.floor() } else { 0.0 };
            ((f * n as f32) as i32).clamp(0, n as i32 - 1) as usize
        };
        let x = texel(u, self.width);
        if !has_height {
            return f32::from(self.column_mean[x]) / 255.0;
        }
        let y = self.height as usize - 1 - texel(v, self.height); // row 0 is the top
        f32::from(self.alpha[y * self.width as usize + x]) / 255.0
    }
}

/// An error of the pattern store, with a stable diagnostic code.
#[derive(Clone, Debug, PartialEq)]
pub enum PatternError {
    /// Sheet side outside `[1, ATLAS_SIZE]`.
    SheetSize {
        /// Width.
        width: u32,
        /// Height.
        height: u32,
    },
    /// Pitch below `width * bytes per pixel`.
    Pitch {
        /// The pitch.
        pitch: usize,
        /// The least pitch.
        min: usize,
    },
    /// Fewer pixel bytes than the last row needs.
    PixelsTooShort {
        /// Bytes needed.
        needed: usize,
        /// Bytes given.
        got: usize,
    },
    /// `descs[index]`: the region is empty, outside the sheet or larger than [`MAX_REGION`].
    Rect {
        /// Index in the batch.
        index: usize,
        /// The desc.
        desc: PatternDesc,
        /// Sheet width.
        sheet_width: u32,
        /// Sheet height.
        sheet_height: u32,
    },
    /// `descs[index]`: `repeat_width` not finite and `> 0`, or `repeat_height` not finite and
    /// `>= 0`.
    Repeat {
        /// Index in the batch.
        index: usize,
        /// `"repeat_width"` or `"repeat_height"`.
        field: &'static str,
        /// The value.
        value: f32,
    },
    /// More than [`MAX_PATTERNS`] would be alive.
    TooMany {
        /// Alive now.
        alive: u32,
        /// Asked for.
        asked: usize,
    },
    /// `descs[index]` does not fit into the atlas any more.
    AtlasFull {
        /// Index in the batch.
        index: usize,
    },
    /// `ids[index]` is not a live pattern (0, destroyed or never created).
    InvalidHandle {
        /// Index in the batch.
        index: usize,
        /// The raw id.
        raw: u32,
    },
    /// `ids[index]` names a pattern twice.
    Duplicate {
        /// Index in the batch.
        index: usize,
    },
    /// A slot was reused so often that its generation no longer fits the raw id.
    IdsExhausted,
}

impl PatternError {
    /// The stable diagnostic code (`knorke.light2d.pattern.*`).
    pub fn code(&self) -> DiagnosticCode {
        DiagnosticCode::new(match self {
            Self::SheetSize { .. } | Self::Pitch { .. } | Self::PixelsTooShort { .. } => {
                "knorke.light2d.pattern.sheet"
            }
            Self::Rect { .. } | Self::Repeat { .. } => "knorke.light2d.pattern.desc",
            Self::TooMany { .. } => "knorke.light2d.pattern.too_many",
            Self::AtlasFull { .. } => "knorke.light2d.pattern.atlas_full",
            Self::InvalidHandle { .. } => "knorke.light2d.pattern.invalid_handle",
            Self::Duplicate { .. } => "knorke.light2d.pattern.duplicate",
            Self::IdsExhausted => "knorke.light2d.pattern.ids_exhausted",
        })
    }

    /// As a diagnostic.
    pub fn to_diagnostic(&self) -> Diagnostic {
        Diagnostic::error(self.code(), self.to_string())
    }
}

impl core::fmt::Display for PatternError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SheetSize { width, height } => write!(
                f,
                "pattern sheet: width and height must be 1..{ATLAS_SIZE} (got {width} x {height})"
            ),
            Self::Pitch { pitch, min } => write!(
                f,
                "pattern sheet: pitch must be at least width * bytes per pixel = {min} (got {pitch})"
            ),
            Self::PixelsTooShort { needed, got } => write!(
                f,
                "pattern sheet: pixels must hold {needed} bytes (got {got})"
            ),
            Self::Rect {
                index,
                desc: d,
                sheet_width,
                sheet_height,
            } => write!(
                f,
                "descs[{index}]: rect ({}, {}, {} x {}) must be 1 x 1 .. {MAX_REGION} x \
                 {MAX_REGION} and inside the {sheet_width} x {sheet_height} sheet",
                d.x, d.y, d.width, d.height
            ),
            Self::Repeat {
                index,
                field,
                value,
            } => write!(
                f,
                "descs[{index}]: {field} must be finite and {} (got {value})",
                if *field == "repeat_width" {
                    "> 0"
                } else {
                    ">= 0, 0 stretches over the height span"
                }
            ),
            Self::TooMany { alive, asked } => write!(
                f,
                "too many patterns (at most {MAX_PATTERNS} alive at once, {alive} alive, {asked} \
                 more asked for); destroy some"
            ),
            Self::AtlasFull { index } => write!(
                f,
                "descs[{index}]: the pattern atlas is full (all live patterns together at most \
                 {ATLAS_SIZE} x {ALPHA_ROWS} texels, regions in shelves of 16-row steps; the last \
                 {MEAN_ROWS} of the {ATLAS_SIZE} rows hold the column means); destroy some"
            ),
            Self::InvalidHandle { index, raw } => write!(
                f,
                "ids[{index}]: pattern {raw:#010x} is not live (0, destroyed or never created)"
            ),
            Self::Duplicate { index } => write!(f, "ids[{index}]: the pattern is named twice"),
            Self::IdsExhausted => write!(
                f,
                "a pattern slot was reused too often for its 20-bit generation"
            ),
        }
    }
}

/// A shelf of the packer: rows `y .. y + height`, its free columns as bits.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shelf {
    y: u32,
    height: u32,
    used: u32,
    free: [u64; COLUMN_WORDS],
}

/// First fit in a free-bit set of `bits` bits: the start of the first maximal free run of at
/// least `len` bits, taken (cleared) from its front.
fn take_run(words: &mut [u64], bits: u32, len: u32) -> Option<u32> {
    let free = |words: &[u64], i: u32| (words[(i / 64) as usize] >> (i % 64)) & 1 != 0;
    let mut i = 0;
    while i < bits {
        if !free(words, i) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bits && free(words, i) {
            i += 1;
        }
        if i - start >= len {
            set_run(words, start, len, false);
            return Some(start);
        }
    }
    None
}

fn set_run(words: &mut [u64], start: u32, len: u32, free: bool) {
    for i in start..start + len {
        let (w, b) = ((i / 64) as usize, i % 64);
        if free {
            words[w] |= 1 << b;
        } else {
            words[w] &= !(1 << b);
        }
    }
}

/// The shelf packer of the alpha band.
#[derive(Clone, Debug, PartialEq)]
struct Packer {
    /// Rows without a shelf.
    rows: [u64; ROW_WORDS],
    shelves: Vec<Shelf>,
}

impl Default for Packer {
    fn default() -> Self {
        Self {
            rows: [!0; ROW_WORDS],
            shelves: Vec::new(),
        }
    }
}

fn shelf_height(h: u32) -> u32 {
    h.div_ceil(SHELF_STEP) * SHELF_STEP
}

impl Packer {
    fn take(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let height = shelf_height(h);
        for shelf in &mut self.shelves {
            if shelf.height == height
                && let Some(x) = take_run(&mut shelf.free, ATLAS_SIZE, w)
            {
                shelf.used += 1;
                return Some((x, shelf.y));
            }
        }
        let y = take_run(&mut self.rows, ALPHA_ROWS, height)?;
        let mut shelf = Shelf {
            y,
            height,
            used: 1,
            free: [!0; COLUMN_WORDS],
        };
        set_run(&mut shelf.free, 0, w, false);
        self.shelves.push(shelf);
        Some((0, y))
    }

    fn give(&mut self, p: &Pattern) {
        let height = shelf_height(p.height);
        if let Some(i) = self
            .shelves
            .iter()
            .position(|s| s.y == p.atlas_y && s.height == height)
        {
            let shelf = &mut self.shelves[i];
            shelf.used -= 1;
            if shelf.used == 0 {
                // Empty: the rows go back, the shelf goes.
                set_run(&mut self.rows, shelf.y, shelf.height, true);
                self.shelves.remove(i);
            } else {
                set_run(&mut shelf.free, p.atlas_x, p.width, true);
            }
        }
    }
}

/// The patterns of one scene. Every mutation validates first and leaves the state unchanged on
/// error.
#[derive(Clone, Debug, Default)]
pub struct PatternStore {
    patterns: Slots<Pattern>,
    /// Alpha and column means of every live pattern, packed (destroy closes the gap).
    texels: Vec<u8>,
    packer: Packer,
    /// The packer before a batch, so a failing one leaves it whole.
    spare: Packer,
    version: u64,
}

impl PatternStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of live patterns.
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// `true` without patterns.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Changes with every successful create, destroy and restore, for the atlas upload.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Cuts `descs.len()` regions out of `sheet` (only alpha is kept) and appends their handles to
    /// `out`. All or nothing: on error nothing is created and `out` is unchanged. Returns `true`
    /// if the sheet has no alpha channel, so every texel is opaque (Chiaro's
    /// `CHI_WARN_PATTERN_NO_ALPHA`): then pass [`Warning::PatternNoAlpha`] to the plan's
    /// `add_pending_warnings` for the debug check.
    ///
    /// [`Warning::PatternNoAlpha`]: crate::frame_warnings::Warning::PatternNoAlpha
    ///
    /// # Errors
    /// A bad sheet or desc, [`MAX_PATTERNS`] exceeded, a full atlas, or
    /// [`PatternError::IdsExhausted`].
    pub fn create(
        &mut self,
        sheet: &Sheet<'_>,
        descs: &[PatternDesc],
        out: &mut Vec<PatternId>,
    ) -> Result<bool, PatternError> {
        if descs.is_empty() {
            return Ok(false);
        }
        check_sheet(sheet)?;
        for (index, d) in descs.iter().enumerate() {
            check_desc(index, d, sheet)?;
        }
        let alive = self.patterns.len() as u32;
        if alive as usize + descs.len() > MAX_PATTERNS as usize {
            return Err(PatternError::TooMany {
                alive,
                asked: descs.len(),
            });
        }
        // Place everything on a copy of the packer; the store changes only after the last check.
        self.spare.clone_from(&self.packer);
        let mut placed = Vec::with_capacity(descs.len());
        for (index, d) in descs.iter().enumerate() {
            let place = self.spare.take(d.width, d.height);
            placed.push(place.ok_or(PatternError::AtlasFull { index })?);
        }
        let first = out.len();
        for (d, &(atlas_x, atlas_y)) in descs.iter().zip(&placed) {
            let start = self.texels.len() as u32;
            let mean = read_region(sheet, d, &mut self.texels);
            let id = self.patterns.insert(Pattern {
                atlas_x,
                atlas_y,
                width: d.width,
                height: d.height,
                repeat_width: d.repeat_width,
                repeat_height: d.repeat_height,
                mean,
                start,
            });
            out.push(id);
            if id.generation().get() > MAX_GENERATION {
                // Undo: a reused slot's generation no longer fits the raw id.
                for &id in &out[first..] {
                    self.remove(id);
                }
                out.truncate(first);
                return Err(PatternError::IdsExhausted);
            }
        }
        core::mem::swap(&mut self.packer, &mut self.spare);
        self.version += 1;
        Ok(sheet.format.alpha_offset().is_none())
    }

    /// Destroys the patterns `ids`. Obstacles that name them act as without; the host tells the
    /// obstacle store with
    /// [`on_patterns_destroyed`](crate::ObstacleStore::on_patterns_destroyed).
    ///
    /// # Errors
    /// An id that is not live, or one named twice; nothing changes.
    pub fn destroy(&mut self, ids: &[PatternId]) -> Result<(), PatternError> {
        for (index, &id) in ids.iter().enumerate() {
            if !self.patterns.contains(id) {
                return Err(PatternError::InvalidHandle {
                    index,
                    raw: raw_id(id),
                });
            }
        }
        for (index, &id) in ids.iter().enumerate() {
            if ids[..index].contains(&id) {
                return Err(PatternError::Duplicate { index });
            }
        }
        if ids.is_empty() {
            return Ok(());
        }
        for &id in ids {
            let p = *self.patterns.get(id).expect("checked above");
            self.packer.give(&p);
            self.remove(id);
        }
        self.version += 1;
        Ok(())
    }

    /// Removes a pattern and closes its gap in the arena (no allocation).
    fn remove(&mut self, id: PatternId) {
        let Some(p) = self.patterns.remove(id) else {
            return;
        };
        let (start, len) = (p.start as usize, p.bytes());
        self.texels.copy_within(start + len.., start);
        self.texels.truncate(self.texels.len() - len);
        for (_, q) in self.patterns.iter_mut() {
            if q.start > p.start {
                q.start -= len as u32;
            }
        }
    }

    /// `true` if `id` names a live pattern.
    pub fn contains(&self, id: PatternId) -> bool {
        self.patterns.contains(id)
    }

    /// The pattern, or `None` for a stale handle.
    pub fn get(&self, id: PatternId) -> Option<PatternView<'_>> {
        self.patterns.get(id).map(|p| self.view(p))
    }

    /// The pattern of a raw id (`0` and stale ids give `None`).
    pub fn get_raw(&self, raw: u32) -> Option<PatternView<'_>> {
        self.get(from_raw(raw)?)
    }

    /// Every live pattern, in slot order (the order of the GPU records).
    pub fn iter(&self) -> impl Iterator<Item = (PatternId, PatternView<'_>)> {
        self.patterns.iter().map(|(id, p)| (id, self.view(p)))
    }

    /// Atlas texture rows for the live patterns and the alpha rows among them: the alpha rows in
    /// use, then one column-mean row per [`MEANS_PER_ROW`] patterns, rounded up to 16 (Chiaro
    /// `PatternAtlas::RowsFor`). At most [`ATLAS_SIZE`]: alpha only lies in the alpha band.
    pub fn atlas_rows(&self) -> (u32, u32) {
        let alpha = self
            .patterns
            .iter()
            .map(|(_, p)| p.atlas_y + p.height)
            .max()
            .unwrap_or(0);
        let means = (self.patterns.len() as u32).div_ceil(MEANS_PER_ROW);
        ((alpha + means).div_ceil(SHELF_STEP) * SHELF_STEP, alpha)
    }

    fn view(&self, p: &Pattern) -> PatternView<'_> {
        let start = p.start as usize;
        let texels = (p.width * p.height) as usize;
        PatternView {
            atlas_x: p.atlas_x,
            atlas_y: p.atlas_y,
            width: p.width,
            height: p.height,
            repeat_width: p.repeat_width,
            repeat_height: p.repeat_height,
            mean: p.mean,
            alpha: &self.texels[start..start + texels],
            column_mean: &self.texels[start + texels..start + texels + p.width as usize],
        }
    }

    fn restore_state(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.patterns.get_into(r)?;
        let n = r.get_len()?;
        self.texels.clear();
        self.texels.extend_from_slice(r.take(n)?);
        let mut bytes = 0usize;
        for (_, p) in self.patterns.iter() {
            let fits = (1..=MAX_REGION).contains(&p.width)
                && (1..=MAX_REGION).contains(&p.height)
                // Subtractions: the sides are bounded above, a sum could overflow.
                && p.atlas_x <= ATLAS_SIZE - p.width
                && p.atlas_y <= ALPHA_ROWS - p.height
                && p.start as usize + p.bytes() <= self.texels.len();
            if !fits {
                return Err(SnapshotError::Invalid("pattern region"));
            }
            bytes += p.bytes();
        }
        if bytes != self.texels.len() || self.patterns.len() > MAX_PATTERNS as usize {
            return Err(SnapshotError::Invalid("pattern texels"));
        }
        for word in &mut self.packer.rows {
            *word = r.get()?;
        }
        let shelves = r.get_len()?;
        self.packer.shelves.clear();
        for _ in 0..shelves {
            let mut shelf = Shelf {
                y: r.get()?,
                height: r.get()?,
                used: r.get()?,
                free: [0; COLUMN_WORDS],
            };
            for word in &mut shelf.free {
                *word = r.get()?;
            }
            if shelf.height == 0
                || !shelf.height.is_multiple_of(SHELF_STEP)
                || shelf
                    .y
                    .checked_add(shelf.height)
                    .is_none_or(|end| end > ALPHA_ROWS)
            {
                return Err(SnapshotError::Invalid("pattern shelf"));
            }
            self.packer.shelves.push(shelf);
        }
        // Every pattern must sit in a shelf of its height: `destroy` gives it back there.
        let homeless = self.patterns.iter().any(|(_, p)| {
            !self
                .packer
                .shelves
                .iter()
                .any(|s| s.y == p.atlas_y && s.height == shelf_height(p.height))
        });
        if homeless {
            return Err(SnapshotError::Invalid("pattern shelf"));
        }
        Ok(())
    }
}

impl PatternSource for PatternStore {
    fn mean_opacity(&self, id: u32) -> Option<f32> {
        self.get_raw(id).map(|p| p.mean)
    }
}

fn check_sheet(s: &Sheet<'_>) -> Result<(), PatternError> {
    if !(1..=ATLAS_SIZE).contains(&s.width) || !(1..=ATLAS_SIZE).contains(&s.height) {
        return Err(PatternError::SheetSize {
            width: s.width,
            height: s.height,
        });
    }
    let row = s.width as usize * s.format.bytes_per_pixel();
    if s.pitch < row {
        return Err(PatternError::Pitch {
            pitch: s.pitch,
            min: row,
        });
    }
    let needed = (s.height as usize - 1) * s.pitch + row;
    if s.pixels.len() < needed {
        return Err(PatternError::PixelsTooShort {
            needed,
            got: s.pixels.len(),
        });
    }
    Ok(())
}

fn check_desc(index: usize, d: &PatternDesc, s: &Sheet<'_>) -> Result<(), PatternError> {
    let inside = |at: u32, len: u32, side: u32| u64::from(at) + u64::from(len) <= u64::from(side);
    if !(1..=MAX_REGION).contains(&d.width)
        || !(1..=MAX_REGION).contains(&d.height)
        || !inside(d.x, d.width, s.width)
        || !inside(d.y, d.height, s.height)
    {
        return Err(PatternError::Rect {
            index,
            desc: *d,
            sheet_width: s.width,
            sheet_height: s.height,
        });
    }
    let repeat = |field, value: f32| PatternError::Repeat {
        index,
        field,
        value,
    };
    if !d.repeat_width.is_finite() || !(d.repeat_width > 0.0) {
        return Err(repeat("repeat_width", d.repeat_width));
    }
    if !d.repeat_height.is_finite() || !(d.repeat_height >= 0.0) {
        return Err(repeat("repeat_height", d.repeat_height));
    }
    Ok(())
}

/// Appends the region's alpha (top row first) and its column means to `out`; returns the mean.
fn read_region(s: &Sheet<'_>, d: &PatternDesc, out: &mut Vec<u8>) -> f32 {
    let (w, h) = (d.width as usize, d.height as usize);
    let bpp = s.format.bytes_per_pixel();
    let start = out.len();
    out.resize(start + w * (h + 1), 255);
    let mut total = 0u64;
    for x in 0..w {
        let mut column = 0u32;
        for y in 0..h {
            let at = (d.y as usize + y) * s.pitch + (d.x as usize + x) * bpp;
            let a = s.format.alpha_offset().map_or(255, |o| s.pixels[at + o]);
            out[start + y * w + x] = a;
            column += u32::from(a);
        }
        // Round to nearest, halves up.
        out[start + w * h + x] = ((column + h as u32 / 2) / h as u32) as u8;
        total += u64::from(column);
    }
    (total as f64 / (255.0 * (w * h) as f64)) as f32
}

// ---- snapshots (ADR 0010) ----

impl SnapshotValue for Pattern {
    fn put(&self, w: &mut Writer<'_>) {
        for v in [self.atlas_x, self.atlas_y, self.width, self.height] {
            w.put(&v);
        }
        for v in [self.repeat_width, self.repeat_height, self.mean] {
            w.put(&v);
        }
        w.put(&self.start);
    }

    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Self {
            atlas_x: r.get()?,
            atlas_y: r.get()?,
            width: r.get()?,
            height: r.get()?,
            repeat_width: r.get()?,
            repeat_height: r.get()?,
            mean: r.get()?,
            start: r.get()?,
        })
    }
}

impl Snapshot for PatternStore {
    const ID: &'static str = "knorke.light2d.patterns";
    const VERSION: u16 = 1;

    /// The slots, the texel arena, then the packer (free rows, shelves in order: the order decides
    /// where the next pattern goes).
    fn save(&self, w: &mut Writer<'_>) {
        w.put(&self.patterns);
        w.put_len(self.texels.len());
        w.put_bytes(&self.texels);
        for word in &self.packer.rows {
            w.put(word);
        }
        w.put_len(self.packer.shelves.len());
        for s in &self.packer.shelves {
            for v in [s.y, s.height, s.used] {
                w.put(&v);
            }
            for word in &s.free {
                w.put(word);
            }
        }
    }

    fn restore(&mut self, version: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        crate::check_version::<Self>(version)?;
        self.version += 1;
        let result = self.restore_state(r);
        if result.is_err() {
            *self = Self {
                version: self.version,
                ..Self::default()
            };
        }
        result
    }
}
