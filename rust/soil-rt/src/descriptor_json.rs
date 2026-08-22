//! Descriptors as data (impl plan §4 step 7), in the tr-grammar §7
//! conventions — internally tagged sums, records — as if descriptors
//! were already Soil values. `Named` shapes serialize by *name* (ids
//! are per-instance); loading is two-pass (declare all, then define
//! all), which handles recursive types for free.
//!
//! This format feeds the fixtures corpus and the C ABI's registration
//! entry point, and seeds plan 02's `--types env.json` contract — but
//! it is not frozen here; plan 02 freezes it in `docs/soil0-cli.md`.
//!
//! ```json
//! [{"name": "SummaryRow",
//!   "strategy": {"tag": "Structural"},
//!   "body": {"tag": "Record", "value": [
//!     {"name": "count", "shape": {"tag": "U64"}, "ignored": null},
//!     {"name": "mean",  "shape": {"tag": "F64"}, "ignored": null}]}}]
//! ```
//!
//! Shapes: `{"tag": "I64"}` …, `{"tag": "List", "value": <shape>}`,
//! `{"tag": "Map", "value": {"key": <shape>, "value": <shape>}}`,
//! `{"tag": "Named", "value": "Tree"}`, `{"tag": "Closure"}`.
//! Ignored defaults: `null`, `{"tag": "Const", "value": <scalar>}`, or
//! `{"tag": "CopyField", "value": "field_name"}` (the serializable
//! vocabulary; `Native` thunks cannot round-trip through data).

use serde_json::Value as JsonValue;

use crate::descriptor::{
    FieldDesc, IgnoredDefault, Strategy, TypeBody, TypeDesc, TypeId, TypeShape, VariantDesc,
};
use crate::error::{PanicKind, SoilError};
use crate::json;
use crate::Runtime;

fn err(message: impl std::fmt::Display) -> SoilError {
    SoilError::new(
        PanicKind::DecodeError,
        format!("descriptor JSON: {message}"),
    )
}

/// Load a JSON array of descriptors into the registry. Returns the new
/// `TypeId`s in array order.
///
/// Not transactional: on error the registry may retain declarations
/// from the failed load, so discard the runtime rather than retrying
/// into it.
pub fn load_descriptors(runtime: &mut Runtime, json: &str) -> Result<Vec<TypeId>, SoilError> {
    let root: JsonValue =
        serde_json::from_str(json).map_err(|e| err(format!("invalid JSON: {e}")))?;
    let JsonValue::Array(entries) = root else {
        return Err(err("expected a top-level array"));
    };

    // Pass 1: declare every name, so shapes may reference any of them.
    let mut ids = Vec::with_capacity(entries.len());
    for entry in &entries {
        let name = get_str(entry, "name")?;
        ids.push(runtime.registry.declare(name)?);
    }

    // Pass 2: build and define each descriptor.
    for (entry, id) in entries.iter().zip(&ids) {
        let desc = parse_desc(runtime, entry)?;
        runtime.registry.define(*id, desc)?;
    }
    Ok(ids)
}

/// Serialize registered types back to the descriptor JSON format.
/// Fails on `Native` ignored-defaults, which are code, not data.
pub fn descriptors_to_json(runtime: &Runtime, ids: &[TypeId]) -> Result<String, SoilError> {
    let mut out = String::from("[");
    for (i, id) in ids.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let desc = runtime.registry.get(*id)?;
        out.push_str("{\"name\":");
        push_str(&desc.name, &mut out);
        out.push_str(",\"strategy\":{\"tag\":");
        push_str(
            match desc.strategy {
                Strategy::Structural => "Structural",
                Strategy::Opaque => "Opaque",
            },
            &mut out,
        );
        out.push_str("},\"body\":");
        match &desc.body {
            TypeBody::Opaque => out.push_str("{\"tag\":\"Opaque\"}"),
            TypeBody::Record(fields) => {
                out.push_str("{\"tag\":\"Record\",\"value\":[");
                for (j, field) in fields.iter().enumerate() {
                    if j > 0 {
                        out.push(',');
                    }
                    out.push_str("{\"name\":");
                    push_str(&field.name, &mut out);
                    out.push_str(",\"shape\":");
                    push_shape(runtime, &field.shape, &mut out)?;
                    out.push_str(",\"ignored\":");
                    match &field.ignored {
                        None => out.push_str("null"),
                        Some(IgnoredDefault::Const(value)) => {
                            out.push_str("{\"tag\":\"Const\",\"value\":");
                            out.push_str(&json::encode(runtime, value)?);
                            out.push('}');
                        }
                        Some(IgnoredDefault::CopyField { name, .. }) => {
                            out.push_str("{\"tag\":\"CopyField\",\"value\":");
                            push_str(name, &mut out);
                            out.push('}');
                        }
                        Some(IgnoredDefault::Native(_)) => {
                            return Err(err(format!(
                                "field {} of {} has a native default, which is not serializable",
                                field.name, desc.name
                            )));
                        }
                    }
                    out.push('}');
                }
                out.push_str("]}");
            }
            TypeBody::Sum(variants) => {
                out.push_str("{\"tag\":\"Sum\",\"value\":[");
                for (j, variant) in variants.iter().enumerate() {
                    if j > 0 {
                        out.push(',');
                    }
                    out.push_str("{\"name\":");
                    push_str(&variant.name, &mut out);
                    out.push_str(",\"payload\":");
                    match &variant.payload {
                        None => out.push_str("null"),
                        Some(shape) => push_shape(runtime, shape, &mut out)?,
                    }
                    out.push('}');
                }
                out.push_str("]}");
            }
        }
        out.push('}');
    }
    out.push(']');
    Ok(out)
}

/// Parse one shape in the descriptor-JSON encoding (`{"tag": "I64"}`,
/// `{"tag": "Named", "value": "Doc"}`, …) against the runtime's
/// registry. The entry point used by the fixtures harness.
pub fn parse_shape_json(runtime: &Runtime, json: &str) -> Result<TypeShape, SoilError> {
    let tree: JsonValue =
        serde_json::from_str(json).map_err(|e| err(format!("invalid JSON: {e}")))?;
    parse_shape(runtime, &tree)
}

fn push_str(s: &str, out: &mut String) {
    json::encode::push_string(s, out);
}

fn push_shape(runtime: &Runtime, shape: &TypeShape, out: &mut String) -> Result<(), SoilError> {
    let scalar = |tag: &str, out: &mut String| {
        out.push_str("{\"tag\":\"");
        out.push_str(tag);
        out.push_str("\"}");
    };
    match shape {
        TypeShape::I64 => scalar("I64", out),
        TypeShape::U64 => scalar("U64", out),
        TypeShape::I32 => scalar("I32", out),
        TypeShape::U32 => scalar("U32", out),
        TypeShape::I16 => scalar("I16", out),
        TypeShape::U16 => scalar("U16", out),
        TypeShape::I8 => scalar("I8", out),
        TypeShape::U8 => scalar("U8", out),
        TypeShape::BigInt => scalar("BigInt", out),
        TypeShape::F64 => scalar("F64", out),
        TypeShape::Utf8 => scalar("Utf8", out),
        TypeShape::Bytes => scalar("Bytes", out),
        TypeShape::Unit => scalar("Unit", out),
        TypeShape::Closure => scalar("Closure", out),
        TypeShape::List(elem) => {
            out.push_str("{\"tag\":\"List\",\"value\":");
            push_shape(runtime, elem, out)?;
            out.push('}');
        }
        TypeShape::Map(key, value) => {
            out.push_str("{\"tag\":\"Map\",\"value\":{\"key\":");
            push_shape(runtime, key, out)?;
            out.push_str(",\"value\":");
            push_shape(runtime, value, out)?;
            out.push_str("}}");
        }
        TypeShape::Named(id) => {
            // Serialized by name: ids are per-instance (impl plan §1).
            let name = match runtime.registry.get(*id) {
                Ok(desc) => desc.name.clone(),
                Err(e) => return Err(e),
            };
            out.push_str("{\"tag\":\"Named\",\"value\":");
            push_str(&name, out);
            out.push('}');
        }
    }
    Ok(())
}

fn get_str<'a>(value: &'a JsonValue, key: &str) -> Result<&'a str, SoilError> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| err(format!("missing string {key:?}")))
}

fn get_tag(value: &JsonValue) -> Result<&str, SoilError> {
    value
        .get("tag")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| err("missing \"tag\""))
}

fn parse_desc(runtime: &Runtime, entry: &JsonValue) -> Result<TypeDesc, SoilError> {
    let name = get_str(entry, "name")?.to_string();
    let strategy = match get_tag(
        entry
            .get("strategy")
            .ok_or_else(|| err("missing strategy"))?,
    )? {
        "Structural" => Strategy::Structural,
        "Opaque" => Strategy::Opaque,
        other => return Err(err(format!("unknown strategy {other:?}"))),
    };
    let body_json = entry.get("body").ok_or_else(|| err("missing body"))?;
    let body = match get_tag(body_json)? {
        "Opaque" => TypeBody::Opaque,
        "Record" => {
            let JsonValue::Array(field_entries) = body_json
                .get("value")
                .ok_or_else(|| err("record body needs a value"))?
            else {
                return Err(err("record body value must be an array"));
            };
            let mut fields = Vec::with_capacity(field_entries.len());
            for field_entry in field_entries {
                let field_name = get_str(field_entry, "name")?.to_string();
                let shape = parse_shape(
                    runtime,
                    field_entry
                        .get("shape")
                        .ok_or_else(|| err(format!("field {field_name} needs a shape")))?,
                )?;
                let ignored = match field_entry.get("ignored") {
                    None | Some(JsonValue::Null) => None,
                    Some(spec) => Some(parse_ignored(runtime, spec, &field_name, &shape)?),
                };
                fields.push(FieldDesc {
                    name: field_name,
                    shape,
                    ignored,
                });
            }
            resolve_copy_indices(&mut fields)?;
            TypeBody::Record(fields)
        }
        "Sum" => {
            let JsonValue::Array(variant_entries) = body_json
                .get("value")
                .ok_or_else(|| err("sum body needs a value"))?
            else {
                return Err(err("sum body value must be an array"));
            };
            let mut variants = Vec::with_capacity(variant_entries.len());
            for variant_entry in variant_entries {
                let variant_name = get_str(variant_entry, "name")?.to_string();
                let payload = match variant_entry.get("payload") {
                    None | Some(JsonValue::Null) => None,
                    Some(shape_json) => Some(parse_shape(runtime, shape_json)?),
                };
                variants.push(VariantDesc {
                    name: variant_name,
                    payload,
                });
            }
            TypeBody::Sum(variants)
        }
        other => return Err(err(format!("unknown body tag {other:?}"))),
    };
    Ok(TypeDesc {
        name,
        strategy,
        body,
    })
}

fn parse_shape(runtime: &Runtime, shape_json: &JsonValue) -> Result<TypeShape, SoilError> {
    let tag = get_tag(shape_json)?;
    Ok(match tag {
        "I64" => TypeShape::I64,
        "U64" => TypeShape::U64,
        "I32" => TypeShape::I32,
        "U32" => TypeShape::U32,
        "I16" => TypeShape::I16,
        "U16" => TypeShape::U16,
        "I8" => TypeShape::I8,
        "U8" => TypeShape::U8,
        "BigInt" => TypeShape::BigInt,
        "F64" => TypeShape::F64,
        "Utf8" => TypeShape::Utf8,
        "Bytes" => TypeShape::Bytes,
        "Unit" => TypeShape::Unit,
        "Closure" => TypeShape::Closure,
        "List" => TypeShape::List(Box::new(parse_shape(
            runtime,
            shape_json
                .get("value")
                .ok_or_else(|| err("List shape needs a value"))?,
        )?)),
        "Map" => {
            let value = shape_json
                .get("value")
                .ok_or_else(|| err("Map shape needs a value"))?;
            TypeShape::Map(
                Box::new(parse_shape(
                    runtime,
                    value.get("key").ok_or_else(|| err("Map needs a key"))?,
                )?),
                Box::new(parse_shape(
                    runtime,
                    value.get("value").ok_or_else(|| err("Map needs a value"))?,
                )?),
            )
        }
        "Named" => {
            let name = shape_json
                .get("value")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| err("Named shape needs a name"))?;
            let id = runtime
                .registry
                .lookup(name)
                .ok_or_else(|| err(format!("unknown type {name:?}")))?;
            TypeShape::Named(id)
        }
        other => return Err(err(format!("unknown shape tag {other:?}"))),
    })
}

fn parse_ignored(
    runtime: &Runtime,
    spec: &JsonValue,
    field_name: &str,
    shape: &TypeShape,
) -> Result<IgnoredDefault, SoilError> {
    match get_tag(spec)? {
        "Const" => {
            if !is_scalar(shape) {
                return Err(err(format!(
                    "field {field_name}: Const defaults are limited to scalar shapes"
                )));
            }
            let raw = spec
                .get("value")
                .ok_or_else(|| err(format!("field {field_name}: Const needs a value")))?
                .to_string();
            let value = json::decode(runtime, shape, &raw)?;
            Ok(IgnoredDefault::Const(value))
        }
        "CopyField" => {
            let target = spec
                .get("value")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| err(format!("field {field_name}: CopyField needs a field name")))?;
            Ok(IgnoredDefault::CopyField {
                name: target.to_string(),
                index: usize::MAX, // resolved by resolve_copy_indices
            })
        }
        other => Err(err(format!(
            "field {field_name}: unknown ignored-default tag {other:?}"
        ))),
    }
}

fn resolve_copy_indices(fields: &mut [FieldDesc]) -> Result<(), SoilError> {
    let non_ignored: Vec<String> = fields
        .iter()
        .filter(|f| f.ignored.is_none())
        .map(|f| f.name.clone())
        .collect();
    for field in fields.iter_mut() {
        if let Some(IgnoredDefault::CopyField { name, index }) = &mut field.ignored {
            *index = non_ignored.iter().position(|n| n == name).ok_or_else(|| {
                err(format!(
                    "CopyField target {name:?} is not a non-ignored field"
                ))
            })?;
        }
    }
    Ok(())
}

fn is_scalar(shape: &TypeShape) -> bool {
    !matches!(
        shape,
        TypeShape::List(_) | TypeShape::Map(..) | TypeShape::Named(_) | TypeShape::Closure
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{decode, encode};
    use crate::ops;
    use crate::value::Value;

    const SAMPLE: &str = r#"[
      {"name": "Tree",
       "strategy": {"tag": "Structural"},
       "body": {"tag": "Sum", "value": [
         {"name": "Leaf", "payload": {"tag": "I64"}},
         {"name": "Node", "payload": {"tag": "List", "value": {"tag": "Named", "value": "Tree"}}}]}},
      {"name": "Doc",
       "strategy": {"tag": "Structural"},
       "body": {"tag": "Record", "value": [
         {"name": "text", "shape": {"tag": "Utf8"}, "ignored": null},
         {"name": "copy", "shape": {"tag": "Utf8"}, "ignored": {"tag": "CopyField", "value": "text"}},
         {"name": "version", "shape": {"tag": "U64"}, "ignored": {"tag": "Const", "value": 1}}]}},
      {"name": "PyHandle",
       "strategy": {"tag": "Opaque"},
       "body": {"tag": "Opaque"}}
    ]"#;

    #[test]
    fn load_recursive_and_round_trip() {
        let mut rt = Runtime::new();
        let ids = load_descriptors(&mut rt, SAMPLE).unwrap();
        assert_eq!(ids.len(), 3);
        assert_eq!(rt.registry.get(ids[0]).unwrap().name, "Tree");

        let serialized = descriptors_to_json(&rt, &ids).unwrap();
        // Round trip: loading the serialized form into a fresh runtime
        // reproduces the serialized form byte-for-byte.
        let mut rt2 = Runtime::new();
        let ids2 = load_descriptors(&mut rt2, &serialized).unwrap();
        assert_eq!(descriptors_to_json(&rt2, &ids2).unwrap(), serialized);
    }

    #[test]
    fn loaded_defaults_refill_on_decode() {
        let mut rt = Runtime::new();
        let ids = load_descriptors(&mut rt, SAMPLE).unwrap();
        let doc_shape = TypeShape::Named(ids[1]);
        let value = decode(&rt, &doc_shape, "{\"text\":\"hi\"}").unwrap();
        let Value::Record(record) = &value else {
            panic!()
        };
        // copy = CopyField("text"), version = Const(1)
        assert!(ops::eq(&rt, &record.fields[1], &record.fields[0]).unwrap());
        assert!(matches!(record.fields[2], Value::U64(1)));
        assert_eq!(encode(&rt, &value).unwrap(), "{\"text\":\"hi\"}");
    }

    #[test]
    fn unknown_named_type_rejected() {
        let mut rt = Runtime::new();
        let bad = r#"[{"name": "A", "strategy": {"tag": "Structural"},
          "body": {"tag": "Record", "value": [
            {"name": "x", "shape": {"tag": "Named", "value": "Missing"}, "ignored": null}]}}]"#;
        assert!(load_descriptors(&mut rt, bad).is_err());
    }

    #[test]
    fn const_defaults_must_be_scalar() {
        let mut rt = Runtime::new();
        let bad = r#"[{"name": "A", "strategy": {"tag": "Structural"},
          "body": {"tag": "Record", "value": [
            {"name": "xs", "shape": {"tag": "List", "value": {"tag": "I64"}},
             "ignored": {"tag": "Const", "value": []}}]}}]"#;
        assert!(load_descriptors(&mut rt, bad).is_err());
    }

    #[test]
    fn native_defaults_do_not_serialize() {
        let mut rt = Runtime::new();
        let id = rt
            .registry
            .register(TypeDesc {
                name: "N".to_string(),
                strategy: Strategy::Structural,
                body: TypeBody::Record(vec![FieldDesc {
                    name: "x".to_string(),
                    shape: TypeShape::U64,
                    ignored: Some(IgnoredDefault::Native(crate::value::Closure::native(
                        |_| Ok(Value::U64(0)),
                    ))),
                }]),
            })
            .unwrap();
        assert!(descriptors_to_json(&rt, &[id]).is_err());
    }
}
