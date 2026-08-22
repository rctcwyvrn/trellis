//! Type-directed decode (impl plan §4 step 6).
//!
//! Parsing is borrowed from `serde_json` (into an order-preserving,
//! duplicate-rejecting tree — micro-pin §8.7); the walk from tree to
//! `Value` is owned here and is strict: unknown record fields, missing
//! non-ignored fields, *present* `ignored` fields (§8.5), wrong numeric
//! shapes, non-canonical base64 (§8.4), unknown tags, and duplicate map
//! keys are all `DecodeError`s naming the JSON path. Recursion depth is
//! bounded by `serde_json`'s parser limit, which applies while the tree
//! is built.

use std::str::FromStr;

use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::Engine;
use num_bigint::BigInt;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;

use crate::descriptor::{Strategy, TypeBody, TypeShape};
use crate::error::{PanicKind, SoilError};
use crate::value::{MapOrder, MapVal, Record, Ref, SumVal, Value};
use crate::Runtime;

/// Strict base64: standard alphabet, canonical padding required, no
/// trailing bits (micro-pin §8.4).
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::RequireCanonical),
);

/// Decode canonical (or canonically-equivalent) JSON into a value of
/// the given shape — the runtime half of `parse` (design §3.7).
pub fn decode(runtime: &Runtime, shape: &TypeShape, json: &str) -> Result<Value, SoilError> {
    let tree: Tree = serde_json::from_str(json)
        .map_err(|e| SoilError::new(PanicKind::DecodeError, format!("invalid JSON: {e}")))?;
    decode_tree(runtime, shape, &tree, "$")
}

/// An order-preserving JSON tree that rejects duplicate object keys at
/// parse time (`serde_json`'s own map type is last-wins).
enum Tree {
    Null,
    Bool(bool),
    Num(serde_json::Number),
    Str(String),
    Arr(Vec<Tree>),
    Obj(Vec<(String, Tree)>),
}

impl Tree {
    fn kind(&self) -> &'static str {
        match self {
            Tree::Null => "null",
            Tree::Bool(_) => "boolean",
            Tree::Num(_) => "number",
            Tree::Str(_) => "string",
            Tree::Arr(_) => "array",
            Tree::Obj(_) => "object",
        }
    }
}

impl<'de> Deserialize<'de> for Tree {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Tree, D::Error> {
        struct TreeVisitor;

        impl<'de> Visitor<'de> for TreeVisitor {
            type Value = Tree;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("any JSON value")
            }

            fn visit_unit<E>(self) -> Result<Tree, E> {
                Ok(Tree::Null)
            }

            fn visit_bool<E>(self, b: bool) -> Result<Tree, E> {
                Ok(Tree::Bool(b))
            }

            fn visit_i64<E>(self, n: i64) -> Result<Tree, E> {
                Ok(Tree::Num(n.into()))
            }

            fn visit_u64<E>(self, n: u64) -> Result<Tree, E> {
                Ok(Tree::Num(n.into()))
            }

            fn visit_f64<E: de::Error>(self, n: f64) -> Result<Tree, E> {
                serde_json::Number::from_f64(n)
                    .map(Tree::Num)
                    .ok_or_else(|| E::custom("non-finite number"))
            }

            fn visit_str<E>(self, s: &str) -> Result<Tree, E> {
                Ok(Tree::Str(s.to_string()))
            }

            fn visit_string<E>(self, s: String) -> Result<Tree, E> {
                Ok(Tree::Str(s))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Tree, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Tree::Arr(items))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Tree, A::Error> {
                let mut entries: Vec<(String, Tree)> = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, Tree>()? {
                    if entries.iter().any(|(k, _)| *k == key) {
                        return Err(de::Error::custom(format!("duplicate object key {key:?}")));
                    }
                    entries.push((key, value));
                }
                Ok(Tree::Obj(entries))
            }
        }

        deserializer.deserialize_any(TreeVisitor)
    }
}

fn err(path: &str, message: impl std::fmt::Display) -> SoilError {
    SoilError::new(PanicKind::DecodeError, format!("at {path}: {message}"))
}

fn decode_tree(
    runtime: &Runtime,
    shape: &TypeShape,
    tree: &Tree,
    path: &str,
) -> Result<Value, SoilError> {
    match shape {
        TypeShape::I64 => Ok(Value::I64(decode_int(tree, path, "I64")?)),
        TypeShape::U64 => Ok(Value::U64(decode_int(tree, path, "U64")?)),
        TypeShape::I32 => Ok(Value::I32(decode_int(tree, path, "I32")?)),
        TypeShape::U32 => Ok(Value::U32(decode_int(tree, path, "U32")?)),
        TypeShape::I16 => Ok(Value::I16(decode_int(tree, path, "I16")?)),
        TypeShape::U16 => Ok(Value::U16(decode_int(tree, path, "U16")?)),
        TypeShape::I8 => Ok(Value::I8(decode_int(tree, path, "I8")?)),
        TypeShape::U8 => Ok(Value::U8(decode_int(tree, path, "U8")?)),
        TypeShape::BigInt => match tree {
            Tree::Num(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(Value::BigInt(Ref::new(BigInt::from(i))))
                } else if let Some(u) = n.as_u64() {
                    Ok(Value::BigInt(Ref::new(BigInt::from(u))))
                } else {
                    Err(err(path, "BigInt number must be an exact integer"))
                }
            }
            Tree::Str(s) => BigInt::from_str(s)
                .map(|b| Value::BigInt(Ref::new(b)))
                .map_err(|_| err(path, format!("invalid BigInt string {s:?}"))),
            other => Err(err(
                path,
                format!("expected BigInt, found {}", other.kind()),
            )),
        },
        TypeShape::F64 => match tree {
            Tree::Num(n) => Ok(Value::F64(n.as_f64().expect("finite by construction"))),
            Tree::Str(s) => match s.as_str() {
                "NaN" => Ok(Value::F64(f64::NAN)),
                "Inf" => Ok(Value::F64(f64::INFINITY)),
                "-Inf" => Ok(Value::F64(f64::NEG_INFINITY)),
                _ => Err(err(path, format!("invalid F64 string {s:?}"))),
            },
            other => Err(err(path, format!("expected F64, found {}", other.kind()))),
        },
        TypeShape::Utf8 => match tree {
            Tree::Str(s) => Ok(Value::Utf8(Ref::from(s.as_str()))),
            other => Err(err(
                path,
                format!("expected string, found {}", other.kind()),
            )),
        },
        TypeShape::Bytes => match tree {
            Tree::Str(s) => BASE64
                .decode(s)
                .map(|b| Value::Bytes(Ref::from(b)))
                .map_err(|e| err(path, format!("invalid base64: {e}"))),
            other => Err(err(
                path,
                format!("expected base64 string, found {}", other.kind()),
            )),
        },
        TypeShape::Unit => match tree {
            Tree::Null => Ok(Value::Unit),
            other => Err(err(path, format!("expected null, found {}", other.kind()))),
        },
        TypeShape::List(elem) => match tree {
            Tree::Arr(items) => {
                let mut values = Vec::with_capacity(items.len());
                for (i, item) in items.iter().enumerate() {
                    values.push(decode_tree(runtime, elem, item, &format!("{path}[{i}]"))?);
                }
                Ok(Value::List(Ref::new(values)))
            }
            other => Err(err(path, format!("expected array, found {}", other.kind()))),
        },
        TypeShape::Map(key_shape, value_shape) => match tree {
            Tree::Arr(items) => {
                // Decoded maps carry the derived structural order: the
                // JSON boundary has no program closure to hand them.
                let mut map = MapVal::new(MapOrder::Structural);
                for (i, item) in items.iter().enumerate() {
                    let entry_path = format!("{path}[{i}]");
                    let Tree::Obj(entries) = item else {
                        return Err(err(&entry_path, "map entry must be an object"));
                    };
                    let mut key = None;
                    let mut value = None;
                    for (k, v) in entries {
                        match k.as_str() {
                            "key" => key = Some(v),
                            "value" => value = Some(v),
                            other => {
                                return Err(err(
                                    &entry_path,
                                    format!("unexpected map-entry key {other:?}"),
                                ));
                            }
                        }
                    }
                    let (Some(key), Some(value)) = (key, value) else {
                        return Err(err(&entry_path, "map entry needs \"key\" and \"value\""));
                    };
                    let key = decode_tree(runtime, key_shape, key, &format!("{entry_path}.key"))?;
                    let value =
                        decode_tree(runtime, value_shape, value, &format!("{entry_path}.value"))?;
                    if map.get(runtime, &key)?.is_some() {
                        return Err(err(&entry_path, "duplicate map key"));
                    }
                    map.insert(runtime, key, value)?;
                }
                Ok(Value::Map(Ref::new(map)))
            }
            other => Err(err(path, format!("expected array, found {}", other.kind()))),
        },
        TypeShape::Closure => Err(err(path, "functions have no JSON encoding")),
        TypeShape::Named(id) => {
            if *id == runtime.registry.bool_id() {
                return match tree {
                    Tree::Bool(b) => Ok(Value::Sum(Ref::new(SumVal {
                        type_id: *id,
                        variant: if *b { 0 } else { 1 },
                        payload: None,
                    }))),
                    other => Err(err(
                        path,
                        format!("expected boolean, found {}", other.kind()),
                    )),
                };
            }
            let desc = runtime.registry.get(*id)?;
            if desc.strategy == Strategy::Opaque {
                return Err(err(
                    path,
                    format!("type {} is opaque and cannot be decoded", desc.name),
                ));
            }
            match &desc.body {
                TypeBody::Record(fields) => {
                    let Tree::Obj(entries) = tree else {
                        return Err(err(path, format!("expected object, found {}", tree.kind())));
                    };
                    for (key, _) in entries {
                        match fields.iter().find(|f| f.name == *key) {
                            None => {
                                return Err(err(
                                    path,
                                    format!("unknown field {key:?} on {}", desc.name),
                                ));
                            }
                            Some(f) if f.ignored.is_some() => {
                                // Micro-pin §8.5: encode omits ignored
                                // fields, so decode rejects them — one
                                // canonical form in both directions.
                                return Err(err(
                                    path,
                                    format!(
                                        "field {key:?} on {} is ignored and must be omitted",
                                        desc.name
                                    ),
                                ));
                            }
                            Some(_) => {}
                        }
                    }
                    let mut decoded = Vec::with_capacity(fields.len());
                    for field in fields {
                        if field.ignored.is_some() {
                            decoded.push(None);
                            continue;
                        }
                        let Some((_, subtree)) = entries.iter().find(|(k, _)| *k == field.name)
                        else {
                            return Err(err(
                                path,
                                format!("missing field {:?} on {}", field.name, desc.name),
                            ));
                        };
                        decoded.push(Some(decode_tree(
                            runtime,
                            &field.shape,
                            subtree,
                            &format!("{path}.{}", field.name),
                        )?));
                    }
                    // Refill ignored fields from their default thunks,
                    // passing the non-ignored fields in declaration
                    // order (micro-pin §8.6).
                    let thunk_args: Vec<Value> = decoded.iter().flatten().cloned().collect();
                    let mut values = Vec::with_capacity(fields.len());
                    for (field, slot) in fields.iter().zip(decoded) {
                        match slot {
                            Some(value) => values.push(value),
                            None => {
                                let default = field
                                    .ignored
                                    .as_ref()
                                    .expect("slot is empty only for ignored fields");
                                values.push(default.call(&thunk_args)?);
                            }
                        }
                    }
                    Ok(Value::Record(Ref::new(Record {
                        type_id: *id,
                        fields: values.into_boxed_slice(),
                    })))
                }
                TypeBody::Sum(variants) => {
                    let Tree::Obj(entries) = tree else {
                        return Err(err(path, format!("expected object, found {}", tree.kind())));
                    };
                    let mut tag = None;
                    let mut payload_tree = None;
                    for (key, value) in entries {
                        match key.as_str() {
                            "tag" => tag = Some(value),
                            "value" => payload_tree = Some(value),
                            other => {
                                return Err(err(path, format!("unexpected key {other:?} in sum")));
                            }
                        }
                    }
                    let Some(Tree::Str(tag)) = tag else {
                        return Err(err(path, "sum needs a string \"tag\""));
                    };
                    let Some(variant_index) = variants.iter().position(|v| v.name == *tag) else {
                        return Err(err(
                            path,
                            format!("unknown variant {tag:?} of {}", desc.name),
                        ));
                    };
                    let variant = &variants[variant_index];
                    let payload = match (&variant.payload, payload_tree) {
                        (Some(shape), Some(subtree)) => Some(decode_tree(
                            runtime,
                            shape,
                            subtree,
                            &format!("{path}.value"),
                        )?),
                        (None, None) => None,
                        (Some(_), None) => {
                            return Err(err(
                                path,
                                format!("variant {tag} of {} needs a \"value\"", desc.name),
                            ));
                        }
                        (None, Some(_)) => {
                            return Err(err(
                                path,
                                format!("variant {tag} of {} takes no \"value\"", desc.name),
                            ));
                        }
                    };
                    Ok(Value::Sum(Ref::new(SumVal {
                        type_id: *id,
                        variant: u32::try_from(variant_index).expect("validated at registration"),
                        payload,
                    })))
                }
                TypeBody::Opaque => unreachable!("opaque body implies opaque strategy"),
            }
        }
    }
}

/// Decode an integer of any fixed width: the number must be an exact
/// integer (no `1.0`, no floats) and in range for the target.
fn decode_int<T: TryFrom<i128>>(tree: &Tree, path: &str, width: &str) -> Result<T, SoilError> {
    let Tree::Num(n) = tree else {
        return Err(err(
            path,
            format!("expected {width}, found {}", tree.kind()),
        ));
    };
    let wide: i128 = if let Some(i) = n.as_i64() {
        i128::from(i)
    } else if let Some(u) = n.as_u64() {
        i128::from(u)
    } else {
        return Err(err(path, format!("{width} must be an exact integer")));
    };
    T::try_from(wide).map_err(|_| err(path, format!("{n} out of range for {width}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{FieldDesc, TypeDesc, VariantDesc};
    use crate::json::encode;
    use crate::ops;
    use crate::value::Closure;

    fn doc_runtime() -> (Runtime, crate::descriptor::TypeId) {
        let mut rt = Runtime::new();
        let id = rt
            .registry
            .register(TypeDesc {
                name: "Doc".to_string(),
                strategy: Strategy::Structural,
                body: TypeBody::Record(vec![
                    FieldDesc {
                        name: "text".to_string(),
                        shape: TypeShape::Utf8,
                        ignored: None,
                    },
                    FieldDesc {
                        name: "cached_len".to_string(),
                        shape: TypeShape::U64,
                        ignored: Some(crate::descriptor::IgnoredDefault::Native(Closure::native(
                            |args| match &args[0] {
                                Value::Utf8(s) => Ok(Value::U64(s.len() as u64)),
                                _ => unreachable!(),
                            },
                        ))),
                    },
                    FieldDesc {
                        name: "id".to_string(),
                        shape: TypeShape::I64,
                        ignored: None,
                    },
                ]),
            })
            .unwrap();
        (rt, id)
    }

    #[test]
    fn scalar_round_trips_and_strictness() {
        let rt = Runtime::new();
        assert!(matches!(
            decode(&rt, &TypeShape::I64, "-42").unwrap(),
            Value::I64(-42)
        ));
        assert!(decode(&rt, &TypeShape::I64, "1.5").is_err());
        assert!(decode(&rt, &TypeShape::I64, "1.0").is_err());
        assert!(decode(&rt, &TypeShape::I64, "\"1\"").is_err());
        assert!(decode(&rt, &TypeShape::U8, "256").is_err());
        assert!(decode(&rt, &TypeShape::U8, "-1").is_err());
        assert!(matches!(
            decode(&rt, &TypeShape::Unit, "null").unwrap(),
            Value::Unit
        ));
        assert!(decode(&rt, &TypeShape::Unit, "0").is_err());
    }

    #[test]
    fn float_forms() {
        let rt = Runtime::new();
        assert!(matches!(
            decode(&rt, &TypeShape::F64, "1.5").unwrap(),
            Value::F64(x) if x == 1.5
        ));
        assert!(matches!(
            decode(&rt, &TypeShape::F64, "\"NaN\"").unwrap(),
            Value::F64(x) if x.is_nan()
        ));
        assert!(matches!(
            decode(&rt, &TypeShape::F64, "\"-Inf\"").unwrap(),
            Value::F64(x) if x == f64::NEG_INFINITY
        ));
        assert!(decode(&rt, &TypeShape::F64, "\"nan\"").is_err());
    }

    #[test]
    fn bigint_hybrid() {
        let rt = Runtime::new();
        let big = decode(&rt, &TypeShape::BigInt, "\"9007199254740992\"").unwrap();
        let reencoded = encode(&rt, &big).unwrap();
        assert_eq!(reencoded, "\"9007199254740992\"");
        let small = decode(&rt, &TypeShape::BigInt, "12").unwrap();
        assert_eq!(encode(&rt, &small).unwrap(), "12");
        // An exact integer beyond 2^53 is accepted as a number too.
        assert!(decode(&rt, &TypeShape::BigInt, "9007199254740993").is_ok());
        assert!(decode(&rt, &TypeShape::BigInt, "1.5").is_err());
        assert!(decode(&rt, &TypeShape::BigInt, "\"twelve\"").is_err());
    }

    #[test]
    fn bytes_reject_non_canonical_base64() {
        let rt = Runtime::new();
        assert!(matches!(
            decode(&rt, &TypeShape::Bytes, "\"aGVsbG8=\"").unwrap(),
            Value::Bytes(b) if &*b == b"hello"
        ));
        assert!(decode(&rt, &TypeShape::Bytes, "\"aGVsbG8\"").is_err());
        assert!(decode(&rt, &TypeShape::Bytes, "\"a GVsbG8=\"").is_err());
    }

    #[test]
    fn record_strictness_and_ignored_refill() {
        let (rt, id) = doc_runtime();
        let shape = TypeShape::Named(id);
        let value = decode(&rt, &shape, "{\"text\":\"hello\",\"id\":7}").unwrap();
        // The ignored field was refilled from its default thunk.
        let Value::Record(record) = &value else {
            panic!()
        };
        assert!(matches!(record.fields[1], Value::U64(5)));
        // Round trip: re-encoding omits the ignored field again.
        assert_eq!(
            encode(&rt, &value).unwrap(),
            "{\"text\":\"hello\",\"id\":7}"
        );
        // Field order in input JSON does not matter (objects are unordered) …
        let reordered = decode(&rt, &shape, "{\"id\":7,\"text\":\"hello\"}").unwrap();
        assert!(ops::eq(&rt, &value, &reordered).unwrap());
        // … but strictness does.
        assert!(decode(&rt, &shape, "{\"text\":\"hello\"}").is_err());
        assert!(decode(&rt, &shape, "{\"text\":\"hello\",\"id\":7,\"x\":0}").is_err());
        assert!(decode(
            &rt,
            &shape,
            "{\"text\":\"hello\",\"id\":7,\"cached_len\":5}"
        )
        .is_err());
        assert!(decode(&rt, &shape, "{\"text\":\"hello\",\"id\":7,\"id\":8}").is_err());
    }

    #[test]
    fn sum_and_bool() {
        let mut rt = Runtime::new();
        let id = rt
            .registry
            .register(TypeDesc {
                name: "Shape".to_string(),
                strategy: Strategy::Structural,
                body: TypeBody::Sum(vec![
                    VariantDesc {
                        name: "Point".to_string(),
                        payload: None,
                    },
                    VariantDesc {
                        name: "Circle".to_string(),
                        payload: Some(TypeShape::F64),
                    },
                ]),
            })
            .unwrap();
        let shape = TypeShape::Named(id);
        let circle = decode(&rt, &shape, "{\"tag\":\"Circle\",\"value\":2.5}").unwrap();
        assert_eq!(
            encode(&rt, &circle).unwrap(),
            "{\"tag\":\"Circle\",\"value\":2.5}"
        );
        assert!(decode(&rt, &shape, "{\"tag\":\"Point\"}").is_ok());
        assert!(decode(&rt, &shape, "{\"tag\":\"Square\"}").is_err());
        assert!(decode(&rt, &shape, "{\"tag\":\"Point\",\"value\":1}").is_err());
        assert!(decode(&rt, &shape, "{\"tag\":\"Circle\"}").is_err());
        assert!(decode(&rt, &shape, "{\"tag\":\"Circle\",\"value\":2.5,\"x\":0}").is_err());

        let bool_shape = TypeShape::Named(rt.registry.bool_id());
        let t = decode(&rt, &bool_shape, "true").unwrap();
        assert_eq!(encode(&rt, &t).unwrap(), "true");
        assert!(decode(&rt, &bool_shape, "{\"tag\":\"True\"}").is_err());
    }

    #[test]
    fn map_decode_sorts_structurally_and_rejects_duplicates() {
        let rt = Runtime::new();
        let shape = TypeShape::Map(Box::new(TypeShape::I64), Box::new(TypeShape::Utf8));
        let value = decode(
            &rt,
            &shape,
            "[{\"key\":2,\"value\":\"b\"},{\"key\":1,\"value\":\"a\"}]",
        )
        .unwrap();
        assert_eq!(
            encode(&rt, &value).unwrap(),
            "[{\"key\":1,\"value\":\"a\"},{\"key\":2,\"value\":\"b\"}]"
        );
        assert!(decode(
            &rt,
            &shape,
            "[{\"key\":1,\"value\":\"a\"},{\"key\":1,\"value\":\"b\"}]"
        )
        .is_err());
        assert!(decode(&rt, &shape, "[{\"key\":1}]").is_err());
        assert!(decode(&rt, &shape, "[{\"key\":1,\"value\":\"a\",\"z\":0}]").is_err());
    }

    #[test]
    fn opaque_and_closures_do_not_decode() {
        let mut rt = Runtime::new();
        let id = rt
            .registry
            .register(TypeDesc {
                name: "Handle".to_string(),
                strategy: Strategy::Opaque,
                body: TypeBody::Opaque,
            })
            .unwrap();
        assert!(decode(&rt, &TypeShape::Named(id), "\"<handle>\"").is_err());
        assert!(decode(&rt, &TypeShape::Closure, "null").is_err());
    }

    #[test]
    fn duplicate_object_keys_rejected_at_parse() {
        let (rt, id) = doc_runtime();
        let result = decode(
            &rt,
            &TypeShape::Named(id),
            "{\"text\":\"a\",\"text\":\"b\",\"id\":1}",
        );
        assert!(result.is_err());
    }
}
