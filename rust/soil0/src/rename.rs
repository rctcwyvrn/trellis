//! The renamer, impl plan 02 step 5: scope resolution with the
//! no-shadowing rule (spec §5.1), exactly-one-spelling resolution for
//! constructors (spec §5.9) and definition names (spec §5.10),
//! `module::def` / `Type::x` qualification, `_private` visibility,
//! forward-reference detection (contract §7), and the per-definition
//! reference sets — the computed import set (contract §8.3).
//!
//! Refinement predicates are not resolved and contribute no references:
//! refinements are "parsed, retained, otherwise ignored" in soil0
//! (plan 02 scope 4); their resolution arrives with the refinement
//! checker (plan 05).

use crate::ast::*;
use crate::diag::{Code, Diagnostic, Note, Opt};
use crate::kernel;
use crate::manifest::{Program, TypeBody};
use crate::span::{Span, Spanned};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Serialize)]
pub struct RenameOutput {
    pub defs: Vec<DefRefs>,
}

#[derive(Debug, Serialize)]
pub struct DefRefs {
    pub name: String,
    pub refs: RefSets,
}

#[derive(Debug, Default, Serialize)]
pub struct RefSets {
    pub defs: Vec<String>,
    pub types: Vec<String>,
    pub builtins: Vec<String>,
    pub privates: Vec<String>,
}

pub fn rename_program(prog: &Program) -> Result<RenameOutput, Diagnostic> {
    // Definition-level prechecks: builtin collisions and same-module
    // redefinitions.
    for (i, def) in prog.defs.iter().enumerate() {
        if kernel::is_builtin(def.name()) {
            return Err(err_in(
                def.path.clone(),
                Code::Shadowing,
                format!(
                    "definition `{}` collides with the builtin of the same name",
                    def.name()
                ),
                def.ast.span,
                vec![],
            ));
        }
        for other in &prog.defs[..i] {
            if other.name() == def.name() && other.dir == def.dir {
                return Err(err_in(
                    def.path.clone(),
                    Code::Shadowing,
                    format!(
                        "`{}` is defined twice in module `{}`",
                        def.name(),
                        def.module
                    ),
                    def.ast.span,
                    vec![Note {
                        message: format!("first defined in `{}`", other.path),
                        file: Opt(Some(other.path.clone())),
                        span: Opt(Some(other.ast.span)),
                    }],
                ));
            }
        }
    }

    let mut out = Vec::new();
    for (index, _) in prog.defs.iter().enumerate() {
        let mut walker = Walker {
            prog,
            index,
            locals: Vec::new(),
            refs: Refs::default(),
            holes_seen: Vec::new(),
        };
        walker.walk_def()?;
        out.push(DefRefs {
            name: prog.defs[index].name().to_string(),
            refs: walker.refs.into_sets(),
        });
    }
    Ok(RenameOutput { defs: out })
}

#[derive(Default)]
struct Refs {
    defs: BTreeSet<String>,
    types: BTreeSet<String>,
    builtins: BTreeSet<String>,
    privates: BTreeSet<String>,
}

impl Refs {
    fn into_sets(self) -> RefSets {
        RefSets {
            defs: self.defs.into_iter().collect(),
            types: self.types.into_iter().collect(),
            builtins: self.builtins.into_iter().collect(),
            privates: self.privates.into_iter().collect(),
        }
    }
}

fn err_in(file: String, code: Code, message: String, span: Span, notes: Vec<Note>) -> Diagnostic {
    Diagnostic {
        code,
        message,
        file: Opt(Some(file)),
        span: Opt(Some(span)),
        notes,
    }
}

struct Walker<'a> {
    prog: &'a Program,
    index: usize,
    /// Local frames of (name, binding span).
    locals: Vec<Vec<(String, Span)>>,
    refs: Refs,
    /// Hole names seen in this definition (unique, syntax-spec §5.11).
    holes_seen: Vec<(String, Span)>,
}

impl Walker<'_> {
    fn here(&self) -> &crate::manifest::LoadedDef {
        &self.prog.defs[self.index]
    }

    fn err(&self, code: Code, message: String, span: Span, notes: Vec<Note>) -> Diagnostic {
        err_in(self.here().path.clone(), code, message, span, notes)
    }

    fn visible(&self, target: usize) -> bool {
        let t = &self.prog.defs[target];
        !t.is_private() || t.dir == self.here().dir
    }

    /// Indices of visible definitions with this exact name.
    fn visible_defs_named(&self, name: &str) -> Vec<usize> {
        self.prog
            .defs
            .iter()
            .enumerate()
            .filter(|(i, d)| d.name() == name && self.visible(*i))
            .map(|(i, _)| i)
            .collect()
    }

    fn in_locals(&self, name: &str) -> Option<Span> {
        self.locals
            .iter()
            .flatten()
            .find(|(n, _)| n == name)
            .map(|(_, s)| *s)
    }

    // ---- binding ----

    fn bind(&mut self, name: &str, span: Span) -> Result<(), Diagnostic> {
        if let Some(prev) = self.in_locals(name) {
            return Err(self.err(
                Code::Shadowing,
                format!("`{name}` is already bound"),
                span,
                vec![Note {
                    message: "previous binding here".to_string(),
                    file: Opt(Some(self.here().path.clone())),
                    span: Opt(Some(prev)),
                }],
            ));
        }
        if let Some(&i) = self.visible_defs_named(name).first() {
            let d = &self.prog.defs[i];
            return Err(self.err(
                Code::Shadowing,
                format!("`{name}` shadows the definition in `{}`", d.path),
                span,
                vec![Note {
                    message: "definition here".to_string(),
                    file: Opt(Some(d.path.clone())),
                    span: Opt(Some(d.ast.span)),
                }],
            ));
        }
        if kernel::is_builtin(name) {
            return Err(self.err(
                Code::Shadowing,
                format!("`{name}` shadows the builtin of the same name"),
                span,
                vec![],
            ));
        }
        self.locals
            .last_mut()
            .expect("bind requires an open frame")
            .push((name.to_string(), span));
        Ok(())
    }

    // ---- value-name resolution ----

    fn resolve_value(&mut self, name: &str, span: Span) -> Result<(), Diagnostic> {
        if self.in_locals(name).is_some() {
            return Ok(());
        }
        let candidates = self.visible_defs_named(name);
        match candidates.len() {
            0 => {
                if kernel::is_builtin(name) {
                    self.refs.builtins.insert(name.to_string());
                    return Ok(());
                }
                // A private definition that exists but is invisible gets
                // the sharper error.
                if name.starts_with('_') && self.prog.defs.iter().any(|d| d.name() == name) {
                    return Err(self.err(
                        Code::PrivateCrossModule,
                        format!("`{name}` is private to another module"),
                        span,
                        vec![],
                    ));
                }
                Err(self.err(
                    Code::UnknownName,
                    format!("`{name}` is not defined"),
                    span,
                    vec![],
                ))
            }
            1 => {
                self.check_forward(candidates[0], name, span)?;
                if name.starts_with('_') {
                    self.refs.privates.insert(name.to_string());
                } else {
                    self.refs.defs.insert(name.to_string());
                }
                Ok(())
            }
            _ => Err(self.ambiguous_name(name, span, &candidates)),
        }
    }

    fn ambiguous_name(&self, name: &str, span: Span, candidates: &[usize]) -> Diagnostic {
        let notes = candidates
            .iter()
            .map(|&i| Note {
                message: format!("defined in `{}`", self.prog.defs[i].path),
                file: Opt(Some(self.prog.defs[i].path.clone())),
                span: Opt(Some(self.prog.defs[i].ast.span)),
            })
            .collect();
        self.err(
            Code::AmbiguousName,
            format!("`{name}` is defined in several modules; write `module::{name}`"),
            span,
            notes,
        )
    }

    fn check_forward(&self, target: usize, name: &str, span: Span) -> Result<(), Diagnostic> {
        if target > self.index {
            return Err(self.err(
                Code::ForwardReference,
                format!(
                    "`{name}` appears later in the manifest; definitions must be listed callee-before-caller (contract §7)"
                ),
                span,
                vec![],
            ));
        }
        Ok(())
    }

    fn resolve_qualified(&mut self, space: &str, name: &str, span: Span) -> Result<(), Diagnostic> {
        let is_type_space = space.chars().next().is_some_and(|c| c.is_ascii_uppercase());
        if !is_type_space {
            // module::def
            let candidates: Vec<usize> = self
                .prog
                .defs
                .iter()
                .enumerate()
                .filter(|(i, d)| d.name() == name && d.module == space && self.visible(*i))
                .map(|(i, _)| i)
                .collect();
            return match candidates.len() {
                0 => Err(self.err(
                    Code::UnknownQualified,
                    format!("`{space}::{name}` does not name a definition"),
                    span,
                    vec![],
                )),
                1 => {
                    // Exactly-one-spelling (spec §5.10): qualification is
                    // legal only when the bare name is ambiguous.
                    let bare =
                        self.visible_defs_named(name).len() + usize::from(kernel::is_builtin(name));
                    if bare <= 1 {
                        return Err(self.err(
                            Code::NeedlessQualification,
                            format!(
                                "`{name}` is unambiguous; write it bare (one spelling per context)"
                            ),
                            span,
                            vec![],
                        ));
                    }
                    self.check_forward(candidates[0], name, span)?;
                    self.refs.defs.insert(format!("{space}::{name}"));
                    Ok(())
                }
                _ => Err(self.ambiguous_name(name, span, &candidates)),
            };
        }
        // Type::x
        let type_known =
            self.prog.types.contains_key(space) || kernel::builtin_type_arity(space).is_some();
        if !type_known {
            return Err(self.err(
                Code::UnknownType,
                format!("`{space}` is not a known type"),
                span,
                vec![],
            ));
        }
        let starts_upper = name.chars().next().is_some_and(|c| c.is_ascii_uppercase());
        if starts_upper {
            return self.resolve_ctor(Some(space), name, span);
        }
        if kernel::DERIVED_FNS.contains(&name) {
            self.refs.types.insert(space.to_string());
            self.refs.builtins.insert(format!("{space}::{name}"));
            return Ok(());
        }
        if kernel::NUMERIC_OPS.contains(&name) && kernel::is_numeric_type(space) {
            self.refs.types.insert(space.to_string());
            self.refs.builtins.insert(format!("{space}::{name}"));
            return Ok(());
        }
        Err(self.err(
            Code::UnknownQualified,
            format!("`{space}::{name}` is not a derived function or numeric primitive"),
            span,
            vec![],
        ))
    }

    /// Constructor resolution, spec §5.9: exactly one legal spelling.
    fn resolve_ctor(
        &mut self,
        qualifier: Option<&str>,
        name: &str,
        span: Span,
    ) -> Result<(), Diagnostic> {
        let owners = self
            .prog
            .variant_owners
            .get(name)
            .cloned()
            .unwrap_or_default();
        match qualifier {
            None => match owners.len() {
                0 => Err(self.err(
                    Code::UnknownConstructor,
                    format!("no type in scope has a constructor `{name}`"),
                    span,
                    vec![],
                )),
                1 => Ok(()),
                _ => Err(self.err(
                    Code::AmbiguousConstructor,
                    format!("`{name}` belongs to several types; write `Type::{name}`"),
                    span,
                    owners
                        .iter()
                        .map(|t| Note {
                            message: format!("a constructor of `{t}`"),
                            file: Opt(None),
                            span: Opt(None),
                        })
                        .collect(),
                )),
            },
            Some(ty) => {
                let has = self.prog.types.get(ty).is_some_and(|td| {
                    matches!(&td.body, TypeBody::Sum { variants }
                        if variants.iter().any(|v| v.name == name))
                });
                if !has {
                    return Err(self.err(
                        Code::UnknownConstructor,
                        format!("`{ty}` has no constructor `{name}`"),
                        span,
                        vec![],
                    ));
                }
                if owners.len() <= 1 {
                    return Err(self.err(
                        Code::NeedlessQualification,
                        format!(
                            "`{name}` is unambiguous; write it bare (one spelling per context)"
                        ),
                        span,
                        vec![],
                    ));
                }
                self.refs.types.insert(ty.to_string());
                Ok(())
            }
        }
    }

    // ---- types ----

    fn resolve_type(&mut self, ty: &Spanned<Type>) -> Result<(), Diagnostic> {
        match &ty.item {
            Type::Arrow { dom, cod, .. } => {
                self.resolve_type(dom)?;
                self.resolve_type(cod)
            }
            Type::Con { name, args } => {
                if kernel::builtin_type_arity(name).is_none() && !self.prog.types.contains_key(name)
                {
                    return Err(self.err(
                        Code::UnknownType,
                        format!("`{name}` is not a known type"),
                        ty.span,
                        vec![],
                    ));
                }
                self.refs.types.insert(name.clone());
                for a in args {
                    self.resolve_type(a)?;
                }
                Ok(())
            }
            Type::TVar { .. } => Ok(()),
            // Refinements: base type is real; the predicate is ignored
            // (module doc).
            Type::Refined { base, .. } => self.resolve_type(base),
        }
    }

    // ---- the walk ----

    fn walk_def(&mut self) -> Result<(), Diagnostic> {
        let def = &self.here().ast.item;
        let sig = def.sig.clone();
        let params = def.params.clone();
        let decreases = def.decreases.clone();
        let body = def.body.clone();

        self.resolve_type(&sig)?;
        self.locals.push(Vec::new());
        for p in &params {
            self.bind(&p.name, p.span)?;
        }
        if let Opt(Some(d)) = &decreases {
            self.walk_expr(d)?;
        }
        self.walk_expr(&body)?;
        self.locals.pop();
        Ok(())
    }

    fn walk_expr(&mut self, e: &Spanned<Expr>) -> Result<(), Diagnostic> {
        match &e.item {
            Expr::Let {
                is_rec,
                bindings,
                body,
            } => {
                if *is_rec {
                    self.locals.push(Vec::new());
                    for b in bindings {
                        let binder = b.binder.0.as_ref().expect("parser rejects `_` in let rec");
                        self.bind(&binder.name, binder.span)?;
                    }
                    for b in bindings {
                        self.walk_expr(&b.value)?;
                    }
                    self.walk_expr(body)?;
                    self.locals.pop();
                } else {
                    for b in bindings {
                        self.walk_expr(&b.value)?;
                    }
                    self.locals.push(Vec::new());
                    for b in bindings {
                        if let Opt(Some(binder)) = &b.binder {
                            self.bind(&binder.name, binder.span)?;
                        }
                    }
                    self.walk_expr(body)?;
                    self.locals.pop();
                }
                Ok(())
            }
            Expr::Fun { params, body } => {
                self.locals.push(Vec::new());
                for p in params {
                    self.bind(&p.name, p.span)?;
                }
                self.walk_expr(body)?;
                self.locals.pop();
                Ok(())
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.walk_expr(cond)?;
                self.walk_expr(then_branch)?;
                self.walk_expr(else_branch)
            }
            Expr::Match { scrutinee, arms } => {
                self.walk_expr(scrutinee)?;
                for arm in arms {
                    self.locals.push(Vec::new());
                    self.walk_pattern(&arm.item.pattern)?;
                    self.walk_expr(&arm.item.body)?;
                    self.locals.pop();
                }
                Ok(())
            }
            Expr::OrE { lhs, rhs } | Expr::AndE { lhs, rhs } => {
                self.walk_expr(lhs)?;
                self.walk_expr(rhs)
            }
            Expr::NotE { operand } | Expr::Neg { operand } => self.walk_expr(operand),
            Expr::Cmp { lhs, rhs, .. } | Expr::Arith { lhs, rhs, .. } => {
                self.walk_expr(lhs)?;
                self.walk_expr(rhs)
            }
            Expr::App { r#fn, arg } => {
                self.walk_expr(r#fn)?;
                self.walk_expr(arg)
            }
            Expr::Path { root, .. } => self.resolve_value(root, e.span),
            Expr::Qualified { space, name } => self.resolve_qualified(space, name, e.span),
            Expr::CtorE { name } => self.resolve_ctor(None, name, e.span),
            Expr::RecordE { update, fields } => {
                if let Opt(Some(base)) = update {
                    self.resolve_value(&base.root, e.span)?;
                }
                for f in fields {
                    self.walk_expr(&f.value)?;
                }
                Ok(())
            }
            Expr::Literal { .. } => Ok(()),
            Expr::Hole { name } => {
                if let Some((_, prev)) = self.holes_seen.iter().find(|(n, _)| n == name) {
                    return Err(self.err(
                        Code::Shadowing,
                        format!(
                            "hole `?{name}` is already used (hole names are unique per definition)"
                        ),
                        e.span,
                        vec![Note {
                            message: "first used here".to_string(),
                            file: Opt(Some(self.here().path.clone())),
                            span: Opt(Some(*prev)),
                        }],
                    ));
                }
                self.holes_seen.push((name.clone(), e.span));
                Ok(())
            }
            Expr::Annot { expr, ty } => {
                self.walk_expr(expr)?;
                self.resolve_type(ty)
            }
        }
    }

    fn walk_pattern(&mut self, p: &Spanned<Pattern>) -> Result<(), Diagnostic> {
        match &p.item {
            Pattern::PWild | Pattern::PLit { .. } => Ok(()),
            Pattern::PBind { binder } => self.bind(&binder.name, binder.span),
            Pattern::PCtor {
                type_name,
                name,
                arg,
            } => {
                self.resolve_ctor(type_name.0.as_deref(), name, p.span)?;
                if let Opt(Some(inner)) = arg {
                    self.walk_pattern(inner)?;
                }
                Ok(())
            }
            Pattern::PRecord { fields, .. } => {
                for f in fields {
                    match &f.pattern {
                        Opt(Some(inner)) => self.walk_pattern(inner)?,
                        // Punning binds the field name; the record
                        // pattern's span stands in for the binder's.
                        Opt(None) => self.bind(&f.name, p.span)?,
                    }
                }
                Ok(())
            }
        }
    }
}
