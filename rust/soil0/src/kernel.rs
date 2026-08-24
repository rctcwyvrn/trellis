//! The pre-registered kernel, contract §6.1, and the builtin name table,
//! contract §11 (strict-minimum inventory, resolved 2026-08-22). Names
//! and shapes here are provisional prelude surface — plan 04 adopts or
//! revises them with the user.

use crate::diag::Opt;
use crate::manifest::{FieldD, Strategy, TypeBody, TypeDef, VariantD};
use crate::types::SigType;

/// Built-in type constructors (not declarations): scalars plus `List`
/// and `Map`, with their arities.
pub fn builtin_type_arity(name: &str) -> Option<usize> {
    match name {
        "I64" | "U64" | "I32" | "U32" | "I16" | "U16" | "I8" | "U8" | "BigInt" | "F64" | "Utf8"
        | "Bytes" | "Unit" => Some(0),
        "List" => Some(1),
        "Map" => Some(2),
        _ => None,
    }
}

/// Value-level builtins reachable by bare name, with their signatures in
/// Soil type syntax (contract §11) — parsed by the ordinary type parser,
/// so the table cannot drift from the grammar.
pub const BUILTIN_SIGS: &[(&str, &str)] = &[
    ("world_fs", "World -> Fs"),
    ("world_clock", "World -> Clock"),
    ("world_rand", "World -> Rand"),
    ("fs_read_bytes", "Fs -> Path -> io (Result Bytes FsError)"),
    ("clock_now", "Clock -> io I64"),
    ("rand_u64", "Rand -> io U64"),
    ("fake_fs", "Map Utf8 Utf8 -> Fs"),
    ("fake_clock", "I64 -> Clock"),
    ("fake_rand", "U64 -> Rand"),
    ("utf8_decode", "Bytes -> Result Utf8 Utf8Error"),
    ("utf8_encode", "Utf8 -> Bytes"),
    ("unit", "Unit"),
    ("list_len", "List a -> I64"),
    ("list_nth", "List a -> I64 -> panic a"),
    ("list_empty", "List a"),
    ("list_append", "List a -> a -> List a"),
];

pub fn builtin_sig(name: &str) -> Option<&'static str> {
    BUILTIN_SIGS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, s)| *s)
}

pub fn is_builtin(name: &str) -> bool {
    builtin_sig(name).is_some()
}

/// The derived-function set reached as `Type::name` (design §3.7).
pub const DERIVED_FNS: &[&str] = &["eq", "show", "compare", "hash"];

/// Numeric primitives reached as `Type::op` (contract §11).
pub const NUMERIC_OPS: &[&str] = &["add", "sub", "mul", "div", "mod", "neg"];

pub fn is_numeric_type(name: &str) -> bool {
    matches!(
        name,
        "I64" | "U64" | "I32" | "U32" | "I16" | "U16" | "I8" | "U8" | "BigInt" | "F64"
    )
}

fn sum(name: &str, params: &[&str], variants: Vec<VariantD>) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        params: params.iter().map(|s| s.to_string()).collect(),
        strategy: Strategy::Structural,
        body: TypeBody::Sum { variants },
    }
}

fn opaque(name: &str) -> TypeDef {
    TypeDef {
        name: name.to_string(),
        params: Vec::new(),
        strategy: Strategy::Opaque,
        body: TypeBody::OpaqueBody,
    }
}

fn variant(name: &str, payload: Option<SigType>) -> VariantD {
    VariantD {
        name: name.to_string(),
        payload: Opt(payload),
        record_fields: None,
    }
}

/// A variant with an inline record payload (soil-type grammar §4.1) —
/// kernel-internal for now, not expressible in `env.json` v1 (contract
/// §13.5).
fn record_variant(name: &str, fields: Vec<(&str, SigType)>) -> VariantD {
    VariantD {
        name: name.to_string(),
        payload: Opt(None),
        record_fields: Some(
            fields
                .into_iter()
                .map(|(n, shape)| FieldD {
                    name: n.to_string(),
                    shape,
                    ignored: Opt(None),
                })
                .collect(),
        ),
    }
}

/// The kernel type declarations (contract §6.1).
pub fn kernel_typedefs() -> Vec<TypeDef> {
    let path = || SigType::con("Path");
    vec![
        // Bool is pre-registered by soil-rt as well; declared here for
        // name/variant resolution.
        sum(
            "Bool",
            &[],
            vec![variant("True", None), variant("False", None)],
        ),
        sum(
            "Option",
            &["a"],
            vec![
                variant("None", None),
                variant("Some", Some(SigType::var("a"))),
            ],
        ),
        sum(
            "Result",
            &["a", "e"],
            vec![
                variant("Ok", Some(SigType::var("a"))),
                variant("Err", Some(SigType::var("e"))),
            ],
        ),
        TypeDef {
            name: "Path".to_string(),
            params: Vec::new(),
            strategy: Strategy::Structural,
            body: TypeBody::Alias {
                ty: SigType::con("Utf8"),
            },
        },
        sum(
            "FsError",
            &[],
            vec![
                record_variant("NotFound", vec![("path", path())]),
                record_variant(
                    "ReadFailed",
                    vec![("path", path()), ("message", SigType::con("Utf8"))],
                ),
                record_variant("NotUtf8", vec![("path", path())]),
            ],
        ),
        sum(
            "Utf8Error",
            &[],
            vec![record_variant(
                "InvalidUtf8",
                vec![("at", SigType::con("U64"))],
            )],
        ),
        opaque("World"),
        opaque("Fs"),
        opaque("Net"),
        opaque("Clock"),
        opaque("Env"),
        opaque("Proc"),
        opaque("Rand"),
        opaque("Py"),
    ]
}
