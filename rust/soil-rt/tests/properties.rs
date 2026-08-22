//! Property tests (impl plan §4 step 6): JSON round-trip identity,
//! encode determinism, eq/hash agreement, and the compare total-order
//! laws, over randomly generated well-typed values.
//!
//! Shapes here exclude `Named` (records and sums are covered by the
//! fixtures corpus and unit tests); that keeps structural compare
//! registry-free, so map values can be built inside strategies with a
//! throwaway runtime.

use std::cmp::Ordering;

use num_bigint::BigInt;
use proptest::collection::vec;
use proptest::prelude::*;
use soil_rt::{json, ops, MapOrder, MapVal, Ref, Runtime, TypeShape, Value};

fn scalar_shape() -> impl Strategy<Value = TypeShape> {
    prop_oneof![
        Just(TypeShape::I64),
        Just(TypeShape::U64),
        Just(TypeShape::I32),
        Just(TypeShape::U32),
        Just(TypeShape::I16),
        Just(TypeShape::U16),
        Just(TypeShape::I8),
        Just(TypeShape::U8),
        Just(TypeShape::F64),
        Just(TypeShape::Utf8),
        Just(TypeShape::Bytes),
        Just(TypeShape::Unit),
        Just(TypeShape::BigInt),
    ]
}

fn arb_shape() -> impl Strategy<Value = TypeShape> {
    scalar_shape().prop_recursive(3, 16, 4, |inner| {
        prop_oneof![
            inner
                .clone()
                .prop_map(|elem| TypeShape::List(Box::new(elem))),
            (scalar_shape(), inner).prop_map(|(k, v)| TypeShape::Map(Box::new(k), Box::new(v))),
        ]
    })
}

fn arb_value(shape: &TypeShape) -> BoxedStrategy<Value> {
    match shape {
        TypeShape::I64 => any::<i64>().prop_map(Value::I64).boxed(),
        TypeShape::U64 => any::<u64>().prop_map(Value::U64).boxed(),
        TypeShape::I32 => any::<i32>().prop_map(Value::I32).boxed(),
        TypeShape::U32 => any::<u32>().prop_map(Value::U32).boxed(),
        TypeShape::I16 => any::<i16>().prop_map(Value::I16).boxed(),
        TypeShape::U16 => any::<u16>().prop_map(Value::U16).boxed(),
        TypeShape::I8 => any::<i8>().prop_map(Value::I8).boxed(),
        TypeShape::U8 => any::<u8>().prop_map(Value::U8).boxed(),
        TypeShape::F64 => prop_oneof![
            8 => any::<f64>(),
            1 => Just(f64::NAN),
            1 => Just(f64::INFINITY),
            1 => Just(f64::NEG_INFINITY),
            1 => Just(-0.0),
            1 => Just(0.0),
        ]
        .prop_map(Value::F64)
        .boxed(),
        TypeShape::Utf8 => any::<String>()
            .prop_map(|s| Value::Utf8(Ref::from(s)))
            .boxed(),
        TypeShape::Bytes => vec(any::<u8>(), 0..32)
            .prop_map(|b| Value::Bytes(Ref::from(b)))
            .boxed(),
        TypeShape::Unit => Just(Value::Unit).boxed(),
        TypeShape::BigInt => any::<i128>()
            .prop_map(|n| Value::BigInt(Ref::new(BigInt::from(n))))
            .boxed(),
        TypeShape::List(elem) => vec(arb_value(elem), 0..4)
            .prop_map(|items| Value::List(Ref::new(items)))
            .boxed(),
        TypeShape::Map(key, value) => vec((arb_value(key), arb_value(value)), 0..4)
            .prop_map(|entries| {
                // Structural compare on Named-free shapes never touches
                // the registry, so a throwaway runtime suffices.
                let rt = Runtime::new();
                let mut map = MapVal::new(MapOrder::Structural);
                for (k, v) in entries {
                    map.insert(&rt, k, v).unwrap();
                }
                Value::Map(Ref::new(map))
            })
            .boxed(),
        TypeShape::Named(_) | TypeShape::Closure => unreachable!("not generated"),
    }
}

fn shape_and_values(n: usize) -> impl Strategy<Value = (TypeShape, Vec<Value>)> {
    arb_shape().prop_flat_map(move |shape| {
        let values = vec(arb_value(&shape), n..=n);
        (Just(shape), values)
    })
}

proptest! {
    #[test]
    fn round_trip_identity_and_determinism((shape, values) in shape_and_values(1)) {
        let rt = Runtime::new();
        let value = &values[0];
        let encoded = json::encode(&rt, value).unwrap();
        prop_assert_eq!(&json::encode(&rt, value).unwrap(), &encoded);
        let decoded = json::decode(&rt, &shape, &encoded).unwrap();
        prop_assert!(ops::eq(&rt, value, &decoded).unwrap());
        prop_assert_eq!(json::encode(&rt, &decoded).unwrap(), encoded);
    }

    #[test]
    fn eq_implies_hash_agreement((_, values) in shape_and_values(2)) {
        let rt = Runtime::new();
        let (a, b) = (&values[0], &values[1]);
        prop_assert_eq!(
            ops::hash(&rt, a).unwrap(),
            ops::hash(&rt, &a.clone()).unwrap()
        );
        if ops::eq(&rt, a, b).unwrap() {
            prop_assert_eq!(ops::hash(&rt, a).unwrap(), ops::hash(&rt, b).unwrap());
        }
    }

    #[test]
    fn compare_is_a_total_order((_, values) in shape_and_values(3)) {
        let rt = Runtime::new();
        let (a, b, c) = (&values[0], &values[1], &values[2]);
        // Reflexivity (NaN included: one logical NaN).
        prop_assert_eq!(ops::compare(&rt, a, a).unwrap(), Ordering::Equal);
        // Antisymmetry.
        prop_assert_eq!(
            ops::compare(&rt, a, b).unwrap(),
            ops::compare(&rt, b, a).unwrap().reverse()
        );
        // Transitivity of <=.
        let ab = ops::compare(&rt, a, b).unwrap();
        let bc = ops::compare(&rt, b, c).unwrap();
        if ab != Ordering::Greater && bc != Ordering::Greater {
            prop_assert_ne!(ops::compare(&rt, a, c).unwrap(), Ordering::Greater);
        }
    }
}
