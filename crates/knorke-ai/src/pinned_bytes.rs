//! Test-only: pins the exact snapshot bytes of values (ADR 0010).
//!
//! A round trip cannot see a swapped tag number or field order, because writer and reader
//! change together; these tests compare against fixed bytes written down once. On a
//! mismatch the message lists every changed case with its new bytes in the same
//! `value => "hex"` form, ready to paste after a deliberate format change (which also
//! invalidates saved snapshots).

/// Snapshot bytes of `v` as lowercase hex.
pub(crate) fn hex<T: knorke_types::SnapshotValue>(v: &T) -> String {
    use std::fmt::Write as _;
    let mut bytes = Vec::new();
    knorke_types::Writer::new(&mut bytes).put(v);
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// `assert_bytes!(value => "hex", ...)`: every value writes exactly these bytes.
macro_rules! assert_bytes {
    ($($v:expr => $hex:literal),+ $(,)?) => {{
        let mut changed = String::new();
        $(
            let got = $crate::pinned_bytes::hex(&$v);
            if got != $hex {
                changed += &format!("{} => \"{}\",\n", stringify!($v), got);
            }
        )+
        assert!(changed.is_empty(), "snapshot bytes changed:\n{changed}");
    }};
}
