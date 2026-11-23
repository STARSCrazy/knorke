//! The shader pack format `.kpak` (ADR 0014 §3): compiled shaders of one crate in one
//! backend format, with their reflection, in one data file.
//!
//! A game ships only the pack for its backend (`knorke-light2d.spirv.kpak` for Vulkan,
//! `….msl.kpak` for Metal, `….dxil.kpak` for D3D12). `cargo xtask shaders` writes packs with
//! [`write_pack`]; [`ShaderPack::parse`] reads them without copying from a borrowed byte
//! slice. Bad input never panics: every problem is a [`Diagnostic`] with a
//! `knorke.gpu.shader_pack.*` code.
//!
//! # Layout (format version 1, all integers little-endian)
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 8 | magic `KNRKPAK\0` |
//! | 8 | 2 | format version ([`FORMAT_VERSION`]) |
//! | 10 | 2 | shader format ([`ShaderFormat`]: 1 SPIR-V, 2 MSL, 3 DXIL) |
//! | 12 | 4 | program count `n` |
//! | 16 | 8 | source checksum (FNV-1a 64 of the shader sources, see [`source_checksum`]) |
//! | 24 | 8 | content checksum (FNV-1a 64 of the whole file with this field read as zero) |
//! | 32 | 4 | string table offset |
//! | 36 | 4 | string table length |
//! | 40 | 36·n | program table, sorted by name (strictly ascending bytes) |
//! | … | | string table (UTF-8 names and entry points, not NUL-terminated) |
//! | … | | code blobs, each starting 4-byte aligned, zero padding between |
//!
//! Program table entry (36 bytes):
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | name offset (into the string table) |
//! | 4 | 4 | name length (non-zero) |
//! | 8 | 4 | entry point offset (into the string table) |
//! | 12 | 4 | entry point length (non-zero) |
//! | 16 | 1 | stage ([`Stage`]: 0 vertex, 1 fragment) |
//! | 17 | 3 | reserved, zero |
//! | 20 | 2 | samplers |
//! | 22 | 2 | storage textures |
//! | 24 | 2 | storage buffers |
//! | 26 | 2 | uniform buffers |
//! | 28 | 4 | code offset (from the start of the file) |
//! | 32 | 4 | code length (non-zero) |
//!
//! Names are the shader file names without format extension (`composite.frag`). MSL blobs are
//! followed by at least one zero byte (not counted in the length), so they are C strings.

use knorke_types::{Diagnostic, DiagnosticCode};

/// The file does not start with [`MAGIC`].
pub const SHADER_PACK_BAD_MAGIC: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.shader_pack.bad_magic");
/// The format version is not [`FORMAT_VERSION`].
pub const SHADER_PACK_VERSION: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.shader_pack.version");
/// The shader format field is not a known [`ShaderFormat`].
pub const SHADER_PACK_UNKNOWN_FORMAT: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.shader_pack.unknown_format");
/// The file is shorter than its header or tables say.
pub const SHADER_PACK_TRUNCATED: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.shader_pack.truncated");
/// The content checksum does not match: the file was damaged.
pub const SHADER_PACK_CHECKSUM: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.shader_pack.checksum");
/// A table entry is inconsistent (range out of bounds, bad UTF-8, unsorted names, …).
pub const SHADER_PACK_CORRUPT: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.shader_pack.corrupt");
/// The pack was built from other shader sources than the caller expects.
pub const SHADER_PACK_SOURCE_MISMATCH: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.shader_pack.source_mismatch");
/// A program the caller needs is not in the pack.
pub const SHADER_PACK_MISSING_PROGRAM: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.shader_pack.missing_program");
/// The device cannot use the pack's shader format.
pub const SHADER_PACK_UNSUPPORTED_FORMAT: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.shader_pack.unsupported_format");
/// Reading the pack file failed.
pub const SHADER_PACK_IO: DiagnosticCode = DiagnosticCode::new("knorke.gpu.shader_pack.io");
/// [`write_pack`] got input it cannot encode.
pub const SHADER_PACK_INVALID_INPUT: DiagnosticCode =
    DiagnosticCode::new("knorke.gpu.shader_pack.invalid_input");

/// First eight bytes of every pack.
pub const MAGIC: [u8; 8] = *b"KNRKPAK\0";
/// The format version this crate reads and writes.
pub const FORMAT_VERSION: u16 = 1;
/// Size of the file header in bytes.
pub const HEADER_LEN: usize = 40;
/// Size of one program table entry in bytes.
pub const ENTRY_LEN: usize = 36;

const CHECKSUM_AT: usize = 24;

/// Backend shader format of a pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShaderFormat {
    /// SPIR-V for Vulkan (default on Windows and Linux).
    Spirv = 1,
    /// Metal Shading Language source (macOS, iOS).
    Msl = 2,
    /// DXIL for D3D12 (optional).
    Dxil = 3,
}

impl ShaderFormat {
    /// All formats, in the order of preference when a device accepts several (ADR 0014).
    pub const ALL: [Self; 3] = [Self::Spirv, Self::Msl, Self::Dxil];

    /// Decodes the header field.
    pub fn from_u16(v: u16) -> Option<Self> {
        match v {
            1 => Some(Self::Spirv),
            2 => Some(Self::Msl),
            3 => Some(Self::Dxil),
            _ => None,
        }
    }

    /// The part of the file name that names the format: `spirv`, `msl`, `dxil`.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Spirv => "spirv",
            Self::Msl => "msl",
            Self::Dxil => "dxil",
        }
    }

    /// Extension of the compiled blobs shadercross writes (`spv`, `msl`, `dxil`).
    pub fn blob_extension(self) -> &'static str {
        match self {
            Self::Spirv => "spv",
            Self::Msl => "msl",
            Self::Dxil => "dxil",
        }
    }

    /// The pack file name for a crate: `knorke-light2d` → `knorke-light2d.spirv.kpak`.
    pub fn pack_file_name(self, pack: &str) -> String {
        format!("{pack}.{}.kpak", self.tag())
    }

    /// Default entry point: SPIRV-Cross renames `main` to `main0` when it emits MSL.
    pub fn default_entry_point(self) -> &'static str {
        match self {
            Self::Msl => "main0",
            Self::Spirv | Self::Dxil => "main",
        }
    }
}

/// Pipeline stage of a program.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {
    /// Vertex shader.
    Vertex = 0,
    /// Fragment shader.
    Fragment = 1,
}

impl Stage {
    fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Vertex),
            1 => Some(Self::Fragment),
            _ => None,
        }
    }
}

/// Resource counts of one shader, from shadercross reflection. SDL_GPU needs them to create
/// the shader, and [`crate`] users check their bindings against them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Reflection {
    /// Texture-sampler pairs.
    pub samplers: u16,
    /// Storage textures.
    pub storage_textures: u16,
    /// Storage buffers.
    pub storage_buffers: u16,
    /// Uniform buffers.
    pub uniform_buffers: u16,
}

/// One program of a pack, borrowed from the pack bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Program<'a> {
    /// Name, e.g. `composite.frag`.
    pub name: &'a str,
    /// Pipeline stage.
    pub stage: Stage,
    /// Entry point function name.
    pub entry_point: &'a str,
    /// Resource counts.
    pub reflection: Reflection,
    /// The compiled code (or MSL source text).
    pub code: &'a [u8],
}

/// A validated, borrowed view of a pack.
#[derive(Clone, Copy, Debug)]
pub struct ShaderPack<'a> {
    bytes: &'a [u8],
    format: ShaderFormat,
    count: usize,
    source_checksum: u64,
    strings: (usize, usize),
}

fn corrupt(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::error(SHADER_PACK_CORRUPT, msg)
}

fn read_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn read_u64(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        b.get(at..at.checked_add(8)?)?.try_into().ok()?,
    ))
}

/// FNV-1a 64-bit hash, continued from `state` (start with [`FNV_OFFSET`]).
pub fn fnv1a64(mut state: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        state ^= u64::from(b);
        state = state.wrapping_mul(0x0000_0100_0000_01b3);
    }
    state
}

/// Start value of [`fnv1a64`].
pub const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

/// Content checksum of a pack: the whole file with the checksum field read as zero.
fn content_checksum(bytes: &[u8]) -> u64 {
    let h = fnv1a64(FNV_OFFSET, &bytes[..CHECKSUM_AT]);
    let h = fnv1a64(h, &[0; 8]);
    fnv1a64(h, &bytes[CHECKSUM_AT + 8..])
}

/// Source checksum over named source files: name and content of each, length-prefixed so
/// that moving bytes between files changes the result. `files` must be sorted by name.
pub fn source_checksum<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> u64 {
    let mut h = FNV_OFFSET;
    for (name, content) in files {
        h = fnv1a64(h, &(name.len() as u64).to_le_bytes());
        h = fnv1a64(h, name.as_bytes());
        h = fnv1a64(h, &(content.len() as u64).to_le_bytes());
        h = fnv1a64(h, content);
    }
    h
}

impl<'a> ShaderPack<'a> {
    /// Validates `bytes` completely (header, checksum, every table entry) and returns a view.
    ///
    /// Never panics. Errors: [`SHADER_PACK_TRUNCATED`], [`SHADER_PACK_BAD_MAGIC`],
    /// [`SHADER_PACK_VERSION`], [`SHADER_PACK_UNKNOWN_FORMAT`], [`SHADER_PACK_CHECKSUM`],
    /// [`SHADER_PACK_CORRUPT`].
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Diagnostic> {
        let pack = Self::parse_header(bytes)?;
        let stored = read_u64(bytes, CHECKSUM_AT).unwrap_or(0);
        let actual = content_checksum(bytes);
        if stored != actual {
            return Err(Diagnostic::error(
                SHADER_PACK_CHECKSUM,
                format!(
                    "shader pack checksum {stored:#018x} does not match content {actual:#018x}"
                ),
            ));
        }
        let mut previous: Option<&str> = None;
        for i in 0..pack.count {
            let p = pack.decode(i)?;
            if let Some(prev) = previous
                && prev.as_bytes() >= p.name.as_bytes()
            {
                return Err(corrupt(format!(
                    "program names not strictly ascending: '{prev}' before '{}'",
                    p.name
                )));
            }
            previous = Some(p.name);
        }
        Ok(pack)
    }

    /// View of bytes that passed [`parse`](Self::parse) before: header only, no checksum
    /// pass. Entry accessors still check bounds, so other input cannot panic either.
    #[cfg_attr(not(feature = "sdl"), allow(dead_code))]
    pub(crate) fn parse_validated(bytes: &'a [u8]) -> Option<Self> {
        Self::parse_header(bytes).ok()
    }

    /// Header and table bounds only; no checksum, no entries.
    fn parse_header(bytes: &'a [u8]) -> Result<Self, Diagnostic> {
        if bytes.len() < HEADER_LEN {
            if bytes.len() >= MAGIC.len() && bytes[..MAGIC.len()] != MAGIC {
                return Err(bad_magic());
            }
            return Err(Diagnostic::error(
                SHADER_PACK_TRUNCATED,
                format!(
                    "shader pack has {} bytes, header needs {HEADER_LEN}",
                    bytes.len()
                ),
            ));
        }
        if bytes[..MAGIC.len()] != MAGIC {
            return Err(bad_magic());
        }
        let version = read_u16(bytes, 8).unwrap_or(0);
        if version != FORMAT_VERSION {
            return Err(Diagnostic::error(
                SHADER_PACK_VERSION,
                format!("shader pack format version {version}, this build reads {FORMAT_VERSION}"),
            ));
        }
        let raw_format = read_u16(bytes, 10).unwrap_or(0);
        let format = ShaderFormat::from_u16(raw_format).ok_or_else(|| {
            Diagnostic::error(
                SHADER_PACK_UNKNOWN_FORMAT,
                format!("unknown shader format {raw_format} in shader pack"),
            )
        })?;
        let count = read_u32(bytes, 12).unwrap_or(0) as usize;
        let table_end = count
            .checked_mul(ENTRY_LEN)
            .and_then(|t| t.checked_add(HEADER_LEN))
            .filter(|&end| end <= bytes.len())
            .ok_or_else(|| {
                Diagnostic::error(
                    SHADER_PACK_TRUNCATED,
                    format!(
                        "program table of {count} entries exceeds the file ({} bytes)",
                        bytes.len()
                    ),
                )
            })?;
        let s_off = read_u32(bytes, 32).unwrap_or(0) as usize;
        let s_len = read_u32(bytes, 36).unwrap_or(0) as usize;
        match s_off.checked_add(s_len) {
            Some(end) if end <= bytes.len() => {}
            _ => {
                return Err(Diagnostic::error(
                    SHADER_PACK_TRUNCATED,
                    "string table exceeds the file",
                ));
            }
        }
        if s_off < table_end {
            return Err(corrupt("string table overlaps the program table"));
        }
        Ok(Self {
            bytes,
            format,
            count,
            source_checksum: read_u64(bytes, 16).unwrap_or(0),
            strings: (s_off, s_len),
        })
    }

    fn string(&self, off: u32, len: u32, what: &str, index: usize) -> Result<&'a str, Diagnostic> {
        let (s_off, s_len) = self.strings;
        let (off, len) = (off as usize, len as usize);
        let bytes = off
            .checked_add(len)
            .filter(|&end| end <= s_len && len > 0)
            .and_then(|end| self.bytes.get(s_off + off..s_off + end))
            .ok_or_else(|| {
                corrupt(format!(
                    "program {index}: {what} outside the string table or empty"
                ))
            })?;
        core::str::from_utf8(bytes)
            .map_err(|_| corrupt(format!("program {index}: {what} is not UTF-8")))
    }

    fn decode(&self, index: usize) -> Result<Program<'a>, Diagnostic> {
        let at = HEADER_LEN + index * ENTRY_LEN;
        let e = self
            .bytes
            .get(at..at + ENTRY_LEN)
            .ok_or_else(|| corrupt(format!("program {index} outside the table")))?;
        let u32_at = |o: usize| read_u32(e, o).unwrap_or(0);
        let u16_at = |o: usize| read_u16(e, o).unwrap_or(0);
        let name = self.string(u32_at(0), u32_at(4), "name", index)?;
        let entry_point = self.string(u32_at(8), u32_at(12), "entry point", index)?;
        let stage = Stage::from_u8(e[16])
            .ok_or_else(|| corrupt(format!("program '{name}': unknown stage {}", e[16])))?;
        if e[17..20] != [0, 0, 0] {
            return Err(corrupt(format!(
                "program '{name}': reserved bytes not zero"
            )));
        }
        let reflection = Reflection {
            samplers: u16_at(20),
            storage_textures: u16_at(22),
            storage_buffers: u16_at(24),
            uniform_buffers: u16_at(26),
        };
        let (c_off, c_len) = (u32_at(28) as usize, u32_at(32) as usize);
        let code = c_off
            .checked_add(c_len)
            .filter(|_| c_len > 0 && c_off >= HEADER_LEN + self.count * ENTRY_LEN)
            .and_then(|end| self.bytes.get(c_off..end))
            .ok_or_else(|| corrupt(format!("program '{name}': code outside the file or empty")))?;
        Ok(Program {
            name,
            stage,
            entry_point,
            reflection,
            code,
        })
    }

    /// The backend format of every program in the pack.
    pub fn format(&self) -> ShaderFormat {
        self.format
    }

    /// Checksum of the shader sources the pack was built from.
    pub fn source_checksum(&self) -> u64 {
        self.source_checksum
    }

    /// Fails with [`SHADER_PACK_SOURCE_MISMATCH`] unless the pack was built from the sources
    /// with checksum `expected` (e.g. one a crate records at build time).
    pub fn expect_source_checksum(&self, expected: u64) -> Result<(), Diagnostic> {
        if self.source_checksum == expected {
            Ok(())
        } else {
            Err(Diagnostic::error(
                SHADER_PACK_SOURCE_MISMATCH,
                format!(
                    "shader pack built from sources {:#018x}, expected {expected:#018x}",
                    self.source_checksum
                ),
            ))
        }
    }

    /// Number of programs.
    pub fn len(&self) -> usize {
        self.count
    }

    /// Whether the pack has no programs.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The program at table position `index` (programs are sorted by name).
    pub fn program(&self, index: usize) -> Option<Program<'a>> {
        if index >= self.count {
            return None;
        }
        self.decode(index).ok()
    }

    /// Table position of the program called `name` (binary search, no allocation).
    pub fn find(&self, name: &str) -> Option<usize> {
        let (mut lo, mut hi) = (0, self.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let p = self.decode(mid).ok()?;
            match p.name.as_bytes().cmp(name.as_bytes()) {
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
                core::cmp::Ordering::Equal => return Some(mid),
            }
        }
        None
    }

    /// The program called `name`, or [`SHADER_PACK_MISSING_PROGRAM`].
    pub fn get(&self, name: &str) -> Result<Program<'a>, Diagnostic> {
        self.find(name)
            .and_then(|i| self.program(i))
            .ok_or_else(|| {
                Diagnostic::error(
                    SHADER_PACK_MISSING_PROGRAM,
                    format!("shader pack has no program '{name}'"),
                )
            })
    }

    /// All programs in name order.
    pub fn iter(&self) -> impl Iterator<Item = Program<'a>> + '_ {
        (0..self.count).filter_map(|i| self.program(i))
    }
}

fn bad_magic() -> Diagnostic {
    Diagnostic::error(SHADER_PACK_BAD_MAGIC, "not a shader pack (bad magic)")
}

/// Input of [`write_pack`]: one program with owned data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramSource {
    /// Name, e.g. `composite.frag`.
    pub name: String,
    /// Pipeline stage.
    pub stage: Stage,
    /// Entry point function name.
    pub entry_point: String,
    /// Resource counts.
    pub reflection: Reflection,
    /// Compiled code.
    pub code: Vec<u8>,
}

fn u32_len(n: usize, what: &str) -> Result<u32, Diagnostic> {
    u32::try_from(n)
        .map_err(|_| Diagnostic::error(SHADER_PACK_INVALID_INPUT, format!("{what} exceeds 4 GiB")))
}

/// Encodes a pack. Programs are sorted by name; duplicate or empty names, empty entry
/// points or empty code fail with [`SHADER_PACK_INVALID_INPUT`].
pub fn write_pack(
    format: ShaderFormat,
    source_checksum: u64,
    programs: &[ProgramSource],
) -> Result<Vec<u8>, Diagnostic> {
    let invalid = |m: String| Diagnostic::error(SHADER_PACK_INVALID_INPUT, m);
    let mut order: Vec<&ProgramSource> = programs.iter().collect();
    order.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    for w in order.windows(2) {
        if w[0].name == w[1].name {
            return Err(invalid(format!("duplicate program '{}'", w[0].name)));
        }
    }
    let mut strings = Vec::new();
    let mut spans = Vec::with_capacity(order.len());
    for p in &order {
        if p.name.is_empty() || p.entry_point.is_empty() || p.code.is_empty() {
            return Err(invalid(format!(
                "program '{}': name, entry point and code must be non-empty",
                p.name
            )));
        }
        let name_off = u32_len(strings.len(), "string table")?;
        strings.extend_from_slice(p.name.as_bytes());
        let entry_off = u32_len(strings.len(), "string table")?;
        strings.extend_from_slice(p.entry_point.as_bytes());
        spans.push((name_off, entry_off));
    }
    let table_end = HEADER_LEN + order.len() * ENTRY_LEN;
    let mut out = vec![0u8; table_end];
    let strings_off = out.len();
    out.extend_from_slice(&strings);

    out[..8].copy_from_slice(&MAGIC);
    out[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    out[10..12].copy_from_slice(&(format as u16).to_le_bytes());
    out[12..16].copy_from_slice(&u32_len(order.len(), "program count")?.to_le_bytes());
    out[16..24].copy_from_slice(&source_checksum.to_le_bytes());
    out[32..36].copy_from_slice(&u32_len(strings_off, "header")?.to_le_bytes());
    out[36..40].copy_from_slice(&u32_len(strings.len(), "string table")?.to_le_bytes());

    for (i, (p, (name_off, entry_off))) in order.iter().zip(spans).enumerate() {
        // MSL is source text: keep at least one zero byte after it.
        let pad = if format == ShaderFormat::Msl { 1 } else { 0 };
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
        let code_off = u32_len(out.len(), "pack")?;
        out.extend_from_slice(&p.code);
        out.resize(out.len() + pad, 0);
        let e = HEADER_LEN + i * ENTRY_LEN;
        let entry = &mut out[e..e + ENTRY_LEN];
        entry[0..4].copy_from_slice(&name_off.to_le_bytes());
        entry[4..8].copy_from_slice(&u32_len(p.name.len(), "name")?.to_le_bytes());
        entry[8..12].copy_from_slice(&entry_off.to_le_bytes());
        entry[12..16].copy_from_slice(&u32_len(p.entry_point.len(), "entry point")?.to_le_bytes());
        entry[16] = p.stage as u8;
        let r = p.reflection;
        entry[20..22].copy_from_slice(&r.samplers.to_le_bytes());
        entry[22..24].copy_from_slice(&r.storage_textures.to_le_bytes());
        entry[24..26].copy_from_slice(&r.storage_buffers.to_le_bytes());
        entry[26..28].copy_from_slice(&r.uniform_buffers.to_le_bytes());
        entry[28..32].copy_from_slice(&code_off.to_le_bytes());
        entry[32..36].copy_from_slice(&u32_len(p.code.len(), "code")?.to_le_bytes());
    }
    let sum = content_checksum(&out);
    out[CHECKSUM_AT..CHECKSUM_AT + 8].copy_from_slice(&sum.to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog(name: &str, stage: Stage, samplers: u16, code: &[u8]) -> ProgramSource {
        ProgramSource {
            name: name.into(),
            stage,
            entry_point: "main".into(),
            reflection: Reflection {
                samplers,
                storage_textures: 0,
                storage_buffers: 1,
                uniform_buffers: 2,
            },
            code: code.to_vec(),
        }
    }

    fn sample() -> Vec<u8> {
        write_pack(
            ShaderFormat::Spirv,
            0x1234,
            &[
                prog("tile.frag", Stage::Fragment, 1, &[9, 9, 9]),
                prog("fullscreen.vert", Stage::Vertex, 0, &[1, 2, 3, 4, 5]),
                prog("composite.frag", Stage::Fragment, 6, &[7; 17]),
            ],
        )
        .unwrap()
    }

    /// Recomputes the content checksum so that structural checks behind it are reached.
    fn forge(mut b: Vec<u8>) -> Vec<u8> {
        if b.len() >= HEADER_LEN {
            let sum = content_checksum(&b);
            b[CHECKSUM_AT..CHECKSUM_AT + 8].copy_from_slice(&sum.to_le_bytes());
        }
        b
    }

    fn code_of(bytes: &[u8]) -> DiagnosticCode {
        ShaderPack::parse(bytes).unwrap_err().code
    }

    #[test]
    fn round_trip_sorted_with_lookup() {
        let bytes = sample();
        let pack = ShaderPack::parse(&bytes).unwrap();
        assert_eq!(pack.format(), ShaderFormat::Spirv);
        assert_eq!(pack.source_checksum(), 0x1234);
        assert_eq!(pack.len(), 3);
        let names: Vec<_> = pack.iter().map(|p| p.name).collect();
        assert_eq!(names, ["composite.frag", "fullscreen.vert", "tile.frag"]);
        let c = pack.get("composite.frag").unwrap();
        assert_eq!(c.stage, Stage::Fragment);
        assert_eq!(c.reflection.samplers, 6);
        assert_eq!(c.reflection.storage_buffers, 1);
        assert_eq!(c.reflection.uniform_buffers, 2);
        assert_eq!(c.code, &[7; 17]);
        assert_eq!(c.entry_point, "main");
        assert_eq!(pack.get("fullscreen.vert").unwrap().code, &[1, 2, 3, 4, 5]);
        assert_eq!(pack.find("missing.frag"), None);
        assert_eq!(
            pack.get("missing.frag").unwrap_err().code,
            SHADER_PACK_MISSING_PROGRAM
        );
        assert!(pack.program(3).is_none());
        // Zero copy: the code is a slice of the input.
        let range = bytes.as_ptr_range();
        assert!(range.contains(&c.code.as_ptr()));
        assert_eq!(c.code.as_ptr() as usize % 4, bytes.as_ptr() as usize % 4);
    }

    #[test]
    fn source_checksum_check() {
        let bytes = sample();
        let pack = ShaderPack::parse(&bytes).unwrap();
        assert!(pack.expect_source_checksum(0x1234).is_ok());
        assert_eq!(
            pack.expect_source_checksum(1).unwrap_err().code,
            SHADER_PACK_SOURCE_MISMATCH
        );
        let a = source_checksum([("a.hlsl", &b"xy"[..]), ("b.hlsl", &b"z"[..])]);
        let b = source_checksum([("a.hlsl", &b"x"[..]), ("b.hlsl", &b"yz"[..])]);
        assert_ne!(a, b);
    }

    #[test]
    fn msl_blobs_are_nul_terminated() {
        let bytes = write_pack(
            ShaderFormat::Msl,
            0,
            &[prog("a.frag", Stage::Fragment, 0, b"fragment main0(")],
        )
        .unwrap();
        let pack = ShaderPack::parse(&bytes).unwrap();
        let code = pack.get("a.frag").unwrap().code;
        let end = code.as_ptr() as usize - bytes.as_ptr() as usize + code.len();
        assert_eq!(bytes[end], 0);
    }

    #[test]
    fn empty_pack_is_valid() {
        let bytes = write_pack(ShaderFormat::Dxil, 7, &[]).unwrap();
        let pack = ShaderPack::parse(&bytes).unwrap();
        assert!(pack.is_empty());
        assert_eq!(pack.find("x"), None);
    }

    #[test]
    fn writer_rejects_bad_input() {
        let dup = [
            prog("a.frag", Stage::Fragment, 0, &[1]),
            prog("a.frag", Stage::Fragment, 0, &[2]),
        ];
        assert_eq!(
            write_pack(ShaderFormat::Spirv, 0, &dup).unwrap_err().code,
            SHADER_PACK_INVALID_INPUT
        );
        let empty = [prog("a.frag", Stage::Fragment, 0, &[])];
        assert_eq!(
            write_pack(ShaderFormat::Spirv, 0, &empty).unwrap_err().code,
            SHADER_PACK_INVALID_INPUT
        );
    }

    #[test]
    fn header_errors() {
        let good = sample();
        assert_eq!(code_of(&[]), SHADER_PACK_TRUNCATED);
        assert_eq!(code_of(&good[..10]), SHADER_PACK_TRUNCATED);
        assert_eq!(code_of(b"NOTAPACKATALL"), SHADER_PACK_BAD_MAGIC);

        let mut b = good.clone();
        b[0] = b'X';
        assert_eq!(code_of(&b), SHADER_PACK_BAD_MAGIC);

        let mut b = good.clone();
        b[8] = 2;
        assert_eq!(code_of(&forge(b)), SHADER_PACK_VERSION);

        let mut b = good.clone();
        b[10] = 9;
        assert_eq!(code_of(&forge(b)), SHADER_PACK_UNKNOWN_FORMAT);

        let mut b = good.clone();
        b[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(code_of(&forge(b)), SHADER_PACK_TRUNCATED);

        let mut b = good.clone();
        b[36..40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(code_of(&forge(b)), SHADER_PACK_TRUNCATED);

        let mut b = good.clone();
        b[32..36].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(code_of(&forge(b)), SHADER_PACK_CORRUPT);
    }

    #[test]
    fn damaged_content_fails_the_checksum() {
        let good = sample();
        let mut b = good.clone();
        let last = b.len() - 1;
        b[last] ^= 0x40;
        assert_eq!(code_of(&b), SHADER_PACK_CHECKSUM);
    }

    #[test]
    fn entry_errors_behind_a_valid_checksum() {
        let good = sample();
        let e = HEADER_LEN; // first entry
        let set = |off: usize, v: &[u8]| {
            let mut b = good.clone();
            b[off..off + v.len()].copy_from_slice(v);
            code_of(&forge(b))
        };
        assert_eq!(set(e, &u32::MAX.to_le_bytes()), SHADER_PACK_CORRUPT); // name off
        assert_eq!(set(e + 4, &0u32.to_le_bytes()), SHADER_PACK_CORRUPT); // empty name
        assert_eq!(set(e + 12, &999u32.to_le_bytes()), SHADER_PACK_CORRUPT); // entry len
        assert_eq!(set(e + 16, &[7]), SHADER_PACK_CORRUPT); // stage
        assert_eq!(set(e + 18, &[1]), SHADER_PACK_CORRUPT); // reserved
        assert_eq!(set(e + 28, &u32::MAX.to_le_bytes()), SHADER_PACK_CORRUPT); // code off
        assert_eq!(set(e + 28, &0u32.to_le_bytes()), SHADER_PACK_CORRUPT); // code in header
        assert_eq!(set(e + 32, &0u32.to_le_bytes()), SHADER_PACK_CORRUPT); // empty code
        assert_eq!(set(e + 32, &u32::MAX.to_le_bytes()), SHADER_PACK_CORRUPT); // code len
        // Swapping the first two entries breaks the name order.
        let mut b = good.clone();
        let (a, rest) = b[e..].split_at_mut(ENTRY_LEN);
        a.swap_with_slice(&mut rest[..ENTRY_LEN]);
        assert_eq!(code_of(&forge(b)), SHADER_PACK_CORRUPT);
        // Invalid UTF-8 in a name.
        let pack = ShaderPack::parse(&good).unwrap();
        let s_off = pack.strings.0;
        let mut b = good.clone();
        b[s_off] = 0xff;
        assert_eq!(code_of(&forge(b)), SHADER_PACK_CORRUPT);
    }

    /// Fuzz-like sweep: every truncation and every single-byte change (with and without a
    /// repaired checksum) is either rejected with a diagnostic or parses into a pack whose
    /// accessors stay in bounds. Nothing panics.
    #[test]
    fn no_panic_on_truncated_or_mutated_input() {
        let good = sample();
        for len in 0..good.len() {
            assert!(
                ShaderPack::parse(&good[..len]).is_err(),
                "truncated to {len}"
            );
        }
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        for i in 0..good.len() {
            for _ in 0..4 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let mut b = good.clone();
                b[i] ^= (state as u8) | 1;
                assert!(ShaderPack::parse(&b).is_err(), "unrepaired mutation at {i}");
                let b = forge(b);
                if let Ok(pack) = ShaderPack::parse(&b) {
                    for p in pack.iter() {
                        assert!(pack.find(p.name).is_some());
                    }
                    let _ = pack.get("composite.frag");
                }
            }
        }
    }
}
