// Origin: chiaro @ 9bc5b9b, src/lighting/PixelCodec.hpp and PixelCodec.cpp, plus the static inline
// pixel encoders of include/Chiaro/Chiaro.h and Chiaro_bytes.h (CHI_MaterialPixel, CHI_SurfacePixel,
// CHI_EncodeMediumPixel, CHI_MATERIAL_B).
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! CPU encoders of the surface, material and medium buffers (P6).
//!
//! The byte layout is the contract with the shaders and must not change: rounding of the 8-bit
//! codecs is bit-exact with the C++ library (the tests keep a line-by-line port of the C++
//! reference encoder).
//!
//! Redesign against the C++ version: the C ABI (`CHI_*` structs, raw pointers, SDL error string)
//! is gone. A pixel is `[u8; 4]` in RGBA memory order (`u32::from_le_bytes` gives the
//! `SDL_PIXELFORMAT_RGBA32` value on every CPU); a missing normal is `Option<Vec3>`; [`Material`]
//! carries `metal` and `wet` as booleans instead of flag bits; normal maps are converted from
//! borrowed slices into a host buffer ([`convert_normal_map`]) and errors are typed
//! ([`NormalMapError`]) with stable diagnostic codes. Unknown flag bits and unknown presets
//! cannot occur any more.

use knorke_math::{Vec3, det};
use knorke_types::{Diagnostic, DiagnosticCode};

/// Both bytes of the normal (0, 0, 1): `(NORMAL_FLAT, NORMAL_FLAT)` decodes to exactly up.
pub const NORMAL_FLAT: u8 = 128;
/// A of a wet material entry written by the encoders (any 192..=254 is wet).
pub const MATERIAL_A_WET: u8 = 224;
/// Bits 7..5 of the material B byte: specular step 0..=7 (0 = matte).
pub const MATERIAL_SPECULAR_SHIFT: u32 = 5;
/// The specular bits of the material B byte.
pub const MATERIAL_SPECULAR_MASK: u8 = 0xE0;
/// Bit 4 of the material B byte: metal.
pub const MATERIAL_METAL_BIT: u8 = 0x10;
/// Bits 3..0 of the material B byte: gloss step 0..=15, Blinn-Phong exponent `2^(1 + gloss * 7/15)`.
pub const MATERIAL_GLOSS_MASK: u8 = 0x0F;
/// Number of entries of the media table; the kind of medium is below this.
pub const MEDIUM_COUNT: u32 = 16;

/// The B byte of a material pixel: specular step, metal, gloss step (out-of-range bits are masked off).
#[inline]
pub fn material_b(specular: u32, metal: bool, gloss: u32) -> u8 {
    // The masks keep every term within a byte.
    (((specular & 7) << MATERIAL_SPECULAR_SHIFT) as u8)
        | if metal { MATERIAL_METAL_BIT } else { 0 }
        | ((gloss as u8) & MATERIAL_GLOSS_MASK)
}

/// A value in `[0, 1]` as a step `0..=steps`, round half up; NaN is 0.
#[inline]
pub fn quantize_unit(v: f32, steps: u32) -> u32 {
    if !(v > 0.0) {
        return 0;
    }
    if v >= 1.0 {
        return steps;
    }
    (v * steps as f32 + 0.5) as u32
}

/// The shared normal codec into two bytes. Zero, NaN or infinite input is "up".
fn normal_bytes(x: f32, y: f32, z: f32) -> [u8; 2] {
    const FLAT: [u8; 2] = [NORMAL_FLAT, NORMAL_FLAT];
    let len = (x * x + y * y + z * z).sqrt();
    if !(len > 0.0) || !len.is_finite() {
        return FLAT;
    }
    let (x, y, z) = (x / len, y / len, det::max(z / len, 0.0));
    let sum = x.abs() + y.abs() + z;
    if !(sum > 0.0) {
        return FLAT; // straight down: no horizon direction to clamp to
    }
    let (px, py) = (x / sum, y / sum);
    let byte = |q: f32| ((q * 127.0).round() as i32 + 128).clamp(1, 255) as u8;
    [byte(px + py), byte(px - py)]
}

/// Encodes a normal into the two bytes of the shared normal codec. `None` is the up normal;
/// the input is normalized here and a zero or non-finite vector is up as well.
pub fn encode_normal(normal: Option<Vec3>) -> [u8; 2] {
    match normal {
        Some(n) => normal_bytes(n.x, n.y, n.z),
        None => [NORMAL_FLAT, NORMAL_FLAT],
    }
}

/// A normal map byte as a component in `[-1, 1]`, centred on 128 like the codec:
/// `(128, 128, 255)` is exactly up.
fn unit(b: u8) -> f32 {
    ((f32::from(b) - 128.0) / 127.0).clamp(-1.0, 1.0)
}

/// Decodes two normal bytes back to a unit normal.
pub fn decode_normal(x: u8, y: u8) -> Vec3 {
    let (qx, qy) = (unit(x), unit(y));
    let (px, py) = ((qx + qy) * 0.5, (qx - qy) * 0.5);
    let pz = 1.0 - px.abs() - py.abs();
    let len = (px * px + py * py + pz * pz).sqrt();
    Vec3::new(px / len, py / len, pz / len)
}

/// `round(height / step)` clamped to `0..=255`; NaN is 0.
fn height_byte(height: f32, height_step: f32) -> u8 {
    let q = (height / height_step).round();
    let steps = if q > 0.0 {
        if q < 255.0 { q } else { 255.0 }
    } else {
        0.0
    };
    steps as u8
}

/// One surface-buffer pixel: R the height in steps, G and B the world-space normal, A 255.
/// `height` is rounded to whole steps and clamped to 0..=255 steps; a zero normal is up.
pub fn surface_pixel(height: f32, height_step: f32, normal: Vec3) -> [u8; 4] {
    let n = normal_bytes(normal.x, normal.y, normal.z);
    [height_byte(height, height_step), n[0], n[1], 255]
}

/// [`surface_pixel`] as a vertex colour (bytes / 255). Height is linear across a planar face,
/// so one colour per vertex encodes a wall or a roof when drawn into an UNORM target.
pub fn surface_color(height: f32, height_step: f32, normal: Vec3) -> [f32; 4] {
    let n = normal_bytes(normal.x, normal.y, normal.z);
    [
        f32::from(height_byte(height, height_step)) / 255.0,
        f32::from(n[0]) / 255.0,
        f32::from(n[1]) / 255.0,
        1.0,
    ]
}

/// One material-buffer pixel from integer steps (`specular` 0..=7, `gloss` 0..=15, both clamped).
/// `normal` is the detail normal in tangent space; `None` is flat.
pub fn encode_material_pixel(
    normal: Option<Vec3>,
    specular: i32,
    metal: bool,
    gloss: i32,
) -> [u8; 4] {
    let n = encode_normal(normal);
    let b = material_b(
        specular.clamp(0, 7) as u32,
        metal,
        gloss.clamp(0, 15) as u32,
    );
    [n[0], n[1], b, 255]
}

/// A surface material: highlight strength and sharpness, quantized to 8 specular and 16 gloss
/// steps when encoded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Highlight strength in `[0, 1]`; 0 is matte.
    pub specular: f32,
    /// `[0, 1]`: 0 broad (Blinn-Phong exponent 2) to 1 mirror-sharp (256).
    pub gloss: f32,
    /// The highlight takes the scene colour instead of the light colour.
    pub metal: bool,
    /// Wet (paving, puddles): only wet pixels get the streak; encoded in the material A byte.
    pub wet: bool,
}

/// Material presets; (specular, gloss) in the comments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MaterialPreset {
    /// 0, 0: cloth, earth, plaster.
    Matte,
    /// 0.3, 0.4: painted wood, skin.
    Satin,
    /// 0.6, 0.6.
    Plastic,
    /// 0.9, 0.95: windows.
    Glass,
    /// 0.6, 0.5, metal.
    MetalDull,
    /// 1, 0.9, metal.
    MetalPolished,
    /// 0.15, 0.3.
    AsphaltDry,
    /// 0.55, 1, wet (bytes: specular 4, gloss 15).
    AsphaltWet,
    /// 1, 1, wet.
    Puddle,
}

impl MaterialPreset {
    /// All presets in declaration order.
    pub const ALL: [Self; 9] = [
        Self::Matte,
        Self::Satin,
        Self::Plastic,
        Self::Glass,
        Self::MetalDull,
        Self::MetalPolished,
        Self::AsphaltDry,
        Self::AsphaltWet,
        Self::Puddle,
    ];
}

const fn mat(specular: f32, gloss: f32, metal: bool, wet: bool) -> Material {
    Material {
        specular,
        gloss,
        metal,
        wet,
    }
}

impl Material {
    /// Cloth, earth, plaster.
    pub const MATTE: Self = mat(0.0, 0.0, false, false);
    /// Painted wood, skin.
    pub const SATIN: Self = mat(0.3, 0.4, false, false);
    /// Plastic.
    pub const PLASTIC: Self = mat(0.6, 0.6, false, false);
    /// Windows.
    pub const GLASS: Self = mat(0.9, 0.95, false, false);
    /// Dull metal.
    pub const METAL_DULL: Self = mat(0.6, 0.5, true, false);
    /// Polished metal.
    pub const METAL_POLISHED: Self = mat(1.0, 0.9, true, false);
    /// Dry asphalt.
    pub const ASPHALT_DRY: Self = mat(0.15, 0.3, false, false);
    /// Wet asphalt: step 4, tuned for the energy normalization (E49, Q13).
    pub const ASPHALT_WET: Self = mat(0.55, 1.0, false, true);
    /// Puddle.
    pub const PUDDLE: Self = mat(1.0, 1.0, false, true);

    /// The material of a preset.
    pub const fn preset(preset: MaterialPreset) -> Self {
        match preset {
            MaterialPreset::Matte => Self::MATTE,
            MaterialPreset::Satin => Self::SATIN,
            MaterialPreset::Plastic => Self::PLASTIC,
            MaterialPreset::Glass => Self::GLASS,
            MaterialPreset::MetalDull => Self::METAL_DULL,
            MaterialPreset::MetalPolished => Self::METAL_POLISHED,
            MaterialPreset::AsphaltDry => Self::ASPHALT_DRY,
            MaterialPreset::AsphaltWet => Self::ASPHALT_WET,
            MaterialPreset::Puddle => Self::PUDDLE,
        }
    }

    /// The B byte of a material pixel: quantized specular and gloss plus the metal bit.
    pub fn byte_b(&self) -> u8 {
        material_b(
            quantize_unit(self.specular, 7),
            self.metal,
            quantize_unit(self.gloss, 15),
        )
    }

    /// The A byte of a material pixel: 224 wet, 255 dry.
    pub fn byte_a(&self) -> u8 {
        if self.wet { MATERIAL_A_WET } else { 255 }
    }
}

impl Default for Material {
    fn default() -> Self {
        Self::MATTE
    }
}

/// One material-buffer pixel: R and G the detail normal, B the quantized material, A 255 (dry)
/// or 224 (wet). `normal` is in tangent space (x screen-right, y screen-up, z out of the
/// surface), normalized here; `None` is flat.
pub fn material_pixel(material: Material, normal: Option<Vec3>) -> [u8; 4] {
    let n = encode_normal(normal);
    [n[0], n[1], material.byte_b(), material.byte_a()]
}

/// [`material_pixel`] with a flat normal as a vertex colour (bytes / 255).
pub fn material_color(material: Material) -> [f32; 4] {
    [
        128.0 / 255.0,
        128.0 / 255.0,
        f32::from(material.byte_b()) / 255.0,
        f32::from(material.byte_a()) / 255.0,
    ]
}

/// The kind of medium is not below [`MEDIUM_COUNT`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediumOutOfRange(pub u32);

impl core::fmt::Display for MediumOutOfRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "medium {} must be below MEDIUM_COUNT ({MEDIUM_COUNT})",
            self.0
        )
    }
}

impl std::error::Error for MediumOutOfRange {}

impl MediumOutOfRange {
    /// As a diagnostic with a stable code.
    pub fn to_diagnostic(&self) -> Diagnostic {
        Diagnostic::error(
            DiagnosticCode::new("knorke.light2d.medium.out_of_range"),
            self.to_string(),
        )
    }
}

/// One medium-buffer pixel: R the top edge, G `255 - bottom edge` (both in steps of
/// `height_step`), B the kind of medium times 16, A the density.
///
/// `density` is clamped to `[0, 1]` (NaN is 0); the edges are rounded and clamped like heights.
///
/// # Errors
/// [`MediumOutOfRange`] if `medium >= MEDIUM_COUNT`.
pub fn encode_medium_pixel(
    density: f32,
    bottom: f32,
    top: f32,
    medium: u32,
    height_step: f32,
) -> Result<[u8; 4], MediumOutOfRange> {
    if medium >= MEDIUM_COUNT {
        return Err(MediumOutOfRange(medium));
    }
    Ok([
        height_byte(top, height_step),
        255 - height_byte(bottom, height_step),
        (medium * 16) as u8,
        quantize_unit(density, 255) as u8,
    ])
}

/// How [`convert_normal_map`] reads a normal map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NormalMapFlags {
    /// The normal map has y pointing down (DirectX style).
    pub green_down: bool,
    /// Mirror the pixels horizontally and negate x.
    pub flip_x: bool,
    /// Mirror the pixels vertically and negate y.
    pub flip_y: bool,
}

/// An RGBA image read for its alpha channel only.
#[derive(Clone, Copy, Debug)]
pub struct AlphaSource<'a> {
    /// `width x height` RGBA pixels in source orientation (e.g. the colour sprite).
    pub rgba: &'a [u8],
    /// Bytes per row, at least `width * 4`.
    pub pitch: usize,
}

/// How [`convert_normal_map`] turns a normal map into material pixels. The maps hold one byte
/// per pixel, `width * height`, tightly packed, in source orientation like the normal map.
#[derive(Clone, Copy, Debug, Default)]
pub struct NormalMapOptions<'a> {
    /// Orientation conventions.
    pub flags: NormalMapFlags,
    /// Constant material (default matte).
    pub material: Material,
    /// 0..=255 mapped linearly to the 8 specular steps; replaces `material.specular`.
    pub specular_map: Option<&'a [u8]>,
    /// 0..=255 mapped linearly to the 16 gloss steps; replaces `material.gloss`.
    pub gloss_map: Option<&'a [u8]>,
    /// Metal from 128 upwards; replaces `material.metal`.
    pub metal_map: Option<&'a [u8]>,
    /// Alpha source; `None` takes the alpha of the normal map.
    pub alpha: Option<AlphaSource<'a>>,
}

/// Why [`convert_normal_map`] refused its input. Nothing is written in that case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NormalMapError {
    /// A side is 0 or above 65536.
    InvalidSize {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
    /// A row of the normal map is longer than its pitch.
    PitchTooSmall {
        /// The given pitch.
        pitch: usize,
        /// `width * 4`.
        min: usize,
    },
    /// A row of the alpha source is longer than its pitch.
    AlphaPitchTooSmall {
        /// The given pitch.
        pitch: usize,
        /// `width * 4`.
        min: usize,
    },
    /// A buffer is shorter than the image needs.
    BufferTooSmall {
        /// Which buffer.
        what: &'static str,
        /// Bytes needed.
        needed: usize,
        /// Bytes given.
        got: usize,
    },
}

impl core::fmt::Display for NormalMapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidSize { width, height } => write!(
                f,
                "normal map size {width}x{height} is invalid; both sides must be in 1..=65536"
            ),
            Self::PitchTooSmall { pitch, min } => {
                write!(f, "normal map pitch {pitch} is smaller than w * 4 = {min}")
            }
            Self::AlphaPitchTooSmall { pitch, min } => write!(
                f,
                "normal map alpha pitch {pitch} is smaller than w * 4 = {min}"
            ),
            Self::BufferTooSmall { what, needed, got } => {
                write!(f, "{what} has {got} bytes but the image needs {needed}")
            }
        }
    }
}

impl std::error::Error for NormalMapError {}

impl NormalMapError {
    /// As a diagnostic with a stable code.
    pub fn to_diagnostic(&self) -> Diagnostic {
        let code = match self {
            Self::InvalidSize { .. } => DiagnosticCode::new("knorke.light2d.normal_map.size"),
            Self::PitchTooSmall { .. } | Self::AlphaPitchTooSmall { .. } => {
                DiagnosticCode::new("knorke.light2d.normal_map.pitch")
            }
            Self::BufferTooSmall { .. } => DiagnosticCode::new("knorke.light2d.normal_map.buffer"),
        };
        Diagnostic::error(code, self.to_string())
    }
}

/// Converts an ordinary RGBA normal map (`rgb / 255 * 2 - 1`, bytes centred on 128, so
/// `(128, 128, 255)` is exactly flat) into material-buffer pixels ([`material_pixel`]).
///
/// Pixels whose alpha (from `options.alpha` or the normal map) is below 128 become
/// `(0, 0, 0, 0)`, all others get A 255 (224 for a wet material), so sprite edges are never
/// blended. Flips apply to the output; the inputs stay in source orientation. `out` receives
/// `width * height * 4` tightly packed bytes.
///
/// # Errors
/// [`NormalMapError`] for a bad size, pitch or too short a buffer; `out` is not touched then.
pub fn convert_normal_map(
    normal_rgba: &[u8],
    width: u32,
    height: u32,
    pitch: usize,
    options: &NormalMapOptions<'_>,
    out: &mut [u8],
) -> Result<(), NormalMapError> {
    if !(1..=65_536).contains(&width) || !(1..=65_536).contains(&height) {
        return Err(NormalMapError::InvalidSize { width, height });
    }
    let (w, h) = (width as usize, height as usize);
    let row_bytes = w * 4;
    if pitch < row_bytes {
        return Err(NormalMapError::PitchTooSmall {
            pitch,
            min: row_bytes,
        });
    }
    if let Some(a) = &options.alpha
        && a.pitch < row_bytes
    {
        return Err(NormalMapError::AlphaPitchTooSmall {
            pitch: a.pitch,
            min: row_bytes,
        });
    }
    let check = |what: &'static str, got: usize, needed: usize| {
        if got < needed {
            Err(NormalMapError::BufferTooSmall { what, needed, got })
        } else {
            Ok(())
        }
    };
    check(
        "normal_rgba",
        normal_rgba.len(),
        (h - 1) * pitch + row_bytes,
    )?;
    if let Some(a) = &options.alpha {
        check("alpha.rgba", a.rgba.len(), (h - 1) * a.pitch + row_bytes)?;
    }
    for (what, map) in [
        ("specular_map", options.specular_map),
        ("gloss_map", options.gloss_map),
        ("metal_map", options.metal_map),
    ] {
        if let Some(m) = map {
            check(what, m.len(), w * h)?;
        }
    }
    check("out_rgba", out.len(), row_bytes * h)?;

    let flags = options.flags;
    let const_specular = quantize_unit(options.material.specular, 7) as i32;
    let const_gloss = quantize_unit(options.material.gloss, 15) as i32;
    let entry_alpha = options.material.byte_a(); // E48
    for y in 0..h {
        let sy = if flags.flip_y { h - 1 - y } else { y };
        for x in 0..w {
            let sx = if flags.flip_x { w - 1 - x } else { x };
            let dst = &mut out[(y * w + x) * 4..][..4];
            let src = &normal_rgba[sy * pitch + sx * 4..][..4];
            let alpha = match &options.alpha {
                Some(a) => a.rgba[sy * a.pitch + sx * 4 + 3],
                None => src[3],
            };
            if alpha < 128 {
                dst.copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            let mut n = Vec3::new(unit(src[0]), unit(src[1]), unit(src[2]));
            if flags.green_down {
                n.y = -n.y;
            }
            if flags.flip_x {
                n.x = -n.x;
            }
            if flags.flip_y {
                n.y = -n.y;
            }
            let mi = sy * w + sx;
            let specular = options
                .specular_map
                .map_or(const_specular, |m| (i32::from(m[mi]) * 7 + 127) / 255);
            let metal = options
                .metal_map
                .map_or(options.material.metal, |m| m[mi] >= 128);
            let gloss = options
                .gloss_map
                .map_or(const_gloss, |m| (i32::from(m[mi]) * 15 + 127) / 255);
            let mut px = encode_material_pixel(Some(n), specular, metal, gloss);
            px[3] = entry_alpha;
            dst.copy_from_slice(&px);
        }
    }
    Ok(())
}
