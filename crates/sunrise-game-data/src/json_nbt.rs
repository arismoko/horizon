//! The wire JSON-to-NBT projection for synchronized registry data and text components.

use serde_json::Value;
use sunrise_core::nbt::{JavaString, Tag};

/// Wire registry data uses `Int` or `Long` for signed integers, `Double` for other numbers, `Byte` for booleans, and an empty compound for null.
pub fn json_to_nbt(v: &Value) -> Tag {
    match v {
        Value::Null => Tag::compound(Vec::<(&str, Tag)>::new()),
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => match n.as_i64() {
            Some(i) => match i32::try_from(i) {
                Ok(i) => Tag::Int(i),
                Err(_) => Tag::Long(i),
            },
            None => Tag::Double(n.as_f64().unwrap_or(0.0).to_bits()),
        },
        Value::String(s) => Tag::String(JavaString::from(s.as_str())),
        Value::Array(a) => Tag::List(a.iter().map(json_to_nbt).collect()),
        Value::Object(o) => Tag::compound(o.iter().map(|(k, v)| (k.as_str(), json_to_nbt(v)))),
    }
}

/// Like [`json_to_nbt`], leaves mixed translation arguments intact so the NBT writer wraps non-compounds under the empty key exactly once.
pub fn component_nbt(v: &Value) -> Tag {
    match v {
        Value::Array(a) => Tag::List(a.iter().map(component_nbt).collect()),
        Value::Object(o) => Tag::compound(o.iter().map(|(k, v)| (k.as_str(), component_nbt(v)))),
        other => json_to_nbt(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_core::nbt::{Limits, read_network, write_network};

    #[test]
    fn json_becomes_nbt() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"a":1,"b":[true,2.5],"c":"x"}"#).unwrap();
        let t = json_to_nbt(&v);
        assert!(matches!(t.get("a"), Some(Tag::Int(1))));
        assert!(matches!(t.get("c"), Some(Tag::String(_))));
        assert!(matches!(t.get("b"), Some(Tag::List(l)) if l.len() == 2));
    }

    /// A mixed `/give` reply preserves its count when the reader unwraps the list.
    #[test]
    fn mixed_translation_arguments_are_wrapped_once() {
        let json = serde_json::json!({
            "translate": "commands.give.success.single",
            "with": [1, {"text": "Raw Iron"}, {"text": "Tester"}],
        });
        let tag = component_nbt(&json);
        let Some(Tag::List(args)) = tag.get("with") else {
            panic!("no arguments: {tag:?}");
        };
        assert_eq!(args[0], Tag::Int(1));
        // Read back as ListTag.addAndUnwrap does: a wrapper wrapped twice stays a compound.
        let bytes = write_network(&tag, Limits::default()).unwrap();
        assert_eq!(read_network(&bytes, Limits::default()).unwrap().0, tag);
    }
}
