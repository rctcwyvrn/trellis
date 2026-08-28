//! Interpreter tests (steps 8–9): arithmetic semantics, recursion,
//! capabilities, and run/test behavior through the library entry points.

use soil0::interp::cmd_run;
use soil0::manifest::{from_parts, EnvFile, Program};

fn prog(files: &[(&str, &str)]) -> Program {
    let env: EnvFile = serde_json::from_str(r#"{"types":[]}"#).unwrap();
    let files: Vec<(String, String)> = files
        .iter()
        .map(|(p, s)| (p.to_string(), s.to_string()))
        .collect();
    from_parts(env, &files).unwrap()
}

fn run(files: &[(&str, &str)], entry: &str, args: &str) -> Result<String, String> {
    cmd_run(prog(files), entry, args).map_err(|d| format!("{}: {}", d.code, d.message))
}

#[test]
fn gcd_runs() {
    let gcd = (
        "a/gcd.soil",
        "gcd : U64 -> U64 -> U64\ndecreases b\ngcd a b = if b == 0 then a else gcd b (a % b)\n",
    );
    assert_eq!(run(&[gcd], "gcd", "[48,18]").unwrap(), "6");
    assert_eq!(run(&[gcd], "gcd", "[17,5]").unwrap(), "1");
}

#[test]
fn floor_division_is_python_semantics() {
    let div = ("a/d.soil", "d : I64 -> I64 -> panic I64\nd a b = a / b\n");
    let md = ("a/m.soil", "m : I64 -> I64 -> panic I64\nm a b = a % b\n");
    assert_eq!(run(&[div], "d", "[-7,2]").unwrap(), "-4");
    assert_eq!(run(&[div], "d", "[7,-2]").unwrap(), "-4");
    assert_eq!(run(&[div], "d", "[-7,-2]").unwrap(), "3");
    assert_eq!(run(&[md], "m", "[-7,2]").unwrap(), "1");
    assert_eq!(run(&[md], "m", "[7,-2]").unwrap(), "-1");
    // Zero divisor panics.
    let err = run(&[div], "d", "[1,0]").unwrap_err();
    assert!(err.contains("divide-by-zero"), "{err}");
}

#[test]
fn overflow_panics() {
    let inc = ("a/inc.soil", "inc : I64 -> panic I64\ninc x = x + 1\n");
    assert_eq!(run(&[inc], "inc", "[41]").unwrap(), "42");
    let err = run(&[inc], "inc", "[9223372036854775807]").unwrap_err();
    assert!(err.contains("overflow"), "{err}");
}

#[test]
fn contextual_literal_widths_at_runtime() {
    let f = ("a/f.soil", "f : U8 -> U8 -> panic U8\nf a b = a + b\n");
    assert_eq!(run(&[f], "f", "[200,55]").unwrap(), "255");
    let err = run(&[f], "f", "[200,56]").unwrap_err();
    assert!(err.contains("overflow"), "{err}");
}

#[test]
fn records_sums_and_matches_run() {
    let f = (
        "a/classify.soil",
        "classify : I64 -> Result Utf8 I64\nclassify x = if x > 0 then Ok \"pos\" else Err x\n",
    );
    assert_eq!(
        run(&[f], "classify", "[3]").unwrap(),
        r#"{"tag":"Ok","value":"pos"}"#
    );
    assert_eq!(
        run(&[f], "classify", "[-2]").unwrap(),
        r#"{"tag":"Err","value":-2}"#
    );
    let unwrap = (
        "a/or_zero.soil",
        "or_zero : Result I64 Utf8 -> I64\nor_zero r = match r with | Ok x -> x | Err _ -> 0\n",
    );
    assert_eq!(
        run(&[unwrap], "or_zero", r#"[{"tag":"Ok","value":7}]"#).unwrap(),
        "7"
    );
    assert_eq!(
        run(&[unwrap], "or_zero", r#"[{"tag":"Err","value":"nope"}]"#).unwrap(),
        "0"
    );
}

#[test]
fn real_fs_capability_injection() {
    let path = std::env::temp_dir().join(format!("soil0-run-{}.txt", std::process::id()));
    std::fs::write(&path, "hello").unwrap();
    let read = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/read_file.soil"
    ))
    .unwrap();
    let out = run(
        &[("prelude/read_file.soil", &read)],
        "read_file",
        &format!("[{:?}]", path.to_str().unwrap()),
    )
    .unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(out, r#"{"tag":"Ok","value":"hello"}"#);
    let missing = run(
        &[("prelude/read_file.soil", &read)],
        "read_file",
        r#"["/definitely/not/here"]"#,
    )
    .unwrap();
    assert!(missing.contains(r#""tag":"NotFound""#), "{missing}");
}

#[test]
fn fake_rand_is_deterministic() {
    let two = (
        "a/two.soil",
        "two : Rand -> io U64\ntwo r = let _ = rand_u64 r in rand_u64 r\n",
    );
    // Through a bundle-style path: build the fake via the interpreter.
    use soil0::interp::{builtin_value, session};
    use soil_rt::Value;
    let s1 = session(prog(&[two])).unwrap();
    let s2 = session(prog(&[two])).unwrap();
    let draw = |s: &soil0::interp::Session| -> u64 {
        let fake = builtin_value(&s.core, "fake_rand").unwrap();
        let Value::Closure(c) = fake else { panic!() };
        let r = c.call(&[Value::U64(42)]).unwrap();
        let entry = soil0::interp::def_value(&s.core, 0).unwrap();
        let Value::Closure(f) = entry else { panic!() };
        let Value::U64(x) = f.call(&[r]).unwrap() else {
            panic!()
        };
        x
    };
    let a = draw(&s1);
    let b = draw(&s2);
    assert_eq!(a, b, "same seed, same stream");
}

#[test]
fn holes_refuse_to_run() {
    let f = (
        "a/f.soil",
        "f : I64 -> I64\nf x = if x > 0 then x else ?rest\n",
    );
    let err = run(&[f], "f", "[1]").unwrap_err();
    assert!(err.starts_with("unfilled-hole"), "{err}");
}

#[test]
fn utf8_string_primitives() {
    // v1.3 (contract §11): split / trim / parse_f64.
    let split = (
        "a/sp.soil",
        "sp : Utf8 -> Utf8 -> List Utf8\nsp sep s = utf8_split sep s\n",
    );
    assert_eq!(
        run(&[split], "sp", r#"[",","1.0,2.5,3.0"]"#).unwrap(),
        r#"["1.0","2.5","3.0"]"#
    );
    // n separators yield n+1 cells; empty input is one empty cell.
    assert_eq!(run(&[split], "sp", r#"[",",""]"#).unwrap(), r#"[""]"#);
    assert_eq!(
        run(&[split], "sp", r#"[",","a,,b"]"#).unwrap(),
        r#"["a","","b"]"#
    );
    // Empty separator: the whole string as one cell (pinned).
    assert_eq!(run(&[split], "sp", r#"["","ab"]"#).unwrap(), r#"["ab"]"#);

    let trim = ("a/tr.soil", "tr : Utf8 -> Utf8\ntr s = utf8_trim s\n");
    assert_eq!(run(&[trim], "tr", r#"["  4.0 \t"]"#).unwrap(), r#""4.0""#);
    assert_eq!(run(&[trim], "tr", r#"[""]"#).unwrap(), r#""""#);

    let parse = (
        "a/pf.soil",
        "pf : Utf8 -> Option F64\npf s = utf8_parse_f64 s\n",
    );
    assert_eq!(
        run(&[parse], "pf", r#"["2.5"]"#).unwrap(),
        r#"{"tag":"Some","value":2.5}"#
    );
    assert_eq!(
        run(&[parse], "pf", r#"["-42"]"#).unwrap(),
        r#"{"tag":"Some","value":-42.0}"#
    );
    // No exponents, no underscores, no junk (contract §11 grammar).
    for bad in ["1e3", "1_000", "x", "", ".", "1.", ".5", "--1", "1.2.3"] {
        assert_eq!(
            run(
                &[parse],
                "pf",
                &format!("[{}]", serde_json::to_string(bad).unwrap())
            )
            .unwrap(),
            r#"{"tag":"None"}"#,
            "{bad} must not parse"
        );
    }
}
