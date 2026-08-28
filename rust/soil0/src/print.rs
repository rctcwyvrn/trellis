//! The canonical printer (impl plan 02 step 11, design §4.8): exactly
//! one text form per AST, layout structural and width-independent —
//! no decision depends on line length. The checked-in examples are the
//! style oracle: printing them is byte-identity (conformance test), and
//! `parse → print` is a fixpoint. Rules in soil-syntax-spec §9.

use crate::ast::*;
use crate::diag::Opt;

pub fn print_file(file: &File) -> String {
    let mut out = String::new();
    for (i, def) in file.defs.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        print_def(&mut out, &def.item);
    }
    out
}

fn print_def(out: &mut String, def: &Def) {
    out.push_str(&def.name);
    out.push_str(" : ");
    out.push_str(&ty(&def.sig.item));
    out.push('\n');
    if let Opt(Some(d)) = &def.decreases {
        out.push_str("decreases ");
        out.push_str(&inline(&d.item));
        out.push('\n');
    }
    out.push_str(&def.name);
    for p in &def.params {
        out.push(' ');
        out.push_str(&p.name);
    }
    out.push_str(" =");
    if is_block(&def.body.item) {
        out.push('\n');
        block(out, &def.body.item, 1);
    } else {
        out.push(' ');
        out.push_str(&inline(&def.body.item));
    }
    out.push('\n');
}

// ---- types ----

fn ty(t: &Type) -> String {
    match t {
        Type::Arrow {
            param,
            dom,
            row,
            cod,
        } => {
            let dom_s = match &param.0 {
                Some(b) => format!("({} : {})", b.name, ty(&dom.item)),
                None => {
                    // Left-nested arrows and refinements need parens; a
                    // refinement is already self-delimiting.
                    let s = ty(&dom.item);
                    if matches!(dom.item, Type::Arrow { .. }) {
                        format!("({s})")
                    } else {
                        s
                    }
                }
            };
            let row_s = row_str(row);
            let cod_s = {
                let s = ty(&cod.item);
                // After a row, an applied type is parenthesized (the
                // examples' style; grammar-optional).
                if !row_s.is_empty()
                    && matches!(&cod.item, Type::Con { args, .. } if !args.is_empty())
                {
                    format!("({s})")
                } else {
                    s
                }
            };
            format!("{dom_s} -> {row_s}{cod_s}")
        }
        Type::Con { name, args } => {
            let mut s = name.clone();
            for a in args {
                s.push(' ');
                s.push_str(&atype(&a.item));
            }
            s
        }
        Type::TVar { name } => name.clone(),
        Type::Refined { binder, base, pred } => {
            format!(
                "{{{} : {} | {}}}",
                binder.name,
                ty(&base.item),
                pred_str(&pred.item)
            )
        }
    }
}

fn atype(t: &Type) -> String {
    match t {
        Type::Con { args, .. } if !args.is_empty() => format!("({})", ty(t)),
        Type::Arrow { .. } => format!("({})", ty(t)),
        _ => ty(t),
    }
}

fn row_str(row: &Row) -> String {
    let mut s = String::new();
    for e in &row.effects {
        s.push_str(match e {
            Effect::Div => "div ",
            Effect::Panic => "panic ",
            Effect::Io => "io ",
            Effect::Ffi => "ffi ",
        });
    }
    if let Opt(Some(v)) = &row.var {
        s.push_str(v);
        s.push(' ');
    }
    s
}

// ---- predicates ----

fn pred_str(p: &Pred) -> String {
    pred_prec(p, 0)
}

fn pred_prec(p: &Pred, level: u8) -> String {
    // Levels: 0 implies, 1 or, 2 and, 3 not/atom.
    let (s, mine) = match p {
        Pred::Implies { lhs, rhs } => (
            format!(
                "{} implies {}",
                pred_prec(&lhs.item, 1),
                pred_prec(&rhs.item, 0)
            ),
            0,
        ),
        Pred::POr { lhs, rhs } => (
            format!("{} or {}", pred_prec(&lhs.item, 1), pred_prec(&rhs.item, 2)),
            1,
        ),
        Pred::PAnd { lhs, rhs } => (
            format!(
                "{} and {}",
                pred_prec(&lhs.item, 2),
                pred_prec(&rhs.item, 3)
            ),
            2,
        ),
        Pred::PNot { pred } => (format!("not {}", pred_prec(&pred.item, 3)), 3),
        Pred::PCmp { op, lhs, rhs } => (
            format!("{} {} {}", pexpr(&lhs.item), cmp_op(*op), pexpr(&rhs.item)),
            3,
        ),
        Pred::Is {
            scrutinee,
            type_name,
            ctor,
            payload,
        } => {
            let q = match &type_name.0 {
                Some(t) => format!("{t}::"),
                None => String::new(),
            };
            let b = match &payload.0 {
                Some(binder) => format!("({})", binder.name),
                None => String::new(),
            };
            (format!("{} is {q}{ctor}{b}", pexpr(&scrutinee.item)), 3)
        }
        Pred::PCall { name, args } => {
            let mut s = name.clone();
            for a in args {
                s.push(' ');
                s.push_str(&pexpr_atom(&a.item));
            }
            (s, 3)
        }
    };
    if mine < level {
        format!("({s})")
    } else {
        s
    }
}

fn pexpr(e: &PExpr) -> String {
    match e {
        PExpr::PArith { op, lhs, rhs } => {
            format!(
                "{} {} {}",
                pexpr(&lhs.item),
                arith_op(*op),
                pexpr_arg(&rhs.item)
            )
        }
        _ => pexpr_arg(e),
    }
}

fn pexpr_arg(e: &PExpr) -> String {
    match e {
        PExpr::PArith { .. } => format!("({})", pexpr(e)),
        PExpr::PCallE { name, args } => {
            let mut s = name.clone();
            for a in args {
                s.push(' ');
                s.push_str(&pexpr_atom(&a.item));
            }
            s
        }
        _ => pexpr_atom(e),
    }
}

fn pexpr_atom(e: &PExpr) -> String {
    match e {
        PExpr::PLiteral { lit } => lit_str(lit),
        PExpr::PPath { root, fields } => {
            let mut s = root.clone();
            for f in fields {
                s.push('.');
                s.push_str(f);
            }
            s
        }
        _ => format!("({})", pexpr(e)),
    }
}

// ---- expressions ----

/// Blocks lay out multi-line in statement positions; everything else is
/// inline.
fn is_block(e: &Expr) -> bool {
    matches!(e, Expr::Let { .. } | Expr::If { .. } | Expr::Match { .. })
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

/// Multi-line layout at `level` (statement position).
fn block(out: &mut String, e: &Expr, level: usize) {
    match e {
        Expr::Let {
            is_rec,
            bindings,
            body,
        } => {
            for b in bindings {
                indent(out, level);
                out.push_str("let ");
                if *is_rec {
                    out.push_str("rec ");
                }
                match &b.binder.0 {
                    Some(binder) => out.push_str(&binder.name),
                    None => out.push('_'),
                }
                out.push_str(" =");
                if is_block(&b.value.item) {
                    out.push('\n');
                    block(out, &b.value.item, level + 1);
                    out.push('\n');
                    indent(out, level);
                    out.push_str("in\n");
                } else if let Expr::Fun { params, body: fb } = &b.value.item {
                    if is_block(&fb.item) {
                        out.push_str(" fun");
                        for p in params {
                            out.push(' ');
                            out.push_str(&p.name);
                        }
                        out.push_str(" ->\n");
                        block(out, &fb.item, level + 1);
                        out.push('\n');
                        indent(out, level);
                        out.push_str("in\n");
                    } else {
                        out.push(' ');
                        out.push_str(&inline(&b.value.item));
                        out.push_str(" in\n");
                    }
                } else {
                    out.push(' ');
                    out.push_str(&inline(&b.value.item));
                    out.push_str(" in\n");
                }
            }
            block_or_inline(out, &body.item, level);
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            indent(out, level);
            out.push_str("if ");
            out.push_str(&inline(&cond.item));
            out.push('\n');
            indent(out, level);
            out.push_str("then");
            branch(out, &then_branch.item, level);
            out.push('\n');
            indent(out, level);
            out.push_str("else");
            branch(out, &else_branch.item, level);
        }
        Expr::Match { scrutinee, arms } => {
            indent(out, level);
            out.push_str("match ");
            out.push_str(&inline(&scrutinee.item));
            out.push_str(" with");
            for arm in arms {
                out.push('\n');
                indent(out, level);
                out.push_str("| ");
                out.push_str(&pattern(&arm.item.pattern.item));
                out.push_str(" ->");
                if is_block(&arm.item.body.item) {
                    out.push('\n');
                    block(out, &arm.item.body.item, level + 1);
                } else {
                    out.push(' ');
                    out.push_str(&inline(&arm.item.body.item));
                }
            }
        }
        _ => {
            indent(out, level);
            out.push_str(&inline(e));
        }
    }
}

fn block_or_inline(out: &mut String, e: &Expr, level: usize) {
    if is_block(e) {
        block(out, e, level);
    } else {
        indent(out, level);
        out.push_str(&inline(e));
    }
}

fn branch(out: &mut String, e: &Expr, level: usize) {
    if is_block(e) {
        out.push('\n');
        block(out, e, level + 1);
    } else {
        out.push(' ');
        out.push_str(&inline(e));
    }
}

/// Precedence levels for inline printing, mirroring syntax-spec §4.
fn prec(e: &Expr) -> u8 {
    match e {
        Expr::Let { .. } | Expr::Fun { .. } | Expr::If { .. } | Expr::Match { .. } => 0,
        Expr::OrE { .. } => 1,
        Expr::AndE { .. } => 2,
        Expr::NotE { .. } => 3,
        Expr::Cmp { .. } => 4,
        Expr::Arith {
            op: ArithOp::Add | ArithOp::Sub,
            ..
        } => 5,
        Expr::Arith { .. } => 6,
        Expr::Neg { .. } => 7,
        Expr::App { .. } => 8,
        _ => 9,
    }
}

fn inline(e: &Expr) -> String {
    inline_prec(e, 0)
}

fn inline_prec(e: &Expr, level: u8) -> String {
    let mine = prec(e);
    let s = match e {
        Expr::Let {
            is_rec,
            bindings,
            body,
        } => {
            let mut s = String::new();
            for (i, b) in bindings.iter().enumerate() {
                s.push_str(if i == 0 {
                    if *is_rec {
                        "let rec "
                    } else {
                        "let "
                    }
                } else {
                    " and "
                });
                match &b.binder.0 {
                    Some(binder) => s.push_str(&binder.name),
                    None => s.push('_'),
                }
                s.push_str(" = ");
                s.push_str(&inline(&b.value.item));
            }
            s.push_str(" in ");
            s.push_str(&inline(&body.item));
            s
        }
        Expr::Fun { params, body } => {
            let mut s = "fun".to_string();
            for p in params {
                s.push(' ');
                s.push_str(&p.name);
            }
            s.push_str(" -> ");
            s.push_str(&inline(&body.item));
            s
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => format!(
            "if {} then {} else {}",
            inline(&cond.item),
            inline(&then_branch.item),
            inline(&else_branch.item)
        ),
        Expr::Match { scrutinee, arms } => {
            let mut s = format!("match {} with", inline(&scrutinee.item));
            for arm in arms {
                s.push_str(" | ");
                s.push_str(&pattern(&arm.item.pattern.item));
                s.push_str(" -> ");
                s.push_str(&inline_prec(&arm.item.body.item, 1));
            }
            s
        }
        Expr::OrE { lhs, rhs } => format!(
            "{} or {}",
            inline_prec(&lhs.item, 1),
            inline_prec(&rhs.item, 2)
        ),
        Expr::AndE { lhs, rhs } => format!(
            "{} and {}",
            inline_prec(&lhs.item, 2),
            inline_prec(&rhs.item, 3)
        ),
        Expr::NotE { operand } => format!("not {}", inline_prec(&operand.item, 3)),
        Expr::Cmp { op, lhs, rhs } => format!(
            "{} {} {}",
            inline_prec(&lhs.item, 5),
            cmp_op(*op),
            inline_prec(&rhs.item, 5)
        ),
        Expr::Arith {
            op: op @ (ArithOp::Add | ArithOp::Sub),
            lhs,
            rhs,
        } => format!(
            "{} {} {}",
            inline_prec(&lhs.item, 5),
            arith_op(*op),
            inline_prec(&rhs.item, 6)
        ),
        Expr::Arith { op, lhs, rhs } => format!(
            "{} {} {}",
            inline_prec(&lhs.item, 6),
            arith_op(*op),
            inline_prec(&rhs.item, 7)
        ),
        // A negated negation must parenthesize: attached `--` would
        // lex as a comment, so `-(-x)` is the one reparseable form
        // (minimal-parens rule: parens exactly where reparsing would
        // otherwise change — or lose — the tree).
        Expr::Neg { operand } if matches!(operand.item, Expr::Neg { .. }) => {
            format!("-({})", inline(&operand.item))
        }
        Expr::Neg { operand } => format!("-{}", inline_prec(&operand.item, 7)),
        Expr::App { r#fn, arg } => format!(
            "{} {}",
            inline_prec(&r#fn.item, 8),
            inline_prec(&arg.item, 9)
        ),
        Expr::Path { root, fields } => {
            let mut s = root.clone();
            for f in fields {
                s.push('.');
                s.push_str(f);
            }
            s
        }
        Expr::Qualified { space, name } => format!("{space}::{name}"),
        Expr::CtorE { name } => name.clone(),
        Expr::RecordE { update, fields } => {
            let mut s = "{ ".to_string();
            if let Opt(Some(base)) = update {
                s.push_str(&base.root);
                for f in &base.fields {
                    s.push('.');
                    s.push_str(f);
                }
                s.push_str(" with ");
            }
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(&f.name);
                s.push_str(" = ");
                s.push_str(&inline(&f.value.item));
            }
            s.push_str(" }");
            s
        }
        Expr::Literal { lit } => lit_str(lit),
        Expr::Annot { expr, ty: t } => {
            format!("({} : {})", inline(&expr.item), ty(&t.item))
        }
        Expr::Hole { name } => format!("?{name}"),
    };
    // Annot and records/holes are self-delimiting; Annot handled above.
    if mine < level && !matches!(e, Expr::Annot { .. }) {
        format!("({s})")
    } else {
        s
    }
}

fn cmp_op(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "==",
        CmpOp::Ne => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    }
}

fn arith_op(op: ArithOp) -> &'static str {
    match op {
        ArithOp::Add => "+",
        ArithOp::Sub => "-",
        ArithOp::Mul => "*",
        ArithOp::Div => "/",
        ArithOp::Mod => "%",
    }
}

fn lit_str(lit: &Lit) -> String {
    match lit {
        Lit::LInt { digits } => digits.clone(),
        Lit::LFloat { text } => text.clone(),
        Lit::LStr { value } => {
            let mut s = "\"".to_string();
            for c in value.chars() {
                match c {
                    '"' => s.push_str("\\\""),
                    '\\' => s.push_str("\\\\"),
                    '\n' => s.push_str("\\n"),
                    '\r' => s.push_str("\\r"),
                    '\t' => s.push_str("\\t"),
                    c if (c as u32) < 0x20 => s.push_str(&format!("\\u{{{:x}}}", c as u32)),
                    c => s.push(c),
                }
            }
            s.push('"');
            s
        }
    }
}

fn pattern(p: &Pattern) -> String {
    match p {
        Pattern::PWild => "_".to_string(),
        Pattern::PBind { binder } => binder.name.clone(),
        Pattern::PLit { lit } => lit_str(lit),
        Pattern::PCtor {
            type_name,
            name,
            arg,
        } => {
            let q = match &type_name.0 {
                Some(t) => format!("{t}::"),
                None => String::new(),
            };
            match &arg.0 {
                None => format!("{q}{name}"),
                Some(sub) => format!("{q}{name} {}", pat_atom(&sub.item)),
            }
        }
        Pattern::PRecord { fields, open } => {
            let mut s = "{ ".to_string();
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                s.push_str(&f.name);
                if let Opt(Some(sub)) = &f.pattern {
                    s.push_str(" = ");
                    s.push_str(&pattern(&sub.item));
                }
            }
            if *open {
                s.push_str(", ..");
            }
            s.push_str(" }");
            s
        }
    }
}

fn pat_atom(p: &Pattern) -> String {
    match p {
        Pattern::PCtor {
            arg: Opt(Some(_)), ..
        } => format!("({})", pattern(p)),
        _ => pattern(p),
    }
}

/// Convenience for tests and the CLI: parse + print.
pub fn canonicalize(src: &str, filename: &str) -> Result<String, crate::diag::Diagnostic> {
    Ok(print_file(&crate::parser::parse_file(src, filename)?))
}
