//! Exhaustiveness and redundancy (step 7): the check pipeline errs where
//! infer alone does not, witnesses in surface syntax, redundant arms.

use soil0::diag::{Code, Diagnostic};
use soil0::exhaust::check_program;
use soil0::infer::infer_program;
use soil0::manifest::{from_parts, EnvFile};
use soil0::rename::rename_program;

fn run_check(files: &[(&str, &str)]) -> Result<(), Diagnostic> {
    let env: EnvFile = serde_json::from_str(r#"{"types":[]}"#).unwrap();
    let files: Vec<(String, String)> = files
        .iter()
        .map(|(p, s)| (p.to_string(), s.to_string()))
        .collect();
    let prog = from_parts(env, &files)?;
    rename_program(&prog)?;
    check_program(&prog)?;
    Ok(())
}

fn check_err(files: &[(&str, &str)]) -> Diagnostic {
    run_check(files).unwrap_err()
}

#[test]
fn infer_does_not_check_exhaustiveness_but_check_does() {
    let f = (
        "a/f.soil",
        "f : Result I64 I64 -> I64\nf r = match r with | Ok x -> x\n",
    );
    // infer alone: fine.
    let env: EnvFile = serde_json::from_str(r#"{"types":[]}"#).unwrap();
    let prog = from_parts(env, &[(f.0.to_string(), f.1.to_string())]).unwrap();
    rename_program(&prog).unwrap();
    assert!(infer_program(&prog).is_ok());
    // check: non-exhaustive with an `Err _` witness.
    let err = check_err(&[f]);
    assert_eq!(err.code, Code::NonExhaustiveMatch);
    assert!(
        err.notes[0].message.contains("`Err _`"),
        "{}",
        err.notes[0].message
    );
}

#[test]
fn complete_matches_pass() {
    let ok = (
        "a/f.soil",
        "f : Result I64 I64 -> I64\nf r = match r with | Ok x -> x | Err e -> e\n",
    );
    assert!(run_check(&[ok]).is_ok());
    let wild = (
        "a/g.soil",
        "g : Result I64 I64 -> I64\ng r = match r with | Ok x -> x | _ -> 0\n",
    );
    assert!(run_check(&[wild]).is_ok());
    let bool_ok = (
        "a/h.soil",
        "h : Bool -> I64\nh b = match b with | True -> 1 | False -> 0\n",
    );
    assert!(run_check(&[bool_ok]).is_ok());
}

#[test]
fn missing_bool_case_witness() {
    let err = check_err(&[(
        "a/f.soil",
        "f : Bool -> I64\nf b = match b with | True -> 1\n",
    )]);
    assert_eq!(err.code, Code::NonExhaustiveMatch);
    assert!(err.notes[0].message.contains("`False`"));
}

#[test]
fn nested_missing_case() {
    let src =
        "f : Option (Result I64 I64) -> I64\nf o = match o with | None -> 0 | Some (Ok x) -> x\n";
    let err = check_err(&[("a/f.soil", src)]);
    assert_eq!(err.code, Code::NonExhaustiveMatch);
    assert!(
        err.notes[0].message.contains("Some"),
        "{}",
        err.notes[0].message
    );
    let full = "f : Option (Result I64 I64) -> I64\nf o = match o with | None -> 0 | Some (Ok x) -> x | Some (Err e) -> e\n";
    assert!(run_check(&[("a/f.soil", full)]).is_ok());
}

#[test]
fn literal_matches_need_a_default() {
    let err = check_err(&[(
        "a/f.soil",
        "f : I64 -> I64\nf x = match x with | 1 -> 1 | 2 -> 2\n",
    )]);
    assert_eq!(err.code, Code::NonExhaustiveMatch);
    assert!(err.notes[0].message.contains("`_`"));
    assert!(run_check(&[(
        "a/f.soil",
        "f : I64 -> I64\nf x = match x with | 1 -> 1 | _ -> 0\n"
    )])
    .is_ok());
}

#[test]
fn redundant_arms() {
    let dup = (
        "a/f.soil",
        "f : Bool -> I64\nf b = match b with | True -> 1 | True -> 2 | False -> 0\n",
    );
    assert_eq!(check_err(&[dup]).code, Code::RedundantArm);
    let after_wild = (
        "a/g.soil",
        "g : Result I64 I64 -> I64\ng r = match r with | _ -> 0 | Ok x -> x\n",
    );
    assert_eq!(check_err(&[after_wild]).code, Code::RedundantArm);
    let covered = (
        "a/h.soil",
        "h : Result I64 I64 -> I64\nh r = match r with | Ok x -> x | Err e -> e | _ -> 9\n",
    );
    assert_eq!(check_err(&[covered]).code, Code::RedundantArm);
}

#[test]
fn inline_record_payload_exhaustiveness() {
    let full = "f : FsError -> Utf8\nf e = match e with | NotFound { path } -> path | ReadFailed { path, .. } -> path | NotUtf8 { path } -> path\n";
    assert!(run_check(&[("a/f.soil", full)]).is_ok());
    let missing = "f : FsError -> Utf8\nf e = match e with | NotFound { path } -> path | NotUtf8 { path } -> path\n";
    let err = check_err(&[("a/f.soil", missing)]);
    assert_eq!(err.code, Code::NonExhaustiveMatch);
    assert!(
        err.notes[0].message.contains("ReadFailed"),
        "{}",
        err.notes[0].message
    );
}

#[test]
fn examples_pass_check() {
    let median = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/csvstats/median.soil"
    ))
    .unwrap();
    let read_file = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/read_file.soil"
    ))
    .unwrap();
    assert!(run_check(&[
        (
            "stubs/len.soil",
            "len : List F64 -> I64\nlen xs = list_len xs\n"
        ),
        (
            "stubs/nth.soil",
            "nth : List F64 -> I64 -> panic F64\nnth xs i = list_nth xs i\n"
        ),
        (
            "csvstats/sort_by.soil",
            "sort_by : (f : F64 -> F64) -> List F64 -> List F64\nsort_by f xs = xs\n"
        ),
        ("csvstats/median.soil", &median),
    ])
    .is_ok());
    assert!(run_check(&[("prelude/read_file.soil", &read_file)]).is_ok());
}
