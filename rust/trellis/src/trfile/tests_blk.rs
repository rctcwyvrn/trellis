//! The test-family block mini-languages: `test` (call-arrow lines,
//! tr-grammar §3.3), `property` (§3.4), `cram` (§3.5), `reference`
//! (§3.6), and `allow` (§3.7). JSON values are parsed with serde_json
//! and kept raw — type-directed canonical decoding happens at bundle
//! assembly (step 7), not here.

use std::collections::BTreeSet;

use soil0::ast::{Pred, Type};
use soil0::span::Spanned;
use soil0::token::Token;

use crate::diag::Diag;

// ---- `test` ----

#[derive(Debug)]
pub struct TestBlock {
    pub withs: Vec<WithBind>,
    pub cases: Vec<TestCase>,
}

#[derive(Debug)]
pub struct WithBind {
    pub name: String,
    pub ctor: String,
    pub args: Vec<serde_json::Value>,
}

#[derive(Debug)]
pub struct TestCase {
    pub args: Vec<TestArg>,
    pub outcome: Outcome,
}

#[derive(Debug)]
pub enum TestArg {
    Json(serde_json::Value),
    Binding(String),
}

#[derive(Debug)]
pub enum Outcome {
    Value(serde_json::Value),
    Panic,
}

pub fn parse_test(content: &str) -> Result<TestBlock, Diag> {
    let mut withs: Vec<WithBind> = Vec::new();
    let mut cases = Vec::new();
    let mut seen_case = false;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("with ") {
            if seen_case {
                return Err(test_err("`with` lines must precede the case lines"));
            }
            withs.push(parse_with(rest)?);
        } else {
            seen_case = true;
            let binds: BTreeSet<&str> = withs.iter().map(|w| w.name.as_str()).collect();
            cases.push(parse_case(line, &binds)?);
        }
    }
    if cases.is_empty() {
        return Err(test_err("a test block needs at least one case line"));
    }
    Ok(TestBlock { withs, cases })
}

/// `<ident> = <ctor> <json>…` (the `with ` prefix already stripped).
fn parse_with(rest: &str) -> Result<WithBind, Diag> {
    let (name, call) = rest
        .split_once('=')
        .ok_or_else(|| test_err("a `with` line is `with name = fake_ctor json…`"))?;
    let name = ident_with(name.trim(), "with-binding name", "tr-test-syntax")?;
    let call = call.trim();
    let (ctor, args_text) = match call.split_once(char::is_whitespace) {
        Some((ctor, rest)) => (ctor, rest),
        None => (call, ""),
    };
    let ctor = ident_with(ctor, "fake-constructor name", "tr-test-syntax")?;
    let mut args = Vec::new();
    let mut stream = serde_json::Deserializer::from_str(args_text).into_iter();
    for value in &mut stream {
        args.push(value.map_err(|e| test_err(format!("bad JSON argument: {e}")))?);
    }
    Ok(WithBind { name, ctor, args })
}

/// `( arg, … ) => outcome`; an ident argument must be a with-binding.
fn parse_case(line: &str, binds: &BTreeSet<&str>) -> Result<TestCase, Diag> {
    let mut rest = line
        .strip_prefix('(')
        .ok_or_else(|| test_err(format!("case line `{line}` must start with `(`")))?
        .trim_start();
    let mut args = Vec::new();
    if let Some(after) = rest.strip_prefix(')') {
        rest = after;
    } else {
        loop {
            let (text, after) = split_arg(rest);
            let text = text.trim();
            let arg = if binds.contains(text) {
                TestArg::Binding(text.to_string())
            } else {
                TestArg::Json(one_json(text).map_err(|_| {
                    test_err(format!("`{text}` is neither JSON nor a with-binding"))
                })?)
            };
            args.push(arg);
            if let Some(after) = after.strip_prefix(',') {
                rest = after.trim_start();
            } else if let Some(after) = after.strip_prefix(')') {
                rest = after;
                break;
            } else {
                return Err(test_err(format!("case line `{line}` is missing `)`")));
            }
        }
    }
    let rest = rest
        .trim_start()
        .strip_prefix("=>")
        .ok_or_else(|| test_err(format!("expected `=>` in case line `{line}`")))?
        .trim();
    let outcome = if rest == "panic" {
        Outcome::Panic
    } else {
        Outcome::Value(one_json(rest).map_err(|e| test_err(format!("bad expected value: {e}")))?)
    };
    Ok(TestCase { args, outcome })
}

/// Split one argument's text at the next top-level `,` or `)` —
/// JSON-aware (bracket depth and string state), because a JSON number
/// directly followed by a delimiter is not self-delimiting for a
/// streaming parser.
fn split_arg(rest: &str) -> (&str, &str) {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in rest.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '[' | '{' => depth += 1,
            ']' | '}' => depth = depth.saturating_sub(1),
            ',' | ')' if depth == 0 => return (&rest[..i], &rest[i..]),
            _ => {}
        }
    }
    (rest, "")
}

fn test_err(message: impl Into<String>) -> Diag {
    Diag::new("tr-test-syntax", message)
}

// ---- `property` ----

#[derive(Debug)]
pub struct PropertyBlock {
    pub foralls: Vec<Forall>,
    pub pred: Spanned<Pred>,
}

#[derive(Debug)]
pub struct Forall {
    pub name: String,
    pub ty: Spanned<Type>,
    pub filter: Option<Spanned<Pred>>,
}

pub fn parse_property(content: &str) -> Result<PropertyBlock, Diag> {
    let mut foralls = Vec::new();
    let mut pred_lines = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match trimmed.strip_prefix("forall ") {
            Some(rest) if pred_lines.is_empty() => foralls.push(parse_forall(rest)?),
            _ => pred_lines.push(trimmed),
        }
    }
    if foralls.is_empty() {
        return Err(prop_err("a property needs at least one `forall` line"));
    }
    if pred_lines.is_empty() {
        return Err(prop_err(
            "a property needs a predicate after its `forall` lines",
        ));
    }
    let pred = soil0::parser::parse_pred_str(&pred_lines.join(" "))
        .map_err(|d| super::soil0_syntax("tr-property-syntax", &d))?;
    Ok(PropertyBlock { foralls, pred })
}

/// `<ident> : <type> [where <predicate>]` (the `forall ` prefix
/// already stripped). The `where` split point is the first
/// bracket-depth-0 `where` token — `where` is a keyword in neither the
/// type nor the predicate grammar, so a depth-0 occurrence can only be
/// the separator.
fn parse_forall(rest: &str) -> Result<Forall, Diag> {
    let (name, after) = rest
        .split_once(':')
        .ok_or_else(|| prop_err("a forall line is `forall name : type [where predicate]`"))?;
    let name = ident_with(name.trim(), "forall binder", "tr-property-syntax")?;
    let tokens =
        soil0::lexer::lex(after).map_err(|d| super::soil0_syntax("tr-property-syntax", &d))?;
    let mut depth = 0usize;
    let mut split_at = None;
    for token in &tokens {
        match &token.item {
            Token::TOp { op } if matches!(op.as_str(), "(" | "{") => depth += 1,
            Token::TOp { op } if matches!(op.as_str(), ")" | "}") => {
                depth = depth.saturating_sub(1)
            }
            Token::TIdent { name } if name == "where" && depth == 0 => {
                split_at = Some(token.span);
                break;
            }
            _ => {}
        }
    }
    let (ty_text, filter_text) = match split_at {
        Some(span) => (&after[..span.start], Some(&after[span.end..])),
        None => (after, None),
    };
    let ty = soil0::parser::parse_type_str(ty_text.trim())
        .map_err(|d| super::soil0_syntax("tr-property-syntax", &d))?;
    let filter = match filter_text {
        Some(text) => Some(
            soil0::parser::parse_pred_str(text.trim())
                .map_err(|d| super::soil0_syntax("tr-property-syntax", &d))?,
        ),
        None => None,
    };
    Ok(Forall { name, ty, filter })
}

fn prop_err(message: impl Into<String>) -> Diag {
    Diag::new("tr-property-syntax", message)
}

// ---- `cram` ----

#[derive(Debug)]
pub struct CramBlock {
    pub files: Vec<CramFile>,
    pub steps: Vec<CramStep>,
}

#[derive(Debug)]
pub struct CramFile {
    pub path: String,
    pub contents: String,
}

#[derive(Debug)]
pub struct CramStep {
    pub command: String,
    pub output: Vec<String>,
    pub exit: i64,
}

pub fn parse_cram(content: &str) -> Result<CramBlock, Diag> {
    let mut files = Vec::new();
    let mut steps: Vec<CramStep> = Vec::new();
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("with file ") {
            if !steps.is_empty() {
                return Err(cram_err("`with file` fixtures must precede the transcript"));
            }
            files.push(parse_cram_file(rest)?);
        } else if let Some(command) = line.strip_prefix("$ ") {
            steps.push(CramStep {
                command: command.to_string(),
                output: Vec::new(),
                exit: 0,
            });
        } else if let Some(step) = steps.last_mut() {
            if let Some(exit) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                step.exit = exit
                    .parse()
                    .map_err(|_| cram_err(format!("bad exit line `{line}`")))?;
            } else {
                step.output.push(line.to_string());
            }
        } else if !line.trim().is_empty() {
            return Err(cram_err(format!(
                "line `{line}` precedes the first `$ ` command"
            )));
        }
    }
    if steps.is_empty() {
        return Err(cram_err("a cram block needs at least one `$ ` command"));
    }
    Ok(CramBlock { files, steps })
}

/// `<json-string> = <json-string>` (the `with file ` prefix already
/// stripped). Both sides are JSON strings so escapes are pinned; `=`
/// inside them cannot confuse the parse because JSON is consumed, not
/// split.
fn parse_cram_file(rest: &str) -> Result<CramFile, Diag> {
    let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<serde_json::Value>();
    let path = next_json_string(&mut stream, "fixture path")?;
    let rest = rest[stream.byte_offset()..].trim_start();
    let rest = rest
        .strip_prefix('=')
        .ok_or_else(|| cram_err("a fixture line is `with file \"path\" = \"contents\"`"))?;
    let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<serde_json::Value>();
    let contents = next_json_string(&mut stream, "fixture contents")?;
    if !rest[stream.byte_offset()..].trim().is_empty() {
        return Err(cram_err("unexpected text after the fixture contents"));
    }
    Ok(CramFile { path, contents })
}

fn next_json_string<'de>(
    stream: &mut serde_json::StreamDeserializer<
        'de,
        serde_json::de::StrRead<'de>,
        serde_json::Value,
    >,
    what: &str,
) -> Result<String, Diag> {
    match stream.next() {
        Some(Ok(serde_json::Value::String(s))) => Ok(s),
        _ => Err(cram_err(format!("{what} must be a JSON string"))),
    }
}

fn cram_err(message: impl Into<String>) -> Diag {
    Diag::new("tr-cram-syntax", message)
}

// ---- `reference` ----

#[derive(Debug)]
pub enum Reference {
    Python { path: String, symbol: String },
    Cli { command: String },
}

pub fn parse_reference(content: &str) -> Result<Reference, Diag> {
    let mut lines = content.lines().filter(|l| !l.trim().is_empty());
    let line = lines
        .next()
        .ok_or_else(|| ref_err("a reference block holds exactly one line"))?;
    if lines.next().is_some() {
        return Err(ref_err("a reference block holds exactly one line"));
    }
    let line = line.trim();
    if let Some(command) = line.strip_prefix("cli ") {
        return Ok(Reference::Cli {
            command: command.trim().to_string(),
        });
    }
    match line.rsplit_once("::") {
        Some((path, symbol)) if !path.is_empty() => Ok(Reference::Python {
            path: path.to_string(),
            symbol: ident_with(symbol, "reference symbol", "tr-reference-syntax")?,
        }),
        _ => Err(ref_err(format!(
            "`{line}` is neither `relpath::symbol` nor `cli command` (tr-grammar §3.6)"
        ))),
    }
}

fn ref_err(message: impl Into<String>) -> Diag {
    Diag::new("tr-reference-syntax", message)
}

// ---- `allow` ----

/// The fixed escape-hatch set (tr-grammar §3.7).
const HATCHES: [&str; 3] = ["partial", "unsafe", "ffi-raw"];

pub fn parse_allow(content: &str) -> Result<Vec<String>, Diag> {
    let mut hatches = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !HATCHES.contains(&line) {
            return Err(Diag::new(
                "tr-allow-unknown",
                format!(
                    "`{line}` is not an escape hatch (tr-grammar §3.7: partial, unsafe, ffi-raw)"
                ),
            ));
        }
        hatches.push(line.to_string());
    }
    Ok(hatches)
}

// ---- shared ----

fn one_json(text: &str) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::from_str(text)
}

fn ident_with(text: &str, what: &str, code: &str) -> Result<String, Diag> {
    let ok = !text.is_empty()
        && text
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(text.to_string())
    } else {
        Err(Diag::new(
            code,
            format!("`{text}` is not a lowercase snake_case {what}"),
        ))
    }
}
