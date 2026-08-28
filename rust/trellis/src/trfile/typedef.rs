//! `soil-type` blocks (tr-grammar §4.1): the typedef skeleton is
//! parsed here over soil0's lexer, with the embedded types and
//! `ignored` defaults parsed through soil0's own parsers by span
//! slicing — one grammar implementation for anything type- or
//! expression-shaped.
//!
//! One disambiguation the grammar leaves open: `type Path = Utf8`
//! parses as both a one-variant sum and an alias. Rule here (flagged
//! for tr-grammar): a body is a **sum** iff it starts with `|` or
//! contains a top-level `|`; a single-variant sum therefore must
//! write the leading `|`, and everything else is an alias.

use soil0::ast::PExpr;
use soil0::span::{Span, Spanned};
use soil0::token::Token;

use crate::diag::Diag;

#[derive(Debug)]
pub struct TypeDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: TypeBody,
}

#[derive(Debug)]
pub enum TypeBody {
    Opaque,
    Record(Vec<Field>),
    Sum(Vec<Ctor>),
    Alias(Spanned<soil0::ast::Type>),
}

#[derive(Debug)]
pub struct Field {
    pub name: String,
    pub ty: Spanned<soil0::ast::Type>,
    pub ignored: Option<Spanned<PExpr>>,
}

#[derive(Debug)]
pub struct Ctor {
    pub name: String,
    pub payload: Option<CtorPayload>,
}

#[derive(Debug)]
pub enum CtorPayload {
    Ty(Spanned<soil0::ast::Type>),
    Record(Vec<Field>),
}

struct Toks<'s> {
    src: &'s str,
    tokens: Vec<Spanned<Token>>,
    pos: usize,
}

pub fn parse(content: &str) -> Result<TypeDef, Diag> {
    let src = content.trim();
    let tokens = soil0::lexer::lex(src).map_err(|d| super::soil0_syntax("tr-type-syntax", &d))?;
    let mut t = Toks {
        src,
        tokens,
        pos: 0,
    };

    t.expect_ident("type")?;
    let name = t.expect_type_name()?;
    let mut params = Vec::new();
    while let Some(Token::TIdent { name }) = t.peek(0) {
        params.push(name.clone());
        t.pos += 1;
    }
    t.expect_op("=")?;

    let body = match t.peek(0) {
        None => return Err(err("a typedef needs a body after `=`")),
        Some(Token::TIdent { name }) if name == "opaque" => {
            t.pos += 1;
            TypeBody::Opaque
        }
        Some(Token::TOp { op }) if op == "{" => TypeBody::Record(t.parse_record()?),
        Some(Token::TOp { op }) if op == "|" => TypeBody::Sum(t.parse_sum()?),
        Some(Token::TTypeName { .. }) if t.has_top_level_bar() => TypeBody::Sum(t.parse_sum()?),
        Some(_) => TypeBody::Alias(t.parse_type_to_end()?),
    };

    if t.peek(0).is_some() {
        return Err(err("unexpected tokens after the typedef body"));
    }
    Ok(TypeDef { name, params, body })
}

impl Toks<'_> {
    fn peek(&self, ahead: usize) -> Option<&Token> {
        self.tokens.get(self.pos + ahead).map(|t| &t.item)
    }

    fn span(&self, at: usize) -> Option<Span> {
        self.tokens.get(at).map(|t| t.span)
    }

    fn expect_ident(&mut self, word: &str) -> Result<(), Diag> {
        match self.peek(0) {
            Some(Token::TIdent { name }) if name == word => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(err(format!("expected `{word}` (tr-grammar §4.1)"))),
        }
    }

    fn expect_type_name(&mut self) -> Result<String, Diag> {
        match self.peek(0) {
            Some(Token::TTypeName { name }) => {
                let name = name.clone();
                self.pos += 1;
                Ok(name)
            }
            _ => Err(err("expected a PascalCase type name")),
        }
    }

    fn expect_op(&mut self, op: &str) -> Result<(), Diag> {
        match self.peek(0) {
            Some(Token::TOp { op: o }) if o == op => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(err(format!("expected `{op}`"))),
        }
    }

    /// Whether a `|` occurs at bracket depth 0 in the remaining tokens.
    fn has_top_level_bar(&self) -> bool {
        let mut depth = 0usize;
        for t in &self.tokens[self.pos..] {
            match &t.item {
                Token::TOp { op } if matches!(op.as_str(), "(" | "{") => depth += 1,
                Token::TOp { op } if matches!(op.as_str(), ")" | "}") => {
                    depth = depth.saturating_sub(1)
                }
                Token::TOp { op } if op == "|" && depth == 0 => return true,
                _ => {}
            }
        }
        false
    }

    /// Index of the next token matching `stop` at bracket depth 0, or
    /// the end of the tokens. The stop check runs before the depth
    /// bookkeeping so a closing bracket can itself be a stop token
    /// (a record field ends at the record's own `}`).
    fn find_top_level(&self, from: usize, stop: impl Fn(&Token) -> bool) -> usize {
        let mut depth = 0usize;
        for (i, t) in self.tokens.iter().enumerate().skip(from) {
            if depth == 0 && stop(&t.item) {
                return i;
            }
            match &t.item {
                Token::TOp { op } if matches!(op.as_str(), "(" | "{") => depth += 1,
                Token::TOp { op } if matches!(op.as_str(), ")" | "}") => {
                    depth = depth.saturating_sub(1)
                }
                _ => {}
            }
        }
        self.tokens.len()
    }

    /// Slice the source between token `from` (inclusive) and token
    /// `to` (exclusive) and parse it as a type.
    fn parse_type_range(&self, from: usize, to: usize) -> Result<Spanned<soil0::ast::Type>, Diag> {
        if from >= to {
            return Err(err("expected a type"));
        }
        let start = self.span(from).expect("in range").start;
        let end = self.span(to - 1).expect("in range").end;
        soil0::parser::parse_type_str(&self.src[start..end])
            .map_err(|d| super::soil0_syntax("tr-type-syntax", &d))
    }

    fn parse_type_to_end(&mut self) -> Result<Spanned<soil0::ast::Type>, Diag> {
        let ty = self.parse_type_range(self.pos, self.tokens.len())?;
        self.pos = self.tokens.len();
        Ok(ty)
    }

    /// `{ field, … }` with the cursor on the `{`.
    fn parse_record(&mut self) -> Result<Vec<Field>, Diag> {
        self.expect_op("{")?;
        let mut fields = Vec::new();
        loop {
            fields.push(self.parse_field()?);
            match self.peek(0) {
                Some(Token::TOp { op }) if op == "," => self.pos += 1,
                Some(Token::TOp { op }) if op == "}" => {
                    self.pos += 1;
                    return Ok(fields);
                }
                _ => return Err(err("expected `,` or `}` in a record body")),
            }
        }
    }

    /// `ident : type [ignored = expr]`, ending before a depth-0 `,`
    /// or `}` (a field's type may itself contain braces — refinements
    /// — so the boundary is depth-relative).
    fn parse_field(&mut self) -> Result<Field, Diag> {
        let name = match self.peek(0) {
            Some(Token::TIdent { name }) => {
                let name = name.clone();
                self.pos += 1;
                name
            }
            _ => return Err(err("expected a field name")),
        };
        self.expect_op(":")?;
        let end = self.find_top_level(
            self.pos,
            |t| matches!(t, Token::TOp { op } if op == "," || op == "}"),
        );
        // An `ignored` marker splits the field's tokens.
        let ignored_at = self.find_top_level(
            self.pos,
            |t| matches!(t, Token::TIdent { name } if name == "ignored"),
        );
        let (ty_end, ignored) = if ignored_at < end {
            (ignored_at, Some(ignored_at))
        } else {
            (end, None)
        };
        let ty = self.parse_type_range(self.pos, ty_end)?;
        self.pos = ty_end;
        let ignored = match ignored {
            None => None,
            Some(at) => {
                self.pos = at + 1;
                self.expect_op("=")?;
                if self.pos >= end {
                    return Err(err("an `ignored` field needs a default expression"));
                }
                let start = self.span(self.pos).expect("in range").start;
                let stop = self.span(end - 1).expect("in range").end;
                let expr = soil0::parser::parse_pexpr_str(&self.src[start..stop])
                    .map_err(|d| super::soil0_syntax("tr-type-syntax", &d))?;
                self.pos = end;
                Some(expr)
            }
        };
        Ok(Field { name, ty, ignored })
    }

    /// `["|"] ctor { "|" ctor }` with variants separated by depth-0
    /// `|`; a payload is a type, or an inline record (design §3.13:
    /// no positional products).
    fn parse_sum(&mut self) -> Result<Vec<Ctor>, Diag> {
        if matches!(self.peek(0), Some(Token::TOp { op }) if op == "|") {
            self.pos += 1;
        }
        let mut ctors = Vec::new();
        loop {
            let name = self.expect_type_name()?;
            let end =
                self.find_top_level(self.pos, |t| matches!(t, Token::TOp { op } if op == "|"));
            let payload = if self.pos == end {
                None
            } else if matches!(self.peek(0), Some(Token::TOp { op }) if op == "{") {
                let fields = self.parse_record()?;
                if self.pos != end {
                    return Err(err("unexpected tokens after a variant's record payload"));
                }
                Some(CtorPayload::Record(fields))
            } else {
                let ty = self.parse_type_range(self.pos, end)?;
                self.pos = end;
                Some(CtorPayload::Ty(ty))
            };
            ctors.push(Ctor { name, payload });
            if self.pos >= self.tokens.len() {
                return Ok(ctors);
            }
            self.expect_op("|")?;
        }
    }
}

fn err(message: impl Into<String>) -> Diag {
    Diag::new("tr-type-syntax", message)
}
