//! Step-9 packer tests: packed-bundle goldens at generous and
//! starvation budgets (the drop order is observable and pinned), the
//! never-drop rule, `budget-too-small`, and the monotonicity property
//! the prefix packer guarantees — a larger budget packs a superset
//! and never a smaller estimate.

use std::path::{Path, PathBuf};

use trellis::bundle;
use trellis::state;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/context_fixtures")
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn scan(root: &Path) -> state::Root {
    let config = trellis::config::load(root).expect("soil.toml");
    state::scan(root, &config).expect("scan")
}

fn assemble(root: &state::Root, def: &str, budget: u64) -> bundle::Bundle {
    let entry = root
        .entries
        .values()
        .find(|e| e.tr.name == def)
        .expect("definition exists");
    bundle::assemble(root, entry, budget).expect("assembles")
}

/// One golden per scenario: every packed file concatenated with
/// banners, so path set, order, and contents are all pinned.
fn check_golden(name: &str, bundle: &bundle::Bundle) {
    let mut rendered = String::new();
    for (rel, contents) in &bundle.files {
        rendered.push_str(&format!("===== {rel} =====\n"));
        rendered.push_str(contents);
        if !contents.ends_with('\n') {
            rendered.push('\n');
        }
    }
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/bundle/{name}.txt"));
    std::fs::create_dir_all(path.parent().unwrap()).expect("golden dir");
    if std::env::var_os("TRELLIS_BLESS").is_some() {
        std::fs::write(&path, &rendered).expect("write golden");
    } else {
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("missing golden {name} (TRELLIS_BLESS=1)"));
        assert_eq!(expected, rendered, "bundle golden {name} drifted");
    }
}

#[test]
fn generous_budget_packs_everything() {
    let root = scan(&fixtures());
    let bundle = assemble(&root, "target", 100_000);
    assert!(bundle.manifest["dropped"].as_array().unwrap().is_empty());
    check_golden("generous", &bundle);
}

/// A budget that holds the mandatory set, `previous.soil`,
/// `reference.py`, and the near callee — the corpus and module prose
/// drop, farthest-tier-first, every drop listed.
#[test]
fn starvation_budget_drops_the_tail() {
    let root = scan(&fixtures());
    let bundle = assemble(&root, "target", 550);
    let dropped: Vec<&str> = bundle.manifest["dropped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["item"].as_str().unwrap())
        .collect();
    assert!(!dropped.is_empty(), "starvation budget must drop");
    check_golden("starvation", &bundle);
}

#[test]
fn spec_and_tests_are_never_dropped() {
    let root = scan(&fixtures());
    // Any budget that assembles at all carries the mandatory set.
    for budget in [400, 800, 2_000, 100_000] {
        match bundle::assemble(
            &root,
            root.entries
                .values()
                .find(|e| e.tr.name == "target")
                .unwrap(),
            budget,
        ) {
            Err(report) => {
                assert_eq!(report.errors[0].code, "budget-too-small");
            }
            Ok(bundle) => {
                let packed: Vec<&str> = bundle.manifest["packed"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().unwrap())
                    .collect();
                assert!(packed.contains(&"spec.md"), "budget {budget}");
                assert!(packed.contains(&"decisions.md"), "budget {budget}");
                assert!(packed.contains(&"tests.json"), "budget {budget}");
            }
        }
    }
}

#[test]
fn tiny_budget_is_a_structured_error() {
    let root = scan(&fixtures());
    let entry = root
        .entries
        .values()
        .find(|e| e.tr.name == "target")
        .unwrap();
    let Err(err) = bundle::assemble(&root, entry, 50) else {
        panic!("a 50-token budget must refuse");
    };
    assert_eq!(err.errors[0].code, "budget-too-small");
}

/// The prefix packer's pinned property: for budgets b1 <= b2 the
/// packed set grows monotonically and the estimate never shrinks.
#[test]
fn packing_is_monotone_in_the_budget() {
    let root = scan(&examples());
    let entry = root
        .entries
        .values()
        .find(|e| e.tr.name == "median")
        .unwrap();
    let mut previous: Option<(Vec<String>, u64)> = None;
    for budget in (400..6_000).step_by(93) {
        let bundle = match bundle::assemble(&root, entry, budget) {
            Ok(bundle) => bundle,
            Err(_) => {
                assert!(previous.is_none(), "assembly cannot regress to failure");
                continue;
            }
        };
        let packed: Vec<String> = bundle.manifest["packed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap().to_string())
            .collect();
        let estimate = bundle.manifest["estimate"].as_u64().unwrap();
        if let Some((prior_packed, prior_estimate)) = &previous {
            for item in prior_packed {
                assert!(
                    packed.contains(item),
                    "budget {budget} lost `{item}` packed at a smaller budget"
                );
            }
            assert!(
                estimate >= *prior_estimate,
                "estimate shrank as the budget grew"
            );
        }
        previous = Some((packed, estimate));
    }
}
