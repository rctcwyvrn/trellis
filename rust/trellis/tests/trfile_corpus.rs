//! Step-3 corpora: golden parse trees over `examples/`, the rejection
//! corpus keyed by daemon error code, and the panic-freedom /
//! determinism properties (impl plan 03 step 3).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use trellis::trfile;

fn repo_examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn tr_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("readable dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "tr") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Every checked-in example parses and validates; the full parse tree
/// is golden (bless with `TRELLIS_BLESS=1`).
#[test]
fn examples_golden() {
    let examples = repo_examples();
    let golden_dir = tests_dir().join("golden/trfile");
    fs::create_dir_all(&golden_dir).expect("golden dir");
    let bless = std::env::var_os("TRELLIS_BLESS").is_some();

    let files = tr_files(&examples);
    assert!(!files.is_empty(), "no examples found");
    for path in files {
        let src = fs::read_to_string(&path).expect("readable example");
        let parsed = match trfile::parse_tr(&src, &path, None) {
            Ok(parsed) => parsed,
            Err(report) => panic!("{} failed to parse: {}", path.display(), report.render()),
        };
        let rendered = format!("{parsed:#?}\n");

        let rel = path
            .strip_prefix(&examples)
            .expect("under examples")
            .to_string_lossy()
            .replace('/', "__");
        let golden_path = golden_dir.join(format!("{rel}.txt"));
        if bless {
            fs::write(&golden_path, &rendered).expect("write golden");
        } else {
            let expected = fs::read_to_string(&golden_path).unwrap_or_else(|_| {
                panic!(
                    "missing golden {} (run with TRELLIS_BLESS=1 to create)",
                    golden_path.display()
                )
            });
            assert_eq!(
                expected,
                rendered,
                "golden mismatch for {} (TRELLIS_BLESS=1 to update)",
                path.display()
            );
        }
    }
}

/// One case per validity/syntax rule, keyed by error code in the
/// directory name (`<code>--<n>/`); an optional `tags.txt` supplies
/// the tag vocabulary.
#[test]
fn rejection_corpus() {
    let reject = tests_dir().join("reject_tr");
    let mut cases = 0;
    for entry in fs::read_dir(&reject).expect("reject_tr dir") {
        let case_dir = entry.expect("entry").path();
        if !case_dir.is_dir() {
            continue;
        }
        let dir_name = case_dir.file_name().unwrap().to_string_lossy().into_owned();
        let expected_code = dir_name
            .split_once("--")
            .unwrap_or_else(|| panic!("case dir `{dir_name}` is not `<code>--<n>`"))
            .0
            .to_string();

        let vocabulary: Option<BTreeSet<String>> = fs::read_to_string(case_dir.join("tags.txt"))
            .ok()
            .map(|text| {
                text.lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| l.trim().to_string())
                    .collect()
            });

        let files = tr_files(&case_dir);
        assert_eq!(
            files.len(),
            1,
            "case `{dir_name}` must hold exactly one .tr"
        );
        let src = fs::read_to_string(&files[0]).expect("readable fixture");
        match trfile::parse_tr(&src, &files[0], vocabulary.as_ref()) {
            Ok(_) => panic!("case `{dir_name}` unexpectedly parsed"),
            Err(report) => assert_eq!(
                report.errors[0].code,
                expected_code,
                "case `{dir_name}`: got {}",
                report.render()
            ),
        }
        cases += 1;
    }
    assert!(cases >= 26, "rejection corpus shrank to {cases} cases");
}

mod props {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Arbitrary input never panics the parser.
        #[test]
        fn never_panics(src in "\\PC*") {
            let _ = trfile::parse_tr(&src, Path::new("fuzz.tr"), None);
        }

        /// Parsing is deterministic.
        #[test]
        fn deterministic(src in "\\PC*") {
            let a = render(trfile::parse_tr(&src, Path::new("fuzz.tr"), None));
            let b = render(trfile::parse_tr(&src, Path::new("fuzz.tr"), None));
            prop_assert_eq!(a, b);
        }
    }

    fn render(result: Result<trfile::TrFile, trellis::diag::ErrorReport>) -> String {
        match result {
            Ok(parsed) => format!("{parsed:#?}"),
            Err(report) => report.render(),
        }
    }
}
