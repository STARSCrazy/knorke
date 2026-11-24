//! The error type every settings struct shares: one macro, one definition.

/// Re-exports for [`settings_error!`](crate::settings_error); not public API.
#[doc(hidden)]
pub mod __private {
    pub use alloc::format;
}

/// Defines `SettingsError` (public) plus the private helpers `check` and `range` in the
/// calling module. Each crate keeps its own type, only the two diagnostic codes differ.
///
/// ```
/// knorke_types::settings_error!(
///     "knorke.demo.settings.not_finite",
///     "knorke.demo.settings.out_of_range"
/// );
/// assert!(range("a", 2.0, 0.0, 1.0, "must be in [0, 1]").is_err());
/// let error = check("a", f32::NAN, true, "").unwrap_err();
/// assert_eq!(error.code().as_str(), "knorke.demo.settings.not_finite");
/// ```
#[macro_export]
macro_rules! settings_error {
    ($not_finite:literal, $out_of_range:literal) => {
        /// Why settings were refused: the first field that breaks its rule.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub enum SettingsError {
            /// A value is NaN or infinite.
            NotFinite {
                /// Field path, e.g. `haze.density`.
                field: &'static str,
                /// The received value.
                value: f32,
            },
            /// A value is outside its allowed range.
            OutOfRange {
                /// Field path, e.g. `glow.scale`.
                field: &'static str,
                /// The rule, e.g. `must be in [0, 64]`.
                rule: &'static str,
                /// The received value.
                value: f32,
            },
        }

        impl SettingsError {
            #[doc = concat!("The stable code: `", $not_finite, "` or `", $out_of_range, "`.")]
            pub fn code(&self) -> $crate::DiagnosticCode {
                match self {
                    Self::NotFinite { .. } => $crate::DiagnosticCode::new($not_finite),
                    Self::OutOfRange { .. } => $crate::DiagnosticCode::new($out_of_range),
                }
            }

            /// As a diagnostic with the field as its path.
            pub fn to_diagnostic(&self) -> $crate::Diagnostic {
                let (Self::NotFinite { field, .. } | Self::OutOfRange { field, .. }) = self;
                $crate::Diagnostic::error(self.code(), self.to_string())
                    .at($crate::__private::format!("settings.{field}"))
            }
        }

        impl core::fmt::Display for SettingsError {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self {
                    Self::NotFinite { field, value } => {
                        write!(f, "settings.{field} must be finite (got {value})")
                    }
                    Self::OutOfRange { field, rule, value } => {
                        write!(f, "settings.{field} {rule} (got {value})")
                    }
                }
            }
        }

        impl core::error::Error for SettingsError {}

        /// `NotFinite` for NaN/infinity, else `OutOfRange` unless `ok`.
        fn check(
            field: &'static str,
            value: f32,
            ok: bool,
            rule: &'static str,
        ) -> Result<(), SettingsError> {
            if !value.is_finite() {
                Err(SettingsError::NotFinite { field, value })
            } else if ok {
                Ok(())
            } else {
                Err(SettingsError::OutOfRange { field, rule, value })
            }
        }

        /// `check` with the rule `lo <= v <= hi`.
        fn range(
            field: &'static str,
            v: f32,
            lo: f32,
            hi: f32,
            rule: &'static str,
        ) -> Result<(), SettingsError> {
            check(field, v, v >= lo && v <= hi, rule)
        }
    };
}
