//! Recursive descent per soil-syntax-spec §3, with the named
//! disambiguations:
//!
//! - **`and`**: at the boolean-`and` level, the two-token lookahead
//!   `binding-name "="` stops the expression so the enclosing `let` can
//!   read the next binding (spec §3.3). `=` never occurs in expressions,
//!   so this is unambiguous.
//! - **Non-associative comparisons**: `a < b < c` is `nonassoc-comparison`.
//! - **Match arm extent**: an arm body extends maximally; `|` attaches to
//!   the innermost open match (parenthesize non-tail nested matches).
//! - **Row vs return type** (spec §3.2): after `->`, a lowercase ident is a
//!   row variable iff a type follows it.
//! - **Equation-start lookahead**: juxtaposition plus no layout means a
//!   type (or a `decreases` expression) would otherwise swallow the
//!   equation head; an ident equal to the definition's name followed by
//!   idents-then-`=` never continues a type or expression. Likewise an
//!   ident followed by `:` begins the next definition (private files).
//! - **Rows** are parsed only immediately after `->` (the only position
//!   where the AST can attach them; the EBNF's `type ::= cod` recursion
//!   would otherwise admit degenerate rows on bare types).

use crate::ast::*;
use crate::diag::{Code, Diagnostic, Opt};
use crate::lexer;
use crate::span::{LineMap, Span, Spanned};
use crate::token::Token;

pub fn parse_file(src: &str, filename: &str) -> Result<File, Diagnostic> {
    let tokens = lexer::lex(src)?;
    let is_private = std::path::Path::new(filename)
        .file_name()
        .map(|f| f == "_private.soil")
        .unwrap_or(false);
    let mut p = Parser {
        tokens,
        pos: 0,
        map: LineMap::new(src),
        src_len: src.len(),
        defname: String::new(),
    };
    let mut defs = Vec::new();
    while p.tok(0).is_some() {
        defs.push(p.parse_def(is_private)?);
    }
    if !is_private {
        match defs.len() {
            0 => return Err(p.err_expected("a definition")),
            1 => {}
            _ => return Err(p.err_at(
                Code::MultipleDefs,
                "a .soil file holds exactly one definition (only `_private.soil` may hold several)"
                    .to_string(),
                defs[1].span,
            )),
        }
    }
    Ok(File { defs })
}

/// Parses a standalone signature `name : type` — the `.tr` `soil-sig`
/// block (tr-grammar §3.1). The daemon links this so the signature
/// grammar has exactly one implementation. Library-only, like the
/// other `_str` entries (non-contractual, soil0-cli §12).
pub fn parse_sig_str(src: &str) -> Result<(String, Spanned<Type>), Diagnostic> {
    let tokens = lexer::lex(src)?;
    let mut p = Parser {
        tokens,
        pos: 0,
        map: LineMap::new(src),
        src_len: src.len(),
        defname: String::new(),
    };
    let name = match p.tok(0) {
        Some(Token::TIdent { name }) => {
            let name = name.clone();
            p.bump();
            name
        }
        _ => return Err(p.err_expected("a definition name")),
    };
    p.expect_op(":")?;
    let ty = p.parse_type()?;
    if p.tok(0).is_some() {
        return Err(p.err_expected("end of signature"));
    }
    Ok((name, ty))
}

/// Parses a standalone predicate — the `.tr` clause bodies
/// (`requires`/`ensures`/`invariant`/`property`, tr-grammar §2.3).
pub fn parse_pred_str(src: &str) -> Result<Spanned<Pred>, Diagnostic> {
    let tokens = lexer::lex(src)?;
    let mut p = Parser {
        tokens,
        pos: 0,
        map: LineMap::new(src),
        src_len: src.len(),
        defname: String::new(),
    };
    let pred = p.parse_pred()?;
    if p.tok(0).is_some() {
        return Err(p.err_expected("end of predicate"));
    }
    Ok(pred)
}

/// Parses a standalone predicate-language expression — the `.tr`
/// `ignored`-field default (tr-grammar §4.1).
pub fn parse_pexpr_str(src: &str) -> Result<Spanned<PExpr>, Diagnostic> {
    let tokens = lexer::lex(src)?;
    let mut p = Parser {
        tokens,
        pos: 0,
        map: LineMap::new(src),
        src_len: src.len(),
        defname: String::new(),
    };
    let expr = p.parse_pexpr()?;
    if p.tok(0).is_some() {
        return Err(p.err_expected("end of expression"));
    }
    Ok(expr)
}

/// Parses a standalone type (builtin signature table, tests).
pub fn parse_type_str(src: &str) -> Result<Spanned<Type>, Diagnostic> {
    let tokens = lexer::lex(src)?;
    let mut p = Parser {
        tokens,
        pos: 0,
        map: LineMap::new(src),
        src_len: src.len(),
        defname: String::new(),
    };
    let ty = p.parse_type()?;
    if p.tok(0).is_some() {
        return Err(p.err_expected("end of type"));
    }
    Ok(ty)
}

const EFFECTS: &[(&str, Effect)] = &[
    ("div", Effect::Div),
    ("panic", Effect::Panic),
    ("io", Effect::Io),
    ("ffi", Effect::Ffi),
];

fn effect_of(name: &str) -> Option<Effect> {
    EFFECTS.iter().find(|(n, _)| *n == name).map(|(_, e)| *e)
}

struct Parser {
    tokens: Vec<Spanned<Token>>,
    pos: usize,
    map: LineMap,
    src_len: usize,
    defname: String,
}

impl Parser {
    // ---- token machinery ----

    fn tok(&self, ahead: usize) -> Option<&Token> {
        self.tokens.get(self.pos + ahead).map(|s| &s.item)
    }

    fn span_here(&self) -> Span {
        match self.tokens.get(self.pos) {
            Some(t) => t.span,
            None => self.map.span(self.src_len, self.src_len),
        }
    }

    fn bump(&mut self) -> Span {
        let s = self.span_here();
        self.pos += 1;
        s
    }

    fn join<T>(&self, a: Span, b: Span, item: T) -> Spanned<T> {
        Spanned {
            span: self.map.span(a.start, b.end),
            item,
        }
    }

    fn is_op(&self, ahead: usize, op: &str) -> bool {
        matches!(self.tok(ahead), Some(Token::TOp { op: o }) if o == op)
    }

    fn is_kw(&self, ahead: usize, word: &str) -> bool {
        matches!(self.tok(ahead), Some(Token::TKeyword { word: w }) if w == word)
    }

    fn is_pred_word(&self, ahead: usize, word: &str) -> bool {
        // `implies` and `is` are predicate-only words, not Soil keywords.
        matches!(self.tok(ahead), Some(Token::TIdent { name }) if name == word)
    }

    fn describe(&self) -> String {
        match self.tok(0) {
            None => "end of file".to_string(),
            Some(Token::TIdent { name }) => format!("`{name}`"),
            Some(Token::TPrivate { name }) => format!("`{name}`"),
            Some(Token::TTypeName { name }) => format!("`{name}`"),
            Some(Token::TKeyword { word }) => format!("`{word}`"),
            Some(Token::TOp { op }) => format!("`{op}`"),
            Some(Token::TInt { digits }) => format!("`{digits}`"),
            Some(Token::TFloat { text }) => format!("`{text}`"),
            Some(Token::TStr { .. }) => "a string literal".to_string(),
            Some(Token::THole { name }) => format!("`?{name}`"),
        }
    }

    fn err_at(&self, code: Code, message: String, span: Span) -> Diagnostic {
        Diagnostic {
            code,
            message,
            file: Opt(None),
            span: Opt(Some(span)),
            notes: Vec::new(),
        }
    }

    fn err_expected(&self, what: &str) -> Diagnostic {
        self.err_at(
            Code::ParseExpected,
            format!("expected {what}, found {}", self.describe()),
            self.span_here(),
        )
    }

    fn expect_op(&mut self, op: &str) -> Result<Span, Diagnostic> {
        if self.is_op(0, op) {
            Ok(self.bump())
        } else {
            Err(self.err_expected(&format!("`{op}`")))
        }
    }

    fn expect_kw(&mut self, word: &str) -> Result<Span, Diagnostic> {
        if self.is_kw(0, word) {
            Ok(self.bump())
        } else {
            Err(self.err_expected(&format!("`{word}`")))
        }
    }

    fn ident_binder(&mut self) -> Result<Binder, Diagnostic> {
        match self.tok(0) {
            Some(Token::TIdent { name }) => {
                let name = name.clone();
                let span = self.bump();
                Ok(Binder { span, name })
            }
            _ => Err(self.err_expected("an identifier")),
        }
    }

    /// `ident | private-ident` with its span.
    fn defname_token(&mut self) -> Result<(String, Span), Diagnostic> {
        match self.tok(0) {
            Some(Token::TIdent { name }) | Some(Token::TPrivate { name }) => {
                let name = name.clone();
                let span = self.bump();
                Ok((name, span))
            }
            _ => Err(self.err_expected("a definition name")),
        }
    }

    /// True if the token at `ahead` begins the equation head:
    /// `name { ident } "="`. The head is the run member equal to the
    /// definition's name; when no member matches (a mismatched equation
    /// name, destined for `name-mismatch`), the run's first ident is
    /// taken as the head so the signature type does not swallow it.
    fn is_equation_start(&self, ahead: usize) -> bool {
        let name = match self.tok(ahead) {
            Some(Token::TIdent { name }) | Some(Token::TPrivate { name }) => name,
            _ => return false,
        };
        let mut defname_later = false;
        let mut j = ahead + 1;
        loop {
            match self.tok(j) {
                Some(Token::TIdent { name: n }) => {
                    if *n == self.defname {
                        defname_later = true;
                    }
                    j += 1;
                }
                Some(Token::TOp { op }) if op == "=" => break,
                _ => return false,
            }
        }
        *name == self.defname || !defname_later
    }

    /// True if the token at `ahead` begins the *next* definition's
    /// signature (`name ":"`, private files).
    fn is_signature_start(&self, ahead: usize) -> bool {
        matches!(
            self.tok(ahead),
            Some(Token::TIdent { .. }) | Some(Token::TPrivate { .. })
        ) && self.is_op(ahead + 1, ":")
    }

    // ---- definitions ----

    fn parse_def(&mut self, is_private: bool) -> Result<Spanned<Def>, Diagnostic> {
        let (name, name_span) = self.defname_token()?;
        if name.starts_with('_') != is_private {
            let msg = if is_private {
                format!(
                    "`{name}`: definitions in `_private.soil` must have private names (`_name`)"
                )
            } else {
                format!("`{name}`: private definitions live only in `_private.soil`")
            };
            return Err(self.err_at(Code::MisplacedPrivateDef, msg, name_span));
        }
        self.defname = name.clone();
        self.expect_op(":")?;
        let sig = self.parse_type()?;
        let decreases = if self.is_kw(0, "decreases") {
            self.bump();
            Opt(Some(self.parse_expr()?))
        } else {
            Opt(None)
        };
        let (eq_name, eq_span) = self.defname_token()?;
        if eq_name != name {
            return Err(self.err_at(
                Code::NameMismatch,
                format!("equation is for `{eq_name}` but the signature names `{name}`"),
                eq_span,
            ));
        }
        let mut params = Vec::new();
        while let Some(Token::TIdent { .. }) = self.tok(0) {
            params.push(self.ident_binder()?);
        }
        self.expect_op("=")?;
        let body = self.parse_expr()?;
        let end = body.span;
        Ok(self.join(
            name_span,
            end,
            Def {
                name,
                sig,
                decreases,
                params,
                body,
            },
        ))
    }

    // ---- types ----

    /// Does the token at `ahead` start a type atom? (Used for application
    /// arguments and the row-variable rule.)
    fn starts_type_atom(&self, ahead: usize) -> bool {
        match self.tok(ahead) {
            Some(Token::TTypeName { .. }) => true,
            Some(Token::TOp { op }) if op == "(" || op == "{" => true,
            Some(Token::TIdent { name }) => {
                effect_of(name).is_none()
                    && !self.is_equation_start(ahead)
                    && !self.is_signature_start(ahead)
            }
            _ => false,
        }
    }

    fn parse_type(&mut self) -> Result<Spanned<Type>, Diagnostic> {
        // Named domain: `( ident : type ) -> …`
        if self.is_op(0, "(")
            && matches!(self.tok(1), Some(Token::TIdent { .. }))
            && self.is_op(2, ":")
        {
            let start = self.bump(); // (
            let binder = self.ident_binder()?;
            self.bump(); // :
            let dom = self.parse_type()?;
            self.expect_op(")")?;
            self.expect_op("->")?;
            let (row, cod) = self.parse_cod()?;
            let end = cod.span;
            return Ok(self.join(
                start,
                end,
                Type::Arrow {
                    param: Opt(Some(binder)),
                    dom: Box::new(dom),
                    row,
                    cod: Box::new(cod),
                },
            ));
        }
        let dom = self.parse_app_type()?;
        if self.is_op(0, "->") {
            self.bump();
            let (row, cod) = self.parse_cod()?;
            let (start, end) = (dom.span, cod.span);
            Ok(self.join(
                start,
                end,
                Type::Arrow {
                    param: Opt(None),
                    dom: Box::new(dom),
                    row,
                    cod: Box::new(cod),
                },
            ))
        } else {
            Ok(dom)
        }
    }

    fn parse_cod(&mut self) -> Result<(Row, Spanned<Type>), Diagnostic> {
        let row = self.parse_row_opt()?;
        let ty = self.parse_type()?;
        Ok((row, ty))
    }

    fn parse_row_opt(&mut self) -> Result<Row, Diagnostic> {
        let mut effects: Vec<Effect> = Vec::new();
        while let Some(Token::TIdent { name }) = self.tok(0) {
            let Some(eff) = effect_of(name) else { break };
            if effects.contains(&eff) {
                return Err(self.err_expected("a distinct effect (duplicate in row)"));
            }
            effects.push(eff);
            self.bump();
        }
        effects.sort();
        let mut var = Opt(None);
        if let Some(Token::TIdent { name }) = self.tok(0) {
            // A row variable iff a type follows (spec §3.2).
            if effect_of(name).is_none()
                && !self.is_equation_start(0)
                && !self.is_signature_start(0)
                && self.starts_type_atom(1)
            {
                var = Opt(Some(name.clone()));
                self.bump();
            }
        }
        Ok(Row { effects, var })
    }

    fn parse_app_type(&mut self) -> Result<Spanned<Type>, Diagnostic> {
        if let Some(Token::TTypeName { name }) = self.tok(0) {
            let name = name.clone();
            let start = self.bump();
            let mut args = Vec::new();
            let mut end = start;
            while self.starts_type_atom(0) {
                let arg = self.parse_atype()?;
                end = arg.span;
                args.push(arg);
            }
            Ok(self.join(start, end, Type::Con { name, args }))
        } else {
            self.parse_atype()
        }
    }

    fn parse_atype(&mut self) -> Result<Spanned<Type>, Diagnostic> {
        match self.tok(0) {
            Some(Token::TTypeName { name }) => {
                let name = name.clone();
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: Type::Con {
                        name,
                        args: Vec::new(),
                    },
                })
            }
            Some(Token::TIdent { name }) => {
                if effect_of(name).is_some() {
                    return Err(
                        self.err_expected("a type (effect names are reserved in type position)")
                    );
                }
                if self.is_equation_start(0) || self.is_signature_start(0) {
                    return Err(self.err_expected("a type"));
                }
                let name = name.clone();
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: Type::TVar { name },
                })
            }
            Some(Token::TOp { op }) if op == "(" => {
                self.bump();
                let ty = self.parse_type()?;
                self.expect_op(")")?;
                Ok(ty)
            }
            Some(Token::TOp { op }) if op == "{" => {
                let start = self.bump();
                let binder = self.ident_binder()?;
                self.expect_op(":")?;
                let base = self.parse_type()?;
                self.expect_op("|")?;
                let pred = self.parse_pred()?;
                let end = self.expect_op("}")?;
                Ok(self.join(
                    start,
                    end,
                    Type::Refined {
                        binder,
                        base: Box::new(base),
                        pred,
                    },
                ))
            }
            _ => Err(self.err_expected("a type")),
        }
    }

    // ---- predicates (tr-grammar §2.3) ----

    fn parse_pred(&mut self) -> Result<Spanned<Pred>, Diagnostic> {
        let lhs = self.parse_pred_disj()?;
        if self.is_pred_word(0, "implies") {
            self.bump();
            let rhs = self.parse_pred()?; // right-assoc
            let (a, b) = (lhs.span, rhs.span);
            return Ok(self.join(
                a,
                b,
                Pred::Implies {
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            ));
        }
        Ok(lhs)
    }

    fn parse_pred_disj(&mut self) -> Result<Spanned<Pred>, Diagnostic> {
        let mut lhs = self.parse_pred_conj()?;
        while self.is_kw(0, "or") {
            self.bump();
            let rhs = self.parse_pred_conj()?;
            let (a, b) = (lhs.span, rhs.span);
            lhs = self.join(
                a,
                b,
                Pred::POr {
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            );
        }
        Ok(lhs)
    }

    fn parse_pred_conj(&mut self) -> Result<Spanned<Pred>, Diagnostic> {
        let mut lhs = self.parse_pred_neg()?;
        while self.is_kw(0, "and") {
            self.bump();
            let rhs = self.parse_pred_neg()?;
            let (a, b) = (lhs.span, rhs.span);
            lhs = self.join(
                a,
                b,
                Pred::PAnd {
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            );
        }
        Ok(lhs)
    }

    fn parse_pred_neg(&mut self) -> Result<Spanned<Pred>, Diagnostic> {
        if self.is_kw(0, "not") {
            let start = self.bump();
            let pred = self.parse_pred_neg()?;
            let end = pred.span;
            return Ok(self.join(
                start,
                end,
                Pred::PNot {
                    pred: Box::new(pred),
                },
            ));
        }
        self.parse_pred_atom()
    }

    fn parse_pred_atom(&mut self) -> Result<Spanned<Pred>, Diagnostic> {
        let save = self.pos;
        if let Ok(p) = self.try_pred_expr_atom() {
            return Ok(p);
        }
        self.pos = save;
        if self.is_op(0, "(") {
            self.bump();
            let p = self.parse_pred()?;
            self.expect_op(")")?;
            return Ok(p);
        }
        Err(self.err_expected("a predicate"))
    }

    /// The comparison / is-test / boolean-call route; the caller restores
    /// on failure and retries as a parenthesized predicate.
    fn try_pred_expr_atom(&mut self) -> Result<Spanned<Pred>, Diagnostic> {
        let lhs = self.parse_pexpr()?;
        if let Some(op) = self.peek_cmp_op() {
            self.bump();
            let rhs = self.parse_pexpr()?;
            let (a, b) = (lhs.span, rhs.span);
            return Ok(self.join(a, b, Pred::PCmp { op, lhs, rhs }));
        }
        if self.is_pred_word(0, "is") {
            self.bump();
            let mut type_name = Opt(None);
            if matches!(self.tok(0), Some(Token::TTypeName { .. })) && self.is_op(1, "::") {
                if let Some(Token::TTypeName { name }) = self.tok(0) {
                    type_name = Opt(Some(name.clone()));
                }
                self.bump();
                self.bump();
            }
            let (ctor, mut end) = match self.tok(0) {
                Some(Token::TTypeName { name }) => {
                    let n = name.clone();
                    (n, self.bump())
                }
                _ => return Err(self.err_expected("a constructor")),
            };
            let mut payload = Opt(None);
            if self.is_op(0, "(") {
                self.bump();
                let binder = self.ident_binder()?;
                end = self.expect_op(")")?;
                payload = Opt(Some(binder));
            }
            let a = lhs.span;
            return Ok(self.join(
                a,
                end,
                Pred::Is {
                    scrutinee: lhs,
                    type_name,
                    ctor,
                    payload,
                },
            ));
        }
        // A bare call is a boolean atom; anything else is not a predicate.
        match lhs.item {
            PExpr::PCallE { name, args } => Ok(Spanned {
                span: lhs.span,
                item: Pred::PCall { name, args },
            }),
            _ => Err(self.err_expected("a comparison, `is` test, or call")),
        }
    }

    fn peek_cmp_op(&self) -> Option<CmpOp> {
        match self.tok(0) {
            Some(Token::TOp { op }) => match op.as_str() {
                "==" => Some(CmpOp::Eq),
                "!=" => Some(CmpOp::Ne),
                "<" => Some(CmpOp::Lt),
                "<=" => Some(CmpOp::Le),
                ">" => Some(CmpOp::Gt),
                ">=" => Some(CmpOp::Ge),
                _ => None,
            },
            _ => None,
        }
    }

    fn parse_pexpr(&mut self) -> Result<Spanned<PExpr>, Diagnostic> {
        let mut lhs = self.parse_pexpr_mul()?;
        loop {
            let op = if self.is_op(0, "+") {
                ArithOp::Add
            } else if self.is_op(0, "-") {
                ArithOp::Sub
            } else {
                break;
            };
            self.bump();
            let rhs = self.parse_pexpr_mul()?;
            let (a, b) = (lhs.span, rhs.span);
            lhs = self.join(
                a,
                b,
                PExpr::PArith {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            );
        }
        Ok(lhs)
    }

    fn parse_pexpr_mul(&mut self) -> Result<Spanned<PExpr>, Diagnostic> {
        let mut lhs = self.parse_pexpr_app()?;
        while self.is_op(0, "*") {
            self.bump();
            let rhs = self.parse_pexpr_app()?;
            let (a, b) = (lhs.span, rhs.span);
            lhs = self.join(
                a,
                b,
                PExpr::PArith {
                    op: ArithOp::Mul,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            );
        }
        Ok(lhs)
    }

    /// Predicate calls are juxtaposed exactly as in terms (tr-grammar
    /// §2.3, resolved 2026-08-22): `len v`, head a bare ident.
    fn parse_pexpr_app(&mut self) -> Result<Spanned<PExpr>, Diagnostic> {
        let head = self.parse_pexpr_atom()?;
        if !self.starts_pexpr_atom(0) {
            return Ok(head);
        }
        let name = match &head.item {
            PExpr::PPath { root, fields } if fields.is_empty() => root.clone(),
            _ => return Err(self.err_expected("an operator (only a bare name can head a call)")),
        };
        let mut args = Vec::new();
        let mut end = head.span;
        while self.starts_pexpr_atom(0) {
            let arg = self.parse_pexpr_atom()?;
            end = arg.span;
            args.push(arg);
        }
        let start = head.span;
        Ok(self.join(start, end, PExpr::PCallE { name, args }))
    }

    /// `is` and `implies` are reserved within predicates and never start
    /// an atom.
    fn starts_pexpr_atom(&self, ahead: usize) -> bool {
        match self.tok(ahead) {
            Some(Token::TInt { .. }) | Some(Token::TFloat { .. }) | Some(Token::TStr { .. }) => {
                true
            }
            Some(Token::TIdent { name }) => name != "is" && name != "implies",
            Some(Token::TOp { op }) => op == "(",
            _ => false,
        }
    }

    fn parse_pexpr_atom(&mut self) -> Result<Spanned<PExpr>, Diagnostic> {
        match self.tok(0) {
            Some(Token::TInt { digits }) => {
                let lit = Lit::LInt {
                    digits: digits.clone(),
                };
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: PExpr::PLiteral { lit },
                })
            }
            Some(Token::TFloat { text }) => {
                let lit = Lit::LFloat { text: text.clone() };
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: PExpr::PLiteral { lit },
                })
            }
            Some(Token::TStr { value }) => {
                let lit = Lit::LStr {
                    value: value.clone(),
                };
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: PExpr::PLiteral { lit },
                })
            }
            Some(Token::TIdent { name }) if name != "is" && name != "implies" => {
                let name = name.clone();
                let start = self.bump();
                let mut fields = Vec::new();
                let mut end = start;
                while self.is_op(0, ".") {
                    match self.tok(1) {
                        Some(Token::TIdent { name: f }) => {
                            fields.push(f.clone());
                            self.bump();
                            end = self.bump();
                        }
                        _ => return Err(self.err_expected("a field name")),
                    }
                }
                Ok(self.join(start, end, PExpr::PPath { root: name, fields }))
            }
            Some(Token::TOp { op }) if op == "(" => {
                self.bump();
                let e = self.parse_pexpr()?;
                self.expect_op(")")?;
                Ok(e)
            }
            _ => Err(self.err_expected("a predicate expression")),
        }
    }

    // ---- expressions ----

    fn parse_expr(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        match self.tok(0) {
            Some(Token::TKeyword { word }) => match word.as_str() {
                "let" => self.parse_let(),
                "fun" => self.parse_fun(),
                "if" => self.parse_if(),
                "match" => self.parse_match(),
                // `not` heads the boolean ladder; other keywords fall
                // through to the ladder's own error.
                _ => self.parse_or(),
            },
            _ => self.parse_or(),
        }
    }

    fn parse_or(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        let mut lhs = self.parse_and()?;
        while self.is_kw(0, "or") {
            self.bump();
            let rhs = self.parse_and()?;
            let (a, b) = (lhs.span, rhs.span);
            lhs = self.join(
                a,
                b,
                Expr::OrE {
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            );
        }
        Ok(lhs)
    }

    /// True if `and` at the current position separates `let` bindings:
    /// the two-token rule (spec §3.3), extended to `_` bindings, which
    /// are never expressions.
    fn and_starts_binding(&self) -> bool {
        let name_ok = matches!(
            self.tok(1),
            Some(Token::TIdent { .. }) | Some(Token::TPrivate { .. })
        ) || self.is_op(1, "_");
        name_ok && self.is_op(2, "=")
    }

    fn parse_and(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        let mut lhs = self.parse_not()?;
        while self.is_kw(0, "and") && !self.and_starts_binding() {
            self.bump();
            let rhs = self.parse_not()?;
            let (a, b) = (lhs.span, rhs.span);
            lhs = self.join(
                a,
                b,
                Expr::AndE {
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            );
        }
        Ok(lhs)
    }

    fn parse_not(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        if self.is_kw(0, "not") {
            let start = self.bump();
            let operand = self.parse_not()?;
            let end = operand.span;
            return Ok(self.join(
                start,
                end,
                Expr::NotE {
                    operand: Box::new(operand),
                },
            ));
        }
        self.parse_cmp()
    }

    fn parse_cmp(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        let lhs = self.parse_add()?;
        let Some(op) = self.peek_cmp_op() else {
            return Ok(lhs);
        };
        self.bump();
        let rhs = self.parse_add()?;
        if let Some(_second) = self.peek_cmp_op() {
            return Err(self.err_at(
                Code::NonassocComparison,
                "comparisons are non-associative: `a < b < c` must be written with `and`"
                    .to_string(),
                self.span_here(),
            ));
        }
        let (a, b) = (lhs.span, rhs.span);
        Ok(self.join(
            a,
            b,
            Expr::Cmp {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
        ))
    }

    fn parse_add(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = if self.is_op(0, "+") {
                ArithOp::Add
            } else if self.is_op(0, "-") {
                ArithOp::Sub
            } else {
                break;
            };
            self.bump();
            let rhs = self.parse_mul()?;
            let (a, b) = (lhs.span, rhs.span);
            lhs = self.join(
                a,
                b,
                Expr::Arith {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            );
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = if self.is_op(0, "*") {
                ArithOp::Mul
            } else if self.is_op(0, "/") {
                ArithOp::Div
            } else if self.is_op(0, "%") {
                ArithOp::Mod
            } else {
                break;
            };
            self.bump();
            let rhs = self.parse_unary()?;
            let (a, b) = (lhs.span, rhs.span);
            lhs = self.join(
                a,
                b,
                Expr::Arith {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            );
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        if self.is_op(0, "-") {
            let start = self.bump();
            let operand = self.parse_unary()?;
            let end = operand.span;
            return Ok(self.join(
                start,
                end,
                Expr::Neg {
                    operand: Box::new(operand),
                },
            ));
        }
        self.parse_app()
    }

    /// Does the current token start an application *argument*? Stops
    /// before the equation head and the next definition's signature.
    fn starts_expr_atom(&self) -> bool {
        match self.tok(0) {
            Some(Token::TInt { .. })
            | Some(Token::TFloat { .. })
            | Some(Token::TStr { .. })
            | Some(Token::THole { .. })
            | Some(Token::TTypeName { .. }) => true,
            Some(Token::TOp { op }) if op == "(" || op == "{" => true,
            Some(Token::TIdent { .. }) | Some(Token::TPrivate { .. }) => {
                !self.is_equation_start(0) && !self.is_signature_start(0)
            }
            _ => false,
        }
    }

    fn parse_app(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        let mut head = self.parse_atom()?;
        while self.starts_expr_atom() {
            let arg = self.parse_atom()?;
            let (a, b) = (head.span, arg.span);
            head = self.join(
                a,
                b,
                Expr::App {
                    r#fn: Box::new(head),
                    arg: Box::new(arg),
                },
            );
        }
        Ok(head)
    }

    fn parse_atom(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        match self.tok(0) {
            Some(Token::TInt { digits }) => {
                let lit = Lit::LInt {
                    digits: digits.clone(),
                };
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: Expr::Literal { lit },
                })
            }
            Some(Token::TFloat { text }) => {
                let lit = Lit::LFloat { text: text.clone() };
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: Expr::Literal { lit },
                })
            }
            Some(Token::TStr { value }) => {
                let lit = Lit::LStr {
                    value: value.clone(),
                };
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: Expr::Literal { lit },
                })
            }
            Some(Token::TIdent { name }) => {
                let root = name.clone();
                let start = self.bump();
                if self.is_op(0, "::") {
                    self.bump();
                    match self.tok(0) {
                        Some(Token::TIdent { name }) => {
                            let name = name.clone();
                            let end = self.bump();
                            Ok(self.join(start, end, Expr::Qualified { space: root, name }))
                        }
                        _ => Err(self.err_expected("a definition name after `::`")),
                    }
                } else {
                    self.parse_path_fields(root, start)
                }
            }
            Some(Token::TPrivate { name }) => {
                let root = name.clone();
                let start = self.bump();
                self.parse_path_fields(root, start)
            }
            Some(Token::TTypeName { name }) => {
                let space = name.clone();
                let start = self.bump();
                if self.is_op(0, "::") {
                    self.bump();
                    match self.tok(0) {
                        Some(Token::TIdent { name }) | Some(Token::TTypeName { name }) => {
                            let name = name.clone();
                            let end = self.bump();
                            Ok(self.join(start, end, Expr::Qualified { space, name }))
                        }
                        _ => Err(self.err_expected("a name after `::`")),
                    }
                } else {
                    Ok(Spanned {
                        span: start,
                        item: Expr::CtorE { name: space },
                    })
                }
            }
            Some(Token::TOp { op }) if op == "(" => {
                let start = self.bump();
                let e = self.parse_expr()?;
                if self.is_op(0, ":") {
                    self.bump();
                    let ty = self.parse_type()?;
                    let end = self.expect_op(")")?;
                    return Ok(self.join(
                        start,
                        end,
                        Expr::Annot {
                            expr: Box::new(e),
                            ty,
                        },
                    ));
                }
                self.expect_op(")")?;
                Ok(e)
            }
            Some(Token::TOp { op }) if op == "{" => self.parse_record(),
            Some(Token::THole { name }) => {
                let name = name.clone();
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: Expr::Hole { name },
                })
            }
            _ => Err(self.err_expected("an expression")),
        }
    }

    fn parse_path_fields(
        &mut self,
        root: String,
        start: Span,
    ) -> Result<Spanned<Expr>, Diagnostic> {
        let mut fields = Vec::new();
        let mut end = start;
        while self.is_op(0, ".") {
            match self.tok(1) {
                Some(Token::TIdent { name }) => {
                    fields.push(name.clone());
                    self.bump();
                    end = self.bump();
                }
                _ => return Err(self.err_expected("a field name")),
            }
        }
        Ok(self.join(start, end, Expr::Path { root, fields }))
    }

    fn parse_record(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        let start = self.bump(); // {
                                 // Update base: a path followed by `with`.
        let save = self.pos;
        let update = 'update: {
            if let Some(Token::TIdent { name }) = self.tok(0) {
                let root = name.clone();
                self.bump();
                let mut fields = Vec::new();
                while self.is_op(0, ".") {
                    match self.tok(1) {
                        Some(Token::TIdent { name }) => {
                            fields.push(name.clone());
                            self.bump();
                            self.bump();
                        }
                        _ => break,
                    }
                }
                if self.is_kw(0, "with") {
                    self.bump();
                    break 'update Opt(Some(PathBase { root, fields }));
                }
            }
            self.pos = save;
            Opt(None)
        };
        let mut fields: Vec<FieldInit> = Vec::new();
        loop {
            let name = match self.tok(0) {
                Some(Token::TIdent { name }) => name.clone(),
                _ => return Err(self.err_expected("a field name")),
            };
            if fields.iter().any(|f| f.name == name) {
                return Err(self.err_at(
                    Code::DuplicateField,
                    format!("field `{name}` appears twice"),
                    self.span_here(),
                ));
            }
            self.bump();
            self.expect_op("=")?;
            let value = self.parse_expr()?;
            fields.push(FieldInit { name, value });
            if self.is_op(0, ",") {
                self.bump();
            } else {
                break;
            }
        }
        let end = self.expect_op("}")?;
        Ok(self.join(start, end, Expr::RecordE { update, fields }))
    }

    fn parse_let(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        let start = self.bump(); // let
        let is_rec = if self.is_kw(0, "rec") {
            self.bump();
            true
        } else {
            false
        };
        let mut bindings = Vec::new();
        loop {
            let binder = if self.is_op(0, "_") {
                if is_rec {
                    return Err(self.err_at(
                        Code::WildcardInLetRec,
                        "`_` cannot be bound in `let rec`".to_string(),
                        self.span_here(),
                    ));
                }
                self.bump();
                Opt(None)
            } else {
                let (name, span) = self.defname_token()?;
                Opt(Some(Binder { span, name }))
            };
            self.expect_op("=")?;
            let value = self.parse_expr()?;
            bindings.push(LetBinding { binder, value });
            if self.is_kw(0, "and") && self.and_starts_binding() {
                self.bump();
            } else {
                break;
            }
        }
        self.expect_kw("in")?;
        let body = self.parse_expr()?;
        let end = body.span;
        Ok(self.join(
            start,
            end,
            Expr::Let {
                is_rec,
                bindings,
                body: Box::new(body),
            },
        ))
    }

    fn parse_fun(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        let start = self.bump(); // fun
        let mut params = vec![self.ident_binder()?];
        while let Some(Token::TIdent { .. }) = self.tok(0) {
            params.push(self.ident_binder()?);
        }
        self.expect_op("->")?;
        let body = self.parse_expr()?;
        let end = body.span;
        Ok(self.join(
            start,
            end,
            Expr::Fun {
                params,
                body: Box::new(body),
            },
        ))
    }

    fn parse_if(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        let start = self.bump(); // if
        let cond = self.parse_expr()?;
        self.expect_kw("then")?;
        let then_branch = self.parse_expr()?;
        self.expect_kw("else")?;
        let else_branch = self.parse_expr()?;
        let end = else_branch.span;
        Ok(self.join(
            start,
            end,
            Expr::If {
                cond: Box::new(cond),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            },
        ))
    }

    fn parse_match(&mut self) -> Result<Spanned<Expr>, Diagnostic> {
        let start = self.bump(); // match
        let scrutinee = self.parse_expr()?;
        self.expect_kw("with")?;
        self.expect_op("|")?;
        let mut arms = Vec::new();
        loop {
            let pattern = self.parse_pattern()?;
            self.expect_op("->")?;
            let body = self.parse_expr()?;
            let (a, b) = (pattern.span, body.span);
            arms.push(self.join(a, b, Arm { pattern, body }));
            if self.is_op(0, "|") {
                self.bump();
            } else {
                break;
            }
        }
        let end = arms.last().expect("at least one arm").span;
        Ok(self.join(
            start,
            end,
            Expr::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
        ))
    }

    // ---- patterns ----

    fn parse_pattern(&mut self) -> Result<Spanned<Pattern>, Diagnostic> {
        match self.tok(0) {
            Some(Token::TTypeName { .. }) => {
                let (type_name, name, start, mut end) = self.parse_ctor_name()?;
                let arg = if self.starts_pat_atom() {
                    let a = self.parse_pat_atom()?;
                    end = a.span;
                    Opt(Some(Box::new(a)))
                } else {
                    Opt(None)
                };
                Ok(self.join(
                    start,
                    end,
                    Pattern::PCtor {
                        type_name,
                        name,
                        arg,
                    },
                ))
            }
            _ => self.parse_pat_atom_inner(true),
        }
    }

    /// `ctor-name ::= [ TypeName "::" ] TypeName` with spans.
    fn parse_ctor_name(&mut self) -> Result<(Opt<String>, String, Span, Span), Diagnostic> {
        let first = match self.tok(0) {
            Some(Token::TTypeName { name }) => name.clone(),
            _ => return Err(self.err_expected("a constructor")),
        };
        let start = self.bump();
        if self.is_op(0, "::") {
            self.bump();
            match self.tok(0) {
                Some(Token::TTypeName { name }) => {
                    let name = name.clone();
                    let end = self.bump();
                    Ok((Opt(Some(first)), name, start, end))
                }
                _ => Err(self.err_expected("a constructor after `::`")),
            }
        } else {
            Ok((Opt(None), first, start, start))
        }
    }

    fn starts_pat_atom(&self) -> bool {
        matches!(
            self.tok(0),
            Some(Token::TIdent { .. })
                | Some(Token::TTypeName { .. })
                | Some(Token::TInt { .. })
                | Some(Token::TFloat { .. })
                | Some(Token::TStr { .. })
        ) || self.is_op(0, "_")
            || self.is_op(0, "{")
            || self.is_op(0, "(")
    }

    fn parse_pat_atom(&mut self) -> Result<Spanned<Pattern>, Diagnostic> {
        self.parse_pat_atom_inner(false)
    }

    /// `allow_arg` distinguishes full `pattern` position (a constructor
    /// may take one atom argument) from `pat-atom` position (it may not).
    fn parse_pat_atom_inner(&mut self, allow_arg: bool) -> Result<Spanned<Pattern>, Diagnostic> {
        match self.tok(0) {
            Some(Token::TOp { op }) if op == "_" => {
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: Pattern::PWild,
                })
            }
            Some(Token::TIdent { name }) => {
                let name = name.clone();
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: Pattern::PBind {
                        binder: Binder { span, name },
                    },
                })
            }
            Some(Token::TInt { digits }) => {
                let lit = Lit::LInt {
                    digits: digits.clone(),
                };
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: Pattern::PLit { lit },
                })
            }
            Some(Token::TFloat { text }) => {
                let lit = Lit::LFloat { text: text.clone() };
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: Pattern::PLit { lit },
                })
            }
            Some(Token::TStr { value }) => {
                let lit = Lit::LStr {
                    value: value.clone(),
                };
                let span = self.bump();
                Ok(Spanned {
                    span,
                    item: Pattern::PLit { lit },
                })
            }
            Some(Token::TTypeName { .. }) => {
                let (type_name, name, start, mut end) = self.parse_ctor_name()?;
                let arg = if allow_arg && self.starts_pat_atom() {
                    let a = self.parse_pat_atom()?;
                    end = a.span;
                    Opt(Some(Box::new(a)))
                } else {
                    Opt(None)
                };
                Ok(self.join(
                    start,
                    end,
                    Pattern::PCtor {
                        type_name,
                        name,
                        arg,
                    },
                ))
            }
            Some(Token::TOp { op }) if op == "{" => self.parse_record_pattern(),
            Some(Token::TOp { op }) if op == "(" => {
                self.bump();
                let p = self.parse_pattern()?;
                self.expect_op(")")?;
                Ok(p)
            }
            _ => Err(self.err_expected("a pattern")),
        }
    }

    fn parse_record_pattern(&mut self) -> Result<Spanned<Pattern>, Diagnostic> {
        let start = self.bump(); // {
        let mut fields: Vec<FieldPat> = Vec::new();
        let mut open = false;
        loop {
            let name = match self.tok(0) {
                Some(Token::TIdent { name }) => name.clone(),
                _ => return Err(self.err_expected("a field name")),
            };
            if fields.iter().any(|f| f.name == name) {
                return Err(self.err_at(
                    Code::DuplicateField,
                    format!("field `{name}` appears twice"),
                    self.span_here(),
                ));
            }
            self.bump();
            let pattern = if self.is_op(0, "=") {
                self.bump();
                Opt(Some(self.parse_pattern()?))
            } else {
                Opt(None)
            };
            fields.push(FieldPat { name, pattern });
            if self.is_op(0, ",") {
                self.bump();
                if self.is_op(0, "..") {
                    self.bump();
                    open = true;
                    break;
                }
            } else {
                break;
            }
        }
        let end = self.expect_op("}")?;
        Ok(self.join(start, end, Pattern::PRecord { fields, open }))
    }
}
