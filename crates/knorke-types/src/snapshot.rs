//! Saving and restoring simulation state, and checksums over it (ADR 0010).
//!
//! Every module that holds simulation state implements [`Snapshot`]: it writes its state
//! into a [`SnapshotBuf`] section and restores it from a [`Reader`]. The host saves all
//! modules into one buffer; that buffer is the keyframe for replays, the rollback point for
//! netcode and the basis of the per-tick [`Checksum`] that detects divergence – per module,
//! because every module has its own section.
//!
//! Format: little-endian, no padding, no pointers. A section is
//! `id_len: u8, id: [u8], version: u16, len: u32, payload: [u8; len]`.
//! The buffer keeps its capacity, so saving every tick does not allocate once warm.

use crate::{
    BitSet, CommandBuffer, EventQueue, Generation, Handle, HandleMap, Revision, Seed, Slots,
    StableHasher, Tick,
};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::hash::Hasher;
use core::ops::Range;

/// 64-bit checksum of state bytes (FxHash, see [`StableHasher`]): equal state → equal checksum
/// on every run and platform. Detects divergence; not a cryptographic hash.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Checksum(pub u64);

impl Checksum {
    /// Checksum of `bytes`.
    pub fn of(bytes: &[u8]) -> Checksum {
        let mut h = StableHasher::default();
        h.write(bytes);
        h.write_usize(bytes.len());
        Checksum(h.finish())
    }
}

/// Why restoring failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// The data ended early.
    UnexpectedEnd,
    /// A different section was found than expected.
    WrongSection {
        /// Expected id.
        expected: &'static str,
        /// Found id.
        found: String,
    },
    /// The section's format version is not supported by this build.
    UnsupportedVersion {
        /// Section id.
        id: &'static str,
        /// Version found.
        version: u16,
    },
    /// A section was not read completely – save and restore disagree.
    TrailingBytes {
        /// Section id.
        id: &'static str,
        /// Unread bytes.
        remaining: usize,
    },
    /// The data is structurally invalid (e.g. a zero generation).
    Invalid(&'static str),
}

impl SnapshotError {
    /// As a diagnostic with a stable code.
    pub fn to_diagnostic(&self) -> crate::Diagnostic {
        use crate::{Diagnostic, DiagnosticCode};
        let (code, msg) = match self {
            Self::UnexpectedEnd => (
                DiagnosticCode::new("knorke.snapshot.truncated"),
                "snapshot data ended early".to_string(),
            ),
            Self::WrongSection { expected, found } => (
                DiagnosticCode::new("knorke.snapshot.wrong_section"),
                alloc::format!("expected section '{expected}', found '{found}'"),
            ),
            Self::UnsupportedVersion { id, version } => (
                DiagnosticCode::new("knorke.snapshot.unsupported_version"),
                alloc::format!("section '{id}' has unsupported version {version}"),
            ),
            Self::TrailingBytes { id, remaining } => (
                DiagnosticCode::new("knorke.snapshot.trailing_bytes"),
                alloc::format!(
                    "section '{id}' has {remaining} unread bytes: save and restore disagree"
                ),
            ),
            Self::Invalid(what) => (
                DiagnosticCode::new("knorke.snapshot.invalid"),
                what.to_string(),
            ),
        };
        Diagnostic::error(code, msg)
    }
}

/// Appends values to a byte buffer.
pub struct Writer<'a> {
    out: &'a mut Vec<u8>,
}

impl<'a> Writer<'a> {
    /// Writes into `out` (appending).
    pub fn new(out: &'a mut Vec<u8>) -> Self {
        Self { out }
    }

    /// Writes raw bytes.
    #[inline]
    pub fn put_bytes(&mut self, b: &[u8]) {
        self.out.extend_from_slice(b);
    }

    /// Writes one value.
    #[inline]
    pub fn put<T: SnapshotValue>(&mut self, v: &T) {
        v.put(self);
    }

    /// Writes a length-prefixed slice.
    pub fn put_slice<T: SnapshotValue>(&mut self, items: &[T]) {
        self.put_len(items.len());
        for i in items {
            i.put(self);
        }
    }

    /// Writes a length as `u32`.
    ///
    /// # Panics
    /// If `len` does not fit in `u32`.
    #[inline]
    pub fn put_len(&mut self, len: usize) {
        self.put(&u32::try_from(len).expect("snapshot: length exceeds u32"));
    }
}

/// Reads values written by a [`Writer`].
#[derive(Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Reads from `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Unread bytes.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Takes the next `n` raw bytes.
    #[inline]
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], SnapshotError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.data.len())
            .ok_or(SnapshotError::UnexpectedEnd)?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    /// Reads one value.
    #[inline]
    pub fn get<T: SnapshotValue>(&mut self) -> Result<T, SnapshotError> {
        T::get(self)
    }

    /// Reads a length written by [`Writer::put_len`].
    #[inline]
    pub fn get_len(&mut self) -> Result<usize, SnapshotError> {
        Ok(self.get::<u32>()? as usize)
    }

    /// Reads a length-prefixed slice into `out`, reusing its capacity and – through
    /// [`SnapshotValue::get_into`] – the allocations of the elements it already holds
    /// (e.g. the inner buffers of a `Vec<Vec<T>>`).
    pub fn get_into_vec<T: SnapshotValue>(
        &mut self,
        out: &mut Vec<T>,
    ) -> Result<(), SnapshotError> {
        let n = self.get_len()?;
        out.truncate(n);
        for v in out.iter_mut() {
            v.get_into(self)?;
        }
        let missing = n - out.len();
        out.reserve(missing.min(self.remaining()));
        for _ in 0..missing {
            out.push(self.get()?);
        }
        Ok(())
    }

    /// Opens the next section, which must have id `id`. Returns its version and a reader
    /// limited to its payload.
    pub fn section(&mut self, id: &'static str) -> Result<(u16, Reader<'a>), SnapshotError> {
        let id_len = self.get::<u8>()? as usize;
        let found = self.take(id_len)?;
        if found != id.as_bytes() {
            return Err(SnapshotError::WrongSection {
                expected: id,
                found: String::from_utf8_lossy(found).into_owned(),
            });
        }
        let version = self.get::<u16>()?;
        let len = self.get::<u32>()? as usize;
        Ok((version, Reader::new(self.take(len)?)))
    }

    /// Fails if bytes are left over (the section was not read completely).
    pub fn finish(&self, id: &'static str) -> Result<(), SnapshotError> {
        match self.remaining() {
            0 => Ok(()),
            remaining => Err(SnapshotError::TrailingBytes { id, remaining }),
        }
    }
}

/// Module-level state that can be saved and restored as one section.
pub trait Snapshot {
    /// Section id, e.g. `knorke.navigation`. Stable: part of the replay format.
    const ID: &'static str;
    /// Format version of the section. Bump when the layout changes; keep reading old
    /// versions in [`Snapshot::restore`] if old replays must stay playable.
    const VERSION: u16;

    /// Writes the state.
    fn save(&self, w: &mut Writer<'_>);

    /// Replaces the state with the saved one. Should reuse existing capacity.
    fn restore(&mut self, version: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError>;
}

/// Position of one section inside a [`SnapshotBuf`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionInfo {
    /// Section id.
    pub id: &'static str,
    /// Format version.
    pub version: u16,
    /// Byte range of the payload.
    pub payload: Range<usize>,
}

/// A reusable snapshot buffer with a table of its sections.
#[derive(Default)]
pub struct SnapshotBuf {
    bytes: Vec<u8>,
    sections: Vec<SectionInfo>,
}

impl SnapshotBuf {
    /// Empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets the content, keeps capacity.
    pub fn clear(&mut self) {
        self.bytes.clear();
        self.sections.clear();
    }

    /// The encoded bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The sections written so far.
    pub fn sections(&self) -> &[SectionInfo] {
        &self.sections
    }

    /// Writes a section with a custom payload writer.
    pub fn write_section(
        &mut self,
        id: &'static str,
        version: u16,
        f: impl FnOnce(&mut Writer<'_>),
    ) {
        let id_len = u8::try_from(id.len()).expect("snapshot: section id longer than 255 bytes");
        self.bytes.push(id_len);
        self.bytes.extend_from_slice(id.as_bytes());
        self.bytes.extend_from_slice(&version.to_le_bytes());
        let len_at = self.bytes.len();
        self.bytes.extend_from_slice(&[0; 4]);
        let start = self.bytes.len();
        f(&mut Writer::new(&mut self.bytes));
        let len =
            u32::try_from(self.bytes.len() - start).expect("snapshot: section larger than 4 GiB");
        self.bytes[len_at..start].copy_from_slice(&len.to_le_bytes());
        self.sections.push(SectionInfo {
            id,
            version,
            payload: start..self.bytes.len(),
        });
    }

    /// Saves `state` as its own section.
    pub fn save<S: Snapshot + ?Sized>(&mut self, state: &S) {
        self.write_section(S::ID, S::VERSION, |w| state.save(w));
    }

    /// Checksum over all bytes.
    pub fn checksum(&self) -> Checksum {
        Checksum::of(&self.bytes)
    }

    /// Checksum per section, in write order.
    pub fn section_checksums(&self) -> impl Iterator<Item = (&'static str, Checksum)> + '_ {
        self.sections
            .iter()
            .map(|s| (s.id, Checksum::of(&self.bytes[s.payload.clone()])))
    }
}

/// Restores `state` from the next section of `r` (which must be `S::ID`).
pub fn restore<S: Snapshot + ?Sized>(
    r: &mut Reader<'_>,
    state: &mut S,
) -> Result<(), SnapshotError> {
    let (version, mut section) = r.section(S::ID)?;
    state.restore(version, &mut section)?;
    section.finish(S::ID)
}

/// A value that can be written into and read from a snapshot.
pub trait SnapshotValue: Sized {
    /// Writes the value.
    fn put(&self, w: &mut Writer<'_>);
    /// Reads a value.
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError>;
    /// Reads into an existing value, reusing its allocations where the type supports it.
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        *self = Self::get(r)?;
        Ok(())
    }
}

macro_rules! le_value {
    ($($t:ty),+) => {$(
        impl SnapshotValue for $t {
            #[inline]
            fn put(&self, w: &mut Writer<'_>) { w.put_bytes(&self.to_le_bytes()); }
            #[inline]
            fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
                let b = r.take(size_of::<$t>())?;
                let mut a = [0u8; size_of::<$t>()];
                a.copy_from_slice(b);
                Ok(<$t>::from_le_bytes(a))
            }
        }
    )+};
}
le_value!(u8, u16, u32, u64, i8, i16, i32, i64);

/// Floats are written bit-exact, except that every NaN becomes the canonical quiet NaN:
/// sign and payload of a computed NaN differ between x86, ARM and WASM, and equal state
/// must give equal bytes and checksums on every platform (ADR 0011).
macro_rules! float_value {
    ($($t:ty => $bits:ty, $nan:expr);+ $(;)?) => {$(
        impl SnapshotValue for $t {
            #[inline]
            fn put(&self, w: &mut Writer<'_>) {
                let bits: $bits = if self.is_nan() { $nan } else { self.to_bits() };
                w.put(&bits);
            }
            #[inline]
            fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
                Ok(<$t>::from_bits(r.get()?))
            }
        }
    )+};
}
float_value! {
    f32 => u32, 0x7FC0_0000;
    f64 => u64, 0x7FF8_0000_0000_0000;
}

impl SnapshotValue for bool {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&(*self as u8));
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        match r.get::<u8>()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(SnapshotError::Invalid("bool must be 0 or 1")),
        }
    }
}

impl SnapshotValue for () {
    fn put(&self, _: &mut Writer<'_>) {}
    fn get(_: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(())
    }
}

impl<T: SnapshotValue> SnapshotValue for Option<T> {
    fn put(&self, w: &mut Writer<'_>) {
        match self {
            None => w.put(&0u8),
            Some(v) => {
                w.put(&1u8);
                v.put(w);
            }
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        match r.get::<u8>()? {
            0 => Ok(None),
            1 => Ok(Some(r.get()?)),
            _ => Err(SnapshotError::Invalid("option tag must be 0 or 1")),
        }
    }
}

impl<A: SnapshotValue, B: SnapshotValue> SnapshotValue for (A, B) {
    fn put(&self, w: &mut Writer<'_>) {
        self.0.put(w);
        self.1.put(w);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok((r.get()?, r.get()?))
    }
}

impl<T: SnapshotValue + Default + Copy, const N: usize> SnapshotValue for [T; N] {
    fn put(&self, w: &mut Writer<'_>) {
        for v in self {
            v.put(w);
        }
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut a = [T::default(); N];
        for v in &mut a {
            *v = r.get()?;
        }
        Ok(a)
    }
}

impl<T: SnapshotValue> SnapshotValue for Vec<T> {
    fn put(&self, w: &mut Writer<'_>) {
        w.put_slice(self);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut v = Vec::new();
        r.get_into_vec(&mut v)?;
        Ok(v)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        r.get_into_vec(self)
    }
}

impl SnapshotValue for Tick {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.0);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Tick(r.get()?))
    }
}
impl SnapshotValue for Seed {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.0);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Seed(r.get()?))
    }
}
impl SnapshotValue for Revision {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.0);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Revision(r.get()?))
    }
}
impl SnapshotValue for Checksum {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.0);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Ok(Checksum(r.get()?))
    }
}
impl SnapshotValue for Generation {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.get());
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Generation::new(r.get()?).ok_or(SnapshotError::Invalid("generation must not be zero"))
    }
}
impl<T: ?Sized> SnapshotValue for Handle<T> {
    fn put(&self, w: &mut Writer<'_>) {
        w.put(&self.to_bits());
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        Handle::from_bits(r.get()?).ok_or(SnapshotError::Invalid("handle with zero generation"))
    }
}

impl SnapshotValue for BitSet {
    fn put(&self, w: &mut Writer<'_>) {
        w.put_slice(self.words());
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut s = BitSet::new();
        s.get_into(r)?;
        Ok(s)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        r.get_into_vec(self.words_vec_mut())
    }
}

impl<T: SnapshotValue> SnapshotValue for Slots<T> {
    fn put(&self, w: &mut Writer<'_>) {
        self.snapshot_put(w);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut s = Slots::new();
        s.snapshot_get_into(r)?;
        Ok(s)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.snapshot_get_into(r)
    }
}

impl<K: ?Sized, V: SnapshotValue> SnapshotValue for HandleMap<K, V> {
    fn put(&self, w: &mut Writer<'_>) {
        self.snapshot_put(w);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut m = HandleMap::new();
        m.snapshot_get_into(r)?;
        Ok(m)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.snapshot_get_into(r)
    }
}

impl<E: SnapshotValue> SnapshotValue for EventQueue<E> {
    fn put(&self, w: &mut Writer<'_>) {
        self.snapshot_put(w);
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut q = EventQueue::new();
        q.snapshot_get_into(r)?;
        Ok(q)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        self.snapshot_get_into(r)
    }
}

impl<C: SnapshotValue> SnapshotValue for CommandBuffer<C> {
    fn put(&self, w: &mut Writer<'_>) {
        w.put_slice(self.pending());
    }
    fn get(r: &mut Reader<'_>) -> Result<Self, SnapshotError> {
        let mut b = CommandBuffer::new();
        b.get_into(r)?;
        Ok(b)
    }
    fn get_into(&mut self, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
        r.get_into_vec(self.commands_vec_mut())
    }
}

/// Implements [`SnapshotValue`] for plain structs: the listed fields in the listed order,
/// nothing else (no length, no tag). Every field must be listed, or the `get` side does not
/// compile.
///
/// `get_into` keeps the default (`*self = get()`), so use it only for types without
/// buffers to reuse; a struct holding a `Vec` needs a hand-written `get_into` to restore
/// without allocating.
///
/// ```
/// # use knorke_types::{Reader, SnapshotValue, Writer, by_fields};
/// #[derive(Debug, PartialEq)]
/// struct Hit { at: u32, damage: i16 }
/// by_fields! { Hit { at, damage } }
///
/// let mut bytes = Vec::new();
/// Writer::new(&mut bytes).put(&Hit { at: 1, damage: -2 });
/// assert_eq!(bytes, [1, 0, 0, 0, 0xfe, 0xff]);
/// assert_eq!(Reader::new(&bytes).get::<Hit>(), Ok(Hit { at: 1, damage: -2 }));
/// ```
#[macro_export]
macro_rules! by_fields {
    ($($t:ty { $($f:ident),+ $(,)? });+ $(;)?) => {$(
        impl $crate::SnapshotValue for $t {
            #[inline]
            fn put(&self, w: &mut $crate::Writer<'_>) { $( w.put(&self.$f); )+ }
            #[inline]
            fn get(
                r: &mut $crate::Reader<'_>,
            ) -> ::core::result::Result<Self, $crate::SnapshotError> {
                ::core::result::Result::Ok(Self { $( $f: r.get()? ),+ })
            }
        }
    )+};
}

/// Implements [`SnapshotValue`] for enums: a `u8` tag, then the variant's fields in the
/// listed order. Tag numbers are written out, so reordering or adding variants in the
/// enum cannot change existing snapshots. An unknown tag on read returns the given error
/// (an expression of type [`SnapshotError`]), before anything else is read.
///
/// Variants are written `tag => Name`, `tag => Name(a, b)` (tuple fields, any binding
/// names) or `tag => Name { x, y }` (named fields). `get_into` keeps the default.
///
/// ```
/// # use knorke_types::{Reader, SnapshotError, SnapshotValue, Writer, by_tag};
/// #[derive(Debug, PartialEq)]
/// enum Order { Halt, Go(i8), Aim { x: u8, y: u8 } }
/// by_tag! { Order, SnapshotError::Invalid("order tag") => {
///     0 => Halt,
///     1 => Go(dir),
///     2 => Aim { x, y },
/// } }
///
/// let mut bytes = Vec::new();
/// Writer::new(&mut bytes).put(&Order::Aim { x: 3, y: 4 });
/// assert_eq!(bytes, [2, 3, 4]);
/// assert_eq!(Reader::new(&[1, 0xff]).get::<Order>(), Ok(Order::Go(-1)));
/// assert_eq!(Reader::new(&[9]).get::<Order>(), Err(SnapshotError::Invalid("order tag")));
/// ```
#[macro_export]
macro_rules! by_tag {
    ($($t:ty, $err:expr => {
        $( $tag:literal => $v:ident $( ( $($a:ident),+ $(,)? ) )? $( { $($f:ident),+ $(,)? } )? ),+ $(,)?
    } );+ $(;)?) => {$(
        impl $crate::SnapshotValue for $t {
            fn put(&self, w: &mut $crate::Writer<'_>) {
                match self {
                    $( Self::$v $( ( $($a),+ ) )? $( { $($f),+ } )? => {
                        let tag: u8 = $tag;
                        w.put(&tag);
                        $( $( w.put($a); )+ )?
                        $( $( w.put($f); )+ )?
                    } )+
                }
            }
            fn get(
                r: &mut $crate::Reader<'_>,
            ) -> ::core::result::Result<Self, $crate::SnapshotError> {
                ::core::result::Result::Ok(match r.get::<u8>()? {
                    $( $tag => Self::$v
                        // Tuple arguments and struct fields are evaluated left to right,
                        // so the fields are read in the listed order.
                        $( ( $( { let $a = r.get()?; $a } ),+ ) )?
                        $( { $( $f: r.get()? ),+ } )?, )+
                    _ => return ::core::result::Result::Err($err),
                })
            }
        }
    )+};
}

/// Snapshot bytes of `value` as lowercase hex. Support for [`assert_bytes!`]; not public API.
#[doc(hidden)]
pub fn __bytes_hex<T: SnapshotValue>(value: &T) -> alloc::string::String {
    use core::fmt::Write as _;
    let mut bytes = Vec::new();
    Writer::new(&mut bytes).put(value);
    bytes.iter().fold(alloc::string::String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// For tests: `assert_bytes!(value => "hex", ...)` checks that every value writes exactly
/// these snapshot bytes (ADR 0010).
///
/// A round trip cannot see a swapped tag number or field order, because writer and reader
/// change together; this compares against bytes written down once. On a mismatch the
/// message lists every changed case with its new bytes in the same `value => "hex"` form,
/// ready to paste after a deliberate format change (which also invalidates saved
/// snapshots). It lives here, next to [`by_fields!`] and [`by_tag!`], so every crate
/// pins its values with the same helper; it is test support, not simulation API.
///
/// ```
/// # use knorke_types::assert_bytes;
/// assert_bytes!(1u16 => "0100", (2u8, -1i8) => "02ff");
/// ```
#[macro_export]
macro_rules! assert_bytes {
    ($($v:expr => $hex:literal),+ $(,)?) => {{
        let mut changed = $crate::__private::String::new();
        $(
            let got = $crate::__bytes_hex(&$v);
            if got != $hex {
                changed += &$crate::__private::format!(
                    "{} => \"{}\",\n", ::core::stringify!($v), got
                );
            }
        )+
        assert!(changed.is_empty(), "snapshot bytes changed:\n{changed}");
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Npc;

    #[derive(Default, Debug, PartialEq)]
    struct Nav {
        tick: Tick,
        agents: Slots<(f32, f32)>,
        paths: HandleMap<Npc, Vec<u32>>,
        dirty: BitSet,
        arrived: EventQueue<u32>,
    }

    impl Snapshot for Nav {
        const ID: &'static str = "knorke.test.nav";
        const VERSION: u16 = 1;
        fn save(&self, w: &mut Writer<'_>) {
            w.put(&self.tick);
            w.put(&self.agents);
            w.put(&self.paths);
            w.put(&self.dirty);
            w.put(&self.arrived);
        }
        fn restore(&mut self, version: u16, r: &mut Reader<'_>) -> Result<(), SnapshotError> {
            if version != 1 {
                return Err(SnapshotError::UnsupportedVersion {
                    id: Self::ID,
                    version,
                });
            }
            self.tick = r.get()?;
            self.agents.get_into(r)?;
            self.paths.get_into(r)?;
            self.dirty.get_into(r)?;
            self.arrived.get_into(r)
        }
    }

    fn sample() -> Nav {
        let mut n = Nav {
            tick: Tick(42),
            ..Default::default()
        };
        let a = n.agents.insert((1.0, 2.0));
        let b = n.agents.insert((3.0, 4.0));
        n.agents.remove(a);
        n.paths.insert(b.cast(), alloc::vec![7, 8, 9]);
        n.dirty.insert(130);
        n.arrived.push(Tick(41), 5);
        n
    }

    #[test]
    fn round_trip_restores_exact_state_including_free_lists() {
        let original = sample();
        let mut buf = SnapshotBuf::new();
        buf.save(&original);
        let mut restored = Nav::default();
        restore(&mut Reader::new(buf.bytes()), &mut restored).unwrap();
        assert_eq!(restored, original);
        // Next insert reuses the same freed slot with the same generation in both.
        let mut o = original;
        let h1 = o.agents.insert((0.0, 0.0));
        let h2 = restored.agents.insert((0.0, 0.0));
        assert_eq!(h1, h2);
    }

    #[test]
    fn checksums_detect_changes_per_section() {
        let mut a = SnapshotBuf::new();
        a.save(&sample());
        a.write_section("knorke.test.other", 1, |w| w.put(&1u32));
        let mut b = SnapshotBuf::new();
        let mut changed = sample();
        changed.tick = Tick(43);
        b.save(&changed);
        b.write_section("knorke.test.other", 1, |w| w.put(&1u32));
        assert_ne!(a.checksum(), b.checksum());
        let diff: Vec<_> = a
            .section_checksums()
            .zip(b.section_checksums())
            .filter(|(x, y)| x != y)
            .map(|(x, _)| x.0)
            .collect();
        assert_eq!(diff, ["knorke.test.nav"]);
    }

    #[test]
    fn errors_are_specific() {
        let mut buf = SnapshotBuf::new();
        buf.write_section("knorke.test.nav", 9, |w| w.put(&1u8));
        let mut n = Nav::default();
        assert_eq!(
            restore(&mut Reader::new(buf.bytes()), &mut n),
            Err(SnapshotError::UnsupportedVersion {
                id: "knorke.test.nav",
                version: 9
            })
        );
        buf.clear();
        buf.write_section("knorke.test.zzz", 1, |_| {});
        assert!(matches!(
            restore(&mut Reader::new(buf.bytes()), &mut n),
            Err(SnapshotError::WrongSection { .. })
        ));
        buf.clear();
        buf.save(&sample());
        let cut = &buf.bytes()[..buf.bytes().len() - 3];
        assert_eq!(
            restore(&mut Reader::new(cut), &mut n),
            Err(SnapshotError::UnexpectedEnd)
        );
        let d = SnapshotError::UnexpectedEnd.to_diagnostic();
        assert_eq!(d.code.as_str(), "knorke.snapshot.truncated");
    }

    #[test]
    fn trailing_bytes_reveal_save_restore_mismatch() {
        let mut buf = SnapshotBuf::new();
        buf.write_section("knorke.test.nav", 1, |w| {
            sample().save(w);
            w.put(&0xFFu8);
        });
        let mut n = Nav::default();
        assert_eq!(
            restore(&mut Reader::new(buf.bytes()), &mut n),
            Err(SnapshotError::TrailingBytes {
                id: "knorke.test.nav",
                remaining: 1
            })
        );
    }

    #[test]
    fn steady_state_saving_reuses_capacity() {
        let n = sample();
        let mut buf = SnapshotBuf::new();
        buf.save(&n);
        let cap = (buf.bytes.capacity(), buf.sections.capacity());
        for _ in 0..10 {
            buf.clear();
            buf.save(&n);
        }
        assert_eq!((buf.bytes.capacity(), buf.sections.capacity()), cap);
    }

    #[test]
    fn restoring_nested_vectors_reuses_inner_buffers() {
        let saved: Vec<Vec<u32>> = alloc::vec![alloc::vec![1, 2, 3], alloc::vec![4]];
        let mut bytes = Vec::new();
        saved.put(&mut Writer::new(&mut bytes));
        // Target with warm inner buffers, as after a previous restore.
        let mut target: Vec<Vec<u32>> = (0..3).map(|_| Vec::with_capacity(1000)).collect();
        target[0].extend([9; 10]);
        target.get_into(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(target, saved);
        assert!(
            target.iter().all(|v| v.capacity() >= 1000),
            "inner buffers were dropped and reallocated"
        );
        // Shrinking and growing still give exact content.
        let mut small: Vec<Vec<u32>> = Vec::new();
        small.get_into(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(small, saved);
    }

    #[test]
    fn nan_is_saved_canonically() {
        // x86 produces a negative quiet NaN for 0/0, ARM and WASM a positive one. The state
        // is the same; the snapshot bytes and therefore the checksums must be, too.
        for bits in [0xFFC0_0000u32, 0x7FC0_0000, 0x7FA1_2345, 0xFFFF_FFFF] {
            let mut b = Vec::new();
            f32::from_bits(bits).put(&mut Writer::new(&mut b));
            assert_eq!(b, 0x7FC0_0000u32.to_le_bytes(), "{bits:#X}");
        }
        let mut b = Vec::new();
        f64::from_bits(0xFFF8_0000_0000_0001).put(&mut Writer::new(&mut b));
        assert_eq!(b, 0x7FF8_0000_0000_0000u64.to_le_bytes());
        // Everything else is bit-exact, including signed zero, infinities and subnormals.
        for v in [
            0.0f32,
            -0.0,
            1.5,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::MIN_POSITIVE / 2.0,
        ] {
            let mut b = Vec::new();
            v.put(&mut Writer::new(&mut b));
            assert_eq!(b, v.to_bits().to_le_bytes());
            assert_eq!(
                f32::get(&mut Reader::new(&b)).unwrap().to_bits(),
                v.to_bits()
            );
        }
    }

    #[test]
    fn bitsets_compare_by_content() {
        let mut grown = BitSet::new();
        grown.insert(500);
        grown.remove(500);
        assert_eq!(grown, BitSet::new());
        assert_eq!(BitSet::with_capacity(256), BitSet::new());
        let mut a = BitSet::with_capacity(256);
        let mut b = BitSet::new();
        a.insert(3);
        b.insert(3);
        assert_eq!(a, b);
        b.insert(64);
        assert_ne!(a, b);
    }

    #[test]
    fn invalid_values_are_rejected() {
        assert!(bool::get(&mut Reader::new(&[2])).is_err());
        assert!(<Generation as SnapshotValue>::get(&mut Reader::new(&[0, 0, 0, 0])).is_err());
        assert!(Option::<u8>::get(&mut Reader::new(&[7])).is_err());
    }
}
