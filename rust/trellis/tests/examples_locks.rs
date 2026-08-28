//! The step-5 examples regeneration driver (impl plan 03 §9.10,
//! policy resolved 2026-08-28): builds every `examples/**/*.lock` from
//! the daemon's own parsing and hashing, blesses with
//! `TRELLIS_BLESS=1`, and byte-compares otherwise — the continuous
//! form of the round-trip exit criterion (final judgment at step 7,
//! which re-runs tests and replaces the carried-forward rows and the
//! hardcoded reference tables below with real runner/rename output).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use trellis::hash;
use trellis::lock::*;
use trellis::trfile;

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
        scope_hash: hash::decisions_scope_hash(&[]),
        applied: vec![],
    }
}

struct Def {
    tr: trfile::TrFile,
    spec_hashes: hash::SpecHashes,
}

fn parse_def(rel: &str) -> Def {
    let path = examples().join(rel);
    let src = fs::read_to_string(&path).expect("readable .tr");
    let tr = trfile::parse_tr(&src, &path, None)
        .unwrap_or_else(|e| panic!("{rel} must parse: {}", e.render()));
    let spec_hashes = hash::spec_hashes(&src, &tr);
    Def { tr, spec_hashes }
}

fn soil_hash_of(rel: &str, refs: &[hash::HashRef]) -> String {
    let path = examples().join(rel);
    let src = fs::read_to_string(&path).expect("readable .soil");
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let form = soil0::hashform::hash_form_file(&src, &name).expect("hash form");
    hash::soil_hash(&form, refs)
}

fn spec_of(def: &Def, pinned: bool) -> Spec {
    Spec {
        provenance: "human".into(),
        hashes: Hashes {
            formal: def.spec_hashes.formal.clone(),
            test: def.spec_hashes.test.clone(),
            prose: def.spec_hashes.prose.clone(),
        },
        prose_state: "fresh".into(),
        blocks: def
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

fn human_lowering(soil_hash: String, helpers: Vec<NamedHash>, calls: Vec<NamedHash>) -> Lowering {
    Lowering {
        soil_hash,
        provenance: "human-verified".into(),
        provider: None,
        model: None,
        private_helpers: helpers,
        calls,
        decisions: no_decisions(),
    }
}

fn fn_checks(termination: &str, refinements: &[&str]) -> Checks {
    Checks::Function {
        types: "ok".into(),
        termination: termination.into(),
        refinements: refinements
            .iter()
            .map(|label| (label.to_string(), "runtime".to_string()))
            .collect(),
        holes: 0,
    }
}

fn row(name: &str, tier: &str, mode: &str, origin: &str, result: &str) -> TestRow {
    TestRow {
        name: name.into(),
        tier: tier.into(),
        mode: mode.into(),
        origin: origin.into(),
        result: result.into(),
    }
}

fn spec_rows(names: &[&str]) -> Vec<TestRow> {
    names
        .iter()
        .map(|n| row(n, "expect", "sandboxed", "spec", "pass"))
        .collect()
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

const BUILTIN: &str = "soil0-builtin/1";

#[test]
fn examples_locks_regenerate() {
    assert_eq!(hash::builtin_referent(), BUILTIN);

    // ---- spec-side parses and hashes ----
    let len = parse_def("csvstats/len.tr");
    let nth = parse_def("csvstats/nth.tr");
    let min = parse_def("csvstats/min.tr");
    let max = parse_def("csvstats/max.tr");
    let sort = parse_def("csvstats/sort.tr");
    let mean = parse_def("csvstats/mean.tr");
    let median = parse_def("csvstats/median.tr");
    let parse_row_def = parse_def("csvstats/parse_row.tr");
    let row_ty = parse_def("csvstats/row.tr");
    let parse_error = parse_def("csvstats/parse_error.tr");
    let module = parse_def("csvstats/_module.tr");
    let read_file = parse_def("read_file.tr");

    // ---- soil hashes, callee-first; reference tables are the
    // step-5 stand-in for rename output (defs and privates by their
    // hashes, user types by formal_hash, builtin functions by the
    // toolchain tag; kernel types are contract surface and edge
    // nowhere) ----
    let len_soil = soil_hash_of("csvstats/len.soil", &[("list_len".into(), BUILTIN.into())]);
    let nth_soil = soil_hash_of("csvstats/nth.soil", &[("list_nth".into(), BUILTIN.into())]);
    let sort_soil = soil_hash_of(
        "csvstats/sort.soil",
        &[
            ("list_len".into(), BUILTIN.into()),
            ("list_nth".into(), BUILTIN.into()),
            ("list_empty".into(), BUILTIN.into()),
            ("list_append".into(), BUILTIN.into()),
        ],
    );
    let minmax_refs: Vec<hash::HashRef> = vec![
        ("len".into(), len_soil.clone()),
        ("nth".into(), nth_soil.clone()),
    ];
    let min_soil = soil_hash_of("csvstats/min.soil", &minmax_refs);
    let max_soil = soil_hash_of("csvstats/max.soil", &minmax_refs);
    let median_soil = soil_hash_of(
        "csvstats/median.soil",
        &[
            ("sort".into(), sort_soil.clone()),
            ("len".into(), len_soil.clone()),
            ("nth".into(), nth_soil.clone()),
        ],
    );
    let parse_cell_soil = {
        let src = fs::read_to_string(examples().join("csvstats/_private.soil")).unwrap();
        let form = soil0::hashform::hash_form_file(&src, "_private.soil").expect("hash form");
        hash::soil_hash(
            &form,
            &[
                ("utf8_parse_f64".into(), BUILTIN.into()),
                ("utf8_trim".into(), BUILTIN.into()),
            ],
        )
    };
    let parse_row_soil = soil_hash_of(
        "csvstats/parse_row.soil",
        &[
            ("_parse_cell".into(), parse_cell_soil.clone()),
            ("utf8_split".into(), BUILTIN.into()),
            ("list_len".into(), BUILTIN.into()),
            ("list_nth".into(), BUILTIN.into()),
            ("list_append".into(), BUILTIN.into()),
            ("list_empty".into(), BUILTIN.into()),
            ("Row".into(), row_ty.spec_hashes.formal.clone()),
            ("ParseError".into(), parse_error.spec_hashes.formal.clone()),
        ],
    );
    let read_file_soil = soil_hash_of(
        "read_file.soil",
        &[
            ("fs_read_bytes".into(), BUILTIN.into()),
            ("utf8_decode".into(), BUILTIN.into()),
        ],
    );

    let stats_py = fs::read(examples().join("ref/stats.py")).expect("reference file");
    let stats_py_hash = hash::digest(&[&stats_py]);

    // ---- the sidecars ----
    let function_lock = |def: &Def,
                         pinned: bool,
                         lowering: Option<Lowering>,
                         checks: Option<Checks>,
                         tests: Vec<TestRow>,
                         oracles: Vec<Oracle>,
                         accepted: bool| Lock {
        lock_format: LOCK_FORMAT,
        name: def.tr.name.clone(),
        kind: "function".into(),
        versions: versions(),
        spec: spec_of(def, pinned),
        lowering,
        checks,
        tests: Some(tests),
        oracles: Some(oracles),
        cycle_hash: None,
        accepted,
    };

    check_lock(
        "csvstats/len.lock",
        function_lock(
            &len,
            false,
            Some(human_lowering(len_soil.clone(), vec![], vec![])),
            Some(fn_checks("verified", &["non-negative"])),
            vec![],
            vec![],
            false,
        ),
    );
    check_lock(
        "csvstats/nth.lock",
        function_lock(
            &nth,
            false,
            Some(human_lowering(nth_soil.clone(), vec![], vec![])),
            Some(fn_checks("verified", &["in-bounds"])),
            vec![],
            vec![],
            false,
        ),
    );
    let minmax_calls = |len_h: &str, nth_h: &str| {
        vec![
            NamedHash {
                name: "len".into(),
                hash: len_h.into(),
            },
            NamedHash {
                name: "nth".into(),
                hash: nth_h.into(),
            },
        ]
    };
    check_lock(
        "csvstats/min.lock",
        function_lock(
            &min,
            false,
            Some(human_lowering(
                min_soil.clone(),
                vec![],
                minmax_calls(&len_soil, &nth_soil),
            )),
            Some(fn_checks("unverified", &["non-empty"])),
            vec![],
            vec![],
            false,
        ),
    );
    check_lock(
        "csvstats/max.lock",
        function_lock(
            &max,
            false,
            Some(human_lowering(
                max_soil.clone(),
                vec![],
                minmax_calls(&len_soil, &nth_soil),
            )),
            Some(fn_checks("unverified", &["non-empty"])),
            vec![],
            vec![],
            false,
        ),
    );
    check_lock(
        "csvstats/sort.lock",
        function_lock(
            &sort,
            false,
            Some(human_lowering(sort_soil.clone(), vec![], vec![])),
            Some(fn_checks("unverified", &["length-preserved"])),
            vec![],
            vec![],
            false,
        ),
    );
    check_lock(
        "csvstats/mean.lock",
        function_lock(&mean, false, None, None, vec![], vec![], false),
    );

    // median: carried rows (step 7 re-runs), derived names in the
    // lock-schema §5 namespace, the reference oracle by real hash.
    let mut median_rows = spec_rows(&["odd-length#1", "even-length#1", "single#1"]);
    median_rows.push(row(
        "order-independent",
        "property",
        "sandboxed",
        "spec",
        "pass",
    ));
    median_rows.push(row(
        "derived:ensures:lower bound",
        "property",
        "sandboxed",
        "derived",
        "pass",
    ));
    median_rows.push(row(
        "derived:ensures:upper bound",
        "property",
        "sandboxed",
        "derived",
        "pass",
    ));
    median_rows.push(row(
        "derived:differential:ref/stats.py::median",
        "differential",
        "sandboxed",
        "derived",
        "pass",
    ));
    check_lock(
        "csvstats/median.lock",
        function_lock(
            &median,
            true,
            Some(human_lowering(
                median_soil,
                vec![],
                vec![
                    NamedHash {
                        name: "sort".into(),
                        hash: sort_soil.clone(),
                    },
                    NamedHash {
                        name: "len".into(),
                        hash: len_soil.clone(),
                    },
                    NamedHash {
                        name: "nth".into(),
                        hash: nth_soil.clone(),
                    },
                ],
            )),
            Some(fn_checks(
                "verified",
                &["non-empty", "lower bound", "upper bound"],
            )),
            median_rows,
            vec![Oracle::Reference {
                kind: "reference".into(),
                path: "ref/stats.py::median".into(),
                hash: stats_py_hash,
            }],
            true,
        ),
    );

    let mut parse_row_rows = spec_rows(&["happy-path#1", "happy-path#2", "errors#1", "errors#2"]);
    parse_row_rows.push(row(
        "scientific-notation#1",
        "expect",
        "sandboxed",
        "spec",
        "xfail",
    ));
    check_lock(
        "csvstats/parse_row.lock",
        function_lock(
            &parse_row_def,
            false,
            Some(human_lowering(
                parse_row_soil.clone(),
                vec![NamedHash {
                    name: "_parse_cell".into(),
                    hash: parse_cell_soil.clone(),
                }],
                vec![],
            )),
            Some(fn_checks("unverified", &[])),
            parse_row_rows,
            vec![],
            false,
        ),
    );

    // Types.
    let type_lock = |def: &Def, invariants: &[&str], tests: Vec<TestRow>, accepted: bool| Lock {
        lock_format: LOCK_FORMAT,
        name: def.tr.name.clone(),
        kind: "type".into(),
        versions: versions(),
        spec: spec_of(def, false),
        lowering: None,
        checks: Some(Checks::Type {
            types: "ok".into(),
            invariants: invariants
                .iter()
                .map(|l| (l.to_string(), "runtime".to_string()))
                .collect(),
        }),
        tests: Some(tests),
        oracles: None,
        cycle_hash: None,
        accepted,
    };
    check_lock(
        "csvstats/row.lock",
        type_lock(
            &row_ty,
            &["non-empty"],
            vec![row(
                "derived:invariant:non-empty",
                "property",
                "sandboxed",
                "derived",
                "pass",
            )],
            true,
        ),
    );
    check_lock(
        "csvstats/parse_error.lock",
        type_lock(&parse_error, &[], vec![], false),
    );

    // The module header.
    check_lock(
        "csvstats/_module.lock",
        Lock {
            lock_format: LOCK_FORMAT,
            name: module.tr.name.clone(),
            kind: "module".into(),
            versions: versions(),
            spec: spec_of(&module, false),
            lowering: None,
            checks: Some(Checks::Module {
                exports: "ok".into(),
            }),
            tests: None,
            oracles: None,
            cycle_hash: None,
            accepted: false,
        },
    );

    // read_file at the root.
    check_lock(
        "read_file.lock",
        function_lock(
            &read_file,
            true,
            Some(human_lowering(read_file_soil.clone(), vec![], vec![])),
            Some(fn_checks("verified", &[])),
            vec![
                row("found-and-missing#1", "expect", "sandboxed", "spec", "pass"),
                row("found-and-missing#2", "expect", "sandboxed", "spec", "pass"),
                row("real-read#1", "cram", "real", "spec", "pass"),
            ],
            vec![],
            true,
        ),
    );
}

/// The derived manifest merges the regenerated sidecars: the private
/// helper materializes with its owner, the audit view is empty, and
/// the merge is deterministic.
#[test]
fn manifest_merges_examples() {
    let mut entries = Vec::new();
    let mut stack = vec![examples()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("dir") {
            let path = entry.expect("entry").path();
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
    assert_eq!(entries.len(), 12, "twelve sidecars expected");
    let manifest = trellis::manifest::merge(entries, BTreeMap::new()).expect("merges");
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
