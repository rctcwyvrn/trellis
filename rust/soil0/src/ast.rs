//! The surface AST, contract §4. Serde derives produce the JSON schema
//! exactly: struct field order is declaration order, adjacent tagging is
//! the tr-grammar §7 sum encoding, and `Opt` is the Soil `Option`
//! encoding. These types are what soilc's Trellis AST definitions must
//! later reproduce (plan 05), so nothing here may drift from the frozen
//! contract.

use crate::diag::Opt;
use crate::span::{Span, Spanned};
use serde::{Deserialize, Serialize};

// ---- files and definitions (contract §4.1) ----

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct File {
    pub defs: Vec<Spanned<Def>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Def {
    pub name: String,
    pub sig: Spanned<Type>,
    pub decreases: Opt<Spanned<Expr>>,
    pub params: Vec<Binder>,
    pub body: Spanned<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Binder {
    pub span: Span,
    pub name: String,
}

// ---- types, rows, predicates (contract §4.2) ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "tag", content = "value")]
pub enum Effect {
    Div,
    Panic,
    Io,
    Ffi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    /// Always in the fixed order `Div, Panic, Io, Ffi`.
    pub effects: Vec<Effect>,
    pub var: Opt<String>,
}

impl Row {
    pub fn total() -> Self {
        Row {
            effects: Vec::new(),
            var: Opt(None),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "tag", content = "value")]
pub enum Type {
    Arrow {
        param: Opt<Binder>,
        dom: Box<Spanned<Type>>,
        row: Row,
        cod: Box<Spanned<Type>>,
    },
    Con {
        name: String,
        args: Vec<Spanned<Type>>,
    },
    TVar {
        name: String,
    },
    Refined {
        binder: Binder,
        base: Box<Spanned<Type>>,
        pred: Spanned<Pred>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "tag", content = "value")]
pub enum Pred {
    Implies {
        lhs: Box<Spanned<Pred>>,
        rhs: Box<Spanned<Pred>>,
    },
    POr {
        lhs: Box<Spanned<Pred>>,
        rhs: Box<Spanned<Pred>>,
    },
    PAnd {
        lhs: Box<Spanned<Pred>>,
        rhs: Box<Spanned<Pred>>,
    },
    PNot {
        pred: Box<Spanned<Pred>>,
    },
    PCmp {
        op: CmpOp,
        lhs: Spanned<PExpr>,
        rhs: Spanned<PExpr>,
    },
    Is {
        scrutinee: Spanned<PExpr>,
        type_name: Opt<String>,
        ctor: String,
        payload: Opt<Binder>,
    },
    PCall {
        name: String,
        args: Vec<Spanned<PExpr>>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "tag", content = "value")]
pub enum PExpr {
    PLiteral {
        lit: Lit,
    },
    PPath {
        root: String,
        fields: Vec<String>,
    },
    /// Only `Add`/`Sub`/`Mul` (tr-grammar §2.3); the parser enforces it.
    PArith {
        op: ArithOp,
        lhs: Box<Spanned<PExpr>>,
        rhs: Box<Spanned<PExpr>>,
    },
    PCallE {
        name: String,
        args: Vec<Spanned<PExpr>>,
    },
}

// ---- expressions (contract §4.3) ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "tag", content = "value")]
pub enum Lit {
    /// Underscores stripped; sign never included.
    LInt { digits: String },
    /// Source text, underscore-free.
    LFloat { text: String },
    /// Escapes decoded.
    LStr { value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "tag", content = "value")]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "tag", content = "value")]
pub enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "tag", content = "value")]
pub enum Expr {
    Let {
        is_rec: bool,
        bindings: Vec<LetBinding>,
        body: Box<Spanned<Expr>>,
    },
    Fun {
        params: Vec<Binder>,
        body: Box<Spanned<Expr>>,
    },
    If {
        cond: Box<Spanned<Expr>>,
        then_branch: Box<Spanned<Expr>>,
        else_branch: Box<Spanned<Expr>>,
    },
    Match {
        scrutinee: Box<Spanned<Expr>>,
        arms: Vec<Spanned<Arm>>,
    },
    OrE {
        lhs: Box<Spanned<Expr>>,
        rhs: Box<Spanned<Expr>>,
    },
    AndE {
        lhs: Box<Spanned<Expr>>,
        rhs: Box<Spanned<Expr>>,
    },
    NotE {
        operand: Box<Spanned<Expr>>,
    },
    Cmp {
        op: CmpOp,
        lhs: Box<Spanned<Expr>>,
        rhs: Box<Spanned<Expr>>,
    },
    Arith {
        op: ArithOp,
        lhs: Box<Spanned<Expr>>,
        rhs: Box<Spanned<Expr>>,
    },
    Neg {
        operand: Box<Spanned<Expr>>,
    },
    App {
        r#fn: Box<Spanned<Expr>>,
        arg: Box<Spanned<Expr>>,
    },
    /// A bare variable is `fields = []`; `root` may be a private-ident.
    Path {
        root: String,
        fields: Vec<String>,
    },
    /// `module::def`, `Type::derived`, or `Type::Ctor` — distinguished
    /// semantically (rename), not syntactically.
    Qualified {
        space: String,
        name: String,
    },
    CtorE {
        name: String,
    },
    RecordE {
        update: Opt<PathBase>,
        fields: Vec<FieldInit>,
    },
    Literal {
        lit: Lit,
    },
    Annot {
        expr: Box<Spanned<Expr>>,
        ty: Spanned<Type>,
    },
    /// A typed hole `?name` (contract v1.1, design §3.16).
    Hole {
        name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LetBinding {
    /// `None` is the `_` discard binding.
    pub binder: Opt<Binder>,
    pub value: Spanned<Expr>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Arm {
    pub pattern: Spanned<Pattern>,
    pub body: Spanned<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathBase {
    pub root: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FieldInit {
    pub name: String,
    pub value: Spanned<Expr>,
}

// ---- patterns (contract §4.4) ----

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "tag", content = "value")]
pub enum Pattern {
    PWild,
    PBind {
        binder: Binder,
    },
    PLit {
        lit: Lit,
    },
    PCtor {
        type_name: Opt<String>,
        name: String,
        arg: Opt<Box<Spanned<Pattern>>>,
    },
    PRecord {
        fields: Vec<FieldPat>,
        open: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FieldPat {
    pub name: String,
    /// `None` is punning.
    pub pattern: Opt<Spanned<Pattern>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sum_and_option_shapes() {
        let ty = Type::Con {
            name: "I64".into(),
            args: vec![],
        };
        assert_eq!(
            serde_json::to_string(&ty).unwrap(),
            r#"{"tag":"Con","value":{"name":"I64","args":[]}}"#
        );
        assert_eq!(
            serde_json::to_string(&Pattern::PWild).unwrap(),
            r#"{"tag":"PWild"}"#
        );
        assert_eq!(
            serde_json::to_string(&Row::total()).unwrap(),
            r#"{"effects":[],"var":{"tag":"None"}}"#
        );
        assert_eq!(
            serde_json::to_string(&Effect::Div).unwrap(),
            r#"{"tag":"Div"}"#
        );
    }

    #[test]
    fn bool_field_is_plain_json_bool() {
        let e = Expr::Let {
            is_rec: true,
            bindings: vec![],
            body: Box::new(Spanned {
                span: crate::span::Span {
                    start: 0,
                    end: 0,
                    line: 1,
                    col: 1,
                },
                item: Expr::Path {
                    root: "x".into(),
                    fields: vec![],
                },
            }),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""is_rec":true"#));
    }
}
