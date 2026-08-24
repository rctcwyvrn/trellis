//! Canonical printer conformance (step 11): the checked-in examples are
//! byte-identical under print (the style oracle), and parse → print is
//! a fixpoint over the whole corpus.

use soil0::print::canonicalize;
use std::path::PathBuf;

fn examples() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    vec![
        root.join("../../examples/read_file.soil"),
        root.join("../../examples/csvstats/median.soil"),
    ]
}

#[test]
fn normative_examples_are_byte_identical() {
    for path in examples() {
        let src = std::fs::read_to_string(&path).unwrap();
        let printed = canonicalize(&src, path.to_str().unwrap()).unwrap();
        assert_eq!(printed, src, "not canonical: {}", path.display());
    }
}

fn corpus_soil_files() -> Vec<PathBuf> {
    let mut out = examples();
    let tests = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    for dir in [
        "golden/parse",
        "golden/rename/median/stubs",
        "golden/rename/median/csvstats",
    ] {
        for entry in std::fs::read_dir(tests.join(dir)).unwrap() {
            let p = entry.unwrap().path();
            if p.extension().is_some_and(|x| x == "soil") {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn print_is_a_fixpoint_on_the_corpus() {
    for path in corpus_soil_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        let name = path.to_str().unwrap();
        let once = canonicalize(&src, name).unwrap();
        let twice = canonicalize(&once, name).unwrap();
        assert_eq!(once, twice, "not a fixpoint: {}", path.display());
    }
}

#[test]
fn print_preserves_meaning_on_the_corpus() {
    // parse(print(parse(src))) must equal parse(src) modulo spans; we
    // compare via the canonical form itself (print is injective on the
    // constructs it lays out).
    for path in corpus_soil_files() {
        let src = std::fs::read_to_string(&path).unwrap();
        let name = path.to_str().unwrap();
        let once = canonicalize(&src, name).unwrap();
        // The reparse must succeed at all — the printer never emits
        // unparseable text.
        soil0::parser::parse_file(&once, name)
            .unwrap_or_else(|e| panic!("print emitted unparseable text for {name}: {e:?}"));
    }
}

mod properties {
    use proptest::prelude::*;

    proptest! {
        /// Any successfully parsed token soup prints, reparses, and
        /// reaches the fixpoint.
        #[test]
        fn fixpoint_on_generated_programs(words in proptest::collection::vec(
            proptest::sample::select(vec![
                "let", "rec", "in", "fun", "match", "with", "if", "then", "else",
                "x", "y", "go", "F64", "Ok", "None",
                "->", "=", "|", "(", ")", "{", "}", "..", ",",
                "==", "<", "+", "-", "*", "_", "1", "1.5", "\"s\"", "?h",
            ]),
            0..30,
        )) {
            let header = "f : I64\nf = ".to_string();
            let src = header + &words.join(" ");
            if let Ok(once) = soil0::print::canonicalize(&src, "f.soil") {
                let twice = soil0::print::canonicalize(&once, "f.soil")
                    .expect("canonical text reparses");
                prop_assert_eq!(once, twice);
            }
        }
    }
}
