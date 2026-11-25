use alloc::borrow::Cow;
use alloc::string::String;
use core::fmt;

/// How bad a [`Diagnostic`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Informational; nothing to fix.
    Info,
    /// Works, but probably not as intended.
    Warning,
    /// The operation failed or the state is invalid.
    Error,
}

impl Severity {
    /// The lowercase word every text and JSON output uses: `info`, `warning` or `error`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// Stable, machine-readable diagnostic code such as `knorke.scene.reference.missing`.
///
/// Codes are part of the public contract that agents match on (plan section 8.3): renaming
/// one is a breaking change. Format: at least two dot-separated segments of `a-z`, `0-9`
/// and `_`, each starting with a letter, the first segment naming the owning module family.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiagnosticCode(Cow<'static, str>);

impl DiagnosticCode {
    /// Creates a code from a literal. Invalid codes fail at compile time in const context.
    ///
    /// ```
    /// use knorke_types::DiagnosticCode;
    /// const MISSING: DiagnosticCode = DiagnosticCode::new("knorke.scene.reference.missing");
    /// ```
    ///
    /// ```compile_fail
    /// use knorke_types::DiagnosticCode;
    /// const BAD: DiagnosticCode = DiagnosticCode::new("Knorke.Scene");
    /// ```
    pub const fn new(code: &'static str) -> Self {
        assert!(is_valid_code(code), "invalid diagnostic code");
        Self(Cow::Borrowed(code))
    }

    /// Parses a code received at runtime (e.g. from a deserialised report).
    pub fn parse(code: &str) -> Option<Self> {
        is_valid_code(code).then(|| Self(Cow::Owned(String::from(code))))
    }

    /// The code as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

const fn is_valid_code(code: &str) -> bool {
    let b = code.as_bytes();
    let mut segments = 0;
    let mut i = 0;
    let mut at_segment_start = true;
    while i < b.len() {
        let c = b[i];
        if at_segment_start {
            if !c.is_ascii_lowercase() {
                return false;
            }
            segments += 1;
            at_segment_start = false;
        } else if c == b'.' {
            at_segment_start = true;
        } else if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_') {
            return false;
        }
        i += 1;
    }
    !at_segment_start && segments >= 2
}

/// A structured finding reported by a module, validator or agent operation.
///
/// Diagnostics are small on purpose: a code to match on, a human message, and optionally
/// the path of the affected item (e.g. `entities/guard.01/components/knorke.ai.utility/1`),
/// so a caller can fix the cause without a state dump.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Severity.
    pub severity: Severity,
    /// Stable code.
    pub code: DiagnosticCode,
    /// Human-readable explanation. Not part of the stable contract.
    pub message: String,
    /// Path of the affected item, if there is one.
    pub path: Option<String>,
}

impl Diagnostic {
    /// Creates a diagnostic without a path.
    pub fn new(severity: Severity, code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
            path: None,
        }
    }

    /// Shorthand for an [`Severity::Error`] diagnostic.
    pub fn error(code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, code, message)
    }

    /// Shorthand for a [`Severity::Warning`] diagnostic.
    pub fn warning(code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, code, message)
    }

    /// Attaches the path of the affected item.
    #[must_use]
    pub fn at(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]", self.severity.as_str(), self.code)?;
        if let Some(path) = &self.path {
            write!(f, " at {path}")?;
        }
        write!(f, ": {}", self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn code_validation() {
        for ok in [
            "knorke.scene",
            "knorke.scene.reference.missing",
            "sagax.nav.a2_star",
        ] {
            assert!(is_valid_code(ok), "{ok}");
        }
        for bad in [
            "",
            "knorke",
            "knorke.",
            ".scene",
            "knorke..x",
            "Knorke.scene",
            "knorke.2d",
            "knorke.sc-ene",
        ] {
            assert!(!is_valid_code(bad), "{bad}");
        }
    }

    #[test]
    fn parse_matches_const() {
        const C: DiagnosticCode = DiagnosticCode::new("knorke.test.code");
        assert_eq!(DiagnosticCode::parse("knorke.test.code"), Some(C));
        assert_eq!(DiagnosticCode::parse("nope"), None);
    }

    #[test]
    fn display_is_compact() {
        let d = Diagnostic::error(
            DiagnosticCode::new("knorke.scene.reference.missing"),
            "no entity 'door.3'",
        )
        .at("entities/guard.01/target");
        assert_eq!(
            d.to_string(),
            "error[knorke.scene.reference.missing] at entities/guard.01/target: no entity 'door.3'"
        );
    }
}
