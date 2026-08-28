//! The `.tr` frontmatter: a hand-rolled two-key YAML subset
//! (impl plan 03 §8.4). `name` is required; `tags` is optional as an
//! inline list (`[a, b]`) or a block list (`- a` lines); unknown keys
//! are errors (tr-grammar §1, §6). Plain scalars only — the format is
//! this subset, so anything else is a syntax error.

use crate::diag::Diag;

pub struct Frontmatter {
    pub name: String,
    /// Byte range of the whole `name:` line (newline included) — the
    /// formal-class slice of the frontmatter; the rest hashes as
    /// prose (tr-grammar §1, impl plan 03 §8.5).
    pub name_span: std::ops::Range<usize>,
    pub tags: Vec<String>,
    /// Byte length of the frontmatter (both `---` fences included,
    /// with the trailing newline) — the markdown body starts here.
    pub len: usize,
}

pub fn parse(src: &str) -> Result<Frontmatter, Diag> {
    let mut lines = LineIter::new(src);
    match lines.next() {
        Some("---") => {}
        _ => {
            return Err(Diag::new(
                "tr-missing-frontmatter",
                "a .tr file begins with `---` frontmatter carrying at least `name` (tr-grammar §1)",
            ))
        }
    }

    let mut name = None;
    let mut name_span = 0..0;
    let mut tags = None;
    loop {
        let line_start = lines.consumed;
        let line = match lines.next() {
            None => {
                return Err(Diag::new(
                    "tr-frontmatter-syntax",
                    "frontmatter is not closed by a `---` line",
                ))
            }
            Some("---") => break,
            Some(line) => line,
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("- ") {
            // A block-list item; legal only directly under `tags:`.
            match &mut tags {
                Some(list) => push_tag(list, rest.trim())?,
                None => {
                    return Err(Diag::new(
                        "tr-frontmatter-syntax",
                        "a `- item` line may only follow `tags:`",
                    ))
                }
            }
            continue;
        }
        let (key, value) = line.split_once(':').ok_or_else(|| {
            Diag::new(
                "tr-frontmatter-syntax",
                format!("frontmatter line `{line}` is not `key: value`"),
            )
        })?;
        match (key.trim(), value.trim()) {
            ("name", value) => {
                if value.is_empty() {
                    return Err(Diag::new(
                        "tr-frontmatter-syntax",
                        "`name:` needs a value on the same line",
                    ));
                }
                name = Some(scalar(value)?);
                name_span = line_start..lines.consumed;
            }
            ("tags", "") => tags = Some(Vec::new()),
            ("tags", value) => {
                let inner = value
                    .strip_prefix('[')
                    .and_then(|v| v.strip_suffix(']'))
                    .ok_or_else(|| {
                        Diag::new(
                            "tr-frontmatter-syntax",
                            "`tags:` takes an inline `[a, b]` list or block-list lines",
                        )
                    })?;
                let mut list = Vec::new();
                for item in inner.split(',') {
                    let item = item.trim();
                    if !item.is_empty() {
                        push_tag(&mut list, item)?;
                    }
                }
                tags = Some(list);
            }
            (unknown, _) => {
                return Err(Diag::new(
                    "tr-unknown-frontmatter-key",
                    format!("unknown frontmatter key `{unknown}` (tr-grammar §1: name, tags)"),
                ))
            }
        }
    }

    Ok(Frontmatter {
        name: name.ok_or_else(|| {
            Diag::new(
                "tr-missing-name",
                "frontmatter must declare `name` (tr-grammar §1)",
            )
        })?,
        name_span,
        tags: tags.unwrap_or_default(),
        len: lines.consumed,
    })
}

fn push_tag(list: &mut Vec<String>, item: &str) -> Result<(), Diag> {
    list.push(scalar(item)?);
    Ok(())
}

/// Plain scalars only: no quotes, no YAML punctuation.
fn scalar(text: &str) -> Result<String, Diag> {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
    if plain {
        Ok(text.to_string())
    } else {
        Err(Diag::new(
            "tr-frontmatter-syntax",
            format!("`{text}` is not a plain scalar (letters, digits, `_`, `-`)"),
        ))
    }
}

struct LineIter<'a> {
    rest: &'a str,
    consumed: usize,
}

impl<'a> LineIter<'a> {
    fn new(src: &'a str) -> Self {
        LineIter {
            rest: src,
            consumed: 0,
        }
    }

    fn next(&mut self) -> Option<&'a str> {
        if self.rest.is_empty() {
            return None;
        }
        let (line, advance) = match self.rest.find('\n') {
            Some(i) => (&self.rest[..i], i + 1),
            None => (self.rest, self.rest.len()),
        };
        self.rest = &self.rest[advance..];
        self.consumed += advance;
        Some(line)
    }
}
