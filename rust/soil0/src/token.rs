//! The token set, contract §4.5. Serde's adjacent tagging produces
//! exactly the tr-grammar §7 sum encoding (`{"tag": …, "value": …}`).

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "tag", content = "value")]
pub enum Token {
    TIdent {
        name: String,
    },
    TPrivate {
        name: String,
    },
    TTypeName {
        name: String,
    },
    TKeyword {
        word: String,
    },
    TOp {
        op: String,
    },
    /// Digits with underscores stripped; sign never included.
    TInt {
        digits: String,
    },
    /// Source text with underscores stripped.
    TFloat {
        text: String,
    },
    /// Escapes decoded.
    TStr {
        value: String,
    },
    /// A typed hole `?name` (v1.1); `name` excludes the `?`.
    THole {
        name: String,
    },
}

pub const KEYWORDS: &[&str] = &[
    "let",
    "rec",
    "and",
    "or",
    "not",
    "in",
    "fun",
    "match",
    "with",
    "if",
    "then",
    "else",
    "decreases",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_json_shapes() {
        assert_eq!(
            serde_json::to_string(&Token::TIdent { name: "gcd".into() }).unwrap(),
            r#"{"tag":"TIdent","value":{"name":"gcd"}}"#
        );
        assert_eq!(
            serde_json::to_string(&Token::TOp { op: "->".into() }).unwrap(),
            r#"{"tag":"TOp","value":{"op":"->"}}"#
        );
    }
}
