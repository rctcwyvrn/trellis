//! The examples regeneration driver, step-7 form (impl plan 03
//! §9.10): every `examples/**/*.lock` is rebuilt from the daemon's
//! real pipeline — scan, compile (rename-derived edges, content
//! hashes), and the step-7 test runner (real rows, real oracle
//! hashes) — blessed with `TRELLIS_BLESS=1` and byte-compared
//! otherwise. This is the round-trip exit criterion in continuous
//! form; only the per-entry *policy* (pinned, accepted, provenance,
//! the carried step-8 cram rows) remains declared by hand.

use std::fs;
use std::path::{Path, PathBuf};

use trellis::lock::*;
use trellis::state::{self, Entry};
use trellis::testrun;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn versions() -> Versions {
    Versions {
        trellis: env!("CARGO_PKG_VERSION").into(),
        soil: "0.1".into(),
    }
}

/// The empty decisions scope (no `decisions` blocks exist in
/// `examples/` yet).
fn no_decisions() -> LoweringDecisions {
    LoweringDecisions {
        scope_hash: trellis::hash::decisions_scope_hash(&[]),
        applied: vec![],
    }
}

fn spec_of(entry: &Entry, pinned: bool) -> Spec {
    Spec {
        provenance: "human".into(),
        hashes: Hashes {
            formal: entry.spec.formal.clone(),
            test: entry.spec.test.clone(),
            prose: entry.spec.prose.clone(),
        },
        prose_state: "fresh".into(),
        blocks: entry
            .tr
            .blocks
            .iter()
            .map(|b| BlockProv {
                block: b.id.clone(),
                author: if b.agent { "agent" } else { "human" }.into(),
            })
            .collect(),
        pinned,
        escape_hatches: vec![],
        decisions: None,
    }
}

fn check_lock(rel: &str, lock: Lock) {
    validate(&lock).unwrap_or_else(|e| panic!("{rel} invalid: {}", e.render()));
    let rendered = write(&lock);
    let path = examples().join(rel);
    if std::env::var_os("TRELLIS_BLESS").is_some() {
        fs::write(&path, &rendered).expect("write lock");
    } else {
        let expected =
            fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing {rel} (TRELLIS_BLESS=1)"));
        assert_eq!(
            expected, rendered,
            "{rel} drifted (TRELLIS_BLESS=1 to update)"
        );
        // Read → write is byte-identical (the writer conformance).
        assert_eq!(
            write(&read(&expected).expect("lock reads")),
            expected,
            "{rel} round-trip"
        );
    }
}

/// Per-entry policy: what the pipeline cannot derive. `pinned` and
/// `accepted` are human declarations; the cram row is carried until
/// step 8 runs it for real.
fn policy(rel: &str) -> (bool, bool) {
    match rel {
        // (pinned, accepted)
        "csvstats/median" => (true, true),
        "csvstats/row" => (false, true), // types are accepted-ungated in v1 (lock-schema §9)
        "read_file" => (true, true),
        _ => (false, false),
    }
}

fn carried_rows(rel: &str) -> Vec<TestRow> {
    match rel {
        "read_file" => vec![TestRow {
            name: "real-read#1".into(),
            tier: "cram".into(),
            mode: "real".into(),
            origin: "spec".into(),
            result: "pass".into(),
        }],
        _ => vec![],
    }
}

#[test]
fn examples_locks_regenerate() {
    let root_path = examples();
    let config = trellis::config::load(&root_path).expect("soil.toml");
    let root = state::scan(&root_path, &config).expect("examples scan");
    let compiled = state::compile(&root).expect("examples compile");

    for entry in root.entries.values() {
        let (pinned, accepted) = policy(&entry.rel);
        let carried = carried_rows(&entry.rel);
        let outcome = testrun::test_def(&root, entry, &carried)
            .unwrap_or_else(|e| panic!("{}: {}", entry.rel, e.render()));

        // The only expected diagnostic in the whole corpus is Row's
        // v1 invariant refusal (resolved 2026-08-28).
        let codes: Vec<&str> = outcome.errors.iter().map(|d| d.code.as_str()).collect();
        match entry.rel.as_str() {
            "csvstats/row" => assert_eq!(
                codes,
                ["unsupported-invariant-property"],
                "csvstats/row diagnostics"
            ),
            _ => assert!(codes.is_empty(), "{}: unexpected {codes:?}", entry.rel),
        }
        for row in &outcome.tests {
            assert!(
                matches!(row.result.as_str(), "pass" | "xfail"),
                "{}: `{}` is {} — the checked-in examples must be green",
                entry.rel,
                row.name,
                row.result
            );
        }

        let module = entry
            .rel
            .rsplit_once('/')
            .map(|(m, _)| m.to_string())
            .unwrap_or_default();
        let kind = format!("{:?}", entry.kind).to_lowercase();
        let lock = match kind.as_str() {
            "function" => {
                let lowering = entry.soil.as_ref().map(|_| {
                    let (calls, helpers) =
                        state::lowering_edges(&compiled, &entry.tr.name, &module);
                    Lowering {
                        soil_hash: compiled.soil_hashes[&entry.tr.name].clone(),
                        provenance: "human-verified".into(),
                        provider: None,
                        model: None,
                        private_helpers: helpers,
                        calls,
                        decisions: no_decisions(),
                    }
                });
                Lock {
                    lock_format: LOCK_FORMAT,
                    name: entry.tr.name.clone(),
                    kind: "function".into(),
                    versions: versions(),
                    spec: spec_of(entry, pinned),
                    lowering,
                    checks: outcome.checks.clone(),
                    tests: Some(outcome.tests.clone()),
                    oracles: Some(outcome.oracles.clone()),
                    cycle_hash: None,
                    accepted,
                }
            }
            "type" => Lock {
                lock_format: LOCK_FORMAT,
                name: entry.tr.name.clone(),
                kind: "type".into(),
                versions: versions(),
                spec: spec_of(entry, pinned),
                lowering: None,
                checks: outcome.checks.clone(),
                tests: Some(outcome.tests.clone()),
                oracles: None,
                cycle_hash: None,
                accepted,
            },
            "module" => Lock {
                lock_format: LOCK_FORMAT,
                name: entry.tr.name.clone(),
                kind: "module".into(),
                versions: versions(),
                spec: spec_of(entry, pinned),
                lowering: None,
                checks: Some(Checks::Module {
                    exports: "ok".into(),
                }),
                tests: None,
                oracles: None,
                cycle_hash: None,
                accepted,
            },
            other => panic!("unexpected kind {other} in examples"),
        };
        check_lock(&format!("{}.lock", entry.rel), lock);
    }
}

/// The derived manifest merges the regenerated sidecars: the private
/// helper materializes with its owner, the audit view is empty, and
/// the merge is deterministic.
#[test]
fn manifest_merges_examples() {
    let mut entries = Vec::new();
    let mut stack = vec![examples()];
    while let Some(dir) = stack.pop() {
        for item in fs::read_dir(&dir).expect("dir") {
            let path = item.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "lock") {
                let rel = path
                    .strip_prefix(examples())
                    .unwrap()
                    .with_extension("")
                    .to_string_lossy()
                    .into_owned();
                let lock = read(&fs::read_to_string(&path).unwrap())
                    .unwrap_or_else(|e| panic!("{rel}: {}", e.render()));
                entries.push((rel, lock));
            }
        }
    }
    assert_eq!(entries.len(), 19, "nineteen sidecars expected");
    let manifest =
        trellis::manifest::merge(entries, std::collections::BTreeMap::new()).expect("merges");
    assert_eq!(manifest.soil_private.len(), 1);
    assert_eq!(manifest.soil_private[0].name, "csvstats/_parse_cell");
    assert_eq!(
        manifest.soil_private[0].owners,
        vec!["csvstats/parse_row".to_string()]
    );
    assert!(manifest.escape_hatches.is_empty());
    let rendered = trellis::manifest::write(&manifest);
    assert!(rendered.contains("\"csvstats/_parse_cell\"") || rendered.contains("_parse_cell"));
}
