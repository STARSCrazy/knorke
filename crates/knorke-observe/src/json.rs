//! Minimal JSON output for inspection and diagnostics, without a serde dependency.
//!
//! Output is compact, deterministic (fields in visit order) and valid JSON: non-finite
//! floats become `null`, strings are escaped per RFC 8259.

use crate::{Inspect, Value, Visitor};
use core::fmt::Write as _;
use knorke_types::{Diagnostic, Severity};
use std::string::String;
use std::vec::Vec;

/// Appends `s` as a JSON string literal.
pub fn write_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Appends a number; non-finite values become `null`.
pub fn write_f64(out: &mut String, v: f64) {
    if v.is_finite() {
        let _ = write!(out, "{v}");
    } else {
        out.push_str("null");
    }
}

fn write_value(out: &mut String, v: Value<'_>) {
    match v {
        Value::Bool(b) => out.push_str(if b { "true" } else { "false" }),
        Value::I64(i) => {
            let _ = write!(out, "{i}");
        }
        Value::U64(u) => {
            let _ = write!(out, "{u}");
        }
        Value::F64(f) => write_f64(out, f),
        Value::Str(s) => write_str(out, s),
        Value::F32s(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_f64(out, *x as f64);
            }
            out.push(']');
        }
        // Handles as {"index":..,"generation":..} so they stay readable for agents.
        Value::Handle(bits) => {
            let _ = write!(
                out,
                "{{\"index\":{},\"generation\":{}}}",
                bits as u32,
                bits >> 32
            );
        }
        Value::Tick(t) => {
            let _ = write!(out, "{}", t.0);
        }
    }
}

/// Builds a JSON document from visitor calls.
pub struct JsonVisitor {
    out: String,
    /// Per open container: `true` if it is a list, `true` once it has an element.
    stack: Vec<(bool, bool)>,
}

impl Default for JsonVisitor {
    fn default() -> Self {
        Self::new()
    }
}

impl JsonVisitor {
    /// Starts a top-level object.
    pub fn new() -> Self {
        Self {
            out: String::from("{"),
            stack: Vec::from([(false, false)]),
        }
    }

    /// Closes all open containers and returns the document.
    pub fn finish(mut self) -> String {
        while !self.stack.is_empty() {
            self.end();
        }
        self.out
    }

    fn separator_and_key(&mut self, name: &str) {
        let Some((is_list, has_items)) = self.stack.last_mut() else {
            return; // value after the document was closed: ignored
        };
        if *has_items {
            self.out.push(',');
        }
        *has_items = true;
        if !*is_list {
            write_str(&mut self.out, name);
            self.out.push(':');
        }
    }
}

impl Visitor for JsonVisitor {
    fn value(&mut self, name: &str, value: Value<'_>) {
        if self.stack.is_empty() {
            return;
        }
        self.separator_and_key(name);
        write_value(&mut self.out, value);
    }

    fn begin_object(&mut self, name: &str) {
        if self.stack.is_empty() {
            return;
        }
        self.separator_and_key(name);
        self.out.push('{');
        self.stack.push((false, false));
    }

    fn begin_list(&mut self, name: &str) {
        if self.stack.is_empty() {
            return;
        }
        self.separator_and_key(name);
        self.out.push('[');
        self.stack.push((true, false));
    }

    fn end(&mut self) {
        if let Some((is_list, _)) = self.stack.pop() {
            self.out.push(if is_list { ']' } else { '}' });
        }
    }
}

/// Describes `value` as a JSON object.
pub fn inspect_to_json(value: &(impl Inspect + ?Sized)) -> String {
    let mut v = JsonVisitor::new();
    value.inspect(&mut v);
    v.finish()
}

/// One diagnostic as a JSON object: `{"severity":..,"code":..,"message":..,"path":..}`.
pub fn diagnostic_to_json(d: &Diagnostic) -> String {
    let mut out = String::new();
    out.push_str("{\"severity\":");
    write_str(
        &mut out,
        match d.severity {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        },
    );
    out.push_str(",\"code\":");
    write_str(&mut out, d.code.as_str());
    out.push_str(",\"message\":");
    write_str(&mut out, &d.message);
    out.push_str(",\"path\":");
    match &d.path {
        Some(p) => write_str(&mut out, p),
        None => out.push_str("null"),
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use knorke_types::{DiagnosticCode, Handle, Tick};

    struct Guard {
        name: &'static str,
        pos: [f32; 2],
        alert: bool,
        target: Option<Handle<Guard>>,
    }
    impl Inspect for Guard {
        fn inspect(&self, v: &mut dyn Visitor) {
            v.field("name", self.name);
            v.field("pos", &self.pos[..]);
            v.field("alert", self.alert);
            match self.target {
                Some(h) => v.field("target", h),
                None => v.field("target", "none"),
            }
        }
    }
    struct Scene(Vec<Guard>);
    impl Inspect for Scene {
        fn inspect(&self, v: &mut dyn Visitor) {
            v.field("tick", Tick(7));
            v.list("guards", &self.0);
            v.begin_object("empty");
            v.end();
        }
    }

    #[test]
    fn nested_json() {
        let target = Handle::new(3, knorke_types::Generation::new(2).unwrap());
        let scene = Scene(Vec::from([
            Guard {
                name: "g\"1",
                pos: [1.5, f32::NAN],
                alert: true,
                target: Some(target),
            },
            Guard {
                name: "g2",
                pos: [0.0, 0.0],
                alert: false,
                target: None,
            },
        ]));
        assert_eq!(
            inspect_to_json(&scene),
            r#"{"tick":7,"guards":[{"name":"g\"1","pos":[1.5,null],"alert":true,"target":{"index":3,"generation":2}},{"name":"g2","pos":[0,0],"alert":false,"target":"none"}],"empty":{}}"#
        );
    }

    #[test]
    fn diagnostics() {
        let d = Diagnostic::error(DiagnosticCode::new("knorke.test.fail"), "line\nbreak").at("a/b");
        assert_eq!(
            diagnostic_to_json(&d),
            r#"{"severity":"error","code":"knorke.test.fail","message":"line\nbreak","path":"a/b"}"#
        );
    }

    #[test]
    fn control_chars_escaped() {
        let mut s = String::new();
        write_str(&mut s, "\u{1}");
        assert_eq!(s, r#""\u0001""#);
    }
}
