//! Exhaustiveness and redundancy (impl plan 02 step 7): Maranget-style
//! usefulness over the match sites the checker collects, with
//! constructor sets drawn lazily from the type environment (so recursive
//! types terminate), literal patterns never exhausting their type
//! (contract §9), and witnesses rendered in surface syntax.

use crate::ast::{Lit, Pattern};
use crate::diag::{Code, Diagnostic, Note, Opt};
use crate::infer::{self, conv_sig_prog, InferOutput, MatchSite};
use crate::manifest::{Program, TypeBody};
use crate::span::Span;
use crate::types::Ty;
use std::collections::HashMap;

/// The full static pipeline behind `soil0 check`: everything `infer`
/// does, plus exhaustiveness and redundancy. Success output is
/// `infer`'s (contract §8.7).
pub fn check_program(prog: &Program) -> Result<InferOutput, Diagnostic> {
    let mut defs = Vec::new();
    for index in 0..prog.defs.len() {
        let checked = infer::check_def_all(prog, index)?;
        for site in &checked.matches {
            check_match_site(prog, &prog.defs[index].path, site)?;
        }
        defs.push(checked.info);
    }
    Ok(InferOutput { defs })
}

// ---- simplified patterns ----

#[derive(Debug, Clone, PartialEq, Eq)]
enum Pat {
    Wild,
    /// A constructor of a known head: sum variant (by index) or the
    /// single implicit constructor of a record.
    Ctor {
        key: usize,
        args: Vec<Pat>,
    },
    /// An opaque literal (never part of a complete set).
    Lit(String),
}

/// What a column's type offers for specialization.
enum Head {
    /// A sum: variant names and each variant's argument column types.
    Sum { variants: Vec<(String, Vec<Ty>)> },
    /// A record: one implicit constructor over the (non-ignored) fields.
    Rec { fields: Vec<(String, Ty)> },
    /// Only a wildcard covers it (scalars, functions, opaques, vars).
    Open,
}

fn head_of(prog: &Program, ty: &Ty) -> Head {
    let Ty::Con { name, args } = ty else {
        return Head::Open;
    };
    let Some(td) = prog.types.get(name) else {
        return Head::Open;
    };
    let subst: HashMap<String, Ty> = td.params.iter().cloned().zip(args.clone()).collect();
    match &td.body {
        TypeBody::Sum { variants } => Head::Sum {
            variants: variants
                .iter()
                .map(|v| {
                    let arg_tys = if let Opt(Some(shape)) = &v.payload {
                        vec![conv_sig_prog(prog, shape, &subst)]
                    } else if let Some(fields) = &v.record_fields {
                        fields
                            .iter()
                            .map(|f| conv_sig_prog(prog, &f.shape, &subst))
                            .collect()
                    } else {
                        Vec::new()
                    };
                    (v.name.clone(), arg_tys)
                })
                .collect(),
        },
        TypeBody::Record { fields } => Head::Rec {
            fields: fields
                .iter()
                .filter(|f| f.ignored.0.is_none())
                .map(|f| (f.name.clone(), conv_sig_prog(prog, &f.shape, &subst)))
                .collect(),
        },
        _ => Head::Open,
    }
}

/// Type-directed conversion of a checked pattern (infer has validated
/// it, so lookups are total).
fn convert(prog: &Program, pat: &Pattern, ty: &Ty) -> Pat {
    match pat {
        Pattern::PWild | Pattern::PBind { .. } => Pat::Wild,
        Pattern::PLit { lit } => Pat::Lit(match lit {
            Lit::LInt { digits } => format!("i:{digits}"),
            Lit::LFloat { text } => format!("f:{text}"),
            Lit::LStr { value } => format!("s:{value}"),
        }),
        Pattern::PCtor { name, arg, .. } => {
            let Head::Sum { variants, .. } = head_of(prog, ty) else {
                return Pat::Wild; // defensive; infer prevents this
            };
            let key = variants
                .iter()
                .position(|(n, _)| n == name)
                .expect("checked variant");
            let (_, arg_tys) = &variants[key];
            let args = match (&arg.0, arg_tys.len()) {
                (_, 0) => Vec::new(),
                (Some(sub), 1) if !is_record_expansion(prog, ty, key) => {
                    vec![convert(prog, &sub.item, &arg_tys[0])]
                }
                (Some(sub), _) => {
                    // Inline record payload: expand by the variant's
                    // declared field names.
                    let names = variant_field_names(prog, ty, key);
                    match &sub.item {
                        Pattern::PRecord { fields, .. } => names
                            .iter()
                            .zip(arg_tys)
                            .map(|(n, t)| field_pat(prog, fields, n, t))
                            .collect(),
                        _ => arg_tys.iter().map(|_| Pat::Wild).collect(),
                    }
                }
                (None, _) => arg_tys.iter().map(|_| Pat::Wild).collect(),
            };
            Pat::Ctor { key, args }
        }
        Pattern::PRecord { fields, .. } => {
            let Head::Rec { fields: rec_fields } = head_of(prog, ty) else {
                return Pat::Wild;
            };
            let args = rec_fields
                .iter()
                .map(|(fname, fty)| field_pat(prog, fields, fname, fty))
                .collect();
            Pat::Ctor { key: 0, args }
        }
    }
}

fn is_record_expansion(prog: &Program, ty: &Ty, variant: usize) -> bool {
    let Ty::Con { name, .. } = ty else {
        return false;
    };
    let Some(td) = prog.types.get(name) else {
        return false;
    };
    let TypeBody::Sum { variants } = &td.body else {
        return false;
    };
    variants[variant].record_fields.is_some()
}

fn variant_field_names(prog: &Program, ty: &Ty, variant: usize) -> Vec<String> {
    let Ty::Con { name, .. } = ty else {
        return Vec::new();
    };
    let Some(td) = prog.types.get(name) else {
        return Vec::new();
    };
    let TypeBody::Sum { variants } = &td.body else {
        return Vec::new();
    };
    variants[variant]
        .record_fields
        .as_ref()
        .map(|fs| fs.iter().map(|f| f.name.clone()).collect())
        .unwrap_or_default()
}

fn field_pat(prog: &Program, fps: &[crate::ast::FieldPat], name: &str, ty: &Ty) -> Pat {
    match fps.iter().find(|fp| fp.name == name) {
        Some(fp) => match &fp.pattern {
            Opt(Some(sub)) => convert(prog, &sub.item, ty),
            Opt(None) => Pat::Wild, // punning binds
        },
        None => Pat::Wild, // omitted under `..`
    }
}

// ---- usefulness with witnesses ----

#[derive(Debug, Clone)]
enum Wit {
    Any,
    Ctor { render: String },
}

/// Is `q` useful w.r.t. `matrix`? Returns a witness row when it is.
fn useful(prog: &Program, matrix: &[Vec<Pat>], q: &[Pat], tys: &[Ty]) -> Option<Vec<Wit>> {
    if tys.is_empty() {
        return if matrix.is_empty() {
            Some(Vec::new())
        } else {
            None
        };
    }
    let ty0 = &tys[0];
    match &q[0] {
        Pat::Ctor { key, args } => {
            let (sub_matrix, sub_tys) = specialize(prog, matrix, ty0, *key, args.len(), &tys[1..]);
            let mut sub_q: Vec<Pat> = args.clone();
            sub_q.extend_from_slice(&q[1..]);
            useful(prog, &sub_matrix, &sub_q, &sub_tys).map(|mut w| {
                let arg_wits: Vec<Wit> = w.drain(..args.len()).collect();
                let mut out = vec![render_ctor(prog, ty0, *key, &arg_wits)];
                out.extend(w);
                out
            })
        }
        Pat::Lit(l) => {
            // Rows with the same literal or a wildcard.
            let sub_matrix: Vec<Vec<Pat>> = matrix
                .iter()
                .filter(|row| matches!(&row[0], Pat::Wild) || row[0] == Pat::Lit(l.clone()))
                .map(|row| row[1..].to_vec())
                .collect();
            useful(prog, &sub_matrix, &q[1..], &tys[1..]).map(|mut w| {
                let mut out = vec![Wit::Ctor {
                    render: render_lit(l),
                }];
                out.append(&mut w);
                out
            })
        }
        Pat::Wild => {
            let head = head_of(prog, ty0);
            let complete: Option<Vec<(usize, usize)>> = match &head {
                Head::Sum { variants } => {
                    let present: Vec<usize> = (0..variants.len())
                        .filter(|k| {
                            matrix
                                .iter()
                                .any(|row| matches!(&row[0], Pat::Ctor { key, .. } if key == k))
                        })
                        .collect();
                    if present.len() == variants.len() {
                        Some(
                            variants
                                .iter()
                                .enumerate()
                                .map(|(k, (_, a))| (k, a.len()))
                                .collect(),
                        )
                    } else {
                        None
                    }
                }
                Head::Rec { fields } => {
                    if matrix.iter().any(|row| matches!(&row[0], Pat::Ctor { .. })) {
                        Some(vec![(0, fields.len())])
                    } else {
                        None
                    }
                }
                Head::Open => None,
            };
            match complete {
                Some(ctors) => {
                    for (key, arity) in ctors {
                        let (sub_matrix, sub_tys) =
                            specialize(prog, matrix, ty0, key, arity, &tys[1..]);
                        let mut sub_q = vec![Pat::Wild; arity];
                        sub_q.extend_from_slice(&q[1..]);
                        if let Some(mut w) = useful(prog, &sub_matrix, &sub_q, &sub_tys) {
                            let arg_wits: Vec<Wit> = w.drain(..arity).collect();
                            let mut out = vec![render_ctor(prog, ty0, key, &arg_wits)];
                            out.extend(w);
                            return Some(out);
                        }
                    }
                    None
                }
                None => {
                    // Default matrix: rows whose head is a wildcard.
                    let sub_matrix: Vec<Vec<Pat>> = matrix
                        .iter()
                        .filter(|row| matches!(&row[0], Pat::Wild))
                        .map(|row| row[1..].to_vec())
                        .collect();
                    useful(prog, &sub_matrix, &q[1..], &tys[1..]).map(|mut w| {
                        let wit0 = missing_witness(&head, matrix);
                        let mut out = vec![wit0];
                        out.append(&mut w);
                        out
                    })
                }
            }
        }
    }
}

/// Specialize the matrix by constructor `key` of `ty0`.
fn specialize(
    prog: &Program,
    matrix: &[Vec<Pat>],
    ty0: &Ty,
    key: usize,
    arity: usize,
    rest_tys: &[Ty],
) -> (Vec<Vec<Pat>>, Vec<Ty>) {
    let arg_tys: Vec<Ty> = match head_of(prog, ty0) {
        Head::Sum { variants, .. } => variants[key].1.clone(),
        Head::Rec { fields } => fields.into_iter().map(|(_, t)| t).collect(),
        Head::Open => vec![],
    };
    let mut sub_tys = arg_tys;
    sub_tys.extend_from_slice(rest_tys);
    let sub_matrix = matrix
        .iter()
        .filter_map(|row| match &row[0] {
            Pat::Ctor { key: k, args } if *k == key => {
                let mut r = args.clone();
                r.extend_from_slice(&row[1..]);
                Some(r)
            }
            Pat::Wild => {
                let mut r = vec![Pat::Wild; arity];
                r.extend_from_slice(&row[1..]);
                Some(r)
            }
            _ => None,
        })
        .collect();
    (sub_matrix, sub_tys)
}

/// A witness head for an incomplete column: the first missing
/// constructor, or `_` for open types.
fn missing_witness(head: &Head, matrix: &[Vec<Pat>]) -> Wit {
    match head {
        Head::Sum { variants } => {
            for (k, (name, arg_tys)) in variants.iter().enumerate() {
                let present = matrix
                    .iter()
                    .any(|row| matches!(&row[0], Pat::Ctor { key, .. } if *key == k));
                if !present {
                    let render = if arg_tys.is_empty() {
                        name.clone()
                    } else {
                        format!("{name} _")
                    };
                    return Wit::Ctor { render };
                }
            }
            Wit::Any
        }
        _ => Wit::Any,
    }
}

fn render_lit(l: &str) -> String {
    match l.split_once(':') {
        Some(("s", v)) => format!("{v:?}"),
        Some((_, v)) => v.to_string(),
        None => l.to_string(),
    }
}

fn render_ctor(prog: &Program, ty0: &Ty, key: usize, args: &[Wit]) -> Wit {
    match head_of(prog, ty0) {
        Head::Sum { variants } => {
            let (name, arg_tys) = &variants[key];
            let render = if arg_tys.is_empty() {
                name.clone()
            } else if args.iter().all(|a| matches!(a, Wit::Any)) {
                format!("{name} _")
            } else if arg_tys.len() == 1 {
                format!("{name} ({})", render_wit(&args[0]))
            } else {
                format!("{name} {{ .. }}")
            };
            Wit::Ctor { render }
        }
        Head::Rec { fields } => {
            let inner: Vec<String> = fields
                .iter()
                .zip(args)
                .map(|((n, _), w)| format!("{n} = {}", render_wit(w)))
                .collect();
            Wit::Ctor {
                render: format!("{{ {} }}", inner.join(", ")),
            }
        }
        Head::Open => Wit::Any,
    }
}

fn render_wit(w: &Wit) -> String {
    match w {
        Wit::Any => "_".to_string(),
        Wit::Ctor { render } => render.clone(),
    }
}

// ---- per-match checking ----

pub fn check_match_site(prog: &Program, file: &str, site: &MatchSite) -> Result<(), Diagnostic> {
    let tys = vec![site.scrutinee.clone()];
    let rows: Vec<Vec<Pat>> = site
        .arms
        .iter()
        .map(|p| vec![convert(prog, &p.item, &site.scrutinee)])
        .collect();

    // Redundancy: arm i must be useful against the rows above it.
    for i in 0..rows.len() {
        if useful(prog, &rows[..i], &rows[i], &tys).is_none() {
            return Err(err(
                file,
                Code::RedundantArm,
                "this arm is unreachable: earlier arms already cover it".to_string(),
                site.arms[i].span,
                vec![],
            ));
        }
    }

    // Exhaustiveness: a wildcard must not be useful against all rows.
    if let Some(wit) = useful(prog, &rows, &[Pat::Wild], &tys) {
        let rendered = render_wit(&wit[0]);
        return Err(err(
            file,
            Code::NonExhaustiveMatch,
            "this match does not cover every case".to_string(),
            site.span,
            vec![Note {
                message: format!("not covered: `{rendered}`"),
                file: Opt(None),
                span: Opt(None),
            }],
        ));
    }
    Ok(())
}

fn err(file: &str, code: Code, message: String, span: Span, notes: Vec<Note>) -> Diagnostic {
    Diagnostic {
        code,
        message,
        file: Opt(Some(file.to_string())),
        span: Opt(Some(span)),
        notes,
    }
}
