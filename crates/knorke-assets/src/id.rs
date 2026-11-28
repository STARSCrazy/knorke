use crate::BAD_ID;
use knorke_types::Diagnostic;
use std::fmt;

/// Asset id: a relative path without extension in fixed spelling.
///
/// Segments use `a-z`, `0-9`, `_`, `-` and `.`, separated by `/`; no empty, `.` or `..`
/// segment. One spelling for all systems keeps Linux (case-sensitive) and Windows
/// (case-insensitive) builds finding the same files.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AssetId(String);

impl AssetId {
    /// Checks the spelling; fails with `knorke.assets.bad_id`.
    pub fn parse(id: &str) -> Result<Self, Diagnostic> {
        let ok_char = |c: char| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-' | '.');
        let ok = !id.is_empty()
            && id
                .split('/')
                .all(|s| !s.is_empty() && s != "." && s != ".." && s.chars().all(ok_char));
        if ok {
            Ok(Self(id.to_owned()))
        } else {
            Err(Diagnostic::error(
                BAD_ID,
                format!(
                    "asset id `{id}`: use lower-case a-z, 0-9, `_`, `-`, `.` and `/` between non-empty segments"
                ),
            ))
        }
    }

    /// The id as written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spelling_rules() {
        for good in ["guard", "sprites/guard", "sounds/step-2", "a/b_c/d.e"] {
            assert!(AssetId::parse(good).is_ok(), "{good}");
        }
        for bad in [
            "", "Guard", "a//b", "/a", "a/", "../a", "a/./b", "a b", "a\\b", "ä",
        ] {
            let e = AssetId::parse(bad).unwrap_err();
            assert_eq!(e.code, BAD_ID, "{bad}");
        }
    }
}
