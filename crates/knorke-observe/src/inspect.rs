//! Structured state inspection without serde: a module describes its state to a visitor,
//! the host decides the format (JSON for agents, a debug overlay, a test assertion).

use knorke_types::{Handle, Revision, Tick};

/// One leaf value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'a> {
    /// Boolean.
    Bool(bool),
    /// Signed integer.
    I64(i64),
    /// Unsigned integer.
    U64(u64),
    /// Floating point.
    F64(f64),
    /// Text.
    Str(&'a str),
    /// Small float tuple: vectors, colors, matrices (row of numbers, meaning given by the name).
    F32s(&'a [f32]),
    /// A runtime handle as packed bits ([`Handle::to_bits`]).
    Handle(u64),
    /// A simulation tick.
    Tick(Tick),
}

impl From<bool> for Value<'_> {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
macro_rules! from_int {
    ($variant:ident: $($t:ty),+) => {$(
        impl From<$t> for Value<'_> {
            fn from(v: $t) -> Self { Value::$variant(v as _) }
        }
    )+};
}
from_int!(I64: i8, i16, i32, i64, isize);
from_int!(U64: u8, u16, u32, u64, usize);
impl From<f32> for Value<'_> {
    fn from(v: f32) -> Self {
        Value::F64(v as f64)
    }
}
impl From<f64> for Value<'_> {
    fn from(v: f64) -> Self {
        Value::F64(v)
    }
}
impl<'a> From<&'a str> for Value<'a> {
    fn from(v: &'a str) -> Self {
        Value::Str(v)
    }
}
impl<'a> From<&'a [f32]> for Value<'a> {
    fn from(v: &'a [f32]) -> Self {
        Value::F32s(v)
    }
}
impl<T: ?Sized> From<Handle<T>> for Value<'_> {
    fn from(h: Handle<T>) -> Self {
        Value::Handle(h.to_bits())
    }
}
impl From<Tick> for Value<'_> {
    fn from(t: Tick) -> Self {
        Value::Tick(t)
    }
}
impl From<Revision> for Value<'_> {
    fn from(r: Revision) -> Self {
        Value::U64(r.0)
    }
}

/// Receives a description of some state: named values, nested objects and lists.
///
/// Inside a list, names are ignored (pass `""`).
pub trait Visitor {
    /// A named leaf value.
    fn value(&mut self, name: &str, value: Value<'_>);
    /// A named nested object starts.
    fn begin_object(&mut self, name: &str);
    /// A named list starts.
    fn begin_list(&mut self, name: &str);
    /// The innermost object or list ends.
    fn end(&mut self);

    /// `false` if the visitor only wants the top level (cheap summaries). Implementations
    /// of [`Inspect`] may skip large nested parts then.
    fn wants_detail(&self) -> bool {
        true
    }
}

/// Something that can describe its state.
///
/// Cold path by design (`&mut dyn Visitor`): inspection must be cheap to *offer*, not to run.
pub trait Inspect {
    /// Describes `self` as the fields of the current object.
    fn inspect(&self, v: &mut dyn Visitor);
}

impl dyn Visitor + '_ {
    /// Shorthand: `v.field("hp", npc.hp)`.
    pub fn field<'a>(&mut self, name: &str, value: impl Into<Value<'a>>) {
        self.value(name, value.into());
    }

    /// Shorthand: a nested object described by an [`Inspect`] value.
    pub fn object(&mut self, name: &str, inner: &(impl Inspect + ?Sized)) {
        self.begin_object(name);
        inner.inspect(self);
        self.end();
    }

    /// Shorthand: a list of [`Inspect`] values.
    pub fn list<'i, I: Inspect + 'i>(
        &mut self,
        name: &str,
        items: impl IntoIterator<Item = &'i I>,
    ) {
        self.begin_list(name);
        for item in items {
            self.begin_object("");
            item.inspect(self);
            self.end();
        }
        self.end();
    }
}
