//! `soil-sig` blocks (tr-grammar §3.1), parsed through the linked
//! soil0 parser so the signature grammar has exactly one
//! implementation (impl plan 03 step 3).

use soil0::ast::Type;
use soil0::span::Spanned;

use crate::diag::Diag;

#[derive(Debug)]
pub struct Sig {
    pub name: String,
    pub ty: Spanned<Type>,
}

pub fn parse(content: &str) -> Result<Sig, Diag> {
    let text = content.trim();
    let (name, ty) =
        soil0::parser::parse_sig_str(text).map_err(|d| super::soil0_syntax("tr-sig-syntax", &d))?;
    Ok(Sig { name, ty })
}

/// The declared effect rows along the arrow spine — the function's
/// row is whatever the signature's arrows carry (used by the
/// `tr-panic-without-row` rule, tr-grammar §3.3).
pub fn has_effect(ty: &Spanned<Type>, effect: soil0::ast::Effect) -> bool {
    match &ty.item {
        Type::Arrow { row, cod, .. } => row.effects.contains(&effect) || has_effect(cod, effect),
        _ => false,
    }
}

/// Whether every parameter along the arrow spine is named — required
/// when `requires`/`ensures` reference parameters (tr-grammar §3.1,
/// validity rule 8).
pub fn fully_named(ty: &Spanned<Type>) -> bool {
    match &ty.item {
        Type::Arrow { param, cod, .. } => param.0.is_some() && fully_named(cod),
        _ => true,
    }
}
