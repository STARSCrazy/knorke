// Ported from chiaro (D:\Entwicklung\chiaro @ 9bc5b9b, src/lighting/ProfileImport.hpp, src/lighting/ProfileImport.cpp), zlib licence:
// Copyright (C) 2026 Manuel Vögele. zlib license: this software is provided 'as-is', without any
// express or implied warranty; the origin must not be misrepresented; altered source versions
// must be plainly marked as such (this file is a Rust port) and this notice may not be removed or
// altered from any source distribution. Full text: LICENSE-CHIARO-ZLIB.txt.

//! Import of measured light distributions into the profile table (P9): IES LM-63 (type C
//! photometry) and EULUMDAT (.ldt).
//!
//! Both read from a byte slice, need no terminating zero, accept LF and CRLF line ends and a
//! UTF-8 BOM. Parsers never panic: every malformed input is an [`ImportError`] that names the
//! format, the line and the field. Numbers must be finite and fit a `f32`.
//!
//! Redesign against the C++ version: the output is returned as `Result<Vec<f32>, ImportError>`
//! instead of an out parameter that is left untouched on failure; SDL error strings are gone.
//! The Rust texts keep the C++ wording without the "Chiaro: " prefix.

use crate::profile::{
    PROFILE_AZIMUTH, PROFILE_AZIMUTH_STEP, PROFILE_POLAR, PROFILE_VALUES, ProfileError,
    normalize_profile,
};
use knorke_types::{Diagnostic, DiagnosticCode};

/// More angles than any real file has (0.1 degree steps over 360).
const MAX_ANGLES: usize = 3601;
/// Intensities per file.
const MAX_VALUES: usize = 1 << 22;

/// The kind of an import failure, for the stable diagnostic code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportErrorKind {
    /// The input is empty.
    NoData,
    /// A token or line is not what the format requires.
    Syntax,
    /// The input ends before the format's data is complete.
    Truncated,
    /// Valid text that this importer does not support (another photometric type, a separate tilt file, ...).
    Unsupported,
    /// A number is outside the range the format allows.
    OutOfRange,
    /// The measured data has no light.
    NoLight,
}

/// Why an IES or EULUMDAT file was refused. The message names the format and the place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportError {
    format: &'static str,
    kind: ImportErrorKind,
    line: Option<u32>,
    message: String,
}

impl ImportError {
    fn new(
        format: &'static str,
        kind: ImportErrorKind,
        line: Option<u32>,
        message: String,
    ) -> Self {
        Self {
            format,
            kind,
            line,
            message,
        }
    }

    /// The failure kind.
    pub fn kind(&self) -> ImportErrorKind {
        self.kind
    }

    /// The format, `"IES"` or `"EULUMDAT"`.
    pub fn format(&self) -> &'static str {
        self.format
    }

    /// The 1-based line of the problem, where the format has lines and the problem has one.
    pub fn line(&self) -> Option<u32> {
        self.line
    }

    /// The stable diagnostic code (`knorke.light2d.profile.import.*`).
    pub fn code(&self) -> DiagnosticCode {
        match self.kind {
            ImportErrorKind::NoData => DiagnosticCode::new("knorke.light2d.profile.import.no_data"),
            ImportErrorKind::Syntax => DiagnosticCode::new("knorke.light2d.profile.import.syntax"),
            ImportErrorKind::Truncated => {
                DiagnosticCode::new("knorke.light2d.profile.import.truncated")
            }
            ImportErrorKind::Unsupported => {
                DiagnosticCode::new("knorke.light2d.profile.import.unsupported")
            }
            ImportErrorKind::OutOfRange => {
                DiagnosticCode::new("knorke.light2d.profile.import.out_of_range")
            }
            ImportErrorKind::NoLight => {
                DiagnosticCode::new("knorke.light2d.profile.import.no_light")
            }
        }
    }

    /// As a diagnostic with its stable code.
    pub fn to_diagnostic(&self) -> Diagnostic {
        Diagnostic::error(self.code(), self.message.clone())
    }
}

impl core::fmt::Display for ImportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ImportError {}

impl From<ProfileError> for ImportError {
    fn from(e: ProfileError) -> Self {
        let format = "measured data";
        Self::new(format, ImportErrorKind::NoLight, None, e.to_string())
    }
}

/// Measured planes on the full circle: plane `k` at azimuth `angles[k]` in `[0, 360)`, ascending,
/// with one intensity per gamma angle (ascending in `[0, 180]`). One plane means rotational symmetry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeasuredPlanes {
    /// Azimuth of each plane, degrees.
    pub angles: Vec<f32>,
    /// Gamma angles shared by all planes, degrees.
    pub gammas: Vec<f32>,
    /// `values[k][i]` is the intensity at `angles[k]`, `gammas[i]`.
    pub values: Vec<Vec<f32>>,
}

/// Lines of a byte buffer (LF or CRLF), skipping a UTF-8 BOM.
struct Lines<'a> {
    data: &'a [u8],
    pos: usize,
    line: u32,
}

impl<'a> Lines<'a> {
    fn new(data: &'a [u8]) -> Self {
        let pos = if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
            3
        } else {
            0
        };
        Self { data, pos, line: 0 }
    }

    /// The next line without its end; `None` at the end of the buffer.
    fn next_line(&mut self) -> Option<&'a [u8]> {
        if self.pos >= self.data.len() {
            return None;
        }
        let rest = &self.data[self.pos..];
        let (content, consumed) = match rest.iter().position(|&b| b == b'\n') {
            Some(i) => (&rest[..i], i + 1),
            None => (rest, rest.len()),
        };
        self.pos += consumed;
        self.line += 1;
        let content = match content.last() {
            Some(b'\r') => &content[..content.len() - 1],
            _ => content,
        };
        Some(content)
    }

    fn line(&self) -> u32 {
        self.line
    }

    fn rest(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }
}

fn trim(s: &[u8]) -> &[u8] {
    let is_space = |b: &u8| matches!(*b, b' ' | b'\t' | b'\r');
    let start = s.iter().position(|b| !is_space(b)).unwrap_or(s.len());
    let end = s
        .iter()
        .rposition(|b| !is_space(b))
        .map_or(start, |i| i + 1);
    &s[start..end.max(start)]
}

/// A shortened copy of `s` for error messages: 24 bytes, control characters replaced.
fn quote(s: &[u8]) -> String {
    let mut out: String = s
        .iter()
        .take(24)
        .map(|&b| if b < 32 { '?' } else { char::from(b) })
        .collect();
    if s.len() > 24 {
        out.push_str("...");
    }
    out
}

/// Parses a whole token as a number within the float range (every value ends up as a `f32`); a
/// decimal comma is accepted when `comma` is set.
fn parse_number(token: &[u8], comma: bool) -> Option<f64> {
    let mut t = trim(token);
    if t.first() == Some(&b'+') {
        t = &t[1..];
    }
    if t.is_empty() || t[0] == b'+' {
        return None;
    }
    let buf: Vec<u8> = t
        .iter()
        .map(|&b| if comma && b == b',' { b'.' } else { b })
        .collect();
    let v: f64 = std::str::from_utf8(&buf).ok()?.parse().ok()?;
    (v.is_finite() && v.abs() <= f64::from(f32::MAX)).then_some(v)
}

/// A whole number, written with or without a fractional part of zero ("3" or "3.0").
fn parse_whole(token: &[u8], comma: bool) -> Option<i32> {
    let v = parse_number(token, comma)?;
    if v != v.floor() || v.abs() > 1e9 {
        return None;
    }
    Some(v as i32)
}

/// Linear interpolation of one plane over its gamma angles; 0 outside the measured range.
fn plane_at(gammas: &[f32], values: &[f32], g: f32) -> f32 {
    let eps = 1e-3f32;
    let (Some(&first), Some(&last)) = (gammas.first(), gammas.last()) else {
        return 0.0;
    };
    if g < first - eps || g > last + eps {
        return 0.0;
    }
    if gammas.len() == 1 {
        return values.first().copied().unwrap_or(0.0);
    }
    let i = gammas.partition_point(|&a| a <= g);
    if i == 0 {
        return values[0];
    }
    if i == gammas.len() {
        return values[values.len() - 1];
    }
    let t = (g - gammas[i - 1]) / (gammas[i] - gammas[i - 1]);
    values[i - 1] + (values[i] - values[i - 1]) * t
}

/// Ascending, finite, in `[lo, hi]` (with a small tolerance).
fn check_angles(
    a: &[f32],
    lo: f32,
    hi: f32,
    format: &'static str,
    what: &str,
) -> Result<(), ImportError> {
    for (i, &v) in a.iter().enumerate() {
        if v < lo - 1e-3 || v > hi + 1e-3 {
            return Err(ImportError::new(
                format,
                ImportErrorKind::OutOfRange,
                None,
                format!("{format}: {what} angle {v} is outside [{lo}, {hi}]"),
            ));
        }
        if i > 0 && !(v > a[i - 1]) {
            return Err(ImportError::new(
                format,
                ImportErrorKind::OutOfRange,
                None,
                format!(
                    "{format}: {what} angles must increase ({v} after {})",
                    a[i - 1]
                ),
            ));
        }
    }
    Ok(())
}

/// Adds the planes (angle mod 360, source plane) and sorts them; duplicates keep the first source.
fn assemble(
    mut planes: Vec<(f32, usize)>,
    gammas: &[f32],
    sources: &[Vec<f32>],
) -> Result<MeasuredPlanes, ImportError> {
    for p in &mut planes {
        p.0 %= 360.0;
        if p.0 < 0.0 {
            p.0 += 360.0;
        }
        if p.0 > 360.0 - 1e-3 {
            p.0 = 0.0;
        }
    }
    // Stable, and -0.0 equals 0.0 here, as in the C++ original.
    planes.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
    let mut m = MeasuredPlanes {
        gammas: gammas.to_vec(),
        ..MeasuredPlanes::default()
    };
    for &(angle, source) in &planes {
        if m.angles
            .last()
            .is_some_and(|&last| (angle - last).abs() < 1e-3)
        {
            continue;
        }
        let values = sources.get(source).ok_or_else(|| {
            ImportError::new(
                "measured data",
                ImportErrorKind::Syntax,
                None,
                "measured data: plane index out of range".to_owned(),
            )
        })?;
        m.angles.push(angle);
        m.values.push(values.clone());
    }
    Ok(m)
}

/// Resamples measured planes into a normalized table of [`PROFILE_VALUES`] entries. `what` names
/// the data in the error message (`"IES file"`, ...).
///
/// # Errors
/// [`ProfileError::PlanesInconsistent`] if the planes are empty or do not match the gamma
/// angles; [`ProfileError::NoLight`] if every intensity is 0.
pub fn resample_measured(planes: &MeasuredPlanes, what: &str) -> Result<Vec<f32>, ProfileError> {
    let n = planes.angles.len();
    let consistent = n > 0
        && planes.values.len() == n
        && !planes.gammas.is_empty()
        && planes.values.iter().all(|v| v.len() == planes.gammas.len());
    if !consistent {
        return Err(ProfileError::PlanesInconsistent {
            what: what.to_owned(),
        });
    }
    let mut table = vec![0.0f32; PROFILE_VALUES];
    for j in 0..PROFILE_AZIMUTH {
        let c = j as f32 * PROFILE_AZIMUTH_STEP;
        let (k0, k1, t) = if n > 1 {
            // The last plane at or below c, wrapping below the first one.
            let it = planes.angles.partition_point(|&a| a <= c);
            let k0 = if it == 0 { n - 1 } else { it - 1 };
            let k1 = (k0 + 1) % n;
            let a0 = planes.angles[k0];
            let mut a1 = planes.angles[k1];
            let mut x = c;
            if a1 <= a0 {
                a1 += 360.0;
            }
            if x < a0 {
                x += 360.0;
            }
            (k0, k1, ((x - a0) / (a1 - a0)).clamp(0.0, 1.0))
        } else {
            (0, 0, 0.0)
        };
        for i in 0..PROFILE_POLAR {
            let g = i as f32;
            let v0 = plane_at(&planes.gammas, &planes.values[k0], g);
            let v1 = plane_at(&planes.gammas, &planes.values[k1], g);
            table[j * PROFILE_POLAR + i] = v0 + (v1 - v0) * t;
        }
    }
    normalize_profile(&mut table, what)?;
    Ok(table)
}

// ---- IES LM-63 ----------------------------------------------------------------------------------

const IES: &str = "IES";

fn is_ies_delim(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b',')
}

/// The numbers after the TILT line: separated by blanks, tabs, line ends or commas.
struct IesTokens<'a> {
    data: &'a [u8],
    pos: usize,
    line: u32,
}

impl<'a> IesTokens<'a> {
    fn next_token(&mut self) -> Option<&'a [u8]> {
        while self.pos < self.data.len() && is_ies_delim(self.data[self.pos]) {
            if self.data[self.pos] == b'\n' {
                self.line += 1;
            }
            self.pos += 1;
        }
        if self.pos >= self.data.len() {
            return None;
        }
        let start = self.pos;
        while self.pos < self.data.len() && !is_ies_delim(self.data[self.pos]) {
            self.pos += 1;
        }
        Some(&self.data[start..self.pos])
    }

    fn syntax(&self, what: &str, token: &[u8]) -> ImportError {
        ImportError::new(
            IES,
            ImportErrorKind::Syntax,
            Some(self.line),
            format!(
                "IES line {}: expected {what}, got '{}'",
                self.line,
                quote(token)
            ),
        )
    }

    fn ends(&self, what: &str) -> ImportError {
        ImportError::new(
            IES,
            ImportErrorKind::Truncated,
            None,
            format!("IES: the file ends before {what}"),
        )
    }

    fn number(&mut self, what: &str) -> Result<f64, ImportError> {
        let token = self.next_token().ok_or_else(|| self.ends(what))?;
        parse_number(token, false).ok_or_else(|| self.syntax(what, token))
    }

    fn whole(&mut self, what: &str) -> Result<i32, ImportError> {
        let token = self.next_token().ok_or_else(|| self.ends(what))?;
        parse_whole(token, false).ok_or_else(|| {
            ImportError::new(
                IES,
                ImportErrorKind::Syntax,
                Some(self.line),
                format!(
                    "IES line {}: expected {what} (a whole number), got '{}'",
                    self.line,
                    quote(token)
                ),
            )
        })
    }

    fn numbers(&mut self, what: &str, count: usize) -> Result<Vec<f32>, ImportError> {
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let Some(token) = self.next_token() else {
                return Err(ImportError::new(
                    IES,
                    ImportErrorKind::Truncated,
                    None,
                    format!("IES: the file ends after {i} of {count} {what}"),
                ));
            };
            let v = parse_number(token, false).ok_or_else(|| self.syntax(what, token))?;
            out.push(v as f32);
        }
        Ok(out)
    }
}

fn starts_with_no_case(s: &[u8], prefix: &[u8]) -> bool {
    s.len() >= prefix.len()
        && s.iter()
            .zip(prefix)
            .all(|(a, b)| a.to_ascii_uppercase() == *b)
}

/// Parses an IES LM-63 file into a normalized table of [`PROFILE_VALUES`] entries.
///
/// # Errors
/// An [`ImportError`] naming the format, line and field for any malformed or unsupported input.
pub fn import_ies(data: &[u8]) -> Result<Vec<f32>, ImportError> {
    if data.is_empty() {
        return Err(ImportError::new(
            IES,
            ImportErrorKind::NoData,
            None,
            "IES: no data".to_owned(),
        ));
    }
    let mut lines = Lines::new(data);
    let mut tilt: Option<&[u8]> = None;
    while let Some(line) = lines.next_line() {
        let t = trim(line);
        if starts_with_no_case(t, b"TILT") {
            let Some(eq) = t.iter().position(|&b| b == b'=') else {
                return Err(ImportError::new(
                    IES,
                    ImportErrorKind::Syntax,
                    Some(lines.line()),
                    format!(
                        "IES line {}: expected TILT=NONE, TILT=INCLUDE or TILT=<file>",
                        lines.line()
                    ),
                ));
            };
            tilt = Some(trim(&t[eq + 1..]));
            break;
        }
    }
    let Some(tilt) = tilt else {
        return Err(ImportError::new(
            IES,
            ImportErrorKind::Syntax,
            None,
            "IES: no TILT= line (not an IES LM-63 file?)".to_owned(),
        ));
    };
    let mut tokens = IesTokens {
        data: lines.rest(),
        pos: 0,
        line: lines.line() + 1,
    };
    if starts_with_no_case(tilt, b"INCLUDE") {
        let _geometry = tokens.whole("the lamp-to-luminaire geometry of TILT=INCLUDE")?;
        let pairs = tokens.whole("the number of tilt angles")?;
        if pairs < 0 || pairs as usize > MAX_ANGLES {
            return Err(ImportError::new(
                IES,
                ImportErrorKind::OutOfRange,
                None,
                format!("IES: {pairs} tilt angles (expected 0 to {MAX_ANGLES})"),
            ));
        }
        tokens.numbers("tilt angles", pairs as usize)?;
        tokens.numbers("tilt multipliers", pairs as usize)?;
    } else if !starts_with_no_case(tilt, b"NONE") {
        return Err(ImportError::new(
            IES,
            ImportErrorKind::Unsupported,
            None,
            format!(
                "IES: TILT={} refers to a separate tilt file, which is not supported",
                quote(tilt)
            ),
        ));
    }
    tokens.number("the number of lamps")?;
    let _lumens = tokens.number("the lumens per lamp")?;
    let multiplier = tokens.number("the candela multiplier")?;
    let nv = tokens.whole("the number of vertical angles")?;
    let nh = tokens.whole("the number of horizontal angles")?;
    let ptype = tokens.whole("the photometric type")?;
    let _units = tokens.whole("the units type")?;
    for what in [
        "the luminous width",
        "the luminous length",
        "the luminous height",
        "the ballast factor",
        "the ballast-lamp factor (future use)",
        "the input watts",
    ] {
        tokens.number(what)?;
    }
    if ptype != 1 {
        return Err(ImportError::new(
            IES,
            ImportErrorKind::Unsupported,
            None,
            format!("IES: photometric type {ptype} is not supported, only type C (1)"),
        ));
    }
    let nv_ok = nv >= 1 && nv as usize <= MAX_ANGLES;
    let nh_ok = nh >= 1 && nh as usize <= MAX_ANGLES;
    if !nv_ok || !nh_ok || (nv as usize) * (nh as usize) > MAX_VALUES {
        return Err(ImportError::new(
            IES,
            ImportErrorKind::OutOfRange,
            None,
            format!(
                "IES: {nv} vertical and {nh} horizontal angles (expected 1 to {MAX_ANGLES} each)"
            ),
        ));
    }
    if multiplier < 0.0 {
        return Err(ImportError::new(
            IES,
            ImportErrorKind::OutOfRange,
            None,
            format!("IES: the candela multiplier must be >= 0 (got {multiplier})"),
        ));
    }
    let (nv, nh) = (nv as usize, nh as usize);
    let v = tokens.numbers("vertical angles", nv)?;
    let h = tokens.numbers("horizontal angles", nh)?;
    check_angles(&v, 0.0, 180.0, IES, "vertical")?;
    check_angles(&h, 0.0, 360.0, IES, "horizontal")?;
    let mut candela: Vec<Vec<f32>> = Vec::with_capacity(nh);
    for &plane_angle in &h {
        let mut plane = tokens.numbers("candela values", nv)?;
        for c in &mut plane {
            if *c < 0.0 {
                return Err(ImportError::new(
                    IES,
                    ImportErrorKind::OutOfRange,
                    None,
                    format!("IES: candela values must be >= 0 (got {c} in plane {plane_angle})"),
                ));
            }
            let scaled = f64::from(*c) * multiplier;
            if scaled > f64::from(f32::MAX) {
                return Err(ImportError::new(
                    IES,
                    ImportErrorKind::OutOfRange,
                    None,
                    format!(
                        "IES: candela value {c} times the multiplier {multiplier} is too large"
                    ),
                ));
            }
            *c = scaled as f32;
        }
        candela.push(plane);
    }
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
    let (first, last) = (h[0], h[nh - 1]);
    let mut planes: Vec<(f32, usize)> = h.iter().enumerate().map(|(k, &a)| (a, k)).collect();
    if nh == 1 {
        planes = vec![(0.0, 0)];
    } else if near(first, 0.0) && near(last, 90.0) {
        // quadrant symmetry
        for (k, &a) in h.iter().enumerate() {
            planes.push((180.0 - a, k));
            planes.push((180.0 + a, k));
            planes.push((360.0 - a, k));
        }
    } else if near(first, 0.0) && near(last, 180.0) {
        // mirror about the C0-C180 plane
        for (k, &a) in h.iter().enumerate() {
            planes.push((360.0 - a, k));
        }
    } else if near(first, 90.0) && near(last, 270.0) {
        // mirror about the C90-C270 plane
        for (k, &a) in h.iter().enumerate() {
            planes.push((540.0 - a, k));
        }
    } else if !(near(first, 0.0) && near(last, 360.0)) {
        return Err(ImportError::new(
            IES,
            ImportErrorKind::Unsupported,
            None,
            format!(
                "IES: horizontal angles from {first} to {last} are not a supported symmetry (one angle, 0-90, 0-180, 90-270 or 0-360)"
            ),
        ));
    }
    let measured = assemble(planes, &v, &candela)?;
    resample_measured(&measured, "IES file").map_err(ImportError::from)
}

// ---- EULUMDAT -----------------------------------------------------------------------------------

const LDT: &str = "EULUMDAT";

struct Eulumdat<'a> {
    lines: Lines<'a>,
    pending: &'a [u8],
}

impl<'a> Eulumdat<'a> {
    fn ends(&self, what: &str) -> ImportError {
        ImportError::new(
            LDT,
            ImportErrorKind::Truncated,
            None,
            format!(
                "EULUMDAT: the file ends before line {} ({what})",
                self.lines.line() + 1
            ),
        )
    }

    fn text(&mut self, what: &str) -> Result<(), ImportError> {
        match self.lines.next_line() {
            Some(_) => Ok(()),
            None => Err(self.ends(what)),
        }
    }

    fn number(&mut self, what: &str) -> Result<f64, ImportError> {
        let line = self.lines.next_line().ok_or_else(|| self.ends(what))?;
        parse_number(line, true).ok_or_else(|| {
            ImportError::new(
                LDT,
                ImportErrorKind::Syntax,
                Some(self.lines.line()),
                format!(
                    "EULUMDAT line {} ({what}): expected a number, got '{}'",
                    self.lines.line(),
                    quote(trim(line))
                ),
            )
        })
    }

    fn whole(&mut self, what: &str) -> Result<i32, ImportError> {
        let line = self.lines.next_line().ok_or_else(|| self.ends(what))?;
        parse_whole(line, true).ok_or_else(|| {
            ImportError::new(
                LDT,
                ImportErrorKind::Syntax,
                Some(self.lines.line()),
                format!(
                    "EULUMDAT line {} ({what}): expected a whole number, got '{}'",
                    self.lines.line(),
                    quote(trim(line))
                ),
            )
        })
    }

    /// `count` numbers from the following lines, several per line allowed.
    fn numbers(&mut self, what: &str, count: usize) -> Result<Vec<f32>, ImportError> {
        let mut out = Vec::with_capacity(count);
        while out.len() < count {
            if self.pending.is_empty() {
                let Some(line) = self.lines.next_line() else {
                    return Err(ImportError::new(
                        LDT,
                        ImportErrorKind::Truncated,
                        None,
                        format!(
                            "EULUMDAT: the file ends after {} of {count} {what}",
                            out.len()
                        ),
                    ));
                };
                self.pending = trim(line);
                if self.pending.is_empty() {
                    continue;
                }
            }
            let stop = self
                .pending
                .iter()
                .position(|&b| matches!(b, b' ' | b'\t' | b';'));
            let token = match stop {
                Some(i) => &self.pending[..i],
                None => self.pending,
            };
            self.pending = match stop {
                Some(i) => trim(&self.pending[i + 1..]),
                None => &[],
            };
            let v = parse_number(token, true).ok_or_else(|| {
                ImportError::new(
                    LDT,
                    ImportErrorKind::Syntax,
                    Some(self.lines.line()),
                    format!(
                        "EULUMDAT line {} ({what}): expected a number, got '{}'",
                        self.lines.line(),
                        quote(token)
                    ),
                )
            })?;
            out.push(v as f32);
        }
        Ok(out)
    }
}

/// Parses an EULUMDAT (.ldt) file into a normalized table of [`PROFILE_VALUES`] entries.
///
/// # Errors
/// An [`ImportError`] naming the format, line and field for any malformed or unsupported input.
pub fn import_eulumdat(data: &[u8]) -> Result<Vec<f32>, ImportError> {
    if data.is_empty() {
        return Err(ImportError::new(
            LDT,
            ImportErrorKind::NoData,
            None,
            "EULUMDAT: no data".to_owned(),
        ));
    }
    let mut f = Eulumdat {
        lines: Lines::new(data),
        pending: &[],
    };
    // The type indicator and the C-plane and gamma steps are read (the format has them) but not needed.
    f.text("company identification")?;
    let _ityp = f.whole("type indicator Ityp")?;
    let isym = f.whole("symmetry indicator Isym")?;
    let mc = f.whole("number of C-planes Mc")?;
    let _dc = f.number("distance between C-planes Dc")?;
    let ng = f.whole("number of intensities per C-plane Ng")?;
    let _dg = f.number("distance between intensities Dg")?;
    if !(0..=4).contains(&isym) {
        return Err(ImportError::new(
            LDT,
            ImportErrorKind::OutOfRange,
            Some(3),
            format!("EULUMDAT line 3: symmetry indicator Isym must be 0 to 4 (got {isym})"),
        ));
    }
    let max_angles = MAX_ANGLES as i32;
    if !(1..=max_angles).contains(&mc) || !(1..=max_angles).contains(&ng) {
        return Err(ImportError::new(
            LDT,
            ImportErrorKind::OutOfRange,
            None,
            format!(
                "EULUMDAT: Mc = {mc} C-planes and Ng = {ng} intensities per plane (expected 1 to {MAX_ANGLES})"
            ),
        ));
    }
    if mc as usize * ng as usize > MAX_VALUES {
        return Err(ImportError::new(
            LDT,
            ImportErrorKind::OutOfRange,
            None,
            format!(
                "EULUMDAT: Mc x Ng = {mc} x {ng} intensities are too many (at most {MAX_VALUES})"
            ),
        ));
    }
    let divisor = match isym {
        2 => 2,
        3 | 4 => 4,
        _ => 1,
    };
    if mc % divisor != 0 {
        return Err(ImportError::new(
            LDT,
            ImportErrorKind::OutOfRange,
            None,
            format!("EULUMDAT: Isym {isym} needs Mc divisible by {divisor} (got {mc})"),
        ));
    }
    for what in [
        "measurement report number",
        "luminaire name",
        "luminaire number",
        "file name",
        "date and user",
    ] {
        f.text(what)?;
    }
    for what in [
        "length or diameter of the luminaire",
        "width of the luminaire",
        "height of the luminaire",
        "length or diameter of the luminous area",
        "width of the luminous area",
        "height of the luminous area C0",
        "height of the luminous area C90",
        "height of the luminous area C180",
        "height of the luminous area C270",
        "downward flux fraction",
        "light output ratio",
        "conversion factor",
        "tilt during measurement",
    ] {
        f.number(what)?;
    }
    let sets = f.whole("number of standard sets of lamps")?;
    if !(0..=20).contains(&sets) {
        return Err(ImportError::new(
            LDT,
            ImportErrorKind::OutOfRange,
            Some(26),
            format!("EULUMDAT line 26: number of lamp sets must be 0 to 20 (got {sets})"),
        ));
    }
    for _ in 0..sets {
        for what in [
            "number of lamps",
            "type of lamps",
            "total luminous flux",
            "colour temperature",
            "colour rendering index",
            "wattage",
        ] {
            f.text(what)?;
        }
    }
    f.numbers("direct ratios", 10)?;
    let mc = mc as usize;
    let ng = ng as usize;
    let c = f.numbers("C angles", mc)?;
    let g = f.numbers("gamma angles", ng)?;
    check_angles(&c, 0.0, 360.0, LDT, "C")?;
    check_angles(&g, 0.0, 180.0, LDT, "gamma")?;
    let stored = match isym {
        0 => mc,
        1 => 1,
        4 => mc / 4 + 1,
        _ => mc / 2 + 1,
    };
    let values = f.numbers("luminous intensities", stored * ng)?;
    let mut planes: Vec<Vec<f32>> = Vec::with_capacity(stored);
    for s in 0..stored {
        let plane = values[s * ng..(s + 1) * ng].to_vec();
        if let Some(&v) = plane.iter().find(|&&v| v < 0.0) {
            return Err(ImportError::new(
                LDT,
                ImportErrorKind::OutOfRange,
                None,
                format!("EULUMDAT: luminous intensities must be >= 0 (got {v})"),
            ));
        }
        planes.push(plane);
    }
    // Every one of the Mc planes takes its values from a stored plane, by index as the format defines it.
    let mut full: Vec<(f32, usize)> = Vec::with_capacity(mc);
    for (k, &ck) in c.iter().enumerate() {
        let (ki, mci) = (k as i64, mc as i64);
        let s: i64 = match isym {
            0 => ki,
            1 => 0,
            2 => {
                if ki <= mci / 2 {
                    ki
                } else {
                    mci - ki
                }
            } // mirror about C0-C180
            3 => {
                // stored C270 .. C0 .. C90, mirror about C90-C270
                let first = 3 * mci / 4;
                let mut i = (((ki - first) % mci) + mci) % mci;
                if i > mci / 2 {
                    let mirrored = (((mci / 2 - ki) % mci) + mci) % mci;
                    i = (((mirrored - first) % mci) + mci) % mci;
                }
                i
            }
            _ => {
                // stored C0 .. C90, mirror about both planes
                let mut q = ki % (mci / 2);
                if q > mci / 4 {
                    q = mci / 2 - q;
                }
                q
            }
        };
        let s = usize::try_from(s).unwrap_or(usize::MAX);
        if s >= stored {
            return Err(ImportError::new(
                LDT,
                ImportErrorKind::OutOfRange,
                None,
                format!("EULUMDAT: C-plane {k} refers to a missing stored plane"),
            ));
        }
        full.push((if isym == 1 { 0.0 } else { ck }, s));
        if isym == 1 {
            break;
        }
    }
    let measured = assemble(full, &g, &planes)?;
    resample_measured(&measured, "EULUMDAT file").map_err(ImportError::from)
}
