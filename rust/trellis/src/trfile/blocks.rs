//! Fenced-block extraction over CommonMark via `pulldown-cmark`
//! (impl plan 03 §8.3). A fenced block whose info string begins with a
//! reserved word is formal content; everything else — indented code,
//! unreserved fences, headings, paragraphs — is prose (tr-grammar §1).

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::diag::Diag;

/// The reserved block languages (tr-grammar §1).
pub const RESERVED: [&str; 12] = [
    "soil-sig",
    "requires",
    "ensures",
    "test",
    "property",
    "cram",
    "reference",
    "allow",
    "soil-type",
    "invariant",
    "exports",
    "decisions",
];

/// A reserved fenced block, still unparsed.
pub struct RawBlock {
    pub lang: String,
    /// `test`/`property`/`cram` block name from the info string.
    pub name: Option<String>,
    pub xfail: bool,
    /// The tr-grammar §8 `@agent` provenance marker.
    pub agent: bool,
    /// Full info string minus `xfail`/`@agent` modifiers — the lock's
    /// per-block identity (lock-schema §2).
    pub id: String,
    pub content: String,
    /// File-absolute byte range of the whole block, fences included.
    pub span: std::ops::Range<usize>,
}

pub struct Extraction {
    pub blocks: Vec<RawBlock>,
    /// Whether any prose paragraph exists outside reserved blocks
    /// (validity rules 2–3; headings do not count).
    pub has_prose: bool,
}

/// `body` is the markdown after the frontmatter; `base` its byte
/// offset in the file, so block spans are file-absolute.
pub fn extract(body: &str, base: usize) -> Result<Extraction, Vec<Diag>> {
    let mut blocks = Vec::new();
    let mut errors = Vec::new();
    let mut has_prose = false;

    // (info, content, start) of the fenced block being read.
    let mut current: Option<(String, String, usize)> = None;
    let mut in_paragraph = false;

    for (event, range) in Parser::new_ext(body, Options::empty()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                current = Some((info.to_string(), String::new(), range.start));
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Indented)) => {
                // Indented code is prose-class (tr-grammar §1).
                has_prose = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((info, content, start)) = current.take() {
                    match classify(&info, base + start, base + range.end, content) {
                        Ok(Some(block)) => blocks.push(block),
                        Ok(None) => has_prose = true, // unreserved fence
                        Err(diag) => errors.push(diag),
                    }
                }
            }
            Event::Text(text) => match &mut current {
                Some((_, content, _)) => content.push_str(&text),
                None => {
                    if in_paragraph && !text.trim().is_empty() {
                        has_prose = true;
                    }
                }
            },
            Event::Start(Tag::Paragraph) => in_paragraph = true,
            Event::End(TagEnd::Paragraph) => in_paragraph = false,
            _ => {}
        }
    }

    if errors.is_empty() {
        Ok(Extraction { blocks, has_prose })
    } else {
        Err(errors)
    }
}

/// `Ok(None)`: an unreserved fence — prose. The info-string grammar is
/// `<lang> [<name>] [xfail] [@agent]`, name required for exactly the
/// named-test languages (tr-grammar §1).
fn classify(
    info: &str,
    start: usize,
    end: usize,
    content: String,
) -> Result<Option<RawBlock>, Diag> {
    let mut words = info.split_whitespace();
    let lang = match words.next() {
        Some(lang) if RESERVED.contains(&lang) => lang.to_string(),
        _ => return Ok(None),
    };
    let named = matches!(lang.as_str(), "test" | "property" | "cram");

    let mut name = None;
    let mut xfail = false;
    let mut agent = false;
    for word in words {
        match word {
            "xfail" if named && !xfail && !agent => xfail = true,
            "@agent" if !agent => agent = true,
            word if named && name.is_none() && !xfail && !agent && word != "xfail" => {
                name = Some(word.to_string());
            }
            word => {
                return Err(Diag::new(
                    "tr-block-info",
                    format!("unexpected `{word}` in the info string of a `{lang}` block"),
                ));
            }
        }
    }
    if named && name.is_none() {
        return Err(Diag::new(
            "tr-block-info",
            format!("a `{lang}` block needs a name in its info string (tr-grammar §1)"),
        ));
    }

    let id = match &name {
        Some(name) => format!("{lang} {name}"),
        None => lang.clone(),
    };
    Ok(Some(RawBlock {
        lang,
        name,
        xfail,
        agent,
        id,
        content,
        span: start..end,
    }))
}
