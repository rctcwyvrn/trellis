//! `trellis test` (impl plan 03 step 7): expect tests through soil0
//! bundles (contract §10 verbatim), the §8.12 property runner over
//! synthesized wrapper definitions, clause-derived properties, the
//! differential tier, and the results in the lock-schema §5
//! vocabulary. Invariant-derived properties are refused in v1
//! (`unsupported-invariant-property`, resolved 2026-08-28): they are
//! constrained generation in disguise, which arrives with plan 05.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use soil0::ast::{CmpOp, Lit, PExpr, Pred, Type};
use soil0::interp::{self, Session};
use soil0::manifest::{TypeBody, TypeDef};

use crate::config::Config;
use crate::diag::{Diag, ErrorReport};
use crate::lock::{self, Checks, Lock, Oracle, TestRow};
use crate::preflight;
use crate::propgen::{self, Gen, TypeEnv};
use crate::registry;
use crate::state::{self, Entry, Root};
use crate::trfile::{tests_blk, FileKind, Payload};

/// Capability type names (contract §6.1): parameters of these types
/// are injected, never generated or JSON-decoded.
pub const CAPS: [&str; 8] = ["World", "Fs", "Net", "Clock", "Env", "Proc", "Rand", "Py"];

// ---- the per-definition outcome ----

#[derive(Default)]
pub struct Outcome {
    /// `None` when the static pipeline did not complete (the lock's
    /// existing facts are left alone).
    pub checks: Option<Checks>,
    pub tests: Vec<TestRow>,
    pub oracles: Vec<Oracle>,
    /// Refusals, preflight findings, and run problems — the spec
    /// needs attention when any of these exist.
    pub errors: Vec<Diag>,
    /// Failure detail per row (counterexample, diff) — report and
    /// `f.log` material; never stored in the lock (lock-schema §5).
    pub details: Vec<Detail>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Detail {
    pub row: String,
    pub message: String,
}

// ---- signature spine ----

struct SigParts {
    /// (name, base type) per parameter, refinements stripped;
    /// capability-typed parameters are flagged.
    params: Vec<Param>,
    /// Declared row carries `io` or `ffi` — the §8.12 fake-involved
    /// case count applies.
    has_io_ffi: bool,
}

struct Param {
    name: String,
    ty: Type,
    is_cap: bool,
}

fn strip_refined(ty: &Type) -> &Type {
    match ty {
        Type::Refined { base, .. } => strip_refined(&base.item),
        other => other,
    }
}

fn sig_parts(entry: &Entry) -> Option<SigParts> {
    let sig = entry.tr.blocks.iter().find_map(|b| match &b.payload {
        Payload::Sig(sig) => Some(sig),
        _ => None,
    })?;
    let mut params = Vec::new();
    let mut has_io_ffi = false;
    let mut ty = &sig.ty.item;
    loop {
        match strip_refined(ty) {
            Type::Arrow {
                param,
                dom,
                row,
                cod,
            } => {
                has_io_ffi |= row
                    .effects
                    .iter()
                    .any(|e| matches!(e, soil0::ast::Effect::Io | soil0::ast::Effect::Ffi));
                let base = strip_refined(&dom.item).clone();
                let is_cap = matches!(&base, Type::Con { name, args }
                    if args.is_empty() && CAPS.contains(&name.as_str()));
                params.push(Param {
                    name: param
                        .0
                        .as_ref()
                        .map(|b| b.name.clone())
                        .unwrap_or_else(|| format!("p{}", params.len())),
                    ty: base,
                    is_cap,
                });
                ty = &cod.item;
            }
            _result => {
                return Some(SigParts { params, has_io_ffi });
            }
        }
    }
}

// ---- surface-type printing (wrapper signatures) ----

fn type_text(ty: &Type) -> String {
    match ty {
        Type::TVar { name } => name.clone(),
        Type::Con { name, args } if args.is_empty() => name.clone(),
        Type::Con { name, args } => {
            let rendered: Vec<String> = args.iter().map(|a| type_arg_text(&a.item)).collect();
            format!("{name} {}", rendered.join(" "))
        }
        Type::Refined { base, .. } => type_text(&base.item),
        Type::Arrow { .. } => "<function>".into(), // refused upstream
    }
}

fn type_arg_text(ty: &Type) -> String {
    match strip_refined(ty) {
        Type::Con { name, args } if !args.is_empty() => {
            let _ = (name, args);
            format!("({})", type_text(ty))
        }
        _ => type_text(ty),
    }
}

// ---- predicate lowering (micro-pin §8.12) ----

/// Constructor payload shapes, for `is` patterns.
pub enum CtorShape {
    None,
    Payload,
    Record,
}

pub struct PredCtx {
    ctors: BTreeMap<String, CtorShape>,
}

impl PredCtx {
    pub fn new(user_types: &[TypeDef]) -> PredCtx {
        let mut ctors = BTreeMap::new();
        for td in soil0::kernel::kernel_typedefs()
            .iter()
            .chain(user_types.iter())
        {
            if let TypeBody::Sum { variants } = &td.body {
                for v in variants {
                    let shape = if v.payload.0.is_some() {
                        CtorShape::Payload
                    } else if v.record_fields.is_some() {
                        CtorShape::Record
                    } else {
                        CtorShape::None
                    };
                    ctors.insert(v.name.clone(), shape);
                }
            }
        }
        PredCtx { ctors }
    }

    fn wildcard_pattern(&self, ctor: &str) -> String {
        match self.ctors.get(ctor) {
            Some(CtorShape::Payload) => format!("{ctor} _"),
            Some(CtorShape::Record) => format!("{ctor} {{ .. }}"),
            _ => ctor.to_string(),
        }
    }
}

/// Lower a predicate to Soil expression text: `implies` becomes
/// `not a or b`, `is` becomes a match; an `is` antecedent with a
/// binder scopes that binder over the consequent, so the whole
/// implication becomes the match.
pub fn pred_text(pred: &Pred, ctx: &PredCtx) -> String {
    match pred {
        Pred::Implies { lhs, rhs } => {
            if let Pred::Is {
                scrutinee,
                ctor,
                payload,
                ..
            } = &lhs.item
            {
                let pattern = match payload.0.as_ref() {
                    Some(binder) => format!("{ctor} {}", binder.name),
                    None => ctx.wildcard_pattern(ctor),
                };
                return format!(
                    "(match {} with | {pattern} -> {} | _ -> True)",
                    pexpr_text(&scrutinee.item),
                    pred_text(&rhs.item, ctx),
                );
            }
            format!(
                "(not {} or {})",
                pred_text(&lhs.item, ctx),
                pred_text(&rhs.item, ctx)
            )
        }
        Pred::POr { lhs, rhs } => format!(
            "({} or {})",
            pred_text(&lhs.item, ctx),
            pred_text(&rhs.item, ctx)
        ),
        Pred::PAnd { lhs, rhs } => format!(
            "({} and {})",
            pred_text(&lhs.item, ctx),
            pred_text(&rhs.item, ctx)
        ),
        Pred::PNot { pred } => format!("(not {})", pred_text(&pred.item, ctx)),
        Pred::PCmp { op, lhs, rhs } => format!(
            "({} {} {})",
            pexpr_text(&lhs.item),
            cmp_text(*op),
            pexpr_text(&rhs.item)
        ),
        Pred::Is {
            scrutinee,
            ctor,
            payload,
            ..
        } => {
            let pattern = match payload.0.as_ref() {
                Some(binder) => format!("{ctor} {}", binder.name),
                None => ctx.wildcard_pattern(ctor),
            };
            format!(
                "(match {} with | {pattern} -> True | _ -> False)",
                pexpr_text(&scrutinee.item)
            )
        }
        Pred::PCall { name, args } => call_text(name, args),
    }
}

fn cmp_text(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "==",
        CmpOp::Ne => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
    }
}

fn call_text(name: &str, args: &[soil0::span::Spanned<PExpr>]) -> String {
    let rendered: Vec<String> = args.iter().map(|a| pexpr_text(&a.item)).collect();
    format!("({name} {})", rendered.join(" "))
}

/// Structural equality up to rendering — spans ignored (the vacuity
/// probes' notion of "syntactically identical").
pub fn pexpr_eq(a: &PExpr, b: &PExpr) -> bool {
    pexpr_text(a) == pexpr_text(b)
}

fn pexpr_text(e: &PExpr) -> String {
    match e {
        PExpr::PLiteral { lit } => match lit {
            Lit::LInt { digits } => digits.clone(),
            Lit::LFloat { text } => text.clone(),
            Lit::LStr { value } => serde_json::to_string(value).expect("string renders"),
        },
        PExpr::PPath { root, fields } => {
            let mut s = root.clone();
            for f in fields {
                s.push('.');
                s.push_str(f);
            }
            s
        }
        PExpr::PArith { op, lhs, rhs } => {
            let sym = match op {
                soil0::ast::ArithOp::Add => "+",
                soil0::ast::ArithOp::Sub => "-",
                soil0::ast::ArithOp::Mul => "*",
                _ => "+", // parser admits only Add/Sub/Mul (tr-grammar §2.3)
            };
            format!(
                "({} {sym} {})",
                pexpr_text(&lhs.item),
                pexpr_text(&rhs.item)
            )
        }
        PExpr::PCallE { name, args } => call_text(name, args),
    }
}

// ---- wrapper synthesis ----

/// One synthesized single-definition file, appended to the assembly
/// after every real source (wrappers call everything, so they are
/// maximal in the callee-first order). Names are `t7_*` — public,
/// same-module, filename-matching, and colliding with nothing the
/// examples or any sane root define.
pub struct Wrapper {
    pub rel: String,
    pub name: String,
    pub source: String,
}

fn wrapper(dir: &str, name: &str, params: &[(String, String)], body: &str) -> Wrapper {
    let rel = if dir.is_empty() {
        format!("{name}.soil")
    } else {
        format!("{dir}/{name}.soil")
    };
    let sig: Vec<String> = params
        .iter()
        .map(|(n, t)| format!("({n} : {t}) -> "))
        .collect();
    let args: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
    Wrapper {
        rel,
        name: name.to_string(),
        source: format!(
            "{name} : {}Bool\n{name} {} =\n  {body}\n",
            sig.join(""),
            args.join(" ")
        ),
    }
}

// ---- the runner ----

/// Test one definition (function or type): every tier, cram (mode
/// `real`) included since step 8 — the real-mode exclusion is about
/// the lowering sandbox (the MCP `run_tests` tool), not the human CLI.
///
/// Runs on its own generously-stacked thread: the soil0 interpreter
/// recurses with the program (deep `let rec` evaluation), and the
/// 2 MiB spawned-thread default is too small for real property runs
/// regardless of which thread the caller happens to be on.
pub fn test_def(root: &Root, entry: &Entry) -> Result<Outcome, ErrorReport> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn_scoped(scope, || test_def_inner(root, entry))
            .expect("spawn test thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

fn test_def_inner(root: &Root, entry: &Entry) -> Result<Outcome, ErrorReport> {
    match entry.kind {
        FileKind::Type => Ok(type_outcome(root, entry)),
        FileKind::Function if entry.soil.is_some() => function_outcome(root, entry),
        _ => Ok(Outcome::default()),
    }
}

fn clause_blocks<'e>(entry: &'e Entry, kind: &str) -> Vec<&'e crate::trfile::predicate::Clause> {
    entry
        .tr
        .blocks
        .iter()
        .filter(|b| b.id == kind)
        .filter_map(|b| match &b.payload {
            Payload::Clauses(clauses) => Some(clauses.iter()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn type_outcome(root: &Root, entry: &Entry) -> Outcome {
    let mut out = Outcome::default();
    let invariants = clause_blocks(entry, "invariant");
    let types_ok = state::compile(root).is_ok();
    out.checks = Some(Checks::Type {
        types: if types_ok { "ok" } else { "error" }.into(),
        invariants: invariants
            .iter()
            .map(|c| (c.label.clone(), "runtime".to_string()))
            .collect(),
    });
    if !invariants.is_empty() {
        out.errors.push(registry::enrich(Diag::new(
            "unsupported-invariant-property",
            format!(
                "invariant-derived properties for `{}` need constrained generation \
                 and arrive with plan 05; the invariant stays a runtime check \
                 (tr-grammar §4.2, resolved 2026-08-28)",
                entry.tr.name
            ),
        )));
    }
    out
}

fn diagf(errors: &mut Vec<Diag>, d: Diag) {
    errors.push(registry::enrich(d));
}

fn function_outcome(root: &Root, entry: &Entry) -> Result<Outcome, ErrorReport> {
    let mut out = Outcome::default();
    let def = entry.tr.name.clone();
    let module = entry
        .rel
        .rsplit_once('/')
        .map(|(m, _)| m.to_string())
        .unwrap_or_default();
    let Some(sig) = sig_parts(entry) else {
        return Ok(out); // unreachable for a valid function file
    };
    let requires = clause_blocks(entry, "requires");
    let ensures = clause_blocks(entry, "ensures");

    // The environment three ways: the program's, the generators', the
    // predicate context's.
    let env_types: Vec<TypeDef> = state::env_of_root(root)?.types;
    let type_env = TypeEnv::new(&env_types);
    let pred_ctx = PredCtx::new(&env_types);

    // ---- wrapper synthesis ----
    let data_params: Vec<&Param> = sig.params.iter().filter(|p| !p.is_cap).collect();
    let has_caps = data_params.len() != sig.params.len();
    let wrapper_params: Vec<(String, String)> = data_params
        .iter()
        .map(|p| (p.name.clone(), type_text(&p.ty)))
        .collect();
    let requires_guard: Option<String> = match requires.len() {
        0 => None,
        _ => Some(
            requires
                .iter()
                .map(|c| pred_text(&c.pred.item, &pred_ctx))
                .collect::<Vec<_>>()
                .join(" and "),
        ),
    };

    let mut wrappers: Vec<Wrapper> = Vec::new();
    // Requires evaluators — preflight's vacuity probe.
    if !has_caps {
        for (i, clause) in requires.iter().enumerate() {
            wrappers.push(wrapper(
                &module,
                &format!("t7_req_{i}"),
                &wrapper_params,
                &pred_text(&clause.pred.item, &pred_ctx),
            ));
        }
        // Derived-ensures properties (design §4.1), guarded by the
        // requires conjunction: `not req or (let result = f … in ens)`.
        for (i, clause) in ensures.iter().enumerate() {
            let call = format!(
                "(let result = ({def} {}) in {})",
                data_params
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                pred_text(&clause.pred.item, &pred_ctx),
            );
            let body = match &requires_guard {
                Some(guard) => format!("not ({guard}) or {call}"),
                None => call,
            };
            wrappers.push(wrapper(
                &module,
                &format!("t7_ens_{i}"),
                &wrapper_params,
                &body,
            ));
        }
    } else if !ensures.is_empty() || !requires.is_empty() {
        diagf(
            &mut out.errors,
            Diag::new(
                "test-ungenerable-type",
                format!(
                    "`{def}` takes capability parameters; clause-derived properties \
                     need generable inputs and are skipped in v1 (impl plan 03 §8.12)"
                ),
            ),
        );
    }

    // Spec properties. A `where` filter is refused (§9.8): never
    // silently ignored, never rejection-sampled.
    struct Prop<'e> {
        block_name: String,
        xfail: bool,
        wrapper: String,
        binders: Vec<&'e tests_blk::Forall>,
    }
    let mut props: Vec<Prop> = Vec::new();
    for block in &entry.tr.blocks {
        let Payload::Property(prop) = &block.payload else {
            continue;
        };
        let block_name = block.name.clone().unwrap_or_default();
        if prop.foralls.iter().any(|f| f.filter.is_some()) {
            diagf(
                &mut out.errors,
                Diag::new(
                    "unsupported-where-filter",
                    format!(
                        "property `{block_name}` uses a `where` filter; constrained \
                         generation arrives with plan 05 (tr-grammar §3.4)"
                    ),
                ),
            );
            continue;
        }
        let index = props.len();
        wrappers.push(wrapper(
            &module,
            &format!("t7_prop_{index}"),
            &prop
                .foralls
                .iter()
                .map(|f| (f.name.clone(), type_text(&f.ty.item)))
                .collect::<Vec<_>>(),
            &pred_text(&prop.pred.item, &pred_ctx),
        ));
        props.push(Prop {
            block_name,
            xfail: block.xfail,
            wrapper: format!("t7_prop_{index}"),
            binders: prop.foralls.iter().collect(),
        });
    }

    // ---- the two programs ----
    let state::Assembly { env, ordered } = state::assemble(root)?;
    let prog_base = soil0::manifest::from_parts(env, &ordered).map_err(soil0_report)?;

    let mut extended = ordered.clone();
    for w in &wrappers {
        extended.push((w.rel.clone(), w.source.clone()));
    }
    let env2 = state::env_of_root(root)?;
    let session = match interp::session(
        soil0::manifest::from_parts(env2, &extended).map_err(soil0_report)?,
    ) {
        Ok(s) => Some(s),
        Err(d) => {
            // A wrapper that fails the pipeline is a spec bug (the
            // predicate does not type against the signature); expect
            // tests still run below.
            diagf(&mut out.errors, soil0_diag(&d));
            None
        }
    };

    // ---- expect bundles ----
    let bundle = build_bundle(entry);
    let mut expect_results: BTreeMap<String, (String, Option<(String, String)>)> = BTreeMap::new();
    if !bundle.cases.is_empty() {
        let doc = serde_json::to_string(&bundle).expect("bundle serializes");
        match interp::run_bundle(prog_base, &doc) {
            Err(d) => diagf(&mut out.errors, soil0_diag(&d)),
            Ok((report, _exit)) => {
                let parsed: BundleReport =
                    serde_json::from_str(&report).expect("contract §10 output shape");
                for case in parsed.cases {
                    let details = case.details.map(|d| (d.expected, d.actual));
                    expect_results.insert(case.name, (case.result, details));
                }
            }
        }
    }

    // ---- preflight (before any property spends work) ----
    if let Some(s) = &session {
        let cap_flags: Vec<bool> = sig.params.iter().map(|p| p.is_cap).collect();
        preflight::run(
            s,
            entry,
            &def,
            &cap_flags,
            &requires,
            &ensures,
            &env_types,
            &mut out.errors,
        );
    }

    // ---- property + derived runs ----
    let seed_hash = entry
        .spec
        .test
        .clone()
        .unwrap_or_else(|| entry.spec.formal.clone());
    let cases = if sig.has_io_ffi { 32 } else { 128 };

    let mut property_rows: BTreeMap<String, TestRow> = BTreeMap::new();
    if let Some(s) = &session {
        for prop in &props {
            let binder_tys: Result<Vec<_>, Diag> = prop
                .binders
                .iter()
                .map(|f| crate::envgen::sig_type(&f.ty.item))
                .collect();
            let row = match binder_tys {
                Err(d) => {
                    diagf(&mut out.errors, d);
                    continue;
                }
                Ok(tys) => run_property(
                    s,
                    &prop.wrapper,
                    &tys,
                    &type_env,
                    &seed_hash,
                    cases,
                    &prop.block_name,
                    &mut out,
                ),
            };
            let result = match (row, prop.xfail) {
                (true, false) => "pass",
                (true, true) => "xpass",
                (false, true) => "xfail",
                (false, false) => "fail",
            };
            property_rows.insert(
                prop.block_name.clone(),
                test_row(&prop.block_name, "property", "spec", result),
            );
        }
    }

    let mut derived_rows: Vec<TestRow> = Vec::new();
    if let Some(s) = &session {
        if !has_caps {
            let binder_tys: Result<Vec<_>, Diag> = data_params
                .iter()
                .map(|p| crate::envgen::sig_type(&p.ty))
                .collect();
            match binder_tys {
                Err(d) => diagf(&mut out.errors, d),
                Ok(tys) => {
                    for (i, clause) in ensures.iter().enumerate() {
                        let name = format!("derived:ensures:{}", clause.label);
                        let ok = run_property(
                            s,
                            &format!("t7_ens_{i}"),
                            &tys,
                            &type_env,
                            &seed_hash,
                            cases,
                            &name,
                            &mut out,
                        );
                        derived_rows.push(test_row(
                            &name,
                            "property",
                            "derived",
                            if ok { "pass" } else { "fail" },
                        ));
                    }
                }
            }
        }
    }

    // ---- the differential tier ----
    if let Some(s) = &session {
        differential(root, entry, &def, s, has_caps, &mut derived_rows, &mut out);
    } else if reference_of(entry).is_some() {
        // The oracle row still records what the tests depend on.
        record_oracle(root, entry, &mut out);
    }

    // ---- checks facts ----
    if let Some(s) = &session {
        out.checks = checks_facts(s, &def, &requires, &ensures);
    }

    // ---- rows in document order, then derived ----
    for block in &entry.tr.blocks {
        match &block.payload {
            Payload::Tests(t) => {
                let block_name = block.name.clone().unwrap_or_default();
                for k in 1..=t.cases.len() {
                    let name = format!("{block_name}#{k}");
                    if let Some((result, details)) = expect_results.get(&name) {
                        out.tests.push(test_row(&name, "expect", "spec", result));
                        if let Some((expected, actual)) = details {
                            out.details.push(Detail {
                                row: name,
                                message: format!("expected {expected}, got {actual}"),
                            });
                        }
                    }
                }
            }
            Payload::Property(_) => {
                let block_name = block.name.clone().unwrap_or_default();
                if let Some(row) = property_rows.remove(&block_name) {
                    out.tests.push(row);
                }
            }
            Payload::Cram(cram) => {
                let block_name = block.name.clone().unwrap_or_default();
                let cram_out = crate::cram::run_block(&root.root, cram);
                out.errors.extend(cram_out.errors);
                for (k, step) in cram_out.steps.iter().enumerate() {
                    let name = format!("{block_name}#{}", k + 1);
                    let result = match (step.passed, block.xfail) {
                        (true, false) => "pass",
                        (true, true) => "xpass",
                        (false, true) => "xfail",
                        (false, false) => "fail",
                    };
                    out.tests.push(TestRow {
                        name: name.clone(),
                        tier: "cram".into(),
                        mode: "real".into(),
                        origin: "spec".into(),
                        result: result.into(),
                    });
                    if let Some(detail) = &step.detail {
                        out.details.push(Detail {
                            row: name,
                            message: detail.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    out.tests.append(&mut derived_rows);
    Ok(out)
}

fn test_row(name: &str, tier: &str, origin: &str, result: &str) -> TestRow {
    TestRow {
        name: name.into(),
        tier: tier.into(),
        mode: "sandboxed".into(),
        origin: origin.into(),
        result: result.into(),
    }
}

fn soil0_diag(d: &soil0::diag::Diagnostic) -> Diag {
    let mut diag = Diag::new(d.code.as_str(), d.message.clone());
    diag.file = d.file.0.clone();
    diag
}

fn soil0_report(d: soil0::diag::Diagnostic) -> ErrorReport {
    ErrorReport {
        errors: vec![registry::enrich(soil0_diag(&d))],
    }
}

/// Run one wrapper over `count` generated cases; `true` iff every
/// case returned `True`. The stream restarts from the `test_hash`
/// seed per test, so each test's inputs are independent of the
/// others' presence (micro-pin §8.12).
#[allow(clippy::too_many_arguments)]
fn run_property(
    s: &Session,
    wrapper: &str,
    binder_tys: &[soil0::types::SigType],
    type_env: &TypeEnv,
    seed_hash: &str,
    count: usize,
    row_name: &str,
    out: &mut Outcome,
) -> bool {
    let mut gen = Gen::from_hash(seed_hash);
    for _ in 0..count {
        let mut args = Vec::with_capacity(binder_tys.len());
        let mut generated_ok = true;
        for ty in binder_tys {
            match propgen::generate(&mut gen, type_env, ty) {
                Ok(v) => args.push(v),
                Err(d) => {
                    diagf(&mut out.errors, d);
                    generated_ok = false;
                    break;
                }
            }
        }
        if !generated_ok {
            return false;
        }
        match interp::call_json(s, wrapper, &args) {
            Err(d) => {
                diagf(&mut out.errors, soil0_diag(&d));
                return false;
            }
            Ok(Err(panic)) => {
                out.details.push(Detail {
                    row: row_name.into(),
                    message: format!(
                        "counterexample {}: {panic}",
                        serde_json::to_string(&args).expect("args serialize")
                    ),
                });
                return false;
            }
            Ok(Ok(result)) => {
                if result != "true" {
                    out.details.push(Detail {
                        row: row_name.into(),
                        message: format!(
                            "counterexample {}",
                            serde_json::to_string(&args).expect("args serialize")
                        ),
                    });
                    return false;
                }
            }
        }
    }
    true
}

fn reference_of(entry: &Entry) -> Option<&tests_blk::Reference> {
    entry.tr.blocks.iter().find_map(|b| match &b.payload {
        Payload::Reference(r) => Some(r),
        _ => None,
    })
}

fn record_oracle(root: &Root, entry: &Entry, out: &mut Outcome) {
    let Some(reference) = reference_of(entry) else {
        return;
    };
    match reference {
        tests_blk::Reference::Python { path, symbol } => {
            match crate::oracle::hash_reference(&root.root, path) {
                Ok(hash) => out.oracles.push(Oracle::Reference {
                    kind: "reference".into(),
                    path: format!("{path}::{symbol}"),
                    hash,
                }),
                Err(d) => diagf(&mut out.errors, d),
            }
        }
        tests_blk::Reference::Cli { command } => match crate::oracle::hash_cli(command) {
            Ok(hash) => out.oracles.push(Oracle::Cli {
                kind: "cli".into(),
                command: command.clone(),
                hash,
            }),
            Err(d) => diagf(&mut out.errors, d),
        },
    }
}

/// The differential tier (§9.9): each expect case's canonical args
/// through the reference, compared to the Soil result through
/// canonical re-encode. One row per attachment.
fn differential(
    root: &Root,
    entry: &Entry,
    def: &str,
    s: &Session,
    has_caps: bool,
    derived_rows: &mut Vec<TestRow>,
    out: &mut Outcome,
) {
    let Some(reference) = reference_of(entry) else {
        return;
    };
    record_oracle(root, entry, out);
    let line = match reference {
        tests_blk::Reference::Python { path, symbol } => format!("{path}::{symbol}"),
        tests_blk::Reference::Cli { command } => format!("cli {command}"),
    };
    let row_name = format!("derived:differential:{line}");
    if has_caps {
        diagf(
            &mut out.errors,
            Diag::new(
                "oracle-error",
                format!(
                    "`{def}` takes capability parameters; the differential tier \
                     needs a pure entry in v1 (impl plan 03 §9.9)"
                ),
            ),
        );
        return;
    }
    let Ok((_, result_ty)) = interp::entry_tys(s, def) else {
        return;
    };
    let mut all_match = true;
    'blocks: for block in &entry.tr.blocks {
        let Payload::Tests(t) = &block.payload else {
            continue;
        };
        if block.xfail {
            continue; // the expectation is known-wrong; nothing to anchor
        }
        for case in &t.cases {
            if !matches!(case.outcome, tests_blk::Outcome::Value(_)) {
                continue;
            }
            let args: Vec<serde_json::Value> = case
                .args
                .iter()
                .filter_map(|a| match a {
                    tests_blk::TestArg::Json(v) => Some(v.clone()),
                    tests_blk::TestArg::Binding(_) => None,
                })
                .collect();
            let args_value = serde_json::Value::Array(args.clone());
            let reference_out = match reference {
                tests_blk::Reference::Python { path, symbol } => {
                    crate::oracle::run_python(&root.root, path, symbol, &args_value)
                }
                tests_blk::Reference::Cli { command } => {
                    crate::oracle::run_cli(&root.root, command, &args_value)
                }
            };
            let reference_out = match reference_out {
                Ok(v) => v,
                Err(d) => {
                    diagf(&mut out.errors, d);
                    all_match = false;
                    break 'blocks;
                }
            };
            let reference_canon = match interp::canonical_json(s, &result_ty, &reference_out) {
                Ok(c) => c,
                Err(d) => {
                    diagf(&mut out.errors, soil0_diag(&d));
                    all_match = false;
                    break 'blocks;
                }
            };
            let actual = match interp::call_json(s, def, &args) {
                Ok(Ok(v)) => v,
                Ok(Err(panic)) => {
                    out.details.push(Detail {
                        row: row_name.clone(),
                        message: format!("{args_value}: soil panicked: {panic}"),
                    });
                    all_match = false;
                    continue;
                }
                Err(d) => {
                    diagf(&mut out.errors, soil0_diag(&d));
                    all_match = false;
                    break 'blocks;
                }
            };
            if actual != reference_canon {
                out.details.push(Detail {
                    row: row_name.clone(),
                    message: format!("{args_value}: soil {actual} != reference {reference_canon}"),
                });
                all_match = false;
            }
        }
    }
    derived_rows.push(test_row(
        &row_name,
        "differential",
        "derived",
        if all_match { "pass" } else { "fail" },
    ));
}

fn checks_facts(
    s: &Session,
    def: &str,
    requires: &[&crate::trfile::predicate::Clause],
    ensures: &[&crate::trfile::predicate::Clause],
) -> Option<Checks> {
    let idx = s.core.prog.defs.iter().position(|d| d.name() == def)?;
    let info = &s.infer_out.defs[idx];
    let facts = &info.checks;
    let mut refinements: BTreeMap<String, String> = BTreeMap::new();
    for clause in requires.iter().chain(ensures.iter()) {
        refinements.insert(clause.label.clone(), "runtime".into());
    }
    Some(Checks::Function {
        types: "ok".into(),
        termination: facts.termination.to_string(),
        refinements,
        holes: facts.holes.len() as u64,
    })
}

// ---- bundle assembly (contract §10, verbatim shapes) ----

#[derive(Serialize)]
pub struct Bundle {
    pub program: String,
    pub cases: Vec<BundleCase>,
}

#[derive(Serialize)]
pub struct BundleCase {
    pub name: String,
    pub def: String,
    pub binds: Vec<BundleBind>,
    pub args: Vec<serde_json::Value>,
    pub expect: serde_json::Value,
    pub xfail: bool,
}

#[derive(Serialize, Clone)]
pub struct BundleBind {
    pub bind: String,
    pub ctor: String,
    pub args: Vec<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct BundleReport {
    cases: Vec<BundleCaseOut>,
}

#[derive(serde::Deserialize)]
struct BundleCaseOut {
    name: String,
    result: String,
    #[serde(default, deserialize_with = "opt_details")]
    details: Option<BundleDetails>,
}

#[derive(serde::Deserialize)]
struct BundleDetails {
    expected: String,
    actual: String,
}

/// `details` arrives in the sum encoding (`{"tag":"None"}` /
/// `{"tag":"Some","value":…}`, contract §10).
fn opt_details<'de, D>(de: D) -> Result<Option<BundleDetails>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(tag = "tag", content = "value")]
    enum Tagged {
        None,
        Some(BundleDetails),
    }
    Ok(match <Tagged as serde::Deserialize>::deserialize(de)? {
        Tagged::None => None,
        Tagged::Some(d) => Some(d),
    })
}

/// Every expect case of every `test` block, named `block#k`.
pub fn build_bundle(entry: &Entry) -> Bundle {
    let mut cases = Vec::new();
    for block in &entry.tr.blocks {
        let Payload::Tests(t) = &block.payload else {
            continue;
        };
        let block_name = block.name.clone().unwrap_or_default();
        let binds: Vec<BundleBind> = t
            .withs
            .iter()
            .map(|w| BundleBind {
                bind: w.name.clone(),
                ctor: w.ctor.clone(),
                args: w.args.clone(),
            })
            .collect();
        for (k, case) in t.cases.iter().enumerate() {
            cases.push(BundleCase {
                name: format!("{block_name}#{}", k + 1),
                def: entry.tr.name.clone(),
                binds: binds.clone(),
                args: case
                    .args
                    .iter()
                    .map(|a| match a {
                        tests_blk::TestArg::Json(v) => {
                            serde_json::json!({ "tag": "Json", "value": v })
                        }
                        tests_blk::TestArg::Binding(name) => {
                            serde_json::json!({ "tag": "Binding", "value": { "name": name } })
                        }
                    })
                    .collect(),
                expect: match &case.outcome {
                    tests_blk::Outcome::Value(v) => {
                        serde_json::json!({ "tag": "Value", "value": v })
                    }
                    tests_blk::Outcome::Panic => serde_json::json!({ "tag": "Panic" }),
                },
                xfail: block.xfail,
            });
        }
    }
    Bundle {
        program: "program.json".into(),
        cases,
    }
}

// ---- the daemon entry: run, merge, write, report ----

#[derive(Serialize)]
pub struct TestReport {
    pub defs: Vec<DefReport>,
}

#[derive(Serialize)]
pub struct DefReport {
    pub path: String,
    pub tests: Vec<TestRow>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<Detail>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<Diag>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

pub fn run(
    root_path: &Path,
    config: &Config,
    target: Option<&str>,
) -> Result<TestReport, ErrorReport> {
    let root = state::scan(root_path, config)?;
    let targets: Vec<&Entry> = match target {
        Some(name) => {
            let entry = root
                .entries
                .values()
                .find(|e| e.tr.name == name || e.rel == name)
                .ok_or_else(|| {
                    ErrorReport::one("usage", format!("no definition `{name}` in this root"))
                })?;
            if entry.kind == FileKind::Function && entry.soil.is_none() {
                return Err(ErrorReport {
                    errors: vec![registry::enrich(Diag::new(
                        "no-draft",
                        format!("`{name}` has no Soil yet; `trellis lower {name}` first"),
                    ))],
                });
            }
            vec![entry]
        }
        None => root
            .entries
            .values()
            .filter(|e| {
                matches!(e.kind, FileKind::Type)
                    || (e.kind == FileKind::Function && e.soil.is_some())
            })
            .collect(),
    };

    let mut defs = Vec::new();
    for entry in targets {
        let outcome = test_def(&root, entry)?;
        let mut notes = Vec::new();

        // Merge into the sidecar.
        let lock_path = root_path.join(format!("{}.lock", entry.rel));
        if let Some(existing) = &entry.lock {
            let mut lock: Lock = existing.clone();
            if let Some(checks) = &outcome.checks {
                lock.checks = Some(checks.clone());
            }
            if lock.tests.is_some() {
                lock.tests = Some(outcome.tests.clone());
            }
            if lock.oracles.is_some() {
                lock.oracles = Some(outcome.oracles.clone());
            }
            // Lock-schema §8 invariant 1: a failing (or xfail) row
            // demotes `accepted` — the daemon never writes an
            // accepted entry whose tests do not all pass.
            if lock.accepted
                && lock.kind == "function"
                && !outcome.tests.iter().all(|r| r.result == "pass")
            {
                lock.accepted = false;
                notes.push("accepted demoted: not every test passes".into());
            }
            lock::validate(&lock)?;
            let rendered = lock::write(&lock);
            let current = std::fs::read_to_string(&lock_path).ok();
            if current.as_deref() != Some(rendered.as_str()) {
                std::fs::write(&lock_path, rendered)
                    .map_err(|e| ErrorReport::one("io", format!("{}: {e}", lock_path.display())))?;
            }
        } else {
            notes.push("no lock sidecar; `trellis refresh` first".into());
        }

        defs.push(DefReport {
            path: entry.rel.clone(),
            tests: outcome.tests,
            details: outcome.details,
            errors: outcome.errors,
            notes,
        });
    }
    Ok(TestReport { defs })
}
