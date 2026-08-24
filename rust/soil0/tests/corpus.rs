//! The fixture corpus, always through the built binary (conformance is
//! defined by the CLI alone — plan 02 resolved decision).
//!
//! - `tests/golden/parse/<name>.soil` + `<name>.json`: `soil0 parse`
//!   stdout must byte-equal the golden.
//! - `tests/reject/parse/<code>[-<n>].soil`: `soil0 parse` must exit 1
//!   with diagnostic code `<code>` (the registry key, contract §2).

use std::path::Path;
use std::process::Command;

fn soil0(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_soil0"))
        .args(args)
        .output()
        .expect("spawn soil0")
}

fn fixture_dir(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(rel)
}

fn soil_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("missing corpus dir {}: {e}", dir.display()))
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "soil"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "empty corpus dir {}", dir.display());
    files
}

#[test]
fn golden_parse() {
    for soil in soil_files(&fixture_dir("golden/parse")) {
        let expected_path = soil.with_extension("json");
        let expected = std::fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("missing golden {}: {e}", expected_path.display()));
        let out = soil0(&["parse", soil.to_str().unwrap()]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{} failed: {}",
            soil.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            expected,
            "golden mismatch for {}",
            soil.display()
        );
    }
}

/// `tests/golden/rename/<case>/program.json` + `expected.json`.
#[test]
fn golden_rename() {
    let root = fixture_dir("golden/rename");
    for entry in std::fs::read_dir(&root).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        let manifest = dir.join("program.json");
        let expected = std::fs::read_to_string(dir.join("expected.json")).unwrap();
        let out = soil0(&["rename", manifest.to_str().unwrap()]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{} failed: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            expected,
            "{}",
            dir.display()
        );
    }
}

/// `tests/golden/rename/<case>/expected-infer.json`, when present:
/// `soil0 infer` on the same program must byte-equal it.
#[test]
fn golden_infer() {
    let root = fixture_dir("golden/rename");
    let mut found = false;
    for entry in std::fs::read_dir(&root).unwrap() {
        let dir = entry.unwrap().path();
        let expected_path = dir.join("expected-infer.json");
        if !expected_path.exists() {
            continue;
        }
        found = true;
        let expected = std::fs::read_to_string(&expected_path).unwrap();
        let out = soil0(&["infer", dir.join("program.json").to_str().unwrap()]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{} failed: {}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            expected,
            "{}",
            dir.display()
        );
    }
    assert!(found, "no infer goldens");
}

/// `soil0 check`'s success output is `soil0 infer`'s (contract §8.7):
/// byte-compare against the same golden.
#[test]
fn golden_check_equals_infer() {
    let dir = fixture_dir("golden/rename/median");
    let expected = std::fs::read_to_string(dir.join("expected-infer.json")).unwrap();
    let out = soil0(&["check", dir.join("program.json").to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "check failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), expected);
}

/// The executable step-10 checklist (impl plan 02): every class-1 code
/// in the contract §2 registry is exercised by the reject corpus or by
/// a named unit test.
#[test]
fn audit_every_static_rule_is_covered() {
    let mut covered: Vec<String> = Vec::new();
    for f in soil_files(&fixture_dir("reject/parse")) {
        let stem = f.file_stem().unwrap().to_str().unwrap();
        covered.push(
            stem.rsplit_once('-')
                .map_or(stem, |(h, t)| {
                    if t.chars().all(|c| c.is_ascii_digit()) {
                        h
                    } else {
                        stem
                    }
                })
                .to_string(),
        );
    }
    for rel in ["reject/rename", "reject/check"] {
        for entry in std::fs::read_dir(fixture_dir(rel)).unwrap() {
            let dir = entry.unwrap().path();
            if dir.is_dir() {
                covered.push(dir.file_name().unwrap().to_str().unwrap().to_string());
            }
        }
    }
    // Codes exercised by unit tests rather than corpus fixtures, with
    // their home test.
    let unit_covered = [
        ("parse-expected", "parser_unit::rejections"),
        ("unknown-name", "rename_unit::unknown_names_and_types"),
        ("unknown-type", "rename_unit::unknown_names_and_types"),
        (
            "unknown-qualified",
            "rename_unit::qualified_derived_and_numeric",
        ),
        ("unknown-constructor", "rename_unit::constructor_resolution"),
        ("private-cross-module", "rename_unit::private_visibility"),
        ("duplicate-field", "parser_unit::rejections"),
        ("runtime-panic", "corpus::golden_run_median"),
        ("unfilled-hole", "interp_unit::holes_refuse_to_run"),
    ];
    let all_class1 = [
        "unterminated-string",
        "bad-escape",
        "escape-not-scalar",
        "stray-character",
        "parse-expected",
        "nonassoc-comparison",
        "name-mismatch",
        "wildcard-in-let-rec",
        "misplaced-private-def",
        "multiple-defs",
        "shadowing",
        "unknown-name",
        "unknown-type",
        "unknown-qualified",
        "unknown-constructor",
        "ambiguous-constructor",
        "ambiguous-name",
        "needless-qualification",
        "private-cross-module",
        "type-mismatch",
        "effect-violation",
        "operator-polymorphic",
        "non-derivable",
        "annotation-needed",
        "unknown-field",
        "literal-out-of-range",
        "non-exhaustive-match",
        "redundant-arm",
        "missing-record-rest",
        "duplicate-field",
        "runtime-panic",
        "unfilled-hole",
    ];
    for code in all_class1 {
        let ok = covered.iter().any(|c| c == code) || unit_covered.iter().any(|(c, _)| *c == code);
        assert!(ok, "static rule `{code}` has no rejection coverage");
    }
}

/// `soil0 run` end to end: the median program with the real insertion
/// sort computes correct results through the binary (plan 02 exit
/// criterion).
#[test]
fn golden_run_median() {
    let manifest = fixture_dir("golden/rename/median").join("program.json");
    let m = manifest.to_str().unwrap();
    let odd = soil0(&["run", m, "--entry", "median", "--args", "[[3.0,1.0,2.0]]"]);
    assert_eq!(
        odd.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&odd.stderr)
    );
    assert_eq!(String::from_utf8(odd.stdout).unwrap(), "2.0\n");
    let even = soil0(&[
        "run",
        m,
        "--entry",
        "median",
        "--args",
        "[[4.0,1.0,3.0,2.0]]",
    ]);
    assert_eq!(String::from_utf8(even.stdout).unwrap(), "2.5\n");
    // The unproven `len v > 0` refinement fires as a runtime panic.
    let empty = soil0(&["run", m, "--entry", "median", "--args", "[[]]"]);
    assert_eq!(empty.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&empty.stderr).unwrap();
    assert_eq!(report["errors"][0]["code"], "runtime-panic");
}

/// `soil0 test` on the read_file bundle: byte-golden report, exit 0
/// (pass/xfail only).
#[test]
fn golden_test_bundle() {
    let dir = fixture_dir("bundles/read_file");
    let expected = std::fs::read_to_string(dir.join("expected.json")).unwrap();
    let out = soil0(&["test", dir.join("bundle.json").to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8(out.stdout).unwrap(), expected);
}

/// `tests/reject/rename/<code>/program.json`: nonzero exit with the
/// diagnostic code named by the directory.
#[test]
fn reject_rename() {
    reject_manifest_corpus("reject/rename", "rename");
}

/// `tests/reject/check/<code>/program.json`, via `soil0 check`.
#[test]
fn reject_check() {
    reject_manifest_corpus("reject/check", "check");
}

fn reject_manifest_corpus(rel: &str, command: &str) {
    let root = fixture_dir(rel);
    for entry in std::fs::read_dir(&root).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        let code = dir.file_name().unwrap().to_str().unwrap().to_string();
        let manifest = dir.join("program.json");
        let out = soil0(&[command, manifest.to_str().unwrap()]);
        assert_ne!(
            out.status.code(),
            Some(0),
            "{} should be rejected",
            dir.display()
        );
        let report: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
        assert_eq!(
            report["errors"][0]["code"].as_str().unwrap(),
            code,
            "wrong code for {}",
            dir.display()
        );
    }
}

#[test]
fn reject_parse() {
    for soil in soil_files(&fixture_dir("reject/parse")) {
        let stem = soil.file_stem().unwrap().to_str().unwrap();
        let code = stem.rsplit_once('-').map_or(stem, |(head, tail)| {
            if tail.chars().all(|c| c.is_ascii_digit()) {
                head
            } else {
                stem
            }
        });
        let out = soil0(&["parse", soil.to_str().unwrap()]);
        assert_eq!(
            out.status.code(),
            Some(1),
            "{} should be rejected",
            soil.display()
        );
        let report: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
        assert_eq!(
            report["errors"][0]["code"].as_str().unwrap(),
            code,
            "wrong code for {}",
            soil.display()
        );
    }
}
