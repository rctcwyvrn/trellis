//! Step-6 tests (impl plan 03): the invalidation table end-to-end
//! over a temp copy of `examples/`, the decision flows, refresh's
//! writes, and the registry coverage gate.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use trellis::config::{self, Config};
use trellis::state;

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    /// A private copy of `examples/`.
    fn new() -> Fixture {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let root = std::env::temp_dir().join(format!(
            "trellis-state-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        copy_tree(&src, &root);
        Fixture { root }
    }

    fn config(&self) -> Config {
        config::load(&self.root).expect("fixture soil.toml")
    }

    fn edit(&self, rel: &str, from: &str, to: &str) {
        let path = self.root.join(rel);
        let src = fs::read_to_string(&path).expect("readable");
        let out = src.replacen(from, to, 1);
        assert_ne!(src, out, "edit `{from}` did not apply to {rel}");
        fs::write(&path, out).expect("writable");
    }

    fn edit_all(&self, rel: &str, from: &str, to: &str) {
        let path = self.root.join(rel);
        let src = fs::read_to_string(&path).expect("readable");
        let out = src.replace(from, to);
        assert_ne!(src, out, "edit `{from}` did not apply to {rel}");
        fs::write(&path, out).expect("writable");
    }

    fn append(&self, rel: &str, text: &str) {
        let path = self.root.join(rel);
        let mut src = fs::read_to_string(&path).expect("readable");
        src.push_str(text);
        fs::write(&path, src).expect("writable");
    }

    fn statuses(&self) -> BTreeMap<String, (String, Vec<String>)> {
        let root = state::scan(&self.root, &self.config()).expect("scan");
        let compiled = state::compile(&root).expect("compile");
        state::statuses(&root, &compiled.soil_hashes, &compiled.refs)
            .defs
            .into_iter()
            .map(|d| (d.path, (d.status, d.flags)))
            .collect()
    }

    fn refresh(&self) {
        state::refresh(&self.root, &self.config()).expect("refresh");
    }

    fn lock(&self, rel: &str) -> trellis::lock::Lock {
        trellis::lock::read(&fs::read_to_string(self.root.join(rel)).expect("readable"))
            .expect("valid lock")
    }

    fn write_lock(&self, rel: &str, lock: &trellis::lock::Lock) {
        fs::write(self.root.join(rel), trellis::lock::write(lock)).expect("writable");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).expect("mkdir");
    for entry in fs::read_dir(src).expect("readable dir") {
        let entry = entry.expect("entry");
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            fs::copy(&from, &to).expect("copy");
        }
    }
}

fn assert_clean(statuses: &BTreeMap<String, (String, Vec<String>)>) {
    for (path, (_, flags)) in statuses {
        assert!(flags.is_empty(), "{path} unexpectedly flagged: {flags:?}");
    }
}

#[test]
fn clean_tree_is_clean() {
    let fx = Fixture::new();
    let statuses = fx.statuses();
    assert_clean(&statuses);
    assert_eq!(statuses["csvstats/median"].0, "accepted");
    assert_eq!(statuses["csvstats/mean"].0, "unlowered");
    assert_eq!(statuses["csvstats/len"].0, "tested");
    assert_eq!(statuses["read_file"].0, "accepted");

    // The computed content addresses agree with every stored one —
    // the round-trip core of the exit criterion.
    let root = state::scan(&fx.root, &fx.config()).expect("scan");
    let compiled = state::compile(&root).expect("compile");
    for entry in root.entries.values() {
        if let Some(lowering) = entry.lock.as_ref().and_then(|l| l.lowering.as_ref()) {
            assert_eq!(
                compiled.soil_hashes[&entry.tr.name], lowering.soil_hash,
                "{} stored hash must match the recomputation",
                entry.rel
            );
        }
    }
}

#[test]
fn prose_edit_flags_then_refresh_records() {
    let fx = Fixture::new();
    fx.edit(
        "csvstats/median.tr",
        "Returns the median",
        "Computes the median",
    );
    let statuses = fx.statuses();
    assert_eq!(statuses["csvstats/median"].1, vec!["prose-stale"]);

    fx.refresh();
    let lock = fx.lock("csvstats/median.lock");
    assert_eq!(lock.spec.prose_state, "review-suggested");
    let statuses = fx.statuses();
    assert_eq!(statuses["csvstats/median"].1, vec!["review-suggested"]);
}

#[test]
fn formal_edit_flags_and_persists_through_refresh() {
    let fx = Fixture::new();
    fx.edit(
        "csvstats/len.tr",
        "non-negative: result >= 0",
        "non-negative: result >= 1",
    );
    assert_eq!(fx.statuses()["csvstats/len"].1, vec!["formal-stale"]);
    fx.refresh();
    // Formal staleness is only cleared by re-lowering or re-verifying.
    assert_eq!(fx.statuses()["csvstats/len"].1, vec!["formal-stale"]);
}

#[test]
fn test_edit_flags() {
    let fx = Fixture::new();
    fx.edit("csvstats/median.tr", "([42.0]) => 42.0", "([41.0]) => 41.0");
    assert_eq!(fx.statuses()["csvstats/median"].1, vec!["test-stale"]);
}

#[test]
fn local_rename_is_invisible_end_to_end() {
    let fx = Fixture::new();
    fx.edit_all("csvstats/sort.soil", "sort_all", "sift_all");
    assert_clean(&fx.statuses());
}

#[test]
fn own_soil_edit_becomes_hand_edited() {
    let fx = Fixture::new();
    fx.edit("csvstats/min.soil", "go 1 (nth xs 0)", "go 0 (nth xs 0)");
    let statuses = fx.statuses();
    assert_eq!(statuses["csvstats/min"].1, vec!["soil-drift"]);

    fx.refresh();
    let lock = fx.lock("csvstats/min.lock");
    let lowering = lock.lowering.expect("lowered");
    assert_eq!(lowering.provenance, "hand-edited");
    assert_clean(&fx.statuses());
}

#[test]
fn callee_edit_propagates_without_hand_edit_blame() {
    let fx = Fixture::new();
    // A semantic edit to `len` moves every caller's content address…
    fx.edit(
        "csvstats/len.soil",
        "len xs = list_len xs",
        "len xs = list_len xs + 0",
    );
    let statuses = fx.statuses();
    assert_eq!(statuses["csvstats/len"].1, vec!["soil-drift"]);
    for caller in ["csvstats/min", "csvstats/max", "csvstats/median"] {
        assert_eq!(statuses[caller].1, vec!["stale-callees"], "{caller}");
    }

    // …and refresh re-anchors callers without calling them hand-edits.
    fx.refresh();
    assert_clean(&fx.statuses());
    let median = fx.lock("csvstats/median.lock").lowering.expect("lowered");
    assert_eq!(median.provenance, "human-verified");
    let min = fx.lock("csvstats/min.lock").lowering.expect("lowered");
    assert_eq!(min.provenance, "human-verified");
    let len = fx.lock("csvstats/len.lock").lowering.expect("lowered");
    assert_eq!(len.provenance, "hand-edited");
}

// Appended flush against the previous fence: a leading blank line
// would be a prose byte and honestly flag prose-stale.
const DECISIONS_BLOCK: &str = "```decisions\nseparator: cells split on commas\n```\n";

#[test]
fn decision_flows() {
    let fx = Fixture::new();
    fx.append("csvstats/_module.tr", DECISIONS_BLOCK);

    // An added entry flags everything in scope once, via the
    // membership hash; the three-part hashes are untouched.
    let statuses = fx.statuses();
    assert!(
        statuses["csvstats/_module"].1.is_empty(),
        "decisions are not prose: {:?}",
        statuses["csvstats/_module"].1
    );
    for def in ["csvstats/median", "csvstats/len", "csvstats/parse_row"] {
        assert!(
            statuses[def]
                .1
                .contains(&"decision-scope-stale".to_string()),
            "{def}: {:?}",
            statuses[def].1
        );
    }
    // Definitions outside the module are untouched.
    assert!(statuses["read_file"].1.is_empty());

    // Refresh snapshots the entries into the module lock.
    fx.refresh();
    let module = fx.lock("csvstats/_module.lock");
    let snapshot = module.spec.decisions.expect("snapshot");
    assert!(snapshot.contains_key("separator"));

    // Cite the decision from median (as the lowerer will, via
    // write_soil's decisions_applied), stamping current hashes.
    let current = snapshot["separator"].clone();
    let mut median = fx.lock("csvstats/median.lock");
    {
        let lowering = median.lowering.as_mut().expect("lowered");
        lowering.decisions.scope_hash =
            trellis::hash::decisions_scope_hash(&["separator".to_string()]);
        lowering.decisions.applied = vec![trellis::lock::AppliedDecision {
            scope: "module".into(),
            label: "separator".into(),
            hash: current,
        }];
    }
    fx.write_lock("csvstats/median.lock", &median);
    let statuses = fx.statuses();
    assert!(
        statuses["csvstats/median"].1.is_empty(),
        "{:?}",
        statuses["csvstats/median"].1
    );

    // An edit to the entry text flags exactly the citing lowering.
    fx.edit(
        "csvstats/_module.tr",
        "split on commas",
        "split on semicolons",
    );
    let statuses = fx.statuses();
    assert_eq!(
        statuses["csvstats/median"].1,
        vec!["decision-stale:separator"]
    );

    // Editorial reclassification re-stamps the citing edge; nothing
    // else changes.
    let restamped = state::editorial(&fx.root, &fx.config(), "separator").expect("editorial");
    assert_eq!(restamped, 1);
    let statuses = fx.statuses();
    assert!(
        statuses["csvstats/median"].1.is_empty(),
        "{:?}",
        statuses["csvstats/median"].1
    );

    // An unknown label refuses.
    let err = state::editorial(&fx.root, &fx.config(), "nope").unwrap_err();
    assert_eq!(err.errors[0].code, "unknown-decision");
}

#[test]
fn refresh_creates_missing_locks() {
    let fx = Fixture::new();
    fs::remove_file(fx.root.join("csvstats/mean.lock")).expect("removable");
    assert_eq!(fx.statuses()["csvstats/mean"].1, vec!["no-lock"]);
    fx.refresh();
    let lock = fx.lock("csvstats/mean.lock");
    assert_eq!(lock.name, "mean");
    assert!(lock.lowering.is_none());
    assert_clean(&fx.statuses());
}

/// The drift gate's first half (contract §6): every code this crate
/// can emit is in the registry, and the registry itself is coherent.
#[test]
fn registry_covers_every_emitted_code() {
    let registry = trellis::registry::registry();
    assert_eq!(registry.registry_format, 1);

    let mut emitted = std::collections::BTreeSet::new();
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![src_dir];
    let pattern = regex_lite();
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = fs::read_to_string(&path).expect("readable");
                for needle in ["Diag::new(", "ErrorReport::one("] {
                    for (at, _) in text.match_indices(needle) {
                        let rest = &text[at + needle.len()..];
                        if let Some(code) = pattern(rest) {
                            emitted.insert(code);
                        }
                    }
                }
            }
        }
    }
    // Codes routed through helper functions do not appear as literals
    // at the call head, so the scan under-approximates; the explicit
    // list below is the maintained half of the gate.
    let known_emitted = [
        "usage",
        "io",
        "internal",
        "unimplemented",
        "config-no-root",
        "config-io",
        "config-parse",
        "config-unknown-key",
        "config-reserved-section",
        "config-bad-value",
        "toolchain-mismatch",
        "lock-malformed",
        "lock-format",
        "lock-kind-shape",
        "lock-accepted-untested",
        "lock-oracle-unaccepted",
        "tr-missing-frontmatter",
        "tr-frontmatter-syntax",
        "tr-unknown-frontmatter-key",
        "tr-missing-name",
        "tr-undeclared-tag",
        "tr-name-mismatch",
        "tr-block-info",
        "tr-block-kind",
        "tr-block-count",
        "tr-missing-prose",
        "tr-missing-test-block",
        "tr-duplicate-test-name",
        "tr-unnamed-params",
        "tr-panic-without-row",
        "tr-project-location",
        "tr-sig-syntax",
        "tr-predicate-syntax",
        "tr-test-syntax",
        "tr-property-syntax",
        "tr-cram-syntax",
        "tr-reference-syntax",
        "tr-allow-unknown",
        "tr-type-syntax",
        "tr-ignored-default",
        "tr-exports-syntax",
        "tr-decisions-syntax",
        "state-cycle",
        "unknown-decision",
    ];
    emitted.extend(known_emitted.iter().map(|c| c.to_string()));
    for code in &emitted {
        assert!(
            trellis::registry::lookup(code).is_some(),
            "emitted code `{code}` is missing from registry/diagnostics.json"
        );
    }
    // And every soil0 contract §2 code passes through enriched.
    for code in [
        "shadowing",
        "non-exhaustive-match",
        "effect-violation",
        "unfilled-hole",
        "runtime-panic",
        "forward-reference",
    ] {
        assert!(trellis::registry::lookup(code).is_some(), "{code}");
    }
}

/// Extract a `"kebab-code"` literal at the head of the argument list.
fn regex_lite() -> impl Fn(&str) -> Option<String> {
    |rest: &str| {
        let rest = rest.strip_prefix('"')?;
        let end = rest.find('"')?;
        let code = &rest[..end];
        let plausible = !code.is_empty()
            && code
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        plausible.then(|| code.to_string())
    }
}

/// The whole path through the binary: auto-spawned daemon, the
/// status method, the human table.
#[test]
fn status_renders_through_the_binary() {
    let fx = Fixture::new();
    let runtime = fx.root.join(".runtime");
    fs::create_dir_all(&runtime).expect("runtime dir");
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_trellis"))
            .args(args)
            .current_dir(&fx.root)
            .env("XDG_RUNTIME_DIR", &runtime)
            .output()
            .expect("binary runs")
    };
    let out = run(&["status"]);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert_eq!(out.status.code(), Some(0), "{stdout}");
    assert!(stdout.contains("csvstats/median"), "{stdout}");
    assert!(stdout.contains("accepted"), "{stdout}");
    assert!(stdout.contains("unlowered"), "{stdout}");

    let json = run(&["status", "--json"]);
    assert!(String::from_utf8_lossy(&json.stdout).contains("\"defs\""));
    run(&["daemon", "stop"]);
}
