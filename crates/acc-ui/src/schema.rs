//! JSON Schemas generated from [`crate::spec`].

use schemars::generate::SchemaSettings;
use serde_json::{Value, json};

use crate::spec::Spec;

/// The JSON Schema (draft 2020-12) of an `acc-ui/1` spec.
pub fn schema() -> Value {
    let generator = SchemaSettings::draft2020_12().into_generator();
    let mut v = generator.into_root_schema_for::<Spec>().to_value();
    tidy(&mut v);
    // `v` is free-form in Rust, a constant on the wire.
    if let Some(p) = v.pointer_mut("/properties/v") {
        *p = json!({ "const": crate::spec::VERSION, "description": "Always \"acc-ui/1\"." });
    }
    v
}

/// Optional fields are just left out, so drop the `null` alternatives
/// schemars adds for `Option`, and Rust's numeric formats.
fn tidy(v: &mut Value) {
    match v {
        Value::Object(m) => {
            if let Some(Value::Array(types)) = m.get_mut("type") {
                types.retain(|t| t != "null");
                if types.len() == 1 {
                    let t = types.remove(0);
                    m.insert("type".to_owned(), t);
                }
            }
            if let Some(Value::Array(any)) = m.get_mut("anyOf") {
                any.retain(|s| s.get("type").and_then(Value::as_str) != Some("null"));
                if any.len() == 1
                    && let Some(Value::Object(only)) = any.pop()
                {
                    m.remove("anyOf");
                    for (k, val) in only {
                        m.entry(k).or_insert(val);
                    }
                }
            }
            if matches!(
                m.get("format").and_then(Value::as_str),
                Some("uint32" | "uint64" | "double")
            ) {
                m.remove("format");
            }
            m.values_mut().for_each(tidy);
        }
        Value::Array(a) => a.iter_mut().for_each(tidy),
        _ => {}
    }
}
