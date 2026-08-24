//! Type + effect checking, impl plan 02 step 6. Every definition carries
//! a declared signature, so this is *checking* with local inference:
//! HM unification (union-find, naive generalization at `let`), effect
//! rows solved by a growth fixpoint, and the check-facts model (contract
//! §8.5): `io`/`ffi` deficits are hard `effect-violation` errors, while
//! `div`/`panic` deficits become per-definition facts (`termination`,
//! `panic_obligations`), runtime-checked always.

use crate::ast::{self, ArithOp, Effect, Expr, Lit, Pattern, Type as AType};
use crate::diag::{Code, Diagnostic, Opt};
use crate::kernel;
use crate::manifest::{LoadedDef, Program, TypeBody, TypeDef};
use crate::parser;
use crate::span::{Span, Spanned};
use crate::types::{EffSet, RowT, SigType, Store, Tail, Ty, UnifyErr};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

/// Operator span -> resolved type name (elaboration side table).
pub type OpTable = BTreeMap<(usize, usize), String>;

/// A match site recorded for the exhaustiveness pass (step 7): the
/// zonked scrutinee type and the arms' patterns.
pub struct MatchSite {
    pub scrutinee: Ty,
    pub arms: Vec<Spanned<Pattern>>,
    pub span: Span,
}

/// Everything one definition's check produces.
pub struct CheckedDef {
    pub info: DefInfer,
    pub ops: OpTable,
    pub matches: Vec<MatchSite>,
}

// ---- output (contract §8.4) ----

#[derive(Debug, Serialize)]
pub struct InferOutput {
    pub defs: Vec<DefInfer>,
}

#[derive(Debug, Serialize)]
pub struct DefInfer {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: SigType,
    pub row: ast::Row,
    pub checks: Checks,
}

#[derive(Debug, Serialize)]
pub struct Checks {
    pub termination: &'static str,
    pub panic_obligations: Vec<Obligation>,
    /// Typed holes in source order (contract v1.1); omitted when empty
    /// so hole-free v1 outputs are byte-identical.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<HoleOut>,
}

#[derive(Debug, Serialize)]
pub struct HoleOut {
    pub name: String,
    pub span: Span,
    pub ty: SigType,
}

#[derive(Debug, Serialize)]
pub struct Obligation {
    pub kind: ObKind,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "tag", content = "value")]
pub enum ObKind {
    Overflow,
    DivZero,
    CalleePanic { callee: String },
}

pub fn infer_program(prog: &Program) -> Result<InferOutput, Diagnostic> {
    let mut out = Vec::new();
    for index in 0..prog.defs.len() {
        out.push(check_def(prog, index)?);
    }
    Ok(InferOutput { defs: out })
}

/// The `--dump-ast` output — explicitly non-contractual (contract §12):
/// per definition, the resolved operator types by span.
pub fn dump(prog: &Program) -> String {
    let mut defs = Vec::new();
    for index in 0..prog.defs.len() {
        let name = prog.defs[index].name().to_string();
        let ops = match check_def_ops(prog, index) {
            Some(ops) => ops
                .into_iter()
                .map(|((s, e), ty)| serde_json::json!({"start": s, "end": e, "ty": ty}))
                .collect(),
            None => Vec::new(),
        };
        defs.push(serde_json::json!({"name": name, "ops": ops}));
    }
    serde_json::json!({"non_contractual": true, "defs": defs}).to_string()
}

fn check_def_ops(prog: &Program, index: usize) -> Option<OpTable> {
    check_def_full(prog, index).ok().map(|c| c.ops)
}

/// Public for the interpreter (step 8): the operator elaboration table.
pub fn op_table(prog: &Program, index: usize) -> Result<OpTable, Diagnostic> {
    check_def_full(prog, index).map(|c| c.ops)
}

// ---- integer widths ----

fn int_width(name: &str) -> Option<(i128, i128)> {
    Some(match name {
        "I64" => (i64::MIN as i128, i64::MAX as i128),
        "U64" => (0, u64::MAX as i128),
        "I32" => (i32::MIN as i128, i32::MAX as i128),
        "U32" => (0, u32::MAX as i128),
        "I16" => (i16::MIN as i128, i16::MAX as i128),
        "U16" => (0, u16::MAX as i128),
        "I8" => (i8::MIN as i128, i8::MAX as i128),
        "U8" => (0, u8::MAX as i128),
        _ => return None,
    })
}

// ---- per-definition checker ----

#[derive(Clone)]
struct Scheme {
    ty: Ty,
    gen_tys: Vec<crate::types::TvId>,
}

enum What {
    Overflow,
    DivZero,
    Call(String),
}

enum OpCat {
    Cmp,
    Arith(ArithOp),
    Neg,
}

struct Performed {
    row: RowT,
    ctx: RowT,
    span: Span,
    what: What,
}

struct VariantInfo {
    con_ty: Ty,
    payload_ty: Option<Ty>,
    record_fields: Option<Vec<(String, Ty)>>,
}

struct Checker<'a> {
    prog: &'a Program,
    index: usize,
    store: Store,
    locals: Vec<HashMap<String, Scheme>>,
    ctx_stack: Vec<RowT>,
    performed: Vec<Performed>,
    derived_uses: Vec<(Ty, Span)>,
    /// Integer literals typed by context (design §3.6, "as in Rust"):
    /// each gets a fresh variable, range-checked once resolved,
    /// defaulting to I64 when still unconstrained at the end.
    pending_ints: Vec<(Ty, String, Span)>,
    /// Operators resolve at the type known after unification (deferred
    /// past literal defaulting); the context row is captured at the
    /// node.
    pending_ops: Vec<(Ty, OpCat, RowT, Span)>,
    /// Typed holes in source order: (name, span, goal type variable).
    holes: Vec<(String, Span, Ty)>,
    /// Match sites for the exhaustiveness pass (`soil0 check`).
    matches: Vec<(Ty, Vec<Spanned<Pattern>>, Span)>,
    self_recursive: bool,
    /// Elaboration side table: operator spans → resolved type name
    /// (consumed by the interpreter, step 8; dumped by `--dump-ast`).
    pub op_types: BTreeMap<(usize, usize), String>,
}

fn check_def(prog: &Program, index: usize) -> Result<DefInfer, Diagnostic> {
    check_def_full(prog, index).map(|c| c.info)
}

/// Public within the crate for the exhaustiveness pass (step 7).
pub fn check_def_all(prog: &Program, index: usize) -> Result<CheckedDef, Diagnostic> {
    check_def_full(prog, index)
}

fn check_def_full(prog: &Program, index: usize) -> Result<CheckedDef, Diagnostic> {
    let def = &prog.defs[index];
    let ast_def = &def.ast.item;

    let mut ck = Checker {
        prog,
        index,
        store: Store::default(),
        locals: Vec::new(),
        ctx_stack: Vec::new(),
        performed: Vec::new(),
        derived_uses: Vec::new(),
        pending_ints: Vec::new(),
        pending_ops: Vec::new(),
        holes: Vec::new(),
        matches: Vec::new(),
        self_recursive: false,
        op_types: BTreeMap::new(),
    };

    // The definition's own signature is rigid.
    let sig_ty = ck.conv_type(&ast_def.sig, &mut Mode::Rigid)?;

    // Peel one arrow per equation parameter; non-final rows must be
    // empty (an equation defines the fully applied body).
    let mut cur = sig_ty.clone();
    let mut param_tys = Vec::new();
    let mut final_row = RowT::total();
    for (i, p) in ast_def.params.iter().enumerate() {
        let cur_z = ck.store.zonk(&cur);
        match cur_z {
            Ty::Arrow { dom, row, cod } => {
                let resolved = ck.store.resolve_row(&row);
                let last = i + 1 == ast_def.params.len();
                if !(last || resolved.effects.is_empty() && resolved.tail == Tail::Closed) {
                    return Err(ck.err(
                        Code::EffectViolation,
                        "only the innermost arrow of an equation's signature may carry a row"
                            .to_string(),
                        p.span,
                    ));
                }
                if last {
                    final_row = resolved;
                }
                param_tys.push((p.clone(), *dom));
                cur = *cod;
            }
            _ => {
                return Err(ck.err(
                    Code::TypeMismatch,
                    "equation has more parameters than the signature has arrows".to_string(),
                    p.span,
                ))
            }
        }
    }
    let expected_body = cur;

    ck.ctx_stack.push(final_row.clone());
    ck.locals.push(HashMap::new());
    for (binder, ty) in &param_tys {
        ck.locals.last_mut().unwrap().insert(
            binder.name.clone(),
            Scheme {
                ty: ty.clone(),
                gen_tys: vec![],
            },
        );
    }
    if let Opt(Some(d)) = &ast_def.decreases {
        ck.infer_expr(d)?;
    }
    let body_ty = ck.infer_expr(&ast_def.body)?;
    ck.unify(&body_ty, &expected_body, ast_def.body.span)?;
    ck.resolve_pending_ints()?;
    ck.resolve_pending_ops()?;

    // Effect solving: grow open contexts to a fixpoint, close leftovers,
    // then classify deficits.
    loop {
        let mut changed = false;
        for i in 0..ck.performed.len() {
            let needed = ck.store.resolve_row(&ck.performed[i].row);
            let ctx = ck.store.resolve_row(&ck.performed[i].ctx);
            if let Tail::Open(v) = ctx.tail {
                if let Tail::Skolem(s) = &needed.tail {
                    ck.store.skolemize_row(v, s.clone());
                    changed = true;
                    continue;
                }
                let missing = needed.effects.minus(ctx.effects);
                if !missing.is_empty() {
                    ck.store.grow_row(v, missing);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    ck.store.close_open_rows();

    let mut div_deficit = false;
    let mut obligations = Vec::new();
    for p in &ck.performed {
        let needed = ck.store.resolve_row(&p.row);
        let ctx = ck.store.resolve_row(&p.ctx);
        if let Tail::Skolem(s) = &needed.tail {
            if ctx.tail != Tail::Skolem(s.clone()) {
                return Err(ck.err(
                    Code::EffectViolation,
                    format!("a call's polymorphic effects (`{s}`) cannot flow to this context"),
                    p.span,
                ));
            }
        }
        let missing = needed.effects.minus(ctx.effects);
        if missing.contains(Effect::Io) || missing.contains(Effect::Ffi) {
            let what = match &p.what {
                What::Call(c) => c.clone(),
                _ => "arithmetic".to_string(),
            };
            return Err(ck.err(
                Code::EffectViolation,
                format!("`{what}` performs `io`/`ffi`, which the signature's row does not allow"),
                p.span,
            ));
        }
        if missing.contains(Effect::Div) {
            div_deficit = true;
        }
        if missing.contains(Effect::Panic) {
            obligations.push(Obligation {
                kind: match &p.what {
                    What::Overflow => ObKind::Overflow,
                    What::DivZero => ObKind::DivZero,
                    What::Call(c) => ObKind::CalleePanic { callee: c.clone() },
                },
                span: p.span,
            });
        }
    }
    obligations.sort_by_key(|o| (o.span.start, o.span.end));

    // Derivability post-pass: `==`/comparisons/`Type::derived` operand
    // types may not contain arrows.
    for (ty, span) in &ck.derived_uses {
        let z = ck.store.zonk(ty);
        if ty_contains_arrow(&z) {
            return Err(ck.err(
                Code::NonDerivable,
                "deriving `eq`/`compare` on a type containing a function is a type error"
                    .to_string(),
                *span,
            ));
        }
    }

    let declared_div = final_row.effects.contains(Effect::Div);
    let termination = if declared_div {
        "n/a"
    } else if ck.self_recursive || div_deficit {
        "unverified"
    } else {
        "verified"
    };

    let holes = {
        let mut out = Vec::new();
        let mut renamer = GoalRenamer::default();
        for (name, span, ty) in &ck.holes {
            let z = ck.store.zonk(ty);
            out.push(HoleOut {
                name: name.clone(),
                span: *span,
                ty: renamer.sig_of(&z),
            });
        }
        out
    };

    let matches = ck
        .matches
        .iter()
        .map(|(t, arms, span)| MatchSite {
            scrutinee: ck.store.zonk(t),
            arms: arms.clone(),
            span: *span,
        })
        .collect();

    Ok(CheckedDef {
        info: DefInfer {
            name: ast_def.name.clone(),
            ty: canonical_sig(&ast_def.sig),
            row: canonical_row_of_def(ast_def),
            checks: Checks {
                termination,
                panic_obligations: obligations,
                holes,
            },
        },
        ops: ck.op_types,
        matches,
    })
}

/// Renders zonked goal types for hole reports: rigid variables keep
/// their signature names; unresolved unification variables get fresh
/// canonical names (`a0`, `b0`, …) that cannot collide with rigids.
#[derive(Default)]
struct GoalRenamer {
    vars: HashMap<crate::types::TvId, String>,
    next: usize,
}

impl GoalRenamer {
    fn sig_of(&mut self, ty: &Ty) -> SigType {
        match ty {
            Ty::Var(v) => {
                if !self.vars.contains_key(v) {
                    let i = self.next;
                    self.next += 1;
                    let letter = (b'a' + (i % 26) as u8) as char;
                    self.vars.insert(*v, format!("{letter}{}", i / 26));
                }
                SigType::SVar {
                    name: self.vars[v].clone(),
                }
            }
            Ty::Rigid(n) => SigType::SVar { name: n.clone() },
            Ty::Con { name, args } => SigType::SCon {
                name: name.clone(),
                args: args.iter().map(|a| self.sig_of(a)).collect(),
            },
            Ty::Arrow { dom, row, cod } => SigType::SArrow {
                param: Opt(None),
                dom: Box::new(self.sig_of(dom)),
                row: row_to_ast(row),
                cod: Box::new(self.sig_of(cod)),
            },
        }
    }
}

fn row_to_ast(row: &RowT) -> ast::Row {
    ast::Row {
        effects: row.effects.effects(),
        var: Opt(None),
    }
}

/// `SigType` → `Ty` under a parameter substitution, aliases expanded —
/// shared by the checker and the exhaustiveness pass.
pub(crate) fn conv_sig_prog(prog: &Program, s: &SigType, subst: &HashMap<String, Ty>) -> Ty {
    match s {
        SigType::SVar { name } => subst.get(name).cloned().unwrap_or(Ty::Rigid(name.clone())),
        SigType::SCon { name, args } => {
            let args: Vec<Ty> = args.iter().map(|a| conv_sig_prog(prog, a, subst)).collect();
            let alias = prog.types.get(name).and_then(|td| match &td.body {
                TypeBody::Alias { ty } => Some((td.params.clone(), ty.clone())),
                _ => None,
            });
            if let Some((params, target)) = alias {
                let inner: HashMap<String, Ty> = params.into_iter().zip(args).collect();
                return conv_sig_prog(prog, &target, &inner);
            }
            Ty::Con {
                name: name.clone(),
                args,
            }
        }
        SigType::SArrow { .. } => unreachable!("env shapes are arrow-free (contract §6)"),
    }
}

fn ty_contains_arrow(ty: &Ty) -> bool {
    match ty {
        Ty::Arrow { .. } => true,
        Ty::Con { args, .. } => args.iter().any(ty_contains_arrow),
        _ => false,
    }
}

// ---- signature conversion ----

enum Mode {
    /// The definition's own signature (and local annotations): names
    /// become rigid/skolem.
    Rigid,
    /// A callee/builtin signature: names become fresh variables.
    Inst(HashMap<String, Ty>, HashMap<String, RowT>),
}

impl Checker<'_> {
    fn here(&self) -> &LoadedDef {
        &self.prog.defs[self.index]
    }

    fn err(&self, code: Code, message: String, span: Span) -> Diagnostic {
        Diagnostic {
            code,
            message,
            file: Opt(Some(self.here().path.clone())),
            span: Opt(Some(span)),
            notes: Vec::new(),
        }
    }

    fn unify(&mut self, a: &Ty, b: &Ty, span: Span) -> Result<(), Diagnostic> {
        self.store.unify(a, b).map_err(|e| self.unify_err(e, span))
    }

    fn unify_err(&self, e: UnifyErr, span: Span) -> Diagnostic {
        let msg = match e {
            UnifyErr::Mismatch(a, b) => {
                format!("type mismatch: `{}` vs `{}`", render_ty(&a), render_ty(&b))
            }
            UnifyErr::RowMismatch => "effect rows do not agree".to_string(),
            UnifyErr::Occurs => "infinite type".to_string(),
        };
        self.err(Code::TypeMismatch, msg, span)
    }

    fn typedef(&self, name: &str) -> Option<&TypeDef> {
        self.prog.types.get(name)
    }

    fn conv_type(&mut self, ty: &Spanned<AType>, mode: &mut Mode) -> Result<Ty, Diagnostic> {
        match &ty.item {
            AType::Arrow { dom, row, cod, .. } => {
                let d = self.conv_type(dom, mode)?;
                let r = self.conv_row(row, mode);
                let c = self.conv_type(cod, mode)?;
                Ok(Ty::Arrow {
                    dom: Box::new(d),
                    row: r,
                    cod: Box::new(c),
                })
            }
            AType::Con { name, args } => {
                let mut converted = Vec::new();
                for a in args {
                    converted.push(self.conv_type(a, mode)?);
                }
                self.mk_con(name, converted, ty.span)
            }
            AType::TVar { name } => Ok(match mode {
                Mode::Rigid => Ty::Rigid(name.clone()),
                Mode::Inst(map, _) => map
                    .entry(name.clone())
                    .or_insert_with(|| self.store.fresh_ty())
                    .clone(),
            }),
            AType::Refined { base, .. } => self.conv_type(base, mode),
        }
    }

    fn conv_row(&mut self, row: &ast::Row, mode: &mut Mode) -> RowT {
        let effects = EffSet::from_effects(&row.effects);
        let tail = match &row.var {
            Opt(None) => Tail::Closed,
            Opt(Some(v)) => match mode {
                Mode::Rigid => Tail::Skolem(v.clone()),
                Mode::Inst(_, rows) => rows
                    .entry(v.clone())
                    .or_insert_with(|| self.store.fresh_row())
                    .tail
                    .clone(),
            },
        };
        RowT { effects, tail }
    }

    /// Builds a `Con`, checking arity and expanding aliases.
    fn mk_con(&mut self, name: &str, args: Vec<Ty>, span: Span) -> Result<Ty, Diagnostic> {
        let expected = match kernel::builtin_type_arity(name) {
            Some(n) => n,
            None => match self.typedef(name) {
                Some(td) => td.params.len(),
                None => {
                    return Err(self.err(
                        Code::UnknownType,
                        format!("`{name}` is not a known type"),
                        span,
                    ))
                }
            },
        };
        if args.len() != expected {
            return Err(self.err(
                Code::TypeMismatch,
                format!(
                    "`{name}` takes {expected} type arguments, got {}",
                    args.len()
                ),
                span,
            ));
        }
        let alias = self.typedef(name).and_then(|td| match &td.body {
            TypeBody::Alias { ty } => Some((td.params.clone(), ty.clone())),
            _ => None,
        });
        if let Some((params, target)) = alias {
            let subst: HashMap<String, Ty> = params.into_iter().zip(args).collect();
            return Ok(self.conv_sig(&target, &subst));
        }
        Ok(Ty::Con {
            name: name.to_string(),
            args,
        })
    }

    /// Converts a manifest `SigType` (field/payload/alias shapes) under a
    /// parameter substitution. Env shapes are arrow-free (contract §6).
    fn conv_sig(&mut self, s: &SigType, subst: &HashMap<String, Ty>) -> Ty {
        conv_sig_prog(self.prog, s, subst)
    }

    fn instantiate_ast_sig(&mut self, sig: &Spanned<AType>) -> Result<Ty, Diagnostic> {
        let mut mode = Mode::Inst(HashMap::new(), HashMap::new());
        self.conv_type(sig, &mut mode)
    }

    // ---- scopes ----

    fn lookup_local(&self, name: &str) -> Option<Scheme> {
        self.locals.iter().rev().find_map(|f| f.get(name)).cloned()
    }

    fn instantiate_scheme(&mut self, s: &Scheme) -> Ty {
        if s.gen_tys.is_empty() {
            return s.ty.clone();
        }
        let mut map = HashMap::new();
        for id in &s.gen_tys {
            map.insert(*id, self.store.fresh_ty());
        }
        self.subst_vars(&s.ty, &map)
    }

    fn subst_vars(&self, ty: &Ty, map: &HashMap<crate::types::TvId, Ty>) -> Ty {
        match self.store.shallow(ty) {
            Ty::Var(v) => map.get(&v).cloned().unwrap_or(Ty::Var(v)),
            Ty::Rigid(n) => Ty::Rigid(n),
            Ty::Con { name, args } => Ty::Con {
                name,
                args: args.iter().map(|a| self.subst_vars(a, map)).collect(),
            },
            Ty::Arrow { dom, row, cod } => Ty::Arrow {
                dom: Box::new(self.subst_vars(&dom, map)),
                row,
                cod: Box::new(self.subst_vars(&cod, map)),
            },
        }
    }

    fn free_unbound_vars(&self, ty: &Ty, out: &mut Vec<crate::types::TvId>) {
        match self.store.shallow(ty) {
            Ty::Var(v) => {
                if !out.contains(&v) {
                    out.push(v);
                }
            }
            Ty::Rigid(_) => {}
            Ty::Con { args, .. } => {
                for a in &args {
                    self.free_unbound_vars(a, out);
                }
            }
            Ty::Arrow { dom, cod, .. } => {
                self.free_unbound_vars(&dom, out);
                self.free_unbound_vars(&cod, out);
            }
        }
    }

    fn generalize(&self, ty: &Ty) -> Scheme {
        let mut candidates = Vec::new();
        self.free_unbound_vars(ty, &mut candidates);
        let mut env_vars = Vec::new();
        for frame in &self.locals {
            for s in frame.values() {
                self.free_unbound_vars(&s.ty, &mut env_vars);
            }
        }
        let gen_tys = candidates
            .into_iter()
            .filter(|v| !env_vars.contains(v))
            .collect();
        Scheme {
            ty: ty.clone(),
            gen_tys,
        }
    }

    fn ctx(&self) -> RowT {
        self.ctx_stack.last().expect("context row").clone()
    }

    fn perform(&mut self, row: RowT, span: Span, what: What) {
        self.performed.push(Performed {
            row,
            ctx: self.ctx(),
            span,
            what,
        });
    }

    // ---- name resolution (rename has already validated) ----

    fn resolve_value(&mut self, name: &str, span: Span) -> Result<Ty, Diagnostic> {
        if let Some(s) = self.lookup_local(name) {
            return Ok(self.instantiate_scheme(&s));
        }
        if name == self.here().name() {
            self.self_recursive = true;
            let sig = self.here().ast.item.sig.clone();
            return self.conv_type(&sig, &mut Mode::Rigid);
        }
        let candidates: Vec<usize> = self
            .prog
            .defs
            .iter()
            .enumerate()
            .filter(|(i, d)| {
                d.name() == name
                    && (!d.is_private() || d.dir == self.here().dir)
                    && *i != self.index
            })
            .map(|(i, _)| i)
            .collect();
        if let Some(&i) = candidates.first() {
            let sig = self.prog.defs[i].ast.item.sig.clone();
            return self.instantiate_ast_sig(&sig);
        }
        if let Some(src) = kernel::builtin_sig(name) {
            let parsed = parser::parse_type_str(src).expect("builtin signatures parse");
            return self.instantiate_ast_sig(&parsed);
        }
        Err(self.err(Code::UnknownName, format!("`{name}` is not defined"), span))
    }

    /// A derived or numeric `Type::x` scheme.
    fn qualified_scheme(&mut self, space: &str, name: &str, span: Span) -> Result<Ty, Diagnostic> {
        // module::def
        if space.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
            let sig = self
                .prog
                .defs
                .iter()
                .find(|d| d.name() == name && d.module == space)
                .map(|d| d.ast.item.sig.clone())
                .ok_or_else(|| {
                    self.err(
                        Code::UnknownQualified,
                        format!("`{space}::{name}` does not name a definition"),
                        span,
                    )
                })?;
            return self.instantiate_ast_sig(&sig);
        }
        // Type::x
        let applied = self.applied_type(space, span)?;
        let bool_ty = Ty::Con {
            name: "Bool".into(),
            args: vec![],
        };
        let total = RowT::total();
        let arrow = |dom: Ty, row: RowT, cod: Ty| Ty::Arrow {
            dom: Box::new(dom),
            row,
            cod: Box::new(cod),
        };
        let ty = match name {
            "eq" => {
                self.derived_uses.push((applied.clone(), span));
                arrow(
                    applied.clone(),
                    total.clone(),
                    arrow(applied, total, bool_ty),
                )
            }
            "compare" => {
                self.derived_uses.push((applied.clone(), span));
                // -1 / 0 / 1 (provisional kernel surface, plan 04 scope 6).
                let i64t = Ty::Con {
                    name: "I64".into(),
                    args: vec![],
                };
                arrow(applied.clone(), total.clone(), arrow(applied, total, i64t))
            }
            "show" => {
                self.derived_uses.push((applied.clone(), span));
                arrow(
                    applied,
                    total,
                    Ty::Con {
                        name: "Utf8".into(),
                        args: vec![],
                    },
                )
            }
            "hash" => {
                self.derived_uses.push((applied.clone(), span));
                arrow(
                    applied,
                    total,
                    Ty::Con {
                        name: "U64".into(),
                        args: vec![],
                    },
                )
            }
            op if kernel::NUMERIC_OPS.contains(&op) => {
                let row = numeric_op_row(space, op);
                if op == "neg" {
                    arrow(applied.clone(), row, applied)
                } else {
                    arrow(applied.clone(), total, arrow(applied.clone(), row, applied))
                }
            }
            _ => {
                return Err(self.err(
                    Code::UnknownQualified,
                    format!("`{space}::{name}` is not a derived function or numeric primitive"),
                    span,
                ))
            }
        };
        Ok(ty)
    }

    /// `Type` applied to fresh arguments for its parameters.
    fn applied_type(&mut self, name: &str, span: Span) -> Result<Ty, Diagnostic> {
        let n_params = match kernel::builtin_type_arity(name) {
            Some(n) => n,
            None => match self.typedef(name) {
                Some(td) => td.params.len(),
                None => {
                    return Err(self.err(
                        Code::UnknownType,
                        format!("`{name}` is not a known type"),
                        span,
                    ))
                }
            },
        };
        let args: Vec<Ty> = (0..n_params).map(|_| self.store.fresh_ty()).collect();
        self.mk_con(name, args, span)
    }

    fn variant_of(
        &mut self,
        qualifier: Option<&str>,
        name: &str,
        span: Span,
    ) -> Result<VariantInfo, Diagnostic> {
        let owner = match qualifier {
            Some(t) => t.to_string(),
            None => {
                let owners = self
                    .prog
                    .variant_owners
                    .get(name)
                    .cloned()
                    .unwrap_or_default();
                owners.first().cloned().ok_or_else(|| {
                    self.err(
                        Code::UnknownConstructor,
                        format!("no type in scope has a constructor `{name}`"),
                        span,
                    )
                })?
            }
        };
        let td = self.typedef(&owner).cloned().ok_or_else(|| {
            self.err(
                Code::UnknownType,
                format!("`{owner}` is not a known type"),
                span,
            )
        })?;
        let TypeBody::Sum { variants } = &td.body else {
            return Err(self.err(
                Code::UnknownConstructor,
                format!("`{owner}` is not a sum type"),
                span,
            ));
        };
        let v = variants.iter().find(|v| v.name == name).ok_or_else(|| {
            self.err(
                Code::UnknownConstructor,
                format!("`{owner}` has no constructor `{name}`"),
                span,
            )
        })?;
        let args: Vec<Ty> = td.params.iter().map(|_| self.store.fresh_ty()).collect();
        let subst: HashMap<String, Ty> = td.params.iter().cloned().zip(args.clone()).collect();
        let payload = v.payload.0.clone();
        let record_fields = v.record_fields.clone();
        let payload_ty = payload.map(|p| self.conv_sig(&p, &subst));
        let fields = record_fields.map(|fs| {
            fs.iter()
                .map(|f| (f.name.clone(), self.conv_sig(&f.shape, &subst)))
                .collect::<Vec<_>>()
        });
        Ok(VariantInfo {
            con_ty: Ty::Con { name: owner, args },
            payload_ty,
            record_fields: fields,
        })
    }

    // ---- expressions ----

    fn infer_expr(&mut self, e: &Spanned<Expr>) -> Result<Ty, Diagnostic> {
        match &e.item {
            Expr::Literal { lit } => self.literal_ty(lit, None, e.span),
            Expr::Annot { expr, ty } => {
                let target = self.conv_type(ty, &mut Mode::Rigid)?;
                if let Expr::Literal { lit } = &expr.item {
                    return self.literal_ty(lit, Some(&target), expr.span);
                }
                if let Expr::RecordE { update, fields } = &expr.item {
                    return self.record_expr(update, fields, expr.span, Some(&target));
                }
                let inner = self.infer_expr(expr)?;
                self.unify(&inner, &target, e.span)?;
                Ok(target)
            }
            Expr::Path { root, fields } => {
                let mut cur = self.resolve_value(root, e.span)?;
                for f in fields {
                    cur = self.project_field(&cur, f, e.span)?;
                }
                Ok(cur)
            }
            Expr::Qualified { space, name } => {
                if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                    return self.ctor_value(Some(space), name, e.span, None);
                }
                self.qualified_scheme(space, name, e.span)
            }
            Expr::CtorE { name } => self.ctor_value(None, name, e.span, None),
            Expr::App { r#fn, arg } => {
                // Constructor application is special-cased for payload
                // records and better errors.
                match &r#fn.item {
                    Expr::CtorE { name } => {
                        return self.ctor_value(None, name, e.span, Some(arg));
                    }
                    Expr::Qualified { space, name }
                        if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) =>
                    {
                        return self.ctor_value(Some(space), name, e.span, Some(arg));
                    }
                    _ => {}
                }
                let tf = self.infer_expr(r#fn)?;
                let (dom, row, cod) = match self.store.zonk(&tf) {
                    Ty::Arrow { dom, row, cod } => (*dom, row, *cod),
                    Ty::Var(_) => {
                        let d = self.store.fresh_ty();
                        let r = self.store.fresh_row();
                        let c = self.store.fresh_ty();
                        let arrow = Ty::Arrow {
                            dom: Box::new(d.clone()),
                            row: r.clone(),
                            cod: Box::new(c.clone()),
                        };
                        self.unify(&tf, &arrow, r#fn.span)?;
                        (d, r, c)
                    }
                    other => {
                        return Err(self.err(
                            Code::TypeMismatch,
                            format!("`{}` is not a function", render_ty(&other)),
                            r#fn.span,
                        ))
                    }
                };
                let ta = self.infer_expr(arg)?;
                self.unify(&ta, &dom, arg.span)?;
                self.perform(row, e.span, What::Call(describe_callee(&r#fn.item)));
                Ok(cod)
            }
            Expr::Let {
                is_rec,
                bindings,
                body,
            } => {
                if *is_rec {
                    self.self_recursive = true;
                    self.locals.push(HashMap::new());
                    let vars: Vec<Ty> = bindings.iter().map(|_| self.store.fresh_ty()).collect();
                    for (b, v) in bindings.iter().zip(&vars) {
                        let binder = b.binder.0.as_ref().expect("no `_` in let rec");
                        self.locals.last_mut().unwrap().insert(
                            binder.name.clone(),
                            Scheme {
                                ty: v.clone(),
                                gen_tys: vec![],
                            },
                        );
                    }
                    for (b, v) in bindings.iter().zip(&vars) {
                        let t = self.infer_expr(&b.value)?;
                        self.unify(&t, v, b.value.span)?;
                    }
                    let r = self.infer_expr(body);
                    self.locals.pop();
                    r
                } else {
                    let mut typed = Vec::new();
                    for b in bindings {
                        typed.push(self.infer_expr(&b.value)?);
                    }
                    self.locals.push(HashMap::new());
                    for (b, t) in bindings.iter().zip(&typed) {
                        if let Opt(Some(binder)) = &b.binder {
                            let scheme = self.generalize(t);
                            self.locals
                                .last_mut()
                                .unwrap()
                                .insert(binder.name.clone(), scheme);
                        }
                    }
                    let r = self.infer_expr(body);
                    self.locals.pop();
                    r
                }
            }
            Expr::Fun { params, body } => {
                self.locals.push(HashMap::new());
                let mut doms = Vec::new();
                for p in params {
                    let v = self.store.fresh_ty();
                    self.locals.last_mut().unwrap().insert(
                        p.name.clone(),
                        Scheme {
                            ty: v.clone(),
                            gen_tys: vec![],
                        },
                    );
                    doms.push(v);
                }
                let lam_row = self.store.fresh_row();
                self.ctx_stack.push(lam_row.clone());
                let body_ty = self.infer_expr(body)?;
                self.ctx_stack.pop();
                self.locals.pop();
                // Innermost arrow carries the lambda's row; outer arrows
                // are total (full application performs).
                let mut ty = body_ty;
                let mut row = lam_row;
                for d in doms.into_iter().rev() {
                    ty = Ty::Arrow {
                        dom: Box::new(d),
                        row,
                        cod: Box::new(ty),
                    };
                    row = RowT::total();
                }
                Ok(ty)
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let tc = self.infer_expr(cond)?;
                self.unify(
                    &tc,
                    &Ty::Con {
                        name: "Bool".into(),
                        args: vec![],
                    },
                    cond.span,
                )?;
                let tt = self.infer_expr(then_branch)?;
                let te = self.infer_expr(else_branch)?;
                self.unify(&tt, &te, else_branch.span)?;
                Ok(tt)
            }
            Expr::Match { scrutinee, arms } => {
                let ts = self.infer_expr(scrutinee)?;
                let result = self.store.fresh_ty();
                for arm in arms {
                    self.locals.push(HashMap::new());
                    self.check_pattern(&arm.item.pattern, &ts)?;
                    let tb = self.infer_expr(&arm.item.body)?;
                    self.unify(&tb, &result, arm.item.body.span)?;
                    self.locals.pop();
                }
                self.matches.push((
                    ts.clone(),
                    arms.iter().map(|a| a.item.pattern.clone()).collect(),
                    e.span,
                ));
                Ok(result)
            }
            Expr::OrE { lhs, rhs } | Expr::AndE { lhs, rhs } => {
                let b = Ty::Con {
                    name: "Bool".into(),
                    args: vec![],
                };
                let tl = self.infer_expr(lhs)?;
                self.unify(&tl, &b, lhs.span)?;
                let tr = self.infer_expr(rhs)?;
                self.unify(&tr, &b, rhs.span)?;
                Ok(b)
            }
            Expr::NotE { operand } => {
                let b = Ty::Con {
                    name: "Bool".into(),
                    args: vec![],
                };
                let t = self.infer_expr(operand)?;
                self.unify(&t, &b, operand.span)?;
                Ok(b)
            }
            Expr::Cmp { lhs, rhs, .. } => {
                let tl = self.infer_expr(lhs)?;
                let tr = self.infer_expr(rhs)?;
                self.unify(&tl, &tr, rhs.span)?;
                self.pending_ops.push((tl, OpCat::Cmp, self.ctx(), e.span));
                Ok(Ty::Con {
                    name: "Bool".into(),
                    args: vec![],
                })
            }
            Expr::Arith { op, lhs, rhs } => {
                let tl = self.infer_expr(lhs)?;
                let tr = self.infer_expr(rhs)?;
                self.unify(&tl, &tr, rhs.span)?;
                self.pending_ops
                    .push((tl.clone(), OpCat::Arith(*op), self.ctx(), e.span));
                Ok(tl)
            }
            Expr::Neg { operand } => {
                let t = self.infer_expr(operand)?;
                self.pending_ops
                    .push((t.clone(), OpCat::Neg, self.ctx(), e.span));
                Ok(t)
            }
            Expr::RecordE { update, fields } => self.record_expr(update, fields, e.span, None),
            Expr::Hole { name } => {
                let v = self.store.fresh_ty();
                self.holes.push((name.clone(), e.span, v.clone()));
                Ok(v)
            }
        }
    }

    /// Resolves deferred operators (after literal defaulting): the
    /// operand type must be monomorphic; obligations attach to the
    /// context captured at the node.
    fn resolve_pending_ops(&mut self) -> Result<(), Diagnostic> {
        let pending = std::mem::take(&mut self.pending_ops);
        let panic = RowT::closed(EffSet::single(Effect::Panic));
        for (ty, cat, ctx, span) in pending {
            match cat {
                OpCat::Cmp => {
                    let name = self.monomorphic_name(&ty, span)?;
                    self.derived_uses.push((ty, span));
                    self.op_types.insert((span.start, span.end), name);
                }
                OpCat::Arith(op) => {
                    let name = self.numeric_name(&ty, span)?;
                    self.op_types.insert((span.start, span.end), name.clone());
                    let what = match (name.as_str(), op) {
                        ("F64", _) => None,
                        ("BigInt", ArithOp::Div | ArithOp::Mod) => Some(What::DivZero),
                        ("BigInt", _) => None,
                        // Zero divisor and MIN / -1 are both divisor
                        // obligations.
                        (_, ArithOp::Div | ArithOp::Mod) => Some(What::DivZero),
                        _ => Some(What::Overflow),
                    };
                    if let Some(what) = what {
                        self.performed.push(Performed {
                            row: panic.clone(),
                            ctx: ctx.clone(),
                            span,
                            what,
                        });
                    }
                }
                OpCat::Neg => {
                    let name = self.numeric_name(&ty, span)?;
                    self.op_types.insert((span.start, span.end), name.clone());
                    if int_width(&name).is_some() {
                        self.performed.push(Performed {
                            row: panic.clone(),
                            ctx: ctx.clone(),
                            span,
                            what: What::Overflow,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn literal_ty(&mut self, lit: &Lit, target: Option<&Ty>, span: Span) -> Result<Ty, Diagnostic> {
        match lit {
            Lit::LInt { digits } => match target.map(|t| self.store.zonk(t)) {
                Some(Ty::Con { name, args }) if args.is_empty() && int_width(&name).is_some() => {
                    self.range_check(digits, &name, span)?;
                    Ok(Ty::Con { name, args: vec![] })
                }
                None => {
                    let v = self.store.fresh_ty();
                    self.pending_ints.push((v.clone(), digits.clone(), span));
                    Ok(v)
                }
                Some(Ty::Var(id)) => {
                    let v = Ty::Var(id);
                    self.pending_ints.push((v.clone(), digits.clone(), span));
                    Ok(v)
                }
                Some(other) => Err(self.err(
                    Code::TypeMismatch,
                    format!(
                        "an integer literal cannot have type `{}`",
                        render_ty(&other)
                    ),
                    span,
                )),
            },
            Lit::LFloat { .. } => {
                let f = Ty::Con {
                    name: "F64".into(),
                    args: vec![],
                };
                if let Some(t) = target {
                    let t = t.clone();
                    self.unify(&f, &t, span)?;
                }
                Ok(f)
            }
            Lit::LStr { .. } => {
                let s = Ty::Con {
                    name: "Utf8".into(),
                    args: vec![],
                };
                if let Some(t) = target {
                    let t = t.clone();
                    self.unify(&s, &t, span)?;
                }
                Ok(s)
            }
        }
    }

    fn range_check(&self, digits: &str, name: &str, span: Span) -> Result<(), Diagnostic> {
        let (lo, hi) = int_width(name).expect("integer type");
        let value: i128 = digits.parse().map_err(|_| {
            self.err(
                Code::LiteralOutOfRange,
                format!("`{digits}` does not fit any integer type"),
                span,
            )
        })?;
        if value < lo || value > hi {
            return Err(self.err(
                Code::LiteralOutOfRange,
                format!("`{digits}` is out of range for `{name}`"),
                span,
            ));
        }
        Ok(())
    }

    /// Context-typed integer literals (design §3.6): resolved after the
    /// body unifies, defaulting to I64 when still unconstrained.
    fn resolve_pending_ints(&mut self) -> Result<(), Diagnostic> {
        let pending = std::mem::take(&mut self.pending_ints);
        for (ty, digits, span) in pending {
            match self.store.zonk(&ty) {
                Ty::Con { name, args } if args.is_empty() && int_width(&name).is_some() => {
                    self.range_check(&digits, &name, span)?;
                    self.op_types.insert((span.start, span.end), name);
                }
                Ty::Var(_) => {
                    let i64t = Ty::Con {
                        name: "I64".into(),
                        args: vec![],
                    };
                    self.unify(&ty, &i64t, span)?;
                    self.range_check(&digits, "I64", span)?;
                    self.op_types
                        .insert((span.start, span.end), "I64".to_string());
                }
                other => {
                    return Err(self.err(
                        Code::TypeMismatch,
                        format!(
                            "an integer literal cannot have type `{}`",
                            render_ty(&other)
                        ),
                        span,
                    ))
                }
            }
        }
        Ok(())
    }

    fn monomorphic_name(&self, ty: &Ty, span: Span) -> Result<String, Diagnostic> {
        match self.store.zonk(ty) {
            Ty::Con { name, .. } => Ok(name),
            Ty::Var(_) | Ty::Rigid(_) => Err(self.err(
                Code::OperatorPolymorphic,
                "operators are notation, not overloading: the operand type must be monomorphic here (take the function as a parameter instead)"
                    .to_string(),
                span,
            )),
            Ty::Arrow { .. } => Err(self.err(
                Code::NonDerivable,
                "functions cannot be compared".to_string(),
                span,
            )),
        }
    }

    fn numeric_name(&self, ty: &Ty, span: Span) -> Result<String, Diagnostic> {
        let name = self.monomorphic_name(ty, span)?;
        if kernel::is_numeric_type(&name) {
            Ok(name)
        } else {
            Err(self.err(
                Code::TypeMismatch,
                format!("`{name}` is not a numeric type"),
                span,
            ))
        }
    }

    fn project_field(&mut self, ty: &Ty, field: &str, span: Span) -> Result<Ty, Diagnostic> {
        match self.store.zonk(ty) {
            Ty::Con { name, args } => {
                let td = self.typedef(&name).cloned();
                let Some(td) = td else {
                    return Err(self.err(
                        Code::TypeMismatch,
                        format!("`{name}` has no fields"),
                        span,
                    ));
                };
                let TypeBody::Record { fields } = &td.body else {
                    return Err(self.err(
                        Code::TypeMismatch,
                        format!("`{name}` is not a record type"),
                        span,
                    ));
                };
                let f = fields.iter().find(|f| f.name == field).ok_or_else(|| {
                    self.err(
                        Code::UnknownField,
                        format!("`{name}` has no field `{field}`"),
                        span,
                    )
                })?;
                let subst: HashMap<String, Ty> = td.params.iter().cloned().zip(args).collect();
                let shape = f.shape.clone();
                Ok(self.conv_sig(&shape, &subst))
            }
            Ty::Var(_) | Ty::Rigid(_) => Err(self.err(
                Code::AnnotationNeeded,
                format!("cannot determine the record type for field `{field}`; add an annotation"),
                span,
            )),
            Ty::Arrow { .. } => Err(self.err(
                Code::TypeMismatch,
                "a function has no fields".to_string(),
                span,
            )),
        }
    }

    /// A constructor used as a value or applied to one payload argument.
    fn ctor_value(
        &mut self,
        qualifier: Option<&str>,
        name: &str,
        span: Span,
        arg: Option<&Spanned<Expr>>,
    ) -> Result<Ty, Diagnostic> {
        let VariantInfo {
            con_ty,
            payload_ty,
            record_fields,
        } = self.variant_of(qualifier, name, span)?;
        match (payload_ty, record_fields, arg) {
            (None, None, None) => Ok(con_ty),
            (None, None, Some(_)) => Err(self.err(
                Code::TypeMismatch,
                format!("`{name}` takes no payload"),
                span,
            )),
            (Some(_), _, None) | (None, Some(_), None) => Err(self.err(
                Code::TypeMismatch,
                format!("`{name}` takes a payload; constructors must be fully applied"),
                span,
            )),
            (Some(p), _, Some(a)) => {
                let ta = self.infer_expr(a)?;
                self.unify(&ta, &p, a.span)?;
                Ok(con_ty)
            }
            (None, Some(fields), Some(a)) => {
                let Expr::RecordE {
                    update,
                    fields: inits,
                } = &a.item
                else {
                    return Err(self.err(
                        Code::TypeMismatch,
                        format!(
                            "`{name}` carries an inline record payload; write `{name} {{ … }}`"
                        ),
                        a.span,
                    ));
                };
                if update.0.is_some() {
                    return Err(self.err(
                        Code::TypeMismatch,
                        "record update cannot build a constructor payload".to_string(),
                        a.span,
                    ));
                }
                self.check_field_inits(inits, &fields, a.span)?;
                Ok(con_ty)
            }
        }
    }

    fn check_field_inits(
        &mut self,
        inits: &[ast::FieldInit],
        fields: &[(String, Ty)],
        span: Span,
    ) -> Result<(), Diagnostic> {
        for init in inits {
            let Some((_, fty)) = fields.iter().find(|(n, _)| *n == init.name) else {
                return Err(self.err(
                    Code::UnknownField,
                    format!("no field `{}` here", init.name),
                    span,
                ));
            };
            let t = self.infer_expr(&init.value)?;
            let fty = fty.clone();
            self.unify(&t, &fty, init.value.span)?;
        }
        for (n, _) in fields {
            if !inits.iter().any(|i| &i.name == n) {
                return Err(self.err(Code::TypeMismatch, format!("missing field `{n}`"), span));
            }
        }
        Ok(())
    }

    fn record_expr(
        &mut self,
        update: &Opt<ast::PathBase>,
        inits: &[ast::FieldInit],
        span: Span,
        expected: Option<&Ty>,
    ) -> Result<Ty, Diagnostic> {
        if let Opt(Some(base)) = update {
            let mut cur = self.resolve_value(&base.root, span)?;
            for f in &base.fields {
                cur = self.project_field(&cur, f, span)?;
            }
            let (fields, ty) = self.record_fields_of(&cur, span)?;
            for init in inits {
                let Some((_, fty)) = fields.iter().find(|(n, _)| *n == init.name) else {
                    return Err(self.err(
                        Code::UnknownField,
                        format!("no field `{}` on `{}`", init.name, render_ty(&ty)),
                        span,
                    ));
                };
                let t = self.infer_expr(&init.value)?;
                let fty = fty.clone();
                self.unify(&t, &fty, init.value.span)?;
            }
            return Ok(ty);
        }
        if let Some(t) = expected {
            let (fields, ty) = self.record_fields_of(t, span)?;
            self.check_field_inits(inits, &fields, span)?;
            return Ok(ty);
        }
        // Standalone literal: the record type with exactly this
        // (non-ignored) field-name set.
        let mut names: Vec<&str> = inits.iter().map(|i| i.name.as_str()).collect();
        names.sort_unstable();
        let matches: Vec<String> = self
            .prog
            .types
            .values()
            .filter(|td| match &td.body {
                TypeBody::Record { fields } => {
                    let mut fs: Vec<&str> = fields
                        .iter()
                        .filter(|f| f.ignored.0.is_none())
                        .map(|f| f.name.as_str())
                        .collect();
                    fs.sort_unstable();
                    fs == names
                }
                _ => false,
            })
            .map(|td| td.name.clone())
            .collect();
        match matches.as_slice() {
            [] => Err(self.err(
                Code::TypeMismatch,
                "no record type has exactly these fields".to_string(),
                span,
            )),
            [one] => {
                let one = one.clone();
                let ty = self.applied_type(&one, span)?;
                let (fields, _) = self.record_fields_of(&ty, span)?;
                self.check_field_inits(inits, &fields, span)?;
                Ok(ty)
            }
            _ => Err(self.err(
                Code::AnnotationNeeded,
                "several record types have these fields; add an annotation".to_string(),
                span,
            )),
        }
    }

    fn record_fields_of(
        &mut self,
        ty: &Ty,
        span: Span,
    ) -> Result<(Vec<(String, Ty)>, Ty), Diagnostic> {
        match self.store.zonk(ty) {
            Ty::Con { name, args } => {
                let td = self.typedef(&name).cloned();
                let Some(td) = td else {
                    return Err(self.err(
                        Code::TypeMismatch,
                        format!("`{name}` is not a record type"),
                        span,
                    ));
                };
                let TypeBody::Record { fields } = &td.body else {
                    return Err(self.err(
                        Code::TypeMismatch,
                        format!("`{name}` is not a record type"),
                        span,
                    ));
                };
                let subst: HashMap<String, Ty> =
                    td.params.iter().cloned().zip(args.clone()).collect();
                let out = fields
                    .iter()
                    .filter(|f| f.ignored.0.is_none())
                    .map(|f| (f.name.clone(), self.conv_sig(&f.shape.clone(), &subst)))
                    .collect();
                Ok((out, Ty::Con { name, args }))
            }
            Ty::Var(_) | Ty::Rigid(_) => Err(self.err(
                Code::AnnotationNeeded,
                "cannot determine the record type; add an annotation".to_string(),
                span,
            )),
            other => Err(self.err(
                Code::TypeMismatch,
                format!("`{}` is not a record type", render_ty(&other)),
                span,
            )),
        }
    }

    // ---- patterns ----

    fn check_pattern(&mut self, p: &Spanned<Pattern>, ty: &Ty) -> Result<(), Diagnostic> {
        match &p.item {
            Pattern::PWild => Ok(()),
            Pattern::PBind { binder } => {
                self.locals.last_mut().unwrap().insert(
                    binder.name.clone(),
                    Scheme {
                        ty: ty.clone(),
                        gen_tys: vec![],
                    },
                );
                Ok(())
            }
            Pattern::PLit { lit } => {
                let t = self.literal_ty(lit, Some(ty), p.span)?;
                let ty = ty.clone();
                self.unify(&t, &ty, p.span)
            }
            Pattern::PCtor {
                type_name,
                name,
                arg,
            } => {
                let VariantInfo {
                    con_ty,
                    payload_ty,
                    record_fields,
                } = self.variant_of(type_name.0.as_deref(), name, p.span)?;
                let ty = ty.clone();
                self.unify(&con_ty, &ty, p.span)?;
                match (payload_ty, record_fields, &arg.0) {
                    (None, None, None) => Ok(()),
                    (None, None, Some(a)) => Err(self.err(
                        Code::TypeMismatch,
                        format!("`{name}` takes no payload"),
                        a.span,
                    )),
                    (Some(_), _, None) | (None, Some(_), None) => Err(self.err(
                        Code::TypeMismatch,
                        format!("`{name}` carries a payload; match it (or `_`)"),
                        p.span,
                    )),
                    (Some(pt), _, Some(a)) => self.check_pattern(a, &pt),
                    (None, Some(fields), Some(a)) => match &a.item {
                        Pattern::PRecord { fields: fps, open } => {
                            self.check_record_pattern(fps, *open, &fields, a.span)
                        }
                        Pattern::PWild => Ok(()),
                        Pattern::PBind { .. } => Err(self.err(
                            Code::TypeMismatch,
                            format!("`{name}` carries an inline record payload; match its fields"),
                            a.span,
                        )),
                        _ => Err(self.err(
                            Code::TypeMismatch,
                            format!("`{name}` carries an inline record payload"),
                            a.span,
                        )),
                    },
                }
            }
            Pattern::PRecord { fields, open } => {
                let ty = ty.clone();
                let (rec_fields, _) = self.record_fields_of(&ty, p.span)?;
                self.check_record_pattern(fields, *open, &rec_fields, p.span)
            }
        }
    }

    fn check_record_pattern(
        &mut self,
        fps: &[ast::FieldPat],
        open: bool,
        fields: &[(String, Ty)],
        span: Span,
    ) -> Result<(), Diagnostic> {
        for fp in fps {
            let Some((_, fty)) = fields.iter().find(|(n, _)| *n == fp.name) else {
                return Err(self.err(
                    Code::UnknownField,
                    format!("no field `{}` here", fp.name),
                    span,
                ));
            };
            let fty = fty.clone();
            match &fp.pattern {
                Opt(Some(sub)) => self.check_pattern(sub, &fty)?,
                Opt(None) => {
                    self.locals.last_mut().unwrap().insert(
                        fp.name.clone(),
                        Scheme {
                            ty: fty,
                            gen_tys: vec![],
                        },
                    );
                }
            }
        }
        if !open {
            for (n, _) in fields {
                if !fps.iter().any(|fp| &fp.name == n) {
                    return Err(self.err(
                        Code::MissingRecordRest,
                        format!("field `{n}` is not matched; name every field or end with `..`"),
                        span,
                    ));
                }
            }
        }
        Ok(())
    }
}

fn numeric_op_row(ty: &str, op: &str) -> RowT {
    let panic = RowT::closed(EffSet::single(Effect::Panic));
    match (ty, op) {
        ("F64", _) => RowT::total(),
        ("BigInt", "div" | "mod") => panic,
        ("BigInt", _) => RowT::total(),
        _ => panic,
    }
}

fn describe_callee(e: &Expr) -> String {
    match e {
        Expr::Path { root, fields } => {
            let mut s = root.clone();
            for f in fields {
                s.push('.');
                s.push_str(f);
            }
            s
        }
        Expr::Qualified { space, name } => format!("{space}::{name}"),
        Expr::App { r#fn, .. } => describe_callee(&r#fn.item),
        _ => "<expression>".to_string(),
    }
}

fn render_ty(ty: &Ty) -> String {
    match ty {
        Ty::Var(_) => "_".to_string(),
        Ty::Rigid(n) => n.clone(),
        Ty::Con { name, args } => {
            if args.is_empty() {
                name.clone()
            } else {
                let mut s = name.clone();
                for a in args {
                    s.push(' ');
                    let inner = render_ty(a);
                    if inner.contains(' ') {
                        s.push('(');
                        s.push_str(&inner);
                        s.push(')');
                    } else {
                        s.push_str(&inner);
                    }
                }
                s
            }
        }
        Ty::Arrow { dom, cod, .. } => {
            format!("({} -> {})", render_ty(dom), render_ty(cod))
        }
    }
}

// ---- canonical signature output (contract §5, §8.3) ----

struct Renamer {
    map: HashMap<String, String>,
    next: usize,
}

impl Renamer {
    fn new() -> Self {
        Renamer {
            map: HashMap::new(),
            next: 0,
        }
    }

    fn rename(&mut self, name: &str) -> String {
        if let Some(r) = self.map.get(name) {
            return r.clone();
        }
        let i = self.next;
        self.next += 1;
        let letter = (b'a' + (i % 26) as u8) as char;
        let out = if i < 26 {
            letter.to_string()
        } else {
            format!("{letter}{}", i / 26)
        };
        self.map.insert(name.to_string(), out.clone());
        out
    }
}

fn canonical_sig(sig: &Spanned<AType>) -> SigType {
    let mut r = Renamer::new();
    canon(sig, &mut r)
}

fn canon(ty: &Spanned<AType>, r: &mut Renamer) -> SigType {
    match &ty.item {
        AType::Arrow {
            param,
            dom,
            row,
            cod,
        } => SigType::SArrow {
            param: Opt(param.0.as_ref().map(|b| b.name.clone())),
            dom: Box::new(canon(dom, r)),
            row: canon_row(row, r),
            cod: Box::new(canon(cod, r)),
        },
        AType::Con { name, args } => SigType::SCon {
            name: name.clone(),
            args: args.iter().map(|a| canon(a, r)).collect(),
        },
        AType::TVar { name } => SigType::SVar {
            name: r.rename(name),
        },
        AType::Refined { base, .. } => canon(base, r),
    }
}

fn canon_row(row: &ast::Row, r: &mut Renamer) -> ast::Row {
    let mut effects = row.effects.clone();
    effects.sort();
    effects.dedup();
    ast::Row {
        effects,
        var: Opt(row.var.0.as_ref().map(|v| r.rename(v))),
    }
}

/// The definition's own row: the row on the arrow peeled by the last
/// equation parameter (empty for zero parameters), canonically renamed
/// consistently with the signature.
fn canonical_row_of_def(def: &ast::Def) -> ast::Row {
    let mut r = Renamer::new();
    // Walk in the same order as `canon` so shared variables agree.
    let _ = canon(&def.sig, &mut r);
    let mut cur = &def.sig.item;
    let mut row = ast::Row {
        effects: vec![],
        var: Opt(None),
    };
    for (i, _) in def.params.iter().enumerate() {
        if let AType::Arrow {
            row: arrow_row,
            cod,
            ..
        } = cur
        {
            if i + 1 == def.params.len() {
                row = canon_row(arrow_row, &mut r);
            }
            cur = &cod.item;
        }
    }
    row
}
