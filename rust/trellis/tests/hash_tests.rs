//! Step-4 tests (impl plan 03): golden hashes over `examples/`, the
//! hash-form goldens for the `.soil` examples, and **one mutation test
//! per invalidation row** (lock-schema §8) — the table that makes the
//! three-part hash mean what it claims.

use std::fs;
use std::path::{Path, PathBuf};

use trellis::hash;
use trellis::trfile;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn bless() -> bool {
    std::env::var_os("TRELLIS_BLESS").is_some()
}

fn check_golden(path: &Path, rendered: &str, what: &str) {
    if bless() {
        fs::write(path, rendered).expect("write golden");
    } else {
        let expected = fs::read_to_string(path)
            .unwrap_or_else(|_| panic!("missing golden {} (TRELLIS_BLESS=1)", path.display()));
        assert_eq!(
            expected, rendered,
            "golden mismatch for {what} (TRELLIS_BLESS=1)"
        );
    }
}

fn spec_hashes_of(path: &Path) -> (String, hash::SpecHashes) {
    let src = fs::read_to_string(path).expect("readable");
    let parsed = trfile::parse_tr(&src, path, None).expect("example parses");
    let hashes = hash::spec_hashes(&src, &parsed);
    (src, hashes)
}

/// Golden three-part hashes for every example `.tr`, and hash-form +
/// `soil_hash` (under a fixture reference map) for every `.soil`.
#[test]
fn example_hashes_golden() {
    let mut lines = Vec::new();
    let mut tr_files = Vec::new();
    let mut soil_files = Vec::new();
    let mut stack = vec![examples()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "tr") {
                tr_files.push(path);
            } else if path.extension().is_some_and(|e| e == "soil") {
                soil_files.push(path);
            }
        }
    }
    tr_files.sort();
    soil_files.sort();

    for path in &tr_files {
        let (_, hashes) = spec_hashes_of(path);
        let rel = path
            .strip_prefix(examples())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        lines.push(format!(
            "{rel}\n  formal {}\n  test   {}\n  prose  {}",
            hashes.formal,
            hashes.test.as_deref().unwrap_or("-"),
            hashes.prose
        ));
    }

    fs::create_dir_all(golden_dir().join("hashform")).expect("dir");
    for path in &soil_files {
        let src = fs::read_to_string(path).expect("readable");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let form = soil0::hashform::hash_form_file(&src, &name).expect("hash form");
        let rel = path
            .strip_prefix(examples())
            .unwrap()
            .to_string_lossy()
            .replace('/', "__");
        check_golden(
            &golden_dir().join(format!("hashform/{rel}.txt")),
            &form,
            &rel,
        );
        // A fixture reference map: fake-but-stable callee hashes, so
        // the recipe itself is pinned (real resolution is step 6).
        let refs: Vec<hash::HashRef> = vec![
            ("sort_by".into(), "sha256:aa".into()),
            ("len".into(), "sha256:bb".into()),
            ("nth".into(), "sha256:cc".into()),
            ("fs_read_bytes".into(), hash::builtin_referent()),
            ("utf8_decode".into(), hash::builtin_referent()),
        ];
        let rel = path
            .strip_prefix(examples())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        lines.push(format!("{rel}\n  soil   {}", hash::soil_hash(&form, &refs)));
    }

    let rendered = format!("{}\n", lines.join("\n"));
    check_golden(&golden_dir().join("hashes.txt"), &rendered, "hashes.txt");
}

// ---- the invalidation table, one mutation per row ----

fn median() -> (String, hash::SpecHashes) {
    spec_hashes_of(&examples().join("csvstats/median.tr"))
}

fn rehash(src: &str) -> hash::SpecHashes {
    let path = examples().join("csvstats/median.tr");
    let parsed = trfile::parse_tr(src, &path, None).expect("mutant parses");
    hash::spec_hashes(src, &parsed)
}

#[test]
fn prose_edit_moves_only_prose() {
    let (src, before) = median();
    let mutated = src.replacen("Returns the median", "Computes the median", 1);
    assert_ne!(src, mutated, "prose mutation did not apply");
    let after = rehash(&mutated);
    assert_eq!(before.formal, after.formal);
    assert_eq!(before.test, after.test);
    assert_ne!(before.prose, after.prose);
}

#[test]
fn test_edit_moves_only_test() {
    let (src, before) = median();
    let mutated = src.replacen("=> 2.0", "=> 3.0", 1);
    assert_ne!(src, mutated, "test mutation did not apply");
    let after = rehash(&mutated);
    assert_eq!(before.formal, after.formal);
    assert_ne!(before.test, after.test);
    assert_eq!(before.prose, after.prose);
}

#[test]
fn xfail_toggle_moves_only_test() {
    let (src, before) = median();
    let mutated = src.replacen("```test odd-length", "```test odd-length xfail", 1);
    assert_ne!(src, mutated, "xfail mutation did not apply");
    let after = rehash(&mutated);
    assert_eq!(before.formal, after.formal);
    assert_ne!(before.test, after.test);
    assert_eq!(before.prose, after.prose);
}

#[test]
fn formal_edit_moves_only_formal() {
    let (src, before) = median();
    let mutated = src.replacen("List F64) -> F64", "List F64) -> panic F64", 1);
    assert_ne!(src, mutated, "formal mutation did not apply");
    let after = rehash(&mutated);
    assert_ne!(before.formal, after.formal);
    assert_eq!(before.test, after.test);
    assert_eq!(before.prose, after.prose);
}

// ---- decisions: their own class (resolved 2026-08-24) ----

const DECISIONS: &str = "timestamps: all timestamps are UTC seconds (I64)\n  \
     rejected: local time (DST bugs)\nerrors: Result everywhere\n";

fn module_src(decisions: &str) -> String {
    format!(
        "---\nname: m\n---\n\nModule prose.\n\n```exports\nmedian\n```\n\n\
         ```decisions\n{decisions}```\n"
    )
}

fn module_hashes(decisions: &str) -> (hash::SpecHashes, Vec<(String, String)>, String) {
    let src = module_src(decisions);
    let parsed = trfile::parse_tr(&src, Path::new("m/_module.tr"), None).expect("module parses");
    let spec = hash::spec_hashes(&src, &parsed);
    let content = parsed
        .blocks
        .iter()
        .find(|b| b.lang == "decisions")
        .map(|b| b.content.clone())
        .expect("decisions block");
    let entries = hash::decision_entry_hashes(&content);
    let labels: Vec<String> = entries.iter().map(|(l, _)| l.clone()).collect();
    let scope = hash::decisions_scope_hash(&labels);
    (spec, entries, scope)
}

#[test]
fn decision_text_edit_moves_only_that_entry() {
    let (spec_a, entries_a, scope_a) = module_hashes(DECISIONS);
    let (spec_b, entries_b, scope_b) =
        module_hashes(&DECISIONS.replace("UTC seconds", "UTC milliseconds"));
    // The three-part hashes are untouched — decisions are their own
    // class, excluded from prose.
    assert_eq!(spec_a, spec_b);
    assert_eq!(scope_a, scope_b);
    assert_ne!(entries_a[0].1, entries_b[0].1, "edited entry must move");
    assert_eq!(entries_a[1], entries_b[1], "untouched entry must not");
}

#[test]
fn decision_addition_moves_only_the_scope_hash() {
    let (_, entries_a, scope_a) = module_hashes(DECISIONS);
    let added = format!("{DECISIONS}naming: snake_case everywhere\n");
    let (_, entries_b, scope_b) = module_hashes(&added);
    assert_ne!(scope_a, scope_b);
    assert_eq!(
        entries_a[..],
        entries_b[..2],
        "existing entries must not move"
    );
}

// ---- the .soil side: alpha-normalization and canonical form ----

#[test]
fn local_rename_does_not_move_soil_hash() {
    let src = fs::read_to_string(examples().join("csvstats/median.soil")).expect("readable");
    let renamed = src.replace("sorted", "ordered");
    assert_ne!(src, renamed, "rename mutation did not apply");
    let a = soil0::hashform::hash_form_file(&src, "median.soil").expect("form");
    let b = soil0::hashform::hash_form_file(&renamed, "median.soil").expect("form");
    assert_eq!(a, b, "a local rename must not change the hash form");
}

#[test]
fn reformat_does_not_move_soil_hash() {
    let src = fs::read_to_string(examples().join("csvstats/median.soil")).expect("readable");
    let reformatted = src.replace("\n  ", "\n      ").replace(" =\n", " =\n\n");
    assert_ne!(src, reformatted, "reformat mutation did not apply");
    let a = soil0::hashform::hash_form_file(&src, "median.soil").expect("form");
    let b = soil0::hashform::hash_form_file(&reformatted, "median.soil").expect("form");
    assert_eq!(a, b, "formatting must not change the hash form");
}

#[test]
fn callee_hash_change_moves_the_caller() {
    let src = fs::read_to_string(examples().join("csvstats/median.soil")).expect("readable");
    let form = soil0::hashform::hash_form_file(&src, "median.soil").expect("form");
    let before = hash::soil_hash(&form, &[("sort_by".into(), "sha256:aa".into())]);
    let after = hash::soil_hash(&form, &[("sort_by".into(), "sha256:ff".into())]);
    assert_ne!(before, after);
}

/// Hashing consumes no paths: the same bytes hash identically when
/// "located" elsewhere.
#[test]
fn hashes_are_location_independent() {
    let path = examples().join("csvstats/median.tr");
    let src = fs::read_to_string(&path).expect("readable");
    let here = trfile::parse_tr(&src, &path, None).expect("parses");
    let there =
        trfile::parse_tr(&src, Path::new("elsewhere/csvstats/median.tr"), None).expect("parses");
    assert_eq!(
        hash::spec_hashes(&src, &here),
        hash::spec_hashes(&src, &there)
    );
}
