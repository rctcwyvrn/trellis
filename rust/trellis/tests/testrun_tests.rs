//! Step-7 runner tests: bundle-assembly goldens (contract §10
//! verbatim shapes), property determinism under the pinned seed
//! derivation, the pre-flight probes (each with a firing fixture and
//! `sat` as the non-firing counterpart), and the `where`-filter
//! refusal. The examples regeneration test (`examples_locks.rs`) is
//! the green half: it asserts the whole corpus fires nothing but
//! Row's v1 invariant refusal.

use std::path::{Path, PathBuf};

use trellis::state;
use trellis::testrun;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/testrun_fixtures")
}

fn scan(root: &Path) -> state::Root {
    let config = trellis::config::load(root).expect("soil.toml");
    state::scan(root, &config).expect("scan")
}

/// The read_file bundle the daemon assembles matches the checked-in
/// soil0 fixture (`soil0/tests/bundles/read_file/bundle.json`) case
/// for case — one §10 shape, two producers.
#[test]
fn bundle_assembly_matches_soil0_fixture() {
    let root = scan(&examples());
    let entry = &root.entries["read_file"];
    let bundle = testrun::build_bundle(entry);
    let ours = serde_json::to_value(&bundle).expect("serializes");

    let fixture_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../soil0/tests/bundles/read_file/bundle.json");
    let fixture: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture_path).expect("fixture"))
            .expect("fixture parses");
    // The fixture appends one hand-written xfail case beyond the
    // spec's two; the spec-derived prefix must agree exactly.
    let fixture_cases = fixture["cases"].as_array().expect("cases");
    let ours_cases = ours["cases"].as_array().expect("cases");
    assert_eq!(ours_cases[..], fixture_cases[..2]);
}

/// Two runs over the same spec produce identical rows and details:
/// the SplitMix64 stream is seeded from `test_hash` alone
/// (micro-pin §8.12).
#[test]
fn property_runs_are_deterministic() {
    let root = scan(&examples());
    let entry = &root.entries["csvstats/median"];
    let a = testrun::test_def(&root, entry, &[]).expect("run a");
    let b = testrun::test_def(&root, entry, &[]).expect("run b");
    assert_eq!(a.tests, b.tests);
    assert_eq!(
        a.details.iter().map(|d| &d.message).collect::<Vec<_>>(),
        b.details.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

fn fixture_codes(name: &str) -> Vec<String> {
    let root = scan(&fixtures());
    let entry = &root.entries[name];
    let outcome = testrun::test_def(&root, entry, &[]).expect("fixture runs");
    outcome.errors.iter().map(|d| d.code.clone()).collect()
}

#[test]
fn contradiction_fires() {
    assert_eq!(fixture_codes("contra"), ["preflight-contradiction"]);
}

#[test]
fn vacuous_requires_fires() {
    assert_eq!(fixture_codes("vacuous"), ["preflight-vacuous-requires"]);
}

#[test]
fn trivial_ensures_fires() {
    assert_eq!(fixture_codes("trivial"), ["preflight-trivial-ensures"]);
}

#[test]
fn unexercised_variant_fires() {
    assert_eq!(fixture_codes("onlyok"), ["preflight-unexercised-variant"]);
}

#[test]
fn where_filter_is_refused() {
    assert_eq!(fixture_codes("wherefilter"), ["unsupported-where-filter"]);
}

/// The non-firing counterpart: a satisfied requires and a meaningful
/// ensures raise nothing, and the derived row passes.
#[test]
fn sat_fires_nothing() {
    let root = scan(&fixtures());
    let entry = &root.entries["sat"];
    let outcome = testrun::test_def(&root, entry, &[]).expect("runs");
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    let derived: Vec<(&str, &str)> = outcome
        .tests
        .iter()
        .filter(|r| r.origin == "derived")
        .map(|r| (r.name.as_str(), r.result.as_str()))
        .collect();
    assert_eq!(derived, [("derived:ensures:kept", "pass")]);
}

/// The contradictory pair still runs: the second expectation records
/// its honest `fail` while the pre-flight names the contradiction.
#[test]
fn contradictory_pair_rows() {
    let root = scan(&fixtures());
    let entry = &root.entries["contra"];
    let outcome = testrun::test_def(&root, entry, &[]).expect("runs");
    let rows: Vec<(&str, &str)> = outcome
        .tests
        .iter()
        .map(|r| (r.name.as_str(), r.result.as_str()))
        .collect();
    assert_eq!(rows, [("cases#1", "pass"), ("cases#2", "fail")]);
}
