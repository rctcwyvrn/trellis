//! Checker tests: signatures and check facts for the examples, the HOF
//! row-propagation suite (impl plan 02 step 6), and rejections for every
//! infer-phase rule.

use soil0::diag::Code;
use soil0::infer::{infer_program, InferOutput};
use soil0::manifest::{from_parts, EnvFile};
use soil0::rename::rename_program;

fn run(types_json: &str, files: &[(&str, &str)]) -> Result<InferOutput, soil0::diag::Diagnostic> {
    let env: EnvFile = serde_json::from_str(&format!(r#"{{"types":{types_json}}}"#)).unwrap();
    let files: Vec<(String, String)> = files
        .iter()
        .map(|(p, s)| (p.to_string(), s.to_string()))
        .collect();
    let prog = from_parts(env, &files)?;
    rename_program(&prog)?;
    infer_program(&prog)
}

fn code(types_json: &str, files: &[(&str, &str)]) -> Code {
    run(types_json, files).unwrap_err().code
}

fn ob_kinds(out: &InferOutput, def: usize) -> Vec<String> {
    out.defs[def]
        .checks
        .panic_obligations
        .iter()
        .map(|o| {
            serde_json::to_value(&o.kind).unwrap()["tag"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

const POINT_ENV: &str = r#"[{"name":"Point","params":[],"strategy":{"tag":"Structural"},
  "body":{"tag":"Record","value":{"fields":[
    {"name":"x","shape":{"tag":"SCon","value":{"name":"I64","args":[]}},"ignored":{"tag":"None"}},
    {"name":"y","shape":{"tag":"SCon","value":{"name":"I64","args":[]}},"ignored":{"tag":"None"}}]}}}]"#;

#[test]
fn read_file_checks_clean() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/read_file.soil"
    ))
    .unwrap();
    let out = run("[]", &[("prelude/read_file.soil", &src)]).unwrap();
    let d = &out.defs[0];
    // Row: io on the innermost arrow.
    let row = serde_json::to_value(&d.row).unwrap();
    assert_eq!(row["effects"][0]["tag"], "Io");
    assert_eq!(d.checks.termination, "verified");
    assert!(d.checks.panic_obligations.is_empty());
    // Named params survive canonically.
    let ty = serde_json::to_value(&d.ty).unwrap();
    assert_eq!(ty["value"]["param"]["value"], "fs");
}

#[test]
fn median_program_facts() {
    let median = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/csvstats/median.soil"
    ))
    .unwrap();
    let files = [
        (
            "stubs/len.soil",
            "len : List F64 -> I64\nlen xs = list_len xs\n",
        ),
        (
            "stubs/nth.soil",
            "nth : List F64 -> I64 -> panic F64\nnth xs i = list_nth xs i\n",
        ),
        (
            "csvstats/sort_by.soil",
            "sort_by : (f : F64 -> F64) -> List F64 -> List F64\nsort_by f xs = xs\n",
        ),
        ("csvstats/median.soil", median.as_str()),
    ];
    let out = run("[]", &files).unwrap();
    // nth's list_nth panic is covered by its declared row.
    assert_eq!(out.defs[1].checks.termination, "verified");
    assert!(out.defs[1].checks.panic_obligations.is_empty());
    // median (declared total): divisor + overflow + nth obligations.
    let kinds = ob_kinds(&out, 3);
    assert_eq!(
        kinds,
        [
            "DivZero",
            "DivZero",
            "CalleePanic",
            "CalleePanic",
            "Overflow",
            "CalleePanic"
        ]
    );
    assert_eq!(out.defs[3].checks.termination, "verified");
}

const FOLD: (&str, &str) = (
    "hof/fold.soil",
    "fold : (f : b -> a -> e b) -> b -> List a -> e b\nfold f z xs = z\n",
);

#[test]
fn hof_row_propagation() {
    // A pure lambda through fold: caller stays total.
    let pure = (
        "app/sum.soil",
        "sum : List I64 -> I64\nsum xs = fold (fun acc x -> acc) 0 xs\n",
    );
    let out = run("[]", &[FOLD, pure]).unwrap();
    assert!(out.defs[1].checks.panic_obligations.is_empty());
    assert_eq!(out.defs[1].checks.termination, "verified");

    // A panicking lambda through fold: the caller gets the obligation.
    let panicking = (
        "app/first.soil",
        "first : List I64 -> I64\nfirst xs = fold (fun acc x -> list_nth xs 0) 0 xs\n",
    );
    let out = run("[]", &[FOLD, panicking]).unwrap();
    assert_eq!(ob_kinds(&out, 1), ["CalleePanic"]);

    // An io lambda where a pure function is expected: hard error.
    let apply = (
        "hof/apply.soil",
        "apply : (f : I64 -> I64) -> I64\napply f = f 1\n",
    );
    let io_lam = (
        "app/bad.soil",
        "bad : Clock -> I64\nbad c = apply (fun x -> let _ = clock_now c in x)\n",
    );
    assert_eq!(code("[]", &[apply, io_lam]), Code::EffectViolation);
}

#[test]
fn io_from_total_is_effect_violation() {
    let f = (
        "a/f.soil",
        "f : Fs -> Path -> Result Bytes FsError\nf fs p = fs_read_bytes fs p\n",
    );
    assert_eq!(code("[]", &[f]), Code::EffectViolation);
}

#[test]
fn recursion_facts() {
    let rec = ("a/loop.soil", "loop : I64 -> I64\nloop x = loop x\n");
    let out = run("[]", &[rec]).unwrap();
    assert_eq!(out.defs[0].checks.termination, "unverified");
    let declared = ("a/loop.soil", "loop : I64 -> div I64\nloop x = loop x\n");
    let out = run("[]", &[declared]).unwrap();
    assert_eq!(out.defs[0].checks.termination, "n/a");
    let letrec = (
        "a/f.soil",
        "f : I64 -> I64\nf x = let rec go = fun y -> go y in go x\n",
    );
    let out = run("[]", &[letrec]).unwrap();
    assert_eq!(out.defs[0].checks.termination, "unverified");
}

#[test]
fn gcd_obligations() {
    let gcd = (
        "a/gcd.soil",
        "gcd : U64 -> U64 -> U64\ndecreases b\ngcd a b = if b == 0 then a else gcd b (a % b)\n",
    );
    let out = run("[]", &[gcd]).unwrap();
    // Recursive without declared div; `%` is a divisor obligation.
    assert_eq!(out.defs[0].checks.termination, "unverified");
    assert_eq!(ob_kinds(&out, 0), ["DivZero"]);
}

#[test]
fn operator_rules() {
    assert_eq!(
        code(
            "[]",
            &[("a/f.soil", "f : a -> a -> Bool\nf x y = x == y\n")]
        ),
        Code::OperatorPolymorphic
    );
    assert_eq!(
        code(
            "[]",
            &[(
                "a/f.soil",
                "f : (g : I64 -> I64) -> (h : I64 -> I64) -> Bool\nf g h = g == h\n"
            )]
        ),
        Code::NonDerivable
    );
    assert_eq!(
        code(
            "[]",
            &[("a/f.soil", "f : Utf8 -> Utf8 -> Utf8\nf x y = x + y\n")]
        ),
        Code::TypeMismatch
    );
}

#[test]
fn literals_adopt_context() {
    // `b == 0` with b : U64 — the normative gcd shape (design §3.6,
    // "as in Rust").
    assert!(run("[]", &[("a/f.soil", "f : U64 -> Bool\nf b = b == 0\n")]).is_ok());
    // All-literal operands default to I64.
    let out = run("[]", &[("a/f.soil", "f : Bool\nf = 1 == 2\n")]).unwrap();
    assert!(out.defs[0].checks.panic_obligations.is_empty());
    let out = run("[]", &[("a/f.soil", "f : I64\nf = 1 + 2\n")]).unwrap();
    assert_eq!(ob_kinds(&out, 0), ["Overflow"]);
    // Context can reject: a U8-typed literal out of range.
    assert_eq!(
        code("[]", &[("a/f.soil", "f : U8 -> Bool\nf b = b == 300\n")]),
        Code::LiteralOutOfRange
    );
}

#[test]
fn literal_ranges() {
    assert_eq!(
        code("[]", &[("a/f.soil", "f : U8\nf = (300 : U8)\n")]),
        Code::LiteralOutOfRange
    );
    assert_eq!(
        code("[]", &[("a/f.soil", "f : I64\nf = 9223372036854775808\n")]),
        Code::LiteralOutOfRange
    );
    assert!(run("[]", &[("a/f.soil", "f : U32\nf = (4294967295 : U32)\n")]).is_ok());
}

#[test]
fn records_and_fields() {
    let ok = ("a/f.soil", "f : Point -> I64\nf p = p.x + p.y\n");
    let out = run(POINT_ENV, &[ok]).unwrap();
    assert_eq!(ob_kinds(&out, 0), ["Overflow"]);
    assert_eq!(
        code(POINT_ENV, &[("a/f.soil", "f : Point -> I64\nf p = p.z\n")]),
        Code::UnknownField
    );
    // Record literal resolves by unique field set.
    assert!(run(
        POINT_ENV,
        &[("a/f.soil", "f : I64 -> Point\nf n = { x = n, y = n }\n")]
    )
    .is_ok());
    // Missing field.
    assert_eq!(
        code(
            POINT_ENV,
            &[("a/f.soil", "f : I64 -> Point\nf n = { x = n }\n")]
        ),
        Code::TypeMismatch
    );
    // Record update keeps the type.
    assert!(run(
        POINT_ENV,
        &[("a/f.soil", "f : Point -> Point\nf p = { p with x = 1 }\n")]
    )
    .is_ok());
    // Pattern must name every field or end with `..`.
    assert_eq!(
        code(
            POINT_ENV,
            &[(
                "a/f.soil",
                "f : Point -> I64\nf p = match p with | { x } -> x\n"
            )]
        ),
        Code::MissingRecordRest
    );
    assert!(run(
        POINT_ENV,
        &[(
            "a/f.soil",
            "f : Point -> I64\nf p = match p with | { x, .. } -> x\n"
        )]
    )
    .is_ok());
}

#[test]
fn inline_record_payloads() {
    // read_file's NotUtf8 { path = … } is covered by the example test;
    // here the pattern side: matching ReadFailed without all fields.
    let src =
        "f : FsError -> Utf8\nf e = match e with | ReadFailed { path } -> path | _ -> \"x\"\n";
    assert_eq!(code("[]", &[("a/f.soil", src)]), Code::MissingRecordRest);
    let ok = "f : FsError -> Utf8\nf e = match e with | ReadFailed { message, .. } -> message | _ -> \"x\"\n";
    assert!(run("[]", &[("a/f.soil", ok)]).is_ok());
}

#[test]
fn arity_and_row_placement() {
    assert_eq!(
        code("[]", &[("a/f.soil", "f : I64\nf x = x\n")]),
        Code::TypeMismatch
    );
    assert_eq!(
        code(
            "[]",
            &[(
                "a/f.soil",
                "f : Fs -> io (Path -> Result Bytes FsError)\nf fs p = fs_read_bytes fs p\n"
            )]
        ),
        Code::EffectViolation
    );
}

#[test]
fn canonical_renaming() {
    let d = (
        "a/pick.soil",
        "pick : (g : x -> e y) -> x -> e y\npick g v = g v\n",
    );
    let out = run("[]", &[d]).unwrap();
    let ty = serde_json::to_string(&out.defs[0].ty).unwrap();
    // First appearance order: x → a, e → b, y → c.
    let expected = concat!(
        r#"{"tag":"SArrow","value":{"param":{"tag":"Some","value":"g"},"#,
        r#""dom":{"tag":"SArrow","value":{"param":{"tag":"None"},"dom":{"tag":"SVar","value":{"name":"a"}},"#,
        r#""row":{"effects":[],"var":{"tag":"Some","value":"b"}},"cod":{"tag":"SVar","value":{"name":"c"}}}},"#,
        r#""row":{"effects":[],"var":{"tag":"None"}},"#,
        r#""cod":{"tag":"SArrow","value":{"param":{"tag":"None"},"dom":{"tag":"SVar","value":{"name":"a"}},"#,
        r#""row":{"effects":[],"var":{"tag":"Some","value":"b"}},"cod":{"tag":"SVar","value":{"name":"c"}}}}}}"#
    );
    assert_eq!(ty, expected);
    let row = serde_json::to_string(&out.defs[0].row).unwrap();
    assert_eq!(row, r#"{"effects":[],"var":{"tag":"Some","value":"b"}}"#);
}

#[test]
fn polymorphic_row_skolem_flows() {
    // pick's own body calls g (row skolem e) in a context whose row is
    // the same skolem: fine, no obligations.
    let d = (
        "a/pick.soil",
        "pick : (g : x -> e y) -> x -> e y\npick g v = g v\n",
    );
    let out = run("[]", &[d]).unwrap();
    assert!(out.defs[0].checks.panic_obligations.is_empty());
    assert_eq!(out.defs[0].checks.termination, "verified");
}

#[test]
fn typed_holes() {
    // A hole adopts the goal type; the definition still checks.
    let out = run(
        "[]",
        &[(
            "a/f.soil",
            "f : List F64 -> I64\nf xs = if list_len xs == 0 then ?empty else list_len xs\n",
        )],
    )
    .unwrap();
    let holes = serde_json::to_value(&out.defs[0].checks.holes).unwrap();
    assert_eq!(holes[0]["name"], "empty");
    assert_eq!(holes[0]["ty"]["value"]["name"], "I64");
    // Hole-free definitions serialize without a `holes` key (v1 bytes).
    let clean = run("[]", &[("a/g.soil", "g : I64\ng = 1\n")]).unwrap();
    let checks = serde_json::to_value(&clean.defs[0].checks).unwrap();
    assert!(checks.get("holes").is_none());
    // Duplicate hole names are rejected (syntax-spec §5.11).
    assert_eq!(
        code("[]", &[("a/f.soil", "f : I64 -> I64\nf x = ?a + ?a\n")]),
        Code::Shadowing
    );
    // An unconstrained hole gets a canonical fresh variable.
    let free = run(
        "[]",
        &[("a/h.soil", "h : I64\nh = let _ = ?anything in 1\n")],
    )
    .unwrap();
    let holes = serde_json::to_value(&free.defs[0].checks.holes).unwrap();
    assert_eq!(holes[0]["ty"]["tag"], "SVar");
}

#[test]
fn annotation_forms() {
    // Annotated record literal disambiguates.
    let env = r#"[{"name":"A","params":[],"strategy":{"tag":"Structural"},
      "body":{"tag":"Record","value":{"fields":[{"name":"v","shape":{"tag":"SCon","value":{"name":"I64","args":[]}},"ignored":{"tag":"None"}}]}}},
      {"name":"B","params":[],"strategy":{"tag":"Structural"},
      "body":{"tag":"Record","value":{"fields":[{"name":"v","shape":{"tag":"SCon","value":{"name":"I64","args":[]}},"ignored":{"tag":"None"}}]}}}]"#;
    assert_eq!(
        code(env, &[("a/f.soil", "f : I64 -> A\nf n = { v = n }\n")]),
        Code::AnnotationNeeded
    );
    assert!(run(
        env,
        &[("a/f.soil", "f : I64 -> A\nf n = ({ v = n } : A)\n")]
    )
    .is_ok());
}
