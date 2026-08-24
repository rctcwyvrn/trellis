//! Renamer tests: reference sets, both exactly-one-spelling rules
//! (spec §5.9 constructors, §5.10 definitions), shadowing in every
//! binder position, `_private` visibility, forward references.

use soil0::diag::Code;
use soil0::manifest::{from_parts, EnvFile};
use soil0::rename::{rename_program, RenameOutput};

fn env(types_json: &str) -> EnvFile {
    serde_json::from_str(&format!(r#"{{"types":{types_json}}}"#)).unwrap()
}

fn run(types_json: &str, files: &[(&str, &str)]) -> Result<RenameOutput, soil0::diag::Diagnostic> {
    let files: Vec<(String, String)> = files
        .iter()
        .map(|(p, s)| (p.to_string(), s.to_string()))
        .collect();
    let prog = from_parts(env(types_json), &files)?;
    rename_program(&prog)
}

fn code(types_json: &str, files: &[(&str, &str)]) -> Code {
    run(types_json, files).unwrap_err().code
}

const STATUS_ENV: &str = r#"[{"name":"Status","params":[],"strategy":{"tag":"Structural"},
  "body":{"tag":"Sum","value":{"variants":[{"name":"Ok","payload":{"tag":"None"}}]}}}]"#;

#[test]
fn read_file_reference_sets() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/read_file.soil"
    ))
    .unwrap();
    let out = run("[]", &[("prelude/read_file.soil", &src)]).unwrap();
    let refs = &out.defs[0].refs;
    assert_eq!(refs.builtins, ["fs_read_bytes", "utf8_decode"]);
    assert_eq!(refs.types, ["Fs", "FsError", "Path", "Result", "Utf8"]);
    assert!(refs.defs.is_empty());
    assert!(refs.privates.is_empty());
}

const LEN: (&str, &str) = (
    "stubs/len.soil",
    "len : List F64 -> I64\nlen xs = list_len xs\n",
);
const NTH: (&str, &str) = (
    "stubs/nth.soil",
    "nth : List F64 -> I64 -> panic F64\nnth xs i = list_nth xs i\n",
);
const SORT_BY: (&str, &str) = (
    "csvstats/sort_by.soil",
    "sort_by : (f : F64 -> F64) -> List F64 -> List F64\nsort_by f xs = xs\n",
);

#[test]
fn median_matches_the_contract_example() {
    let median = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/csvstats/median.soil"
    ))
    .unwrap();
    let out = run(
        "[]",
        &[LEN, NTH, SORT_BY, ("csvstats/median.soil", &median)],
    )
    .unwrap();
    let refs = &out.defs[3].refs;
    // The contract §8.3 example, verbatim — cross-module bare refs are
    // legal (spec §5.10), and predicates contribute nothing.
    assert_eq!(refs.defs, ["len", "nth", "sort_by"]);
    assert_eq!(refs.types, ["F64", "List"]);
    assert!(refs.builtins.is_empty());
}

#[test]
fn def_name_exactly_one_spelling() {
    let a = ("a/f.soil", "f : I64\nf = 1\n");
    let b = ("b/f.soil", "f : I64\nf = 2\n");
    // Ambiguous bare.
    let caller = ("c/g.soil", "g : I64\ng = f\n");
    assert_eq!(code("[]", &[a, b, caller]), Code::AmbiguousName);
    // Qualified resolves.
    let caller_q = ("c/g.soil", "g : I64\ng = a::f\n");
    let out = run("[]", &[a, b, caller_q]).unwrap();
    assert_eq!(out.defs[2].refs.defs, ["a::f"]);
    // Needless qualification of a unique name.
    let caller_needless = ("c/g.soil", "g : I64\ng = a::f\n");
    assert_eq!(
        code("[]", &[a, caller_needless]),
        Code::NeedlessQualification
    );
}

#[test]
fn forward_reference_is_detected() {
    let caller = ("a/g.soil", "g : I64\ng = h\n");
    let callee = ("a/h.soil", "h : I64\nh = 1\n");
    let err = run("[]", &[caller, callee]).unwrap_err();
    assert_eq!(err.code, Code::ForwardReference);
    assert_eq!(err.code.class(), 2);
    // Self-recursion is not a forward reference.
    let rec = ("a/r.soil", "r : I64 -> div I64\nr x = r x\n");
    assert!(run("[]", &[rec]).is_ok());
}

#[test]
fn shadowing_in_every_binder_position() {
    // Parameter shadows a definition.
    let d = ("a/one.soil", "one : I64\none = 1\n");
    let p = ("a/g.soil", "g : I64 -> I64\ng one = one\n");
    assert_eq!(code("[]", &[d, p]), Code::Shadowing);
    // Let shadows a parameter.
    assert_eq!(
        code(
            "[]",
            &[("a/g.soil", "g : I64 -> I64\ng x = let x = 1 in x\n")]
        ),
        Code::Shadowing
    );
    // Pattern (punned field) shadows a let.
    assert_eq!(
        code(
            "[]",
            &[(
                "a/g.soil",
                "g : I64 -> I64\ng r = let path = 1 in match r with | { path } -> path\n"
            )]
        ),
        Code::Shadowing
    );
    // Parameter shadows a builtin.
    assert_eq!(
        code("[]", &[("a/g.soil", "g : I64 -> I64\ng unit = 1\n")]),
        Code::Shadowing
    );
    // A definition named like a builtin.
    assert_eq!(
        code("[]", &[("a/unit.soil", "unit : I64\nunit = 1\n")]),
        Code::Shadowing
    );
    // Duplicate definition in one module.
    assert_eq!(
        code(
            "[]",
            &[("a/_private.soil", "_h : I64\n_h = 1\n_h : I64\n_h = 2\n")]
        ),
        Code::Shadowing
    );
}

#[test]
fn private_visibility() {
    let private = ("a/_private.soil", "_h : I64\n_h = 1\n");
    let same = ("a/f.soil", "f : I64\nf = _h\n");
    let out = run("[]", &[private, same]).unwrap();
    assert_eq!(out.defs[1].refs.privates, ["_h"]);
    let other = ("b/g.soil", "g : I64\ng = _h\n");
    assert_eq!(code("[]", &[private, other]), Code::PrivateCrossModule);
}

#[test]
fn constructor_resolution() {
    // `Ok` is unique with the kernel alone…
    assert!(run(
        "[]",
        &[("a/f.soil", "f : I64 -> Result I64 I64\nf x = Ok x\n")]
    )
    .is_ok());
    // …ambiguous once Status adds a second `Ok`…
    assert_eq!(
        code(
            STATUS_ENV,
            &[("a/f.soil", "f : I64 -> Result I64 I64\nf x = Ok x\n")]
        ),
        Code::AmbiguousConstructor
    );
    // …resolved by qualification…
    let out = run(
        STATUS_ENV,
        &[(
            "a/f.soil",
            "f : I64 -> Result I64 I64\nf x = Result::Ok x\n",
        )],
    )
    .unwrap();
    assert!(out.defs[0].refs.types.contains(&"Result".to_string()));
    // …but qualifying the still-unique `Err` is an error.
    assert_eq!(
        code(
            STATUS_ENV,
            &[(
                "a/f.soil",
                "f : I64 -> Result I64 I64\nf x = Result::Err x\n"
            )]
        ),
        Code::NeedlessQualification
    );
    // Unknown constructor, bare and qualified.
    assert_eq!(
        code("[]", &[("a/f.soil", "f : I64\nf = Bogus\n")]),
        Code::UnknownConstructor
    );
    assert_eq!(
        code("[]", &[("a/f.soil", "f : I64\nf = Result::Bogus\n")]),
        Code::UnknownConstructor
    );
}

#[test]
fn qualified_derived_and_numeric() {
    let out = run(
        "[]",
        &[(
            "a/f.soil",
            "f : FsError -> FsError -> Bool\nf x y = FsError::eq x y\n",
        )],
    )
    .unwrap();
    assert_eq!(out.defs[0].refs.builtins, ["FsError::eq"]);
    let out2 = run(
        "[]",
        &[("a/f.soil", "f : I64 -> I64\nf x = I64::add x 1\n")],
    )
    .unwrap();
    assert_eq!(out2.defs[0].refs.builtins, ["I64::add"]);
    assert_eq!(
        code("[]", &[("a/f.soil", "f : I64\nf = Utf8::add\n")]),
        Code::UnknownQualified
    );
    assert_eq!(
        code("[]", &[("a/f.soil", "f : I64\nf = m::nope\n")]),
        Code::UnknownQualified
    );
}

#[test]
fn unknown_names_and_types() {
    assert_eq!(
        code("[]", &[("a/f.soil", "f : I64\nf = zork\n")]),
        Code::UnknownName
    );
    assert_eq!(
        code("[]", &[("a/f.soil", "f : Zork\nf = 1\n")]),
        Code::UnknownType
    );
}

#[test]
fn filename_is_identity() {
    assert_eq!(
        code("[]", &[("a/x.soil", "y : I64\ny = 1\n")]),
        Code::NameMismatch
    );
}

#[test]
fn env_validation() {
    // Kernel redeclaration.
    let redecl = r#"[{"name":"Result","params":[],"strategy":{"tag":"Structural"},
        "body":{"tag":"OpaqueBody"}}]"#;
    assert_eq!(
        code(redecl, &[("a/f.soil", "f : I64\nf = 1\n")]),
        Code::MalformedInput
    );
    // SArrow rejected.
    let arrow = r#"[{"name":"Sorter","params":[],"strategy":{"tag":"Structural"},
        "body":{"tag":"Alias","value":{"ty":{"tag":"SArrow","value":{
          "param":{"tag":"None"},
          "dom":{"tag":"SCon","value":{"name":"I64","args":[]}},
          "row":{"effects":[],"var":{"tag":"None"}},
          "cod":{"tag":"SCon","value":{"name":"I64","args":[]}}}}}}}]"#;
    assert_eq!(
        code(arrow, &[("a/f.soil", "f : I64\nf = 1\n")]),
        Code::MalformedInput
    );
    // Undeclared type variable.
    let freevar = r#"[{"name":"Box","params":[],"strategy":{"tag":"Structural"},
        "body":{"tag":"Alias","value":{"ty":{"tag":"SVar","value":{"name":"a"}}}}}]"#;
    assert_eq!(
        code(freevar, &[("a/f.soil", "f : I64\nf = 1\n")]),
        Code::MalformedInput
    );
}
