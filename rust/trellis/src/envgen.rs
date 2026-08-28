//! The env generator (impl plan 03 step 6, contract §6): `.tr`
//! `soil-type` definitions become the soil0 type environment,
//! constructed directly against the linked library (no JSON detour;
//! the JSON shapes remain the CLI contract). Inline-record variant
//! payloads use contract v1.2's `fields` (resolved 2026-08-24).

use soil0::ast::{PExpr, Type};
use soil0::diag::Opt;
use soil0::manifest::{EnvFile, FieldD, IgnoredDefault, Strategy, TypeBody, TypeDef, VariantD};
use soil0::types::SigType;

use crate::diag::Diag;
use crate::trfile::typedef;

/// Build the environment from the root's type definitions.
pub fn env_of(typedefs: &[&typedef::TypeDef]) -> Result<EnvFile, Diag> {
    let mut types = Vec::new();
    for td in typedefs {
        types.push(lower_typedef(td)?);
    }
    Ok(EnvFile { types })
}

fn lower_typedef(td: &typedef::TypeDef) -> Result<TypeDef, Diag> {
    let strategy = match td.body {
        typedef::TypeBody::Opaque => Strategy::Opaque,
        _ => Strategy::Structural,
    };
    let body = match &td.body {
        typedef::TypeBody::Opaque => TypeBody::OpaqueBody,
        typedef::TypeBody::Record(fields) => TypeBody::Record {
            fields: lower_fields(fields)?,
        },
        typedef::TypeBody::Sum(ctors) => TypeBody::Sum {
            variants: ctors
                .iter()
                .map(|ctor| {
                    Ok(match &ctor.payload {
                        None => VariantD {
                            name: ctor.name.clone(),
                            payload: Opt(None),
                            record_fields: None,
                        },
                        Some(typedef::CtorPayload::Ty(ty)) => VariantD {
                            name: ctor.name.clone(),
                            payload: Opt(Some(sig_type(&ty.item)?)),
                            record_fields: None,
                        },
                        Some(typedef::CtorPayload::Record(fields)) => VariantD {
                            name: ctor.name.clone(),
                            payload: Opt(None),
                            record_fields: Some(lower_fields(fields)?),
                        },
                    })
                })
                .collect::<Result<_, Diag>>()?,
        },
        typedef::TypeBody::Alias(ty) => TypeBody::Alias {
            ty: sig_type(&ty.item)?,
        },
    };
    Ok(TypeDef {
        name: td.name.clone(),
        params: td.params.clone(),
        strategy,
        body,
    })
}

fn lower_fields(fields: &[typedef::Field]) -> Result<Vec<FieldD>, Diag> {
    fields
        .iter()
        .map(|f| {
            Ok(FieldD {
                name: f.name.clone(),
                shape: sig_type(&f.ty.item)?,
                ignored: Opt(match &f.ignored {
                    None => None,
                    Some(expr) => Some(ignored_default(&expr.item)?),
                }),
            })
        })
        .collect()
}

/// The `ignored` default (impl plan 01 §8.14): a constant or a copy
/// of a sibling field; general expressions wait for a consumer.
fn ignored_default(expr: &PExpr) -> Result<IgnoredDefault, Diag> {
    match expr {
        PExpr::PLiteral { lit } => Ok(IgnoredDefault::Const {
            value: literal_json(lit),
        }),
        PExpr::PPath { root, fields } if fields.is_empty() => Ok(IgnoredDefault::CopyField {
            field: root.clone(),
        }),
        _ => Err(Diag::new(
            "tr-ignored-default",
            "an `ignored` default must be a literal or a sibling field in v1 \
             (general totals arrive with the prelude)",
        )),
    }
}

fn literal_json(lit: &soil0::ast::Lit) -> serde_json::Value {
    match lit {
        soil0::ast::Lit::LInt { digits } => {
            serde_json::Value::Number(digits.parse::<i64>().map(Into::into).unwrap_or(0.into()))
        }
        soil0::ast::Lit::LFloat { text } => text
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        soil0::ast::Lit::LStr { value } => serde_json::Value::String(value.clone()),
    }
}

/// Surface type → the semantic encoding (contract §5): tyvars to
/// `SVar`, constructors to `SCon`, refinements erased to their base,
/// arrows rejected (`SArrow` is v1-rejected in environments).
pub fn sig_type(ty: &Type) -> Result<SigType, Diag> {
    match ty {
        Type::TVar { name } => Ok(SigType::var(name)),
        Type::Con { name, args } => Ok(SigType::SCon {
            name: name.clone(),
            args: args
                .iter()
                .map(|a| sig_type(&a.item))
                .collect::<Result<_, Diag>>()?,
        }),
        Type::Refined { base, .. } => sig_type(&base.item),
        Type::Arrow { .. } => Err(Diag::new(
            "tr-type-syntax",
            "function-typed components are rejected in type environments \
             (contract §6: SArrow is v1-rejected)",
        )),
    }
}
