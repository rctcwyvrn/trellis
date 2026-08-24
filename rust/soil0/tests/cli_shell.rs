//! Exit-code and stream discipline through the built binary (contract
//! §1). The golden harness always shells out — conformance is defined by
//! the CLI, never the library (plan 02 resolved decision).

use std::process::{Command, Output};

fn soil0(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_soil0"))
        .args(args)
        .output()
        .expect("failed to spawn soil0")
}

#[test]
fn version_is_exact_and_exits_zero() {
    let out = soil0(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!(
            "{{\"soil0_cli\":1,\"soil0\":\"{}\"}}\n",
            env!("CARGO_PKG_VERSION")
        )
    );
    assert!(out.stderr.is_empty());
}

#[test]
fn no_args_is_usage_on_stderr_exit_two() {
    let out = soil0(&[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(report["errors"][0]["code"], "usage");
}

#[test]
fn unknown_command_is_usage() {
    let out = soil0(&["frobnicate"]);
    assert_eq!(out.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(report["errors"][0]["code"], "usage");
    assert!(report["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("frobnicate"));
}

#[test]
fn run_missing_manifest_is_io_exit_two() {
    let out = soil0(&["run", "p.json", "--entry", "x", "--args", "[]"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(report["errors"][0]["code"], "io");
}

#[test]
fn lex_missing_file_is_io_exit_two() {
    let out = soil0(&["lex", "does-not-exist.soil"]);
    assert_eq!(out.status.code(), Some(2));
    let report: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(report["errors"][0]["code"], "io");
}

fn write_temp(name: &str, contents: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("soil0-test-{}-{name}", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn lex_golden_through_the_binary() {
    let path = write_temp("golden.soil", "f : I64\n");
    let out = soil0(&["lex", path.to_str().unwrap()]);
    std::fs::remove_file(&path).ok();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        concat!(
            r#"{"tokens":["#,
            r#"{"span":{"start":0,"end":1,"line":1,"col":1},"item":{"tag":"TIdent","value":{"name":"f"}}},"#,
            r#"{"span":{"start":2,"end":3,"line":1,"col":3},"item":{"tag":"TOp","value":{"op":":"}}},"#,
            r#"{"span":{"start":4,"end":7,"line":1,"col":5},"item":{"tag":"TTypeName","value":{"name":"I64"}}}"#,
            "]}\n"
        )
    );
}

#[test]
fn lex_rejection_is_class_one_with_file() {
    let path = write_temp("stray.soil", "a @ b\n");
    let out = soil0(&["lex", path.to_str().unwrap()]);
    std::fs::remove_file(&path).ok();
    assert_eq!(out.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(report["errors"][0]["code"], "stray-character");
    assert_eq!(report["errors"][0]["file"]["tag"], "Some");
    assert_eq!(report["errors"][0]["span"]["value"]["start"], 2);
}

#[test]
fn stderr_is_single_line_compact_json_in_contract_field_order() {
    let out = soil0(&["lex"]);
    let text = String::from_utf8(out.stderr).unwrap();
    assert_eq!(
        text,
        "{\"errors\":[{\"code\":\"usage\",\"message\":\"lex takes exactly one <file.soil>\",\
         \"file\":{\"tag\":\"None\"},\"span\":{\"tag\":\"None\"},\"notes\":[]}]}\n"
    );
}
