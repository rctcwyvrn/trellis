//! The canonical encoder — this module owns every canonicality
//! guarantee (impl plan §4 step 5). Nothing else in the system may
//! produce value JSON.
//!
//! Canonical form (tr-grammar §7, micro-pins §8): no whitespace; record
//! fields in declaration order with `ignored` fields omitted; map
//! entries in comparator order; shortest-round-trip numerals via
//! `ryu`/`itoa`; the escape set of §8.1; `BigInt` hybrid by range;
//! `Bool` as JSON booleans; opaque values as `"<handle>"`; closures a
//! hard error.

use base64::Engine;

use crate::descriptor::{Strategy, TypeBody};
use crate::error::{PanicKind, SoilError};
use crate::value::Value;
use crate::Runtime;

/// Largest integer magnitude a float-only JSON parser preserves;
/// `BigInt` values beyond ±this encode as strings (design §9.2).
const BIGINT_SAFE_MAX: i64 = (1 << 53) - 1;

/// Encode a value into canonical JSON — the runtime half of `show`
/// (design §3.7). Equal values produce byte-equal output.
pub fn encode(runtime: &Runtime, value: &Value) -> Result<String, SoilError> {
    let mut out = String::new();
    encode_into(runtime, value, &mut out)?;
    Ok(out)
}

fn encode_into(runtime: &Runtime, value: &Value, out: &mut String) -> Result<(), SoilError> {
    match value {
        Value::I64(n) => push_int(*n, out),
        Value::U64(n) => push_int(*n, out),
        Value::I32(n) => push_int(*n, out),
        Value::U32(n) => push_int(*n, out),
        Value::I16(n) => push_int(*n, out),
        Value::U16(n) => push_int(*n, out),
        Value::I8(n) => push_int(*n, out),
        Value::U8(n) => push_int(*n, out),
        Value::F64(x) => push_f64(*x, out),
        Value::BigInt(b) => {
            match i64::try_from(&**b) {
                Ok(n) if n.abs() <= BIGINT_SAFE_MAX => push_int(n, out),
                _ => {
                    // Beyond ±(2^53−1): a decimal string, so the value
                    // survives float-only host JSON parsers.
                    out.push('"');
                    out.push_str(&b.to_string());
                    out.push('"');
                }
            }
        }
        Value::Utf8(s) => push_string(s, out),
        Value::Bytes(b) => {
            out.push('"');
            out.push_str(&base64::engine::general_purpose::STANDARD.encode(&**b));
            out.push('"');
        }
        Value::Unit => out.push_str("null"),
        Value::Record(record) => {
            let desc = runtime.registry.get(record.type_id)?;
            if desc.strategy == Strategy::Opaque {
                out.push_str("\"<handle>\"");
                return Ok(());
            }
            let TypeBody::Record(fields) = &desc.body else {
                return Err(SoilError::new(
                    PanicKind::TypeError,
                    format!("value is a record but type {} is not", desc.name),
                ));
            };
            if fields.len() != record.fields.len() {
                return Err(SoilError::new(
                    PanicKind::TypeError,
                    format!("record of type {} has wrong field count", desc.name),
                ));
            }
            out.push('{');
            let mut first = true;
            for (field_desc, field_value) in fields.iter().zip(record.fields.iter()) {
                if field_desc.ignored.is_some() {
                    continue;
                }
                if !first {
                    out.push(',');
                }
                first = false;
                push_string(&field_desc.name, out);
                out.push(':');
                encode_into(runtime, field_value, out)?;
            }
            out.push('}');
        }
        Value::Sum(sum) => {
            if sum.type_id == runtime.registry.bool_id() {
                // The one special case in the sum encoding (tr-grammar §7).
                out.push_str(if sum.variant == 0 { "true" } else { "false" });
                return Ok(());
            }
            let desc = runtime.registry.get(sum.type_id)?;
            if desc.strategy == Strategy::Opaque {
                out.push_str("\"<handle>\"");
                return Ok(());
            }
            let TypeBody::Sum(variants) = &desc.body else {
                return Err(SoilError::new(
                    PanicKind::TypeError,
                    format!("value is a sum but type {} is not", desc.name),
                ));
            };
            let variant = variants.get(sum.variant as usize).ok_or_else(|| {
                SoilError::new(
                    PanicKind::TypeError,
                    format!("type {} has no variant index {}", desc.name, sum.variant),
                )
            })?;
            out.push_str("{\"tag\":");
            push_string(&variant.name, out);
            match (&variant.payload, &sum.payload) {
                (Some(_), Some(payload)) => {
                    out.push_str(",\"value\":");
                    encode_into(runtime, payload, out)?;
                }
                (None, None) => {}
                _ => {
                    return Err(SoilError::new(
                        PanicKind::TypeError,
                        format!("variant {}::{} payload mismatch", desc.name, variant.name),
                    ));
                }
            }
            out.push('}');
        }
        Value::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                encode_into(runtime, item, out)?;
            }
            out.push(']');
        }
        Value::Map(map) => {
            out.push('[');
            for (i, (key, val)) in map.entries().iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str("{\"key\":");
                encode_into(runtime, key, out)?;
                out.push_str(",\"value\":");
                encode_into(runtime, val, out)?;
                out.push('}');
            }
            out.push(']');
        }
        Value::Closure(_) => {
            return Err(SoilError::new(
                PanicKind::DerivationError,
                "functions have no JSON encoding",
            ));
        }
        Value::Opaque(_) => out.push_str("\"<handle>\""),
    }
    Ok(())
}

fn push_int<T: itoa::Integer>(n: T, out: &mut String) {
    let mut buffer = itoa::Buffer::new();
    out.push_str(buffer.format(n));
}

fn push_f64(x: f64, out: &mut String) {
    if x.is_nan() {
        out.push_str("\"NaN\"");
    } else if x == f64::INFINITY {
        out.push_str("\"Inf\"");
    } else if x == f64::NEG_INFINITY {
        out.push_str("\"-Inf\"");
    } else {
        let mut buffer = ryu::Buffer::new();
        out.push_str(buffer.format(x));
    }
}

/// The canonical string escape set (micro-pin §8.1): `"`, `\`, and
/// control characters U+0000–U+001F only — short forms where they
/// exist, lowercase `\u00xx` otherwise; everything else is raw UTF-8.
pub(crate) fn push_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{FieldDesc, Strategy, TypeBody, TypeDesc, TypeShape, VariantDesc};
    use crate::value::{Closure, MapOrder, MapVal, OpaqueVal, Record, Ref, SumVal};
    use num_bigint::BigInt;

    fn rt() -> Runtime {
        Runtime::new()
    }

    fn enc(runtime: &Runtime, v: &Value) -> String {
        encode(runtime, v).unwrap()
    }

    #[test]
    fn scalars() {
        let rt = rt();
        assert_eq!(enc(&rt, &Value::I64(-42)), "-42");
        assert_eq!(enc(&rt, &Value::U64(u64::MAX)), "18446744073709551615");
        assert_eq!(enc(&rt, &Value::U8(255)), "255");
        assert_eq!(enc(&rt, &Value::Unit), "null");
    }

    #[test]
    fn floats() {
        let rt = rt();
        assert_eq!(enc(&rt, &Value::F64(1.0)), "1.0");
        assert_eq!(enc(&rt, &Value::F64(-0.0)), "-0.0");
        assert_eq!(enc(&rt, &Value::F64(0.0)), "0.0");
        assert_eq!(enc(&rt, &Value::F64(1.5e300)), "1.5e300");
        assert_eq!(enc(&rt, &Value::F64(f64::NAN)), "\"NaN\"");
        assert_eq!(enc(&rt, &Value::F64(f64::INFINITY)), "\"Inf\"");
        assert_eq!(enc(&rt, &Value::F64(f64::NEG_INFINITY)), "\"-Inf\"");
    }

    #[test]
    fn bigint_hybrid_by_range() {
        let rt = rt();
        let safe = BigInt::from((1i64 << 53) - 1);
        let beyond = BigInt::from(1i64 << 53);
        assert_eq!(enc(&rt, &Value::BigInt(Ref::new(safe))), "9007199254740991");
        assert_eq!(
            enc(&rt, &Value::BigInt(Ref::new(beyond))),
            "\"9007199254740992\""
        );
        let negative = BigInt::from(-(1i64 << 53));
        assert_eq!(
            enc(&rt, &Value::BigInt(Ref::new(negative))),
            "\"-9007199254740992\""
        );
    }

    #[test]
    fn string_escapes() {
        let rt = rt();
        let s: Ref<str> = Ref::from("a\"b\\c\nd\te\u{1}f — ok");
        assert_eq!(
            enc(&rt, &Value::Utf8(s)),
            "\"a\\\"b\\\\c\\nd\\te\\u0001f — ok\""
        );
    }

    #[test]
    fn bytes_base64() {
        let rt = rt();
        let b: Ref<[u8]> = Ref::from(&b"hello"[..]);
        assert_eq!(enc(&rt, &Value::Bytes(b)), "\"aGVsbG8=\"");
    }

    #[test]
    fn bool_is_json_booleans() {
        let rt = rt();
        let t = Value::Sum(Ref::new(SumVal {
            type_id: rt.registry.bool_id(),
            variant: 0,
            payload: None,
        }));
        let f = Value::Sum(Ref::new(SumVal {
            type_id: rt.registry.bool_id(),
            variant: 1,
            payload: None,
        }));
        assert_eq!(enc(&rt, &t), "true");
        assert_eq!(enc(&rt, &f), "false");
    }

    #[test]
    fn record_declaration_order_and_ignored_omitted() {
        let mut rt = rt();
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
                        name: "cached_word_count".to_string(),
                        shape: TypeShape::U64,
                        ignored: Some(crate::descriptor::IgnoredDefault::Native(Closure::native(
                            |_| Ok(Value::U64(0)),
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
        let value = Value::Record(Ref::new(Record {
            type_id: id,
            fields: Box::new([Value::Utf8(Ref::from("hi")), Value::U64(2), Value::I64(7)]),
        }));
        assert_eq!(enc(&rt, &value), "{\"text\":\"hi\",\"id\":7}");
    }

    #[test]
    fn sum_internally_tagged() {
        let mut rt = rt();
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
        let point = Value::Sum(Ref::new(SumVal {
            type_id: id,
            variant: 0,
            payload: None,
        }));
        let circle = Value::Sum(Ref::new(SumVal {
            type_id: id,
            variant: 1,
            payload: Some(Value::F64(2.5)),
        }));
        assert_eq!(enc(&rt, &point), "{\"tag\":\"Point\"}");
        assert_eq!(enc(&rt, &circle), "{\"tag\":\"Circle\",\"value\":2.5}");
    }

    #[test]
    fn list_and_map() {
        let rt = rt();
        let list = Value::List(Ref::new(vec![Value::I64(1), Value::I64(2)]));
        assert_eq!(enc(&rt, &list), "[1,2]");

        let mut map = MapVal::new(MapOrder::Structural);
        map.insert(&rt, Value::I64(2), Value::Utf8(Ref::from("b")))
            .unwrap();
        map.insert(&rt, Value::I64(1), Value::Utf8(Ref::from("a")))
            .unwrap();
        assert_eq!(
            enc(&rt, &Value::Map(Ref::new(map))),
            "[{\"key\":1,\"value\":\"a\"},{\"key\":2,\"value\":\"b\"}]"
        );
    }

    #[test]
    fn opaque_and_closure() {
        let mut rt = rt();
        let id = rt
            .registry
            .register(TypeDesc {
                name: "Handle".to_string(),
                strategy: Strategy::Opaque,
                body: TypeBody::Opaque,
            })
            .unwrap();
        let opaque = Value::Opaque(Ref::new(OpaqueVal {
            type_id: id,
            payload: Box::new(()),
        }));
        assert_eq!(enc(&rt, &opaque), "\"<handle>\"");

        let closure = Value::Closure(Ref::new(Closure::native(|_| Ok(Value::Unit))));
        let err = encode(&rt, &closure).unwrap_err();
        assert_eq!(err.kind, PanicKind::DerivationError);
    }

    #[test]
    fn encoding_is_deterministic() {
        let rt = rt();
        let value = Value::List(Ref::new(vec![Value::F64(0.1), Value::Utf8(Ref::from("x"))]));
        assert_eq!(enc(&rt, &value), enc(&rt, &value));
    }
}
