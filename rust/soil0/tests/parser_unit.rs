//! Structural parser tests: the syntax-spec §7 worked examples, both
//! checked-in `.soil` examples, and each parse-level disambiguation.

use soil0::ast::*;
use soil0::diag::Code;
use soil0::parser::parse_file;

fn parse_one(src: &str) -> Def {
    let file = parse_file(src, "f.soil").expect("should parse");
    assert_eq!(file.defs.len(), 1);
    file.defs.into_iter().next().unwrap().item
}

fn err_code(src: &str) -> Code {
    parse_file(src, "f.soil").expect_err("should fail").code
}

const GCD: &str =
    "gcd : U64 -> U64 -> U64\ndecreases b\ngcd a b = if b == 0 then a else gcd b (a % b)\n";

#[test]
fn gcd_shape() {
    let def = parse_one(GCD);
    assert_eq!(def.name, "gcd");
    assert!(def.decreases.0.is_some());
    let params: Vec<&str> = def.params.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(params, ["a", "b"]);
    // Sig: U64 -> (U64 -> U64), right-assoc, all rows empty.
    let Type::Arrow { row, cod, .. } = &def.sig.item else {
        panic!("arrow")
    };
    assert!(row.effects.is_empty());
    assert!(matches!(cod.item, Type::Arrow { .. }));
    // Body: if with a Cmp condition; else branch is nested application.
    let Expr::If {
        cond, else_branch, ..
    } = &def.body.item
    else {
        panic!("if")
    };
    assert!(matches!(cond.item, Expr::Cmp { op: CmpOp::Eq, .. }));
    let Expr::App { r#fn, .. } = &else_branch.item else {
        panic!("app")
    };
    assert!(matches!(r#fn.item, Expr::App { .. })); // gcd b, then (a % b)
}

#[test]
fn read_file_shape() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/read_file.soil"
    ))
    .unwrap();
    let def = parse_one(&src);
    // Named domains, io row on the second arrow.
    let Type::Arrow { param, cod, .. } = &def.sig.item else {
        panic!("arrow")
    };
    assert_eq!(param.0.as_ref().unwrap().name, "fs");
    let Type::Arrow {
        param: p2,
        row,
        cod: result_ty,
        ..
    } = &cod.item
    else {
        panic!("arrow2")
    };
    assert_eq!(p2.0.as_ref().unwrap().name, "path");
    assert_eq!(row.effects, vec![Effect::Io]);
    assert!(
        matches!(&result_ty.item, Type::Con { name, args } if name == "Result" && args.len() == 2)
    );
    // Arms attach to the innermost match: outer has 2 arms, the second
    // arm's body is itself a match with 2 arms.
    let Expr::Match { arms, .. } = &def.body.item else {
        panic!("match")
    };
    assert_eq!(arms.len(), 2);
    let Expr::Match { arms: inner, .. } = &arms[1].item.body.item else {
        panic!("inner match")
    };
    assert_eq!(inner.len(), 2);
}

#[test]
fn median_refinements_and_juxtaposed_predicate_calls() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/csvstats/median.soil"
    ))
    .unwrap();
    let def = parse_one(&src);
    let Type::Arrow {
        param, dom, cod, ..
    } = &def.sig.item
    else {
        panic!("arrow")
    };
    assert_eq!(param.0.as_ref().unwrap().name, "xs");
    // Domain: { v : List F64 | len v > 0 }
    let Type::Refined { binder, pred, .. } = &dom.item else {
        panic!("refined dom")
    };
    assert_eq!(binder.name, "v");
    let Pred::PCmp {
        op: CmpOp::Gt, lhs, ..
    } = &pred.item
    else {
        panic!("cmp")
    };
    let PExpr::PCallE { name, args } = &lhs.item else {
        panic!("juxtaposed call")
    };
    assert_eq!(name, "len");
    assert_eq!(args.len(), 1);
    // Codomain: { r : F64 | min xs <= r and r <= max xs }
    let Type::Refined { pred: rpred, .. } = &cod.item else {
        panic!("refined cod")
    };
    assert!(matches!(rpred.item, Pred::PAnd { .. }));
}

#[test]
fn sum_lengths_lambda_and_row_polymorphism() {
    let src = "sum_lengths : (rows : List Row) -> I64\nsum_lengths rows =\n  fold (fun acc r -> acc + len r.cells) 0 rows\n";
    let def = parse_one(src);
    // fold (fun …) 0 rows: left-nested spine [fold, lam, 0, rows].
    let mut spine = &def.body.item;
    let mut atoms = Vec::new();
    while let Expr::App { r#fn, arg } = spine {
        atoms.push(&arg.item);
        spine = &r#fn.item;
    }
    atoms.push(spine);
    atoms.reverse();
    assert_eq!(atoms.len(), 4);
    assert!(matches!(atoms[0], Expr::Path { root, .. } if root == "fold"));
    let Expr::Fun { params, body } = &atoms[1] else {
        panic!("fun")
    };
    assert_eq!(params.len(), 2);
    // acc + len r.cells: len applied to the path r.cells.
    let Expr::Arith {
        op: ArithOp::Add,
        rhs,
        ..
    } = &body.item
    else {
        panic!("add")
    };
    let Expr::App { arg, .. } = &rhs.item else {
        panic!("len app")
    };
    assert!(
        matches!(&arg.item, Expr::Path { root, fields } if root == "r" && fields == &["cells"])
    );
}

#[test]
fn and_disambiguation() {
    // Boolean: one binding whose value is `a and b`.
    let one = parse_one("f : Bool\nf = let x = a and b in x");
    let Expr::Let { bindings, .. } = &one.body.item else {
        panic!("let")
    };
    assert_eq!(bindings.len(), 1);
    assert!(matches!(bindings[0].value.item, Expr::AndE { .. }));
    // Binding separator: two bindings.
    let two = parse_one("f : Bool\nf = let x = a and y = b in x");
    let Expr::Let { bindings, .. } = &two.body.item else {
        panic!("let")
    };
    assert_eq!(bindings.len(), 2);
}

#[test]
fn row_variable_vs_return_type() {
    // `e (List b)`: row variable.
    let d = parse_one("f : (g : a -> e b) -> e (List b)\nf g = go");
    let Type::Arrow { row, cod, .. } = &d.sig.item else {
        panic!()
    };
    assert_eq!(row.var.0.as_deref(), Some("e"));
    assert!(matches!(&cod.item, Type::Con { name, .. } if name == "List"));
    // Lone ident: return type.
    let d2 = parse_one("f : a -> b\nf x = x");
    let Type::Arrow { row, cod, .. } = &d2.sig.item else {
        panic!()
    };
    assert!(row.var.0.is_none());
    assert!(matches!(&cod.item, Type::TVar { name } if name == "b"));
}

#[test]
fn qualified_and_ctor_forms() {
    let d = parse_one("f : I64\nf = m::helper (Result::Ok x) (Row::eq a b) None");
    // Just check it parses and the spine contains the three heads.
    let mut spine = &d.body.item;
    let mut atoms = Vec::new();
    while let Expr::App { r#fn, arg } = spine {
        atoms.push(&arg.item);
        spine = &r#fn.item;
    }
    atoms.push(spine);
    atoms.reverse();
    assert!(
        matches!(atoms[0], Expr::Qualified { space, name } if space == "m" && name == "helper")
    );
    assert!(matches!(atoms[3], Expr::CtorE { name } if name == "None"));
}

#[test]
fn patterns() {
    let d =
        parse_one("f : I64\nf x = match x with | Result::Ok { path, rest = y, .. } -> y | _ -> 0");
    let Expr::Match { arms, .. } = &d.body.item else {
        panic!()
    };
    let Pattern::PCtor {
        type_name,
        name,
        arg,
    } = &arms[0].item.pattern.item
    else {
        panic!()
    };
    assert_eq!(type_name.0.as_deref(), Some("Result"));
    assert_eq!(name, "Ok");
    let Pattern::PRecord { fields, open } = &arg.0.as_ref().unwrap().item else {
        panic!()
    };
    assert!(*open);
    assert_eq!(fields.len(), 2);
    assert!(fields[0].pattern.0.is_none()); // punning
    assert!(matches!(arms[1].item.pattern.item, Pattern::PWild));
}

#[test]
fn record_update_and_annotation() {
    let d = parse_one("f : I64\nf r = { r.inner with cells = (42 : U32) }");
    let Expr::RecordE { update, fields } = &d.body.item else {
        panic!()
    };
    let base = update.0.as_ref().unwrap();
    assert_eq!(base.root, "r");
    assert_eq!(base.fields, ["inner"]);
    assert!(matches!(fields[0].value.item, Expr::Annot { .. }));
}

#[test]
fn private_file_multiple_defs_and_boundaries() {
    let src = "_go : I64 -> I64\n_go x = x + one\n_two : I64\n_two = 2\n";
    let file = parse_file(src, "sub/_private.soil").expect("private file parses");
    assert_eq!(file.defs.len(), 2);
    assert_eq!(file.defs[0].item.name, "_go");
    assert_eq!(file.defs[1].item.name, "_two");
}

#[test]
fn rejections() {
    assert_eq!(err_code("f : I64\nf = 1 < 2 < 3"), Code::NonassocComparison);
    assert_eq!(err_code("f : I64\ng = 1"), Code::NameMismatch);
    assert_eq!(
        err_code("f : I64\nf = let rec _ = 1 in 2"),
        Code::WildcardInLetRec
    );
    assert_eq!(
        err_code("f : I64\nf = 1\ng : I64\ng = 2"),
        Code::MultipleDefs
    );
    assert_eq!(err_code("_f : I64\n_f = 1"), Code::MisplacedPrivateDef);
    assert_eq!(
        parse_file("f : I64\nf = 1", "_private.soil")
            .unwrap_err()
            .code,
        Code::MisplacedPrivateDef
    );
    assert_eq!(
        err_code("f : I64\nf = { x = 1, x = 2 }"),
        Code::DuplicateField
    );
    assert_eq!(err_code("f : I64\nf = )"), Code::ParseExpected);
    assert_eq!(err_code(""), Code::ParseExpected);
    assert_eq!(err_code("f : io I64\nf = 1"), Code::ParseExpected); // row on a bare type
    assert_eq!(err_code("f : List (io)\nf = 1"), Code::ParseExpected); // effect name in type position
}

mod properties {
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn parser_never_panics(src in "\\PC{0,200}") {
            let _ = soil0::parser::parse_file(&src, "f.soil");
            let _ = soil0::parser::parse_file(&src, "_private.soil");
        }

        #[test]
        fn token_sequences_never_panic(words in proptest::collection::vec(
            proptest::sample::select(vec![
                "let", "rec", "and", "or", "not", "in", "fun", "match", "with", "if",
                "then", "else", "decreases", "x", "ys", "_go", "F64", "List", "Ok",
                "->", "=", "|", ":", "::", ".", ",", "..", "(", ")", "{", "}",
                "==", "!=", "<", "<=", ">", ">=", "+", "-", "*", "/", "%", "_",
                "1", "1.5", "\"s\"", "?h", "io", "div",
            ]),
            0..40,
        )) {
            let src = words.join(" ");
            let _ = soil0::parser::parse_file(&src, "f.soil");
        }

        #[test]
        fn parsing_is_deterministic(src in "\\PC{0,120}") {
            let a = soil0::parser::parse_file(&src, "f.soil");
            let b = soil0::parser::parse_file(&src, "f.soil");
            match (a, b) {
                (Ok(x), Ok(y)) => prop_assert_eq!(x, y),
                (Err(x), Err(y)) => prop_assert_eq!(x.code, y.code),
                _ => prop_assert!(false, "nondeterministic outcome"),
            }
        }
    }
}
