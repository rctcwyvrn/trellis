//! Diagnostics, contract §2: `{"errors":[{code, message, file, span,
//! notes}]}` on stderr, compact single line. Codes are the stable
//! registry; renaming one is a contract change.

use crate::span::Span;
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use std::fmt;

/// The Soil `Option` encoding (tr-grammar §7): `{"tag":"None"}` /
/// `{"tag":"Some","value":…}`. Serde's `Option` (null / bare value) is
/// deliberately not used anywhere a contract schema says `Option`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opt<T>(pub Option<T>);

impl<T: Serialize> Serialize for Opt<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match &self.0 {
            None => {
                let mut m = s.serialize_map(Some(1))?;
                m.serialize_entry("tag", "None")?;
                m.end()
            }
            Some(v) => {
                let mut m = s.serialize_map(Some(2))?;
                m.serialize_entry("tag", "Some")?;
                m.serialize_entry("value", v)?;
                m.end()
            }
        }
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for Opt<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(tag = "tag", content = "value", deny_unknown_fields)]
        enum Repr<T> {
            None,
            Some(T),
        }
        Ok(match Repr::<T>::deserialize(d)? {
            Repr::None => Opt(None),
            Repr::Some(v) => Opt(Some(v)),
        })
    }
}

/// The error-code registry (contract §2). `class()` distinguishes
/// spec-violation diagnostics (exit 1) from usage/environment errors
/// (exit 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    // lex
    UnterminatedString,
    BadEscape,
    EscapeNotScalar,
    StrayCharacter,
    // parse
    ParseExpected,
    NonassocComparison,
    NameMismatch,
    WildcardInLetRec,
    MisplacedPrivateDef,
    MultipleDefs,
    // rename
    Shadowing,
    UnknownName,
    UnknownType,
    UnknownQualified,
    UnknownConstructor,
    AmbiguousConstructor,
    AmbiguousName,
    NeedlessQualification,
    PrivateCrossModule,
    // infer
    TypeMismatch,
    EffectViolation,
    OperatorPolymorphic,
    NonDerivable,
    AnnotationNeeded,
    UnknownField,
    LiteralOutOfRange,
    // match
    NonExhaustiveMatch,
    RedundantArm,
    MissingRecordRest,
    DuplicateField,
    // runtime
    RuntimePanic,
    UnfilledHole,
    // class 2
    Usage,
    Io,
    MalformedInput,
    ForwardReference,
    Internal,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        use Code::*;
        match self {
            UnterminatedString => "unterminated-string",
            BadEscape => "bad-escape",
            EscapeNotScalar => "escape-not-scalar",
            StrayCharacter => "stray-character",
            ParseExpected => "parse-expected",
            NonassocComparison => "nonassoc-comparison",
            NameMismatch => "name-mismatch",
            WildcardInLetRec => "wildcard-in-let-rec",
            MisplacedPrivateDef => "misplaced-private-def",
            MultipleDefs => "multiple-defs",
            Shadowing => "shadowing",
            UnknownName => "unknown-name",
            UnknownType => "unknown-type",
            UnknownQualified => "unknown-qualified",
            UnknownConstructor => "unknown-constructor",
            AmbiguousConstructor => "ambiguous-constructor",
            AmbiguousName => "ambiguous-name",
            NeedlessQualification => "needless-qualification",
            PrivateCrossModule => "private-cross-module",
            TypeMismatch => "type-mismatch",
            EffectViolation => "effect-violation",
            OperatorPolymorphic => "operator-polymorphic",
            NonDerivable => "non-derivable",
            AnnotationNeeded => "annotation-needed",
            UnknownField => "unknown-field",
            LiteralOutOfRange => "literal-out-of-range",
            NonExhaustiveMatch => "non-exhaustive-match",
            RedundantArm => "redundant-arm",
            MissingRecordRest => "missing-record-rest",
            DuplicateField => "duplicate-field",
            RuntimePanic => "runtime-panic",
            UnfilledHole => "unfilled-hole",
            Usage => "usage",
            Io => "io",
            MalformedInput => "malformed-input",
            ForwardReference => "forward-reference",
            Internal => "internal",
        }
    }

    pub fn class(self) -> i32 {
        use Code::*;
        match self {
            Usage | Io | MalformedInput | ForwardReference | Internal => 2,
            _ => 1,
        }
    }
}

impl Serialize for Code {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Note {
    pub message: String,
    pub file: Opt<String>,
    pub span: Opt<Span>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnostic {
    pub code: Code,
    pub message: String,
    pub file: Opt<String>,
    pub span: Opt<Span>,
    pub notes: Vec<Note>,
}

impl Diagnostic {
    /// A location-free diagnostic (usage, io, internal, …).
    pub fn bare(code: Code, message: impl Into<String>) -> Self {
        Diagnostic {
            code,
            message: message.into(),
            file: Opt(None),
            span: Opt(None),
            notes: Vec::new(),
        }
    }
}

/// The stderr document.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub errors: Vec<Diagnostic>,
}

impl Report {
    pub fn one(diag: Diagnostic) -> Self {
        Report { errors: vec![diag] }
    }

    pub fn render(&self) -> String {
        serde_json::to_string(self).expect("diagnostic serialization cannot fail")
    }

    /// Exit code: 2 if any class-2 code is present, else 1.
    pub fn exit_code(&self) -> i32 {
        if self.errors.iter().any(|d| d.code.class() == 2) {
            2
        } else {
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opt_encoding() {
        assert_eq!(
            serde_json::to_string(&Opt::<u32>(None)).unwrap(),
            r#"{"tag":"None"}"#
        );
        assert_eq!(
            serde_json::to_string(&Opt(Some(7u32))).unwrap(),
            r#"{"tag":"Some","value":7}"#
        );
    }

    #[test]
    fn report_golden() {
        let report = Report::one(Diagnostic {
            code: Code::Shadowing,
            message: "`n` is already bound".into(),
            file: Opt(Some("median.soil".into())),
            span: Opt(Some(Span {
                start: 42,
                end: 43,
                line: 3,
                col: 5,
            })),
            notes: vec![Note {
                message: "previous binding here".into(),
                file: Opt(Some("median.soil".into())),
                span: Opt(Some(Span {
                    start: 10,
                    end: 11,
                    line: 1,
                    col: 11,
                })),
            }],
        });
        assert_eq!(
            report.render(),
            r#"{"errors":[{"code":"shadowing","message":"`n` is already bound","file":{"tag":"Some","value":"median.soil"},"span":{"tag":"Some","value":{"start":42,"end":43,"line":3,"col":5}},"notes":[{"message":"previous binding here","file":{"tag":"Some","value":"median.soil"},"span":{"tag":"Some","value":{"start":10,"end":11,"line":1,"col":11}}}]}]}"#
        );
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn bare_report_golden_and_class() {
        let report = Report::one(Diagnostic::bare(Code::Usage, "unknown command `foo`"));
        assert_eq!(
            report.render(),
            r#"{"errors":[{"code":"usage","message":"unknown command `foo`","file":{"tag":"None"},"span":{"tag":"None"},"notes":[]}]}"#
        );
        assert_eq!(report.exit_code(), 2);
    }
}
