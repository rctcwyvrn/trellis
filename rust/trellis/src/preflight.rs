//! The contradiction pre-flight and vacuity probes (design §4.5,
//! impl plan 03 step 7): mechanical checks over the expect-test set,
//! run before any tokens or generation effort are spent. Findings are
//! spec bugs — `fix-spec` repairs, `ask_human` when pinned.

use std::collections::BTreeMap;

use soil0::ast::{CmpOp, Pred};
use soil0::interp::{self, Session};
use soil0::manifest::{TypeBody, TypeDef};

use crate::diag::Diag;
use crate::registry;
use crate::state::Entry;
use crate::testrun::{pred_text, PredCtx};
use crate::trfile::predicate::Clause;
use crate::trfile::{tests_blk, Payload};

#[allow(clippy::too_many_arguments)]
pub fn run(
    s: &Session,
    entry: &Entry,
    def: &str,
    cap_flags: &[bool],
    requires: &[&Clause],
    ensures: &[&Clause],
    env_types: &[TypeDef],
    errors: &mut Vec<Diag>,
) {
    let Ok((param_tys, result_ty)) = interp::entry_tys(s, def) else {
        return;
    };
    let ctx = PredCtx::new(env_types);

    // ---- gather the expect cases ----
    struct CaseView<'e> {
        name: String,
        case: &'e tests_blk::TestCase,
        binds_key: String,
    }
    let mut cases: Vec<CaseView> = Vec::new();
    for block in &entry.tr.blocks {
        let Payload::Tests(t) = &block.payload else {
            continue;
        };
        let block_name = block.name.clone().unwrap_or_default();
        let binds_key = serde_json::to_string(
            &t.withs
                .iter()
                .map(|w| (w.name.as_str(), w.ctor.as_str(), &w.args))
                .collect::<Vec<_>>(),
        )
        .expect("binds serialize");
        for (k, case) in t.cases.iter().enumerate() {
            cases.push(CaseView {
                name: format!("{block_name}#{}", k + 1),
                case,
                binds_key: binds_key.clone(),
            });
        }
    }

    // ---- contradiction: same input, different expected output ----
    let canon = |ty: &soil0::types::Ty, v: &serde_json::Value| -> String {
        interp::canonical_json(s, ty, v).unwrap_or_else(|_| v.to_string())
    };
    let mut seen: BTreeMap<String, (String, String)> = BTreeMap::new(); // input key -> (case, outcome)
    for view in &cases {
        let mut key = view.binds_key.clone();
        for (arg, ty) in view.case.args.iter().zip(&param_tys) {
            key.push('\u{0}');
            match arg {
                tests_blk::TestArg::Json(v) => key.push_str(&canon(ty, v)),
                tests_blk::TestArg::Binding(name) => {
                    key.push_str("bind:");
                    key.push_str(name);
                }
            }
        }
        let outcome = match &view.case.outcome {
            tests_blk::Outcome::Panic => "panic".to_string(),
            tests_blk::Outcome::Value(v) => canon(&result_ty, v),
        };
        match seen.get(&key) {
            Some((other, prior)) if *prior != outcome => {
                errors.push(registry::enrich(Diag::new(
                    "preflight-contradiction",
                    format!(
                        "cases `{}` and `{other}` give the same input different \
                         expected outcomes ({outcome} vs {prior}) — design §4.5",
                        view.name
                    ),
                )));
            }
            Some(_) => {}
            None => {
                seen.insert(key, (view.name.clone(), outcome));
            }
        }
    }

    // ---- vacuity: a `requires` no expect input satisfies ----
    let has_caps = cap_flags.iter().any(|c| *c);
    if !has_caps && !cases.is_empty() {
        for (i, clause) in requires.iter().enumerate() {
            let wrapper = format!("t7_req_{i}");
            let satisfied = cases.iter().any(|view| {
                let args: Vec<serde_json::Value> = view
                    .case
                    .args
                    .iter()
                    .filter_map(|a| match a {
                        tests_blk::TestArg::Json(v) => Some(v.clone()),
                        tests_blk::TestArg::Binding(_) => None,
                    })
                    .collect();
                matches!(interp::call_json(s, &wrapper, &args), Ok(Ok(r)) if r == "true")
            });
            if !satisfied {
                errors.push(registry::enrich(Diag::new(
                    "preflight-vacuous-requires",
                    format!(
                        "no expect-test input satisfies `requires {}:` — the tests \
                         never exercise the declared domain (design §4.5)",
                        clause.label
                    ),
                )));
            }
        }
    }

    // ---- syntactically trivial `ensures` ----
    for clause in ensures {
        if trivial(&clause.pred.item, &ctx) {
            errors.push(registry::enrich(Diag::new(
                "preflight-trivial-ensures",
                format!(
                    "`ensures {}:` is syntactically trivial — it constrains \
                     nothing (design §4.5)",
                    clause.label
                ),
            )));
        }
    }

    // ---- an expect set that never exercises a declared variant ----
    let sums: BTreeMap<&str, &TypeDef> = {
        static KERNEL: std::sync::OnceLock<Vec<TypeDef>> = std::sync::OnceLock::new();
        let kernel = KERNEL.get_or_init(soil0::kernel::kernel_typedefs);
        kernel
            .iter()
            .chain(env_types.iter())
            .map(|td| (td.name.as_str(), td))
            .collect()
    };
    if let soil0::types::Ty::Con { name, .. } = &result_ty {
        if let Some(TypeBody::Sum { variants }) = sums.get(name.as_str()).map(|td| &td.body) {
            let mut exercised: Vec<&str> = Vec::new();
            for view in &cases {
                if let tests_blk::Outcome::Value(v) = &view.case.outcome {
                    match v {
                        serde_json::Value::Bool(b) => {
                            exercised.push(if *b { "True" } else { "False" })
                        }
                        serde_json::Value::Object(obj) => {
                            if let Some(serde_json::Value::String(tag)) = obj.get("tag") {
                                exercised.push(tag.as_str());
                            }
                        }
                        _ => {}
                    }
                }
            }
            let missing: Vec<&str> = variants
                .iter()
                .map(|v| v.name.as_str())
                .filter(|v| !exercised.contains(v))
                .collect();
            if !missing.is_empty() && !cases.is_empty() {
                errors.push(registry::enrich(Diag::new(
                    "preflight-unexercised-variant",
                    format!(
                        "the expect tests never produce {} of result type `{name}` \
                         (design §4.5)",
                        missing
                            .iter()
                            .map(|v| format!("`{v}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )));
            }
        }
    }
}

/// Syntactic triviality: a comparison whose sides render identically
/// under a reflexive operator, an implication whose sides coincide,
/// or `p or not p`.
fn trivial(pred: &Pred, ctx: &PredCtx) -> bool {
    match pred {
        Pred::PCmp { op, lhs, rhs } => {
            matches!(op, CmpOp::Eq | CmpOp::Le | CmpOp::Ge)
                && crate::testrun::pexpr_eq(&lhs.item, &rhs.item)
        }
        Pred::Implies { lhs, rhs } => pred_text(&lhs.item, ctx) == pred_text(&rhs.item, ctx),
        Pred::POr { lhs, rhs } => {
            let (a, b) = (&lhs.item, &rhs.item);
            matches!(b, Pred::PNot { pred } if pred_text(&pred.item, ctx) == pred_text(a, ctx))
                || matches!(a, Pred::PNot { pred } if pred_text(&pred.item, ctx) == pred_text(b, ctx))
        }
        _ => false,
    }
}
