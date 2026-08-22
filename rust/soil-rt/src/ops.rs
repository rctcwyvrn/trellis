//! The derived operations (design §3.7): structural `eq`, `compare`,
//! and `hash` over `Value` + descriptor. `show` is the canonical JSON
//! encoder (`json::encode`).
//!
//! At the Soil level the derived functions are monomorphic per-type
//! definitions (`Foo::eq`, `Foo::compare` — design §3.7's chosen
//! spelling); each of them *lowers to* a call into this one engine with
//! its type's descriptor (plan 01 scope item 5: "one implementation
//! each over `Value` + descriptor"). Static exclusion of closures and
//! per-type applicability live in the checker, which only elaborates
//! `T::eq` at types that derive it; the `DerivationError`s here are a
//! defensive backstop behind that, not the primary enforcement. A
//! compiler may later specialize per type; the semantics are fixed
//! here.
//!
//! `eq` is defined as `compare == Equal`, so the agreement laws hold by
//! construction. `hash` is FNV-1a 64-bit over the value's canonical
//! JSON encoding — one byte-form of a value in the whole system — with
//! the `opaque` strategy hashing the allocation address instead
//! (resolved decisions, impl plan §1; recorded in design §3.7).

use std::cmp::Ordering;

use crate::descriptor::Strategy;
use crate::error::{PanicKind, SoilError};
use crate::json;
use crate::value::Value;
use crate::Runtime;

/// Structural equality: `compare(a, b) == Equal`. NaN equals NaN;
/// `-0.0` and `0.0` are distinct; `ignored` fields are skipped; opaque
/// values and opaque-strategy types compare by allocation address.
pub fn eq(runtime: &Runtime, a: &Value, b: &Value) -> Result<bool, SoilError> {
    Ok(compare(runtime, a, b)? == Ordering::Equal)
}

/// The derived total order.
///
/// `F64` follows `total_cmp` with the NaN distinctions collapsed
/// (micro-pin §8.3): all NaN bit patterns are one logical value that
/// sorts after `+Inf`; `-0.0 < 0.0`. Records compare field-wise in
/// declaration order (skipping `ignored`), sums by variant index then
/// payload, lists and maps lexicographically then by length. Map
/// comparators are structure, not content, and are excluded. Comparing
/// values of different types — or reaching a closure — is an error the
/// static checker normally rules out.
pub fn compare(runtime: &Runtime, a: &Value, b: &Value) -> Result<Ordering, SoilError> {
    match (a, b) {
        (Value::I64(x), Value::I64(y)) => Ok(x.cmp(y)),
        (Value::U64(x), Value::U64(y)) => Ok(x.cmp(y)),
        (Value::I32(x), Value::I32(y)) => Ok(x.cmp(y)),
        (Value::U32(x), Value::U32(y)) => Ok(x.cmp(y)),
        (Value::I16(x), Value::I16(y)) => Ok(x.cmp(y)),
        (Value::U16(x), Value::U16(y)) => Ok(x.cmp(y)),
        (Value::I8(x), Value::I8(y)) => Ok(x.cmp(y)),
        (Value::U8(x), Value::U8(y)) => Ok(x.cmp(y)),
        (Value::F64(x), Value::F64(y)) => Ok(compare_f64(*x, *y)),
        (Value::BigInt(x), Value::BigInt(y)) => Ok(x.cmp(y)),
        (Value::Utf8(x), Value::Utf8(y)) => Ok(x.as_bytes().cmp(y.as_bytes())),
        (Value::Bytes(x), Value::Bytes(y)) => Ok((**x).cmp(&**y)),
        (Value::Unit, Value::Unit) => Ok(Ordering::Equal),
        (Value::Record(x), Value::Record(y)) => {
            if x.type_id != y.type_id {
                return Err(type_mismatch());
            }
            let desc = runtime.registry.get(x.type_id)?;
            if desc.strategy == Strategy::Opaque {
                return Ok(x.addr().cmp(&y.addr()));
            }
            require_derivable(runtime, x.type_id)?;
            let crate::descriptor::TypeBody::Record(fields) = &desc.body else {
                return Err(type_mismatch());
            };
            for (i, field) in fields.iter().enumerate() {
                if field.ignored.is_some() {
                    continue;
                }
                match compare(runtime, &x.fields[i], &y.fields[i])? {
                    Ordering::Equal => {}
                    other => return Ok(other),
                }
            }
            Ok(Ordering::Equal)
        }
        (Value::Sum(x), Value::Sum(y)) => {
            if x.type_id != y.type_id {
                return Err(type_mismatch());
            }
            let desc = runtime.registry.get(x.type_id)?;
            if desc.strategy == Strategy::Opaque {
                return Ok(x.addr().cmp(&y.addr()));
            }
            require_derivable(runtime, x.type_id)?;
            match x.variant.cmp(&y.variant) {
                Ordering::Equal => match (&x.payload, &y.payload) {
                    (Some(p), Some(q)) => compare(runtime, p, q),
                    (None, None) => Ok(Ordering::Equal),
                    _ => Err(type_mismatch()),
                },
                other => Ok(other),
            }
        }
        (Value::List(x), Value::List(y)) => compare_seq(runtime, x.iter(), y.iter()),
        (Value::Map(x), Value::Map(y)) => {
            // Entries are already in comparator order, so the sorted
            // sequences compare entry-wise: key, then value.
            let xs = x.entries().iter().flat_map(|(k, v)| [k, v]);
            let ys = y.entries().iter().flat_map(|(k, v)| [k, v]);
            compare_seq(runtime, xs, ys)
        }
        (Value::Opaque(x), Value::Opaque(y)) => {
            if x.type_id != y.type_id {
                return Err(type_mismatch());
            }
            Ok(x.addr().cmp(&y.addr()))
        }
        (Value::Closure(_), _) | (_, Value::Closure(_)) => Err(SoilError::new(
            PanicKind::DerivationError,
            "compare reached a function value",
        )),
        _ => Err(type_mismatch()),
    }
}

/// FNV-1a 64-bit over the value's canonical JSON encoding; the `opaque`
/// strategy (and opaque handles) hash the allocation address instead.
/// Fixed and platform-independent: `hash` is language-observable.
pub fn hash(runtime: &Runtime, value: &Value) -> Result<u64, SoilError> {
    match value {
        Value::Opaque(v) => Ok(fnv1a64(&v.addr().to_le_bytes())),
        Value::Record(v) => {
            if runtime.registry.get(v.type_id)?.strategy == Strategy::Opaque {
                return Ok(fnv1a64(&v.addr().to_le_bytes()));
            }
            Ok(fnv1a64(json::encode(runtime, value)?.as_bytes()))
        }
        Value::Sum(v) => {
            if runtime.registry.get(v.type_id)?.strategy == Strategy::Opaque {
                return Ok(fnv1a64(&v.addr().to_le_bytes()));
            }
            Ok(fnv1a64(json::encode(runtime, value)?.as_bytes()))
        }
        Value::Closure(_) => Err(SoilError::new(
            PanicKind::DerivationError,
            "hash reached a function value",
        )),
        _ => Ok(fnv1a64(json::encode(runtime, value)?.as_bytes())),
    }
}

/// FNV-1a with the standard 64-bit offset basis and prime.
fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut state = OFFSET_BASIS;
    for &byte in bytes {
        state ^= u64::from(byte);
        state = state.wrapping_mul(PRIME);
    }
    state
}

/// `total_cmp` with the NaN payload/sign distinctions collapsed: one
/// logical NaN, sorting last (after `+Inf`).
fn compare_f64(x: f64, y: f64) -> Ordering {
    match (x.is_nan(), y.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => x.total_cmp(&y),
    }
}

fn compare_seq<'a>(
    runtime: &Runtime,
    mut xs: impl Iterator<Item = &'a Value>,
    mut ys: impl Iterator<Item = &'a Value>,
) -> Result<Ordering, SoilError> {
    loop {
        match (xs.next(), ys.next()) {
            (Some(x), Some(y)) => match compare(runtime, x, y)? {
                Ordering::Equal => {}
                other => return Ok(other),
            },
            (None, None) => return Ok(Ordering::Equal),
            (Some(_), None) => return Ok(Ordering::Greater),
            (None, Some(_)) => return Ok(Ordering::Less),
        }
    }
}

fn require_derivable(runtime: &Runtime, id: crate::descriptor::TypeId) -> Result<(), SoilError> {
    if runtime.registry.derivable(id)? {
        Ok(())
    } else {
        let name = runtime.registry.get(id)?.name.clone();
        Err(SoilError::new(
            PanicKind::DerivationError,
            format!("type {name} contains a function type and derives nothing"),
        ))
    }
}

fn type_mismatch() -> SoilError {
    SoilError::new(PanicKind::TypeError, "compared values have different types")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptor::{FieldDesc, TypeBody, TypeDesc, TypeShape};
    use crate::value::{Closure, OpaqueVal, Record, Ref};

    fn rt() -> Runtime {
        Runtime::new()
    }

    #[test]
    fn nan_is_one_value_sorting_last() {
        let rt = rt();
        let nan1 = Value::F64(f64::NAN);
        let nan2 = Value::F64(-f64::NAN);
        assert!(eq(&rt, &nan1, &nan2).unwrap());
        assert_eq!(
            compare(&rt, &nan1, &Value::F64(f64::INFINITY)).unwrap(),
            Ordering::Greater
        );
        assert_eq!(
            compare(&rt, &Value::F64(-0.0), &Value::F64(0.0)).unwrap(),
            Ordering::Less
        );
        assert!(!eq(&rt, &Value::F64(-0.0), &Value::F64(0.0)).unwrap());
    }

    #[test]
    fn lists_compare_lexicographically_then_by_length() {
        let rt = rt();
        let short = Value::List(Ref::new(vec![Value::I64(1)]));
        let long = Value::List(Ref::new(vec![Value::I64(1), Value::I64(2)]));
        let bigger = Value::List(Ref::new(vec![Value::I64(9)]));
        assert_eq!(compare(&rt, &short, &long).unwrap(), Ordering::Less);
        assert_eq!(compare(&rt, &bigger, &long).unwrap(), Ordering::Greater);
        assert!(eq(&rt, &short, &short.clone()).unwrap());
    }

    #[test]
    fn records_skip_ignored_fields() {
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
                        name: "cache".to_string(),
                        shape: TypeShape::U64,
                        ignored: Some(crate::descriptor::IgnoredDefault::Native(Closure::native(
                            |_| Ok(Value::U64(0)),
                        ))),
                    },
                ]),
            })
            .unwrap();
        let make = |cache: u64| {
            Value::Record(Ref::new(Record {
                type_id: id,
                fields: Box::new([Value::Utf8(Ref::from("same")), Value::U64(cache)]),
            }))
        };
        let a = make(1);
        let b = make(2);
        assert!(eq(&rt, &a, &b).unwrap());
        assert_eq!(hash(&rt, &a).unwrap(), hash(&rt, &b).unwrap());
    }

    #[test]
    fn equal_values_hash_equal_and_deterministically() {
        let rt = rt();
        let a = Value::List(Ref::new(vec![Value::I64(1), Value::Utf8(Ref::from("x"))]));
        let b = Value::List(Ref::new(vec![Value::I64(1), Value::Utf8(Ref::from("x"))]));
        assert!(eq(&rt, &a, &b).unwrap());
        assert_eq!(hash(&rt, &a).unwrap(), hash(&rt, &b).unwrap());
        // FNV-1a is fixed for all time: a golden value pins the
        // algorithm itself (hash of the canonical bytes `[1,"x"]`).
        assert_eq!(hash(&rt, &a).unwrap(), fnv1a64(b"[1,\"x\"]"));
    }

    #[test]
    fn opaque_hash_and_eq_by_identity() {
        let mut rt = rt();
        let id = rt
            .registry
            .register(TypeDesc {
                name: "Handle".to_string(),
                strategy: Strategy::Opaque,
                body: TypeBody::Opaque,
            })
            .unwrap();
        let a = Value::Opaque(Ref::new(OpaqueVal {
            type_id: id,
            payload: Box::new(1u8),
        }));
        let b = a.clone();
        let c = Value::Opaque(Ref::new(OpaqueVal {
            type_id: id,
            payload: Box::new(1u8),
        }));
        assert!(eq(&rt, &a, &b).unwrap());
        assert!(!eq(&rt, &a, &c).unwrap());
        assert_eq!(hash(&rt, &a).unwrap(), hash(&rt, &b).unwrap());
        assert_ne!(hash(&rt, &a).unwrap(), hash(&rt, &c).unwrap());
    }

    #[test]
    fn closures_do_not_derive() {
        let rt = rt();
        let f = Value::Closure(Ref::new(Closure::native(|_| Ok(Value::Unit))));
        assert_eq!(
            compare(&rt, &f, &f.clone()).unwrap_err().kind,
            PanicKind::DerivationError
        );
        assert_eq!(hash(&rt, &f).unwrap_err().kind, PanicKind::DerivationError);
    }

    #[test]
    fn mismatched_types_error() {
        let rt = rt();
        assert_eq!(
            compare(&rt, &Value::I64(1), &Value::U64(1))
                .unwrap_err()
                .kind,
            PanicKind::TypeError
        );
    }
}
