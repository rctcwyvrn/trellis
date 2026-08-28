//! `.tr` parsing (impl plan 03 step 3): frontmatter, reserved fenced
//! blocks, the block mini-languages, kind inference, and every
//! tr-grammar §6 validity rule. Types and predicates are parsed by the
//! linked soil0 parser — one grammar implementation.

pub mod blocks;
pub mod exports;
pub mod frontmatter;
pub mod predicate;
pub mod sig;
pub mod tests_blk;
pub mod typedef;

use std::collections::BTreeSet;
use std::path::Path;

use soil0::ast::Effect;

use crate::diag::{Diag, ErrorReport, Span};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Function,
    Type,
    Module,
    Project,
}

#[derive(Debug)]
pub struct TrFile {
    pub name: String,
    pub tags: Vec<String>,
    pub kind: FileKind,
    /// Byte length of the frontmatter; the markdown body follows.
    pub frontmatter_len: usize,
    /// Byte range of the frontmatter `name:` line (formal-class bytes,
    /// elided from the prose hash — impl plan 03 §8.5).
    pub name_span: std::ops::Range<usize>,
    pub has_prose: bool,
    pub blocks: Vec<Block>,
}

#[derive(Debug)]
pub struct Block {
    /// The lock's per-block identity: info string minus modifiers
    /// (lock-schema §2).
    pub id: String,
    pub lang: String,
    pub name: Option<String>,
    pub xfail: bool,
    /// tr-grammar §8 `@agent` marker (unmarked formal blocks are
    /// human-authored, therefore pinned).
    pub agent: bool,
    /// File-absolute byte range, fences included — the step-4 hash
    /// partition boundary.
    pub span: std::ops::Range<usize>,
    pub content: String,
    pub payload: Payload,
}

#[derive(Debug)]
pub enum Payload {
    Sig(sig::Sig),
    Clauses(Vec<predicate::Clause>),
    Tests(tests_blk::TestBlock),
    Property(tests_blk::PropertyBlock),
    Cram(tests_blk::CramBlock),
    Reference(tests_blk::Reference),
    Allow(Vec<String>),
    TypeDef(typedef::TypeDef),
    Exports(Vec<exports::ExportLine>),
    Decisions(Vec<exports::DecisionEntry>),
}

/// Which file kinds each reserved block is legal in (tr-grammar §1).
fn legal_kinds(lang: &str) -> &'static [FileKind] {
    use FileKind::*;
    match lang {
        "soil-sig" | "requires" | "ensures" | "test" | "property" | "cram" | "reference"
        | "allow" => &[Function],
        "soil-type" | "invariant" => &[Type],
        "exports" => &[Module],
        "decisions" => &[Module, Project],
        _ => unreachable!("unreserved language"),
    }
}

/// At-most-one blocks (tr-grammar §1 table).
const AT_MOST_ONE: [&str; 8] = [
    "soil-sig",
    "requires",
    "ensures",
    "reference",
    "allow",
    "soil-type",
    "invariant",
    "decisions",
];

/// Parse and validate one `.tr` file. `tag_vocabulary` is the
/// `soil.toml` `[tags]` key set when available (`None` skips the
/// undeclared-tag rule, for standalone use).
pub fn parse_tr(
    src: &str,
    path: &Path,
    tag_vocabulary: Option<&BTreeSet<String>>,
) -> Result<TrFile, ErrorReport> {
    let file_name = path.to_string_lossy().into_owned();
    let lines = soil0::span::LineMap::new(src);
    let at = |diag: Diag, start: usize, end: usize| -> Diag {
        let s = lines.span(start, end);
        diag.at(
            &file_name,
            Span {
                start: s.start,
                end: s.end,
                line: s.line,
                col: s.col,
            },
        )
    };

    let mut errors: Vec<Diag> = Vec::new();

    // 1. Frontmatter.
    let fm = match frontmatter::parse(src) {
        Ok(fm) => fm,
        Err(diag) => {
            return Err(ErrorReport {
                errors: vec![at(diag, 0, 0)],
            })
        }
    };

    // 2. Fence extraction over the markdown body.
    let extraction = match blocks::extract(&src[fm.len..], fm.len) {
        Ok(extraction) => extraction,
        Err(diags) => {
            return Err(ErrorReport {
                errors: diags.into_iter().map(|d| at(d, fm.len, fm.len)).collect(),
            })
        }
    };

    // 3. Kind inference: filename first, then a `soil-type` block.
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let kind = match stem.as_str() {
        "_module" => FileKind::Module,
        "_project" => FileKind::Project,
        _ if extraction.blocks.iter().any(|b| b.lang == "soil-type") => FileKind::Type,
        _ => FileKind::Function,
    };

    // 4. Per-block payloads.
    let mut parsed: Vec<Block> = Vec::new();
    for raw in extraction.blocks {
        let payload = match raw.lang.as_str() {
            "soil-sig" => sig::parse(&raw.content).map(Payload::Sig),
            "requires" | "ensures" | "invariant" => {
                predicate::parse_clauses(&raw.content).map(Payload::Clauses)
            }
            "test" => tests_blk::parse_test(&raw.content).map(Payload::Tests),
            "property" => tests_blk::parse_property(&raw.content).map(Payload::Property),
            "cram" => tests_blk::parse_cram(&raw.content).map(Payload::Cram),
            "reference" => tests_blk::parse_reference(&raw.content).map(Payload::Reference),
            "allow" => tests_blk::parse_allow(&raw.content).map(Payload::Allow),
            "soil-type" => typedef::parse(&raw.content).map(Payload::TypeDef),
            "exports" => exports::parse_exports(&raw.content).map(Payload::Exports),
            "decisions" => exports::parse_decisions(&raw.content).map(Payload::Decisions),
            _ => unreachable!("unreserved language"),
        };
        match payload {
            Ok(payload) => parsed.push(Block {
                id: raw.id,
                lang: raw.lang,
                name: raw.name,
                xfail: raw.xfail,
                agent: raw.agent,
                span: raw.span,
                content: raw.content,
                payload,
            }),
            Err(diag) => errors.push(at(diag, raw.span.start, raw.span.end)),
        }
    }

    // 5. Validity rules (tr-grammar §6), collected rather than
    // fail-fast so a corpus file reports everything wrong with it.
    validate(
        &fm,
        kind,
        &parsed,
        extraction.has_prose,
        path,
        &stem,
        &mut errors,
    );

    if let Some(vocabulary) = tag_vocabulary {
        for tag in &fm.tags {
            if !vocabulary.contains(tag) {
                errors.push(Diag::new(
                    "tr-undeclared-tag",
                    format!("tag `{tag}` is not declared in soil.toml [tags] (tr-grammar §1)"),
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(TrFile {
            name: fm.name,
            tags: fm.tags,
            kind,
            frontmatter_len: fm.len,
            name_span: fm.name_span,
            has_prose: extraction.has_prose,
            blocks: parsed,
        })
    } else {
        Err(ErrorReport { errors })
    }
}

fn validate(
    fm: &frontmatter::Frontmatter,
    kind: FileKind,
    blocks: &[Block],
    has_prose: bool,
    path: &Path,
    stem: &str,
    errors: &mut Vec<Diag>,
) {
    let push = |errors: &mut Vec<Diag>, code: &str, message: String| {
        errors.push(Diag::new(code, message));
    };

    // Rule 1 half (name casing/agreement with the filename); rules
    // 2–5 name checks.
    match kind {
        FileKind::Function => {
            if fm.name != stem {
                push(
                    errors,
                    "tr-name-mismatch",
                    format!(
                    "frontmatter name `{}` must equal the filename stem `{stem}` (tr-grammar §6)",
                    fm.name
                ),
                );
            }
        }
        FileKind::Type => {
            let snake = snake_case(&fm.name);
            if snake != stem {
                push(
                    errors,
                    "tr-name-mismatch",
                    format!(
                    "the snake_case of type `{}` is `{snake}`, but the filename stem is `{stem}`",
                    fm.name
                ),
                );
            }
        }
        FileKind::Module | FileKind::Project => {
            let dir = path
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if !dir.is_empty() && fm.name != dir {
                push(
                    errors,
                    "tr-name-mismatch",
                    format!(
                        "frontmatter name `{}` must equal the containing directory `{dir}`",
                        fm.name
                    ),
                );
            }
            if kind == FileKind::Project {
                let beside = path.parent().map(|p| p.join("soil.toml"));
                if !beside.is_some_and(|p| p.is_file()) {
                    push(
                        errors,
                        "tr-project-location",
                        format!(
                            "{} must live at the Soil root, beside soil.toml (tr-grammar §6)",
                            path.display()
                        ),
                    );
                }
            }
        }
    }

    // Block legality per kind, and multiplicities.
    let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
    for block in blocks {
        if !legal_kinds(&block.lang).contains(&kind) {
            push(
                errors,
                "tr-block-kind",
                format!(
                    "a `{}` block is not legal in a {} file (tr-grammar §1)",
                    block.lang,
                    kind_name(kind)
                ),
            );
        }
        *counts.entry(block.lang.as_str()).or_default() += 1;
    }
    for lang in AT_MOST_ONE {
        if counts.get(lang).copied().unwrap_or(0) > 1 {
            push(
                errors,
                "tr-block-count",
                format!("at most one `{lang}` block per file (tr-grammar §1)"),
            );
        }
    }
    if kind == FileKind::Module && counts.get("exports").copied().unwrap_or(0) != 1 {
        push(
            errors,
            "tr-block-count",
            "a module header holds exactly one `exports` block (tr-grammar §6)".into(),
        );
    }
    if kind == FileKind::Type && counts.get("soil-type").copied().unwrap_or(0) > 1 {
        push(
            errors,
            "tr-block-count",
            "a type file holds exactly one `soil-type` block".into(),
        );
    }

    // Prose and test obligations (rules 2–3).
    if matches!(kind, FileKind::Function | FileKind::Type) && !has_prose {
        push(
            errors,
            "tr-missing-prose",
            format!(
                "a {} file needs at least one prose paragraph (tr-grammar §6)",
                kind_name(kind)
            ),
        );
    }
    let test_count = ["test", "property", "cram"]
        .iter()
        .map(|l| counts.get(*l).copied().unwrap_or(0))
        .sum::<usize>();
    if kind == FileKind::Function && test_count == 0 {
        push(
            errors,
            "tr-missing-test-block",
            "a function file needs at least one test, property, or cram block (tr-grammar §6)"
                .into(),
        );
    }

    // Rule 6: unique test-family names.
    let mut names = BTreeSet::new();
    for block in blocks {
        if let Some(name) = &block.name {
            if !names.insert(name.clone()) {
                push(
                    errors,
                    "tr-duplicate-test-name",
                    format!("duplicate test block name `{name}` (tr-grammar §6)"),
                );
            }
        }
    }

    // Rule 7: declared names agree with the frontmatter.
    let the_sig = blocks.iter().find_map(|b| match &b.payload {
        Payload::Sig(sig) => Some(sig),
        _ => None,
    });
    if let Some(sig) = the_sig {
        if sig.name != fm.name {
            push(
                errors,
                "tr-name-mismatch",
                format!(
                    "the soil-sig declares `{}` but the frontmatter names `{}` (tr-grammar §6)",
                    sig.name, fm.name
                ),
            );
        }
    }
    if let Some(td) = blocks.iter().find_map(|b| match &b.payload {
        Payload::TypeDef(td) => Some(td),
        _ => None,
    }) {
        if td.name != fm.name {
            push(
                errors,
                "tr-name-mismatch",
                format!(
                    "the soil-type declares `{}` but the frontmatter names `{}` (tr-grammar §6)",
                    td.name, fm.name
                ),
            );
        }
    }

    // Rule 8: predicates need a fully-named signature (tr-grammar
    // §3.1); enforced whenever requires/ensures exist.
    let has_clauses = blocks
        .iter()
        .any(|b| matches!(b.lang.as_str(), "requires" | "ensures"));
    if has_clauses {
        match the_sig {
            Some(sig) if sig::fully_named(&sig.ty) => {}
            _ => push(
                errors,
                "tr-unnamed-params",
                "requires/ensures need a soil-sig whose parameters are all named (tr-grammar §3.1)"
                    .into(),
            ),
        }
    }

    // §3.3: `panic` outcomes are legal only under a `panic` row.
    let has_panic_outcome = blocks.iter().any(|b| match &b.payload {
        Payload::Tests(t) => t
            .cases
            .iter()
            .any(|c| matches!(c.outcome, tests_blk::Outcome::Panic)),
        _ => false,
    });
    if has_panic_outcome {
        let row_has_panic = the_sig.is_some_and(|s| sig::has_effect(&s.ty, Effect::Panic));
        if !row_has_panic {
            push(errors, "tr-panic-without-row", "a `panic` outcome is only legal when the signature's row carries `panic` (tr-grammar §3.3)".into());
        }
    }
}

fn kind_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Function => "function",
        FileKind::Type => "type",
        FileKind::Module => "module header",
        FileKind::Project => "project header",
    }
}

/// PascalCase → snake_case, the filename form of a type's name
/// (tr-grammar §1).
pub fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Wrap a soil0 diagnostic from an embedded parse (block-relative
/// positions) as a daemon diagnostic; codes are daemon-owned per
/// contract §6.1, with the soil0 code and message carried in the text.
pub(crate) fn soil0_syntax(code: &str, d: &soil0::diag::Diagnostic) -> Diag {
    let position = match &d.span.0 {
        Some(span) => format!(" (block-relative line {}, col {})", span.line, span.col),
        None => String::new(),
    };
    Diag::new(
        code,
        format!("{} [{}]{}", d.message, d.code.as_str(), position),
    )
}
