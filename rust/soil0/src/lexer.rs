//! Hand-written lexer implementing soil-syntax-spec §1: identifier
//! classes, keywords, maximal-munch operators, `--` comments, integer and
//! float literals (underscores stripped), and the fixed string escape set
//! (`\"` `\\` `\n` `\r` `\t` `\u{1–6 hex}`, Unicode scalar values only).

use crate::diag::{Code, Diagnostic, Opt};
use crate::span::{LineMap, Spanned};
use crate::token::{Token, KEYWORDS};

/// Multi-character operators, longest first (maximal munch); then the
/// single-character set. `--` (comment) is handled before either.
const OPS2: &[&str] = &["->", "::", "..", "==", "!=", "<=", ">="];
const OPS1: &[u8] = b"=|:.,(){}<>+-*/%_";

pub fn lex(src: &str) -> Result<Vec<Spanned<Token>>, Diagnostic> {
    Lexer {
        src: src.as_bytes(),
        map: LineMap::new(src),
        pos: 0,
    }
    .run()
}

struct Lexer<'a> {
    src: &'a [u8],
    map: LineMap,
    pos: usize,
}

impl Lexer<'_> {
    fn run(mut self) -> Result<Vec<Spanned<Token>>, Diagnostic> {
        let mut tokens = Vec::new();
        while let Some(b) = self.peek(0) {
            match b {
                b' ' | b'\t' | b'\r' | b'\n' => self.pos += 1,
                b'-' if self.peek(1) == Some(b'-') => {
                    while !matches!(self.peek(0), None | Some(b'\n')) {
                        self.pos += 1;
                    }
                }
                b'a'..=b'z' => tokens.push(self.ident()),
                b'A'..=b'Z' => tokens.push(self.type_name()),
                b'_' if matches!(self.peek(1), Some(b'a'..=b'z')) => tokens.push(self.private()),
                b'0'..=b'9' => tokens.push(self.number()),
                b'?' if matches!(self.peek(1), Some(b'a'..=b'z')) => tokens.push(self.hole()),
                b'"' => tokens.push(self.string()?),
                _ => tokens.push(self.operator()?),
            }
        }
        Ok(tokens)
    }

    fn peek(&self, ahead: usize) -> Option<u8> {
        self.src.get(self.pos + ahead).copied()
    }

    fn spanned(&self, start: usize, item: Token) -> Spanned<Token> {
        Spanned {
            span: self.map.span(start, self.pos),
            item,
        }
    }

    fn error(&self, code: Code, message: String, start: usize) -> Diagnostic {
        Diagnostic {
            code,
            message,
            file: Opt(None),
            span: Opt(Some(self.map.span(start, self.pos))),
            notes: Vec::new(),
        }
    }

    fn take_while(&mut self, pred: impl Fn(u8) -> bool) {
        while matches!(self.peek(0), Some(b) if pred(b)) {
            self.pos += 1;
        }
    }

    fn text(&self, start: usize) -> &str {
        // Safe: every consumer starts at an ASCII boundary and consumes ASCII.
        std::str::from_utf8(&self.src[start..self.pos]).expect("ascii token text")
    }

    fn ident(&mut self) -> Spanned<Token> {
        let start = self.pos;
        self.take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        let name = self.text(start).to_string();
        let item = if KEYWORDS.contains(&name.as_str()) {
            Token::TKeyword { word: name }
        } else {
            Token::TIdent { name }
        };
        self.spanned(start, item)
    }

    fn type_name(&mut self) -> Spanned<Token> {
        let start = self.pos;
        self.take_while(|b| b.is_ascii_alphanumeric());
        let name = self.text(start).to_string();
        self.spanned(start, Token::TTypeName { name })
    }

    fn private(&mut self) -> Spanned<Token> {
        let start = self.pos;
        self.pos += 1; // the underscore
        self.take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        let name = self.text(start).to_string();
        self.spanned(start, Token::TPrivate { name })
    }

    fn hole(&mut self) -> Spanned<Token> {
        let start = self.pos;
        self.pos += 1; // the `?`
        self.take_while(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        let name = self.text(start + 1).to_string();
        self.spanned(start, Token::THole { name })
    }

    fn digits_or_underscores(&mut self) {
        self.take_while(|b| b.is_ascii_digit() || b == b'_');
    }

    fn number(&mut self) -> Spanned<Token> {
        let start = self.pos;
        self.digits_or_underscores();
        // A fraction only if `.` is followed by a digit, so `1..2` lexes as
        // int, `..`, int.
        let mut is_float = false;
        if self.peek(0) == Some(b'.') && matches!(self.peek(1), Some(b'0'..=b'9')) {
            is_float = true;
            self.pos += 1;
            self.digits_or_underscores();
            // An exponent only if `e` is followed by digits (or `-` digits),
            // so `1.5e` lexes as float `1.5` then ident `e`.
            match (self.peek(0), self.peek(1), self.peek(2)) {
                (Some(b'e'), Some(b'0'..=b'9'), _) => {
                    self.pos += 1;
                    self.digits_or_underscores();
                }
                (Some(b'e'), Some(b'-'), Some(b'0'..=b'9')) => {
                    self.pos += 2;
                    self.digits_or_underscores();
                }
                _ => {}
            }
        }
        let text: String = self.text(start).chars().filter(|c| *c != '_').collect();
        let item = if is_float {
            Token::TFloat { text }
        } else {
            Token::TInt { digits: text }
        };
        self.spanned(start, item)
    }

    fn string(&mut self) -> Result<Spanned<Token>, Diagnostic> {
        let start = self.pos;
        self.pos += 1; // opening quote
        let mut value = String::new();
        loop {
            match self.peek(0) {
                None => {
                    return Err(self.error(
                        Code::UnterminatedString,
                        "string literal is not terminated".to_string(),
                        start,
                    ));
                }
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(self.spanned(start, Token::TStr { value }));
                }
                Some(b'\\') => {
                    let esc_start = self.pos;
                    self.pos += 1;
                    match self.peek(0) {
                        Some(b'"') => {
                            value.push('"');
                            self.pos += 1;
                        }
                        Some(b'\\') => {
                            value.push('\\');
                            self.pos += 1;
                        }
                        Some(b'n') => {
                            value.push('\n');
                            self.pos += 1;
                        }
                        Some(b'r') => {
                            value.push('\r');
                            self.pos += 1;
                        }
                        Some(b't') => {
                            value.push('\t');
                            self.pos += 1;
                        }
                        Some(b'u') => {
                            self.pos += 1;
                            value.push(self.unicode_escape(esc_start)?);
                        }
                        None => {
                            return Err(self.error(
                                Code::UnterminatedString,
                                "string literal is not terminated".to_string(),
                                start,
                            ));
                        }
                        Some(other) => {
                            self.pos += 1;
                            return Err(self.error(
                                Code::BadEscape,
                                format!(
                                    "unknown escape `\\{}`; the escape set is \\\" \\\\ \\n \\r \\t \\u{{…}}",
                                    other as char
                                ),
                                esc_start,
                            ));
                        }
                    }
                }
                Some(_) => {
                    // Raw bytes (including multi-byte UTF-8) pass through; the
                    // source is already valid UTF-8.
                    let ch_start = self.pos;
                    self.pos += 1;
                    while matches!(self.peek(0), Some(b) if b & 0xC0 == 0x80) {
                        self.pos += 1;
                    }
                    value.push_str(
                        std::str::from_utf8(&self.src[ch_start..self.pos]).expect("valid utf-8"),
                    );
                }
            }
        }
    }

    fn unicode_escape(&mut self, esc_start: usize) -> Result<char, Diagnostic> {
        if self.peek(0) != Some(b'{') {
            self.take_while(|b| b != b'"' && b != b'\n');
            return Err(self.error(
                Code::BadEscape,
                "\\u must be followed by {1–6 hex digits}".to_string(),
                esc_start,
            ));
        }
        self.pos += 1;
        let digits_start = self.pos;
        self.take_while(|b| b.is_ascii_hexdigit());
        let n_digits = self.pos - digits_start;
        if !(1..=6).contains(&n_digits) || self.peek(0) != Some(b'}') {
            self.take_while(|b| b != b'"' && b != b'\n');
            return Err(self.error(
                Code::BadEscape,
                "\\u must be followed by {1–6 hex digits}".to_string(),
                esc_start,
            ));
        }
        let hex = self.text(digits_start).to_string();
        self.pos += 1; // closing brace
        let code = u32::from_str_radix(&hex, 16).expect("1–6 hex digits fit u32");
        match char::from_u32(code) {
            Some(c) => Ok(c),
            None => Err(self.error(
                Code::EscapeNotScalar,
                format!(
                    "U+{code:04X} is not a Unicode scalar value (surrogates and values above U+10FFFF cannot appear in Utf8)"
                ),
                esc_start,
            )),
        }
    }

    fn operator(&mut self) -> Result<Spanned<Token>, Diagnostic> {
        let start = self.pos;
        for op in OPS2 {
            if self.src[self.pos..].starts_with(op.as_bytes()) {
                self.pos += 2;
                return Ok(self.spanned(
                    start,
                    Token::TOp {
                        op: (*op).to_string(),
                    },
                ));
            }
        }
        let b = self.peek(0).expect("caller checked non-empty");
        if OPS1.contains(&b) {
            self.pos += 1;
            return Ok(self.spanned(
                start,
                Token::TOp {
                    op: (b as char).to_string(),
                },
            ));
        }
        // Advance over one full UTF-8 character for the error span.
        self.pos += 1;
        while matches!(self.peek(0), Some(c) if c & 0xC0 == 0x80) {
            self.pos += 1;
        }
        let ch = std::str::from_utf8(&self.src[start..self.pos]).expect("valid utf-8");
        Err(self.error(
            Code::StrayCharacter,
            format!("stray character `{ch}`"),
            start,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Token> {
        lex(src).unwrap().into_iter().map(|s| s.item).collect()
    }

    fn op(o: &str) -> Token {
        Token::TOp { op: o.into() }
    }
    fn ident(n: &str) -> Token {
        Token::TIdent { name: n.into() }
    }
    fn kw(w: &str) -> Token {
        Token::TKeyword { word: w.into() }
    }

    #[test]
    fn maximal_munch() {
        assert_eq!(
            kinds("->:: .. == != <= >= = : ."),
            vec![
                op("->"),
                op("::"),
                op(".."),
                op("=="),
                op("!="),
                op("<="),
                op(">="),
                op("="),
                op(":"),
                op(".")
            ]
        );
    }

    #[test]
    fn comments_and_idents() {
        assert_eq!(
            kinds("gcd a -- rest -> ignored\nb"),
            vec![ident("gcd"), ident("a"), ident("b")]
        );
    }

    #[test]
    fn keywords_and_effect_names() {
        assert_eq!(kinds("let io"), vec![kw("let"), ident("io")]);
    }

    #[test]
    fn private_and_wildcard() {
        assert_eq!(
            kinds("_ _go _9"),
            vec![
                op("_"),
                Token::TPrivate { name: "_go".into() },
                op("_"),
                Token::TInt { digits: "9".into() }
            ]
        );
    }

    #[test]
    fn numbers() {
        assert_eq!(
            kinds("1_000 1.5e3 1..2 1.5e-3 1.5e x.y"),
            vec![
                Token::TInt {
                    digits: "1000".into()
                },
                Token::TFloat {
                    text: "1.5e3".into()
                },
                Token::TInt { digits: "1".into() },
                op(".."),
                Token::TInt { digits: "2".into() },
                Token::TFloat {
                    text: "1.5e-3".into()
                },
                Token::TFloat { text: "1.5".into() },
                ident("e"),
                ident("x"),
                op("."),
                ident("y"),
            ]
        );
    }

    #[test]
    fn strings_decode() {
        assert_eq!(
            kinds(r#""a\n\t\"\\\u{1F600}é""#),
            vec![Token::TStr {
                value: "a\n\t\"\\😀é".into()
            }]
        );
    }

    #[test]
    fn string_rejections() {
        assert_eq!(lex(r#""ab"#).unwrap_err().code, Code::UnterminatedString);
        assert_eq!(lex(r#""a\x""#).unwrap_err().code, Code::BadEscape);
        assert_eq!(lex(r#""a\u{}""#).unwrap_err().code, Code::BadEscape);
        assert_eq!(lex(r#""a\A""#).unwrap_err().code, Code::BadEscape);
        assert_eq!(lex(r#""a\u{1234567}""#).unwrap_err().code, Code::BadEscape);
        assert_eq!(
            lex(r#""a\u{D800}""#).unwrap_err().code,
            Code::EscapeNotScalar
        );
        assert_eq!(
            lex(r#""a\u{110000}""#).unwrap_err().code,
            Code::EscapeNotScalar
        );
    }

    #[test]
    fn stray_characters() {
        assert_eq!(lex("a @ b").unwrap_err().code, Code::StrayCharacter);
        assert_eq!(lex("a ! b").unwrap_err().code, Code::StrayCharacter);
        assert_eq!(lex("é").unwrap_err().code, Code::StrayCharacter);
    }

    #[test]
    fn spans_are_byte_offsets() {
        let tokens = lex("ab\n cd").unwrap();
        assert_eq!(
            tokens[0].span,
            crate::span::Span {
                start: 0,
                end: 2,
                line: 1,
                col: 1
            }
        );
        assert_eq!(
            tokens[1].span,
            crate::span::Span {
                start: 4,
                end: 6,
                line: 2,
                col: 2
            }
        );
    }
}
