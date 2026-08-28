//! `requires` / `ensures` / `invariant` clause blocks (tr-grammar
//! §3.2): `label: predicate`, one clause per line, predicates parsed
//! through the linked soil0 parser (the tr-grammar §2.3 grammar has
//! exactly one implementation).

use soil0::ast::Pred;
use soil0::span::Spanned;

use crate::diag::Diag;

#[derive(Debug)]
pub struct Clause {
    pub label: String,
    pub pred: Spanned<Pred>,
}

pub fn parse_clauses(content: &str) -> Result<Vec<Clause>, Diag> {
    let mut clauses = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let (label, pred_text) = line.split_once(':').ok_or_else(|| {
            Diag::new(
                "tr-predicate-syntax",
                format!("clause `{line}` is not `label: predicate` (tr-grammar §3.2)"),
            )
        })?;
        let label = label.trim();
        if label.is_empty() {
            return Err(Diag::new(
                "tr-predicate-syntax",
                "a clause label may not be empty (tr-grammar §3.2)",
            ));
        }
        let pred = soil0::parser::parse_pred_str(pred_text.trim())
            .map_err(|d| super::soil0_syntax("tr-predicate-syntax", &d))?;
        clauses.push(Clause {
            label: label.to_string(),
            pred,
        });
    }
    if clauses.is_empty() {
        return Err(Diag::new(
            "tr-predicate-syntax",
            "a clause block needs at least one `label: predicate` line",
        ));
    }
    Ok(clauses)
}
