//! The hash form (soil-syntax-spec §9.1, added 2026-08-24): the
//! canonical text with every **local binder** — equation parameters,
//! `let`/`let rec` binders, `fun` parameters, and pattern binders —
//! printed as `%N`, sites numbered in pre-order within the definition
//! and occurrences printing their binder's number. Definition names,
//! callees, types, constructors, fields, and hole names are untouched.
//! `soil_hash` (plan 03) is computed over this text; renaming a local
//! can therefore never move a hash.
//!
//! Library-only, like the other non-CLI entries (soil0-cli §12): the
//! CLI's `print` stays the display-form oracle, and this transform is
//! the one shared implementation above it. Precision notes the spec
//! leaves to the implementation: a binding construct numbers **all of
//! its binders before its subterms** (uniform for `let` and `let rec`;
//! sound because shadowing is forbidden, so a binding's value can
//! never legally reference an outer use of a sibling binder's name),
//! and a punned record-pattern field expands (`{ index, .. }` →
//! `{ index = %N, .. }`) since the binder and the field name part ways
//! under renaming. The output is not reparsable (`%` is not in the
//! grammar) — it exists to be hashed.

use crate::ast::*;
use crate::diag::{Diagnostic, Opt};
use crate::span::{Span, Spanned};

/// Parse a `.soil` file and return its hash-form text.
pub fn hash_form_file(src: &str, filename: &str) -> Result<String, Diagnostic> {
    let mut file = crate::parser::parse_file(src, filename)?;
    for def in &mut file.defs {
        rename_def(&mut def.item);
    }
    Ok(crate::print::print_file(&file))
}

struct Renamer {
    next: usize,
    /// Scope stack: original name → site number.
    env: Vec<(String, usize)>,
}

impl Renamer {
    fn bind(&mut self, binder: &mut Binder) {
        let n = self.next;
        self.next += 1;
        self.env.push((binder.name.clone(), n));
        binder.name = format!("%{n}");
    }

    fn lookup(&self, name: &str) -> Option<usize> {
        self.env
            .iter()
            .rev()
            .find(|(original, _)| original == name)
            .map(|(_, n)| *n)
    }

    fn rename_use(&self, name: &mut String) {
        if let Some(n) = self.lookup(name) {
            *name = format!("%{n}");
        }
    }

    fn mark(&self) -> usize {
        self.env.len()
    }

    fn reset(&mut self, mark: usize) {
        self.env.truncate(mark);
    }
}

fn rename_def(def: &mut Def) {
    let mut r = Renamer {
        next: 0,
        env: Vec::new(),
    };
    for p in &mut def.params {
        r.bind(p);
    }
    // The signature is untouched (§9.1 excludes refinement binders);
    // `decreases` references the parameters.
    if let Opt(Some(d)) = &mut def.decreases {
        rename_expr(&mut d.item, &mut r);
    }
    rename_expr(&mut def.body.item, &mut r);
}

fn rename_expr(e: &mut Expr, r: &mut Renamer) {
    match e {
        Expr::Let { bindings, body, .. } => {
            let mark = r.mark();
            for b in bindings.iter_mut() {
                if let Opt(Some(binder)) = &mut b.binder {
                    r.bind(binder);
                }
            }
            for b in bindings.iter_mut() {
                rename_expr(&mut b.value.item, r);
            }
            rename_expr(&mut body.item, r);
            r.reset(mark);
        }
        Expr::Fun { params, body } => {
            let mark = r.mark();
            for p in params.iter_mut() {
                r.bind(p);
            }
            rename_expr(&mut body.item, r);
            r.reset(mark);
        }
        Expr::Match { scrutinee, arms } => {
            rename_expr(&mut scrutinee.item, r);
            for arm in arms.iter_mut() {
                let mark = r.mark();
                rename_pattern(&mut arm.item.pattern.item, r);
                rename_expr(&mut arm.item.body.item, r);
                r.reset(mark);
            }
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            rename_expr(&mut cond.item, r);
            rename_expr(&mut then_branch.item, r);
            rename_expr(&mut else_branch.item, r);
        }
        Expr::OrE { lhs, rhs } | Expr::AndE { lhs, rhs } => {
            rename_expr(&mut lhs.item, r);
            rename_expr(&mut rhs.item, r);
        }
        Expr::Cmp { lhs, rhs, .. } | Expr::Arith { lhs, rhs, .. } => {
            rename_expr(&mut lhs.item, r);
            rename_expr(&mut rhs.item, r);
        }
        Expr::NotE { operand } | Expr::Neg { operand } => rename_expr(&mut operand.item, r),
        Expr::App { r#fn, arg } => {
            rename_expr(&mut r#fn.item, r);
            rename_expr(&mut arg.item, r);
        }
        Expr::Path { root, .. } => r.rename_use(root),
        Expr::RecordE { update, fields } => {
            if let Opt(Some(base)) = update {
                r.rename_use(&mut base.root);
            }
            for f in fields.iter_mut() {
                rename_expr(&mut f.value.item, r);
            }
        }
        Expr::Annot { expr, .. } => rename_expr(&mut expr.item, r),
        Expr::Qualified { .. } | Expr::CtorE { .. } | Expr::Literal { .. } | Expr::Hole { .. } => {}
    }
}

fn rename_pattern(p: &mut Pattern, r: &mut Renamer) {
    match p {
        Pattern::PWild | Pattern::PLit { .. } => {}
        Pattern::PBind { binder } => r.bind(binder),
        Pattern::PCtor { arg, .. } => {
            if let Opt(Some(inner)) = arg {
                rename_pattern(&mut inner.item, r);
            }
        }
        Pattern::PRecord { fields, .. } => {
            for f in fields.iter_mut() {
                match &mut f.pattern {
                    Opt(Some(inner)) => rename_pattern(&mut inner.item, r),
                    // Punning binds the field's name; expand so the
                    // binder renames while the field name stays.
                    punned @ Opt(None) => {
                        let mut binder = Binder {
                            span: Span {
                                start: 0,
                                end: 0,
                                line: 1,
                                col: 1,
                            },
                            name: f.name.clone(),
                        };
                        r.bind(&mut binder);
                        *punned = Opt(Some(Spanned {
                            span: binder.span,
                            item: Pattern::PBind { binder },
                        }));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(src: &str) -> String {
        hash_form_file(src, "t.soil").expect("parses")
    }

    #[test]
    fn local_renames_are_invisible() {
        // Body-local renames only: the parameter name is part of the
        // *signature* (predicates reference it — spec surface), so a
        // parameter rename is visible by design; `let`, `fun`, and
        // pattern binders are not.
        let a = form(
            "f : (xs : List F64) -> F64\nf xs =\n  let n = list_len xs in\n  \
             match xs with\n  | ys -> g n ys\n",
        );
        let b = form(
            "f : (xs : List F64) -> F64\nf xs =\n  let m = list_len xs in\n  \
             match xs with\n  | zs -> g m zs\n",
        );
        assert_eq!(a, b);
        assert!(a.contains("%0") && a.contains("%1") && a.contains("%2"));
    }

    #[test]
    fn free_names_and_signature_are_untouched() {
        let out = form("f : (xs : List F64) -> F64\nf xs = g xs\n");
        // Signature keeps the declared parameter name; the body's
        // occurrence renames; the callee does not.
        assert!(out.contains("f : (xs : List F64) -> F64"));
        assert!(out.contains("f %0 = g %0"));
    }

    #[test]
    fn punned_pattern_expands() {
        let out = form("f : Row -> U64\nf r =\n  match r with\n  | { index, .. } -> index\n");
        assert!(out.contains("index = %1"), "{out}");
    }
}
