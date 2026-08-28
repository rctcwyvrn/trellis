//! Hashing (impl plan 03 step 4, micro-pins §8.5–§8.6; design §6).
//!
//! Every digest is SHA-256 over a **length-prefixed** concatenation of
//! parts (8-byte little-endian length before each part), rendered
//! `sha256:` + 64 lowercase hex. Length-prefixing removes
//! concatenation ambiguity; no whitespace normalization happens inside
//! parts — the bytes are the spec. Nothing path-shaped ever enters a
//! hash, so digests are reproducible across machines (impl plan §7).
//!
//! - **Three-part spec hash** (design §6.2, tr-grammar §1): `formal` =
//!   the frontmatter `name` value + each formal-class block's (info
//!   string, content bytes) in document order; `test` = likewise for
//!   the test class (the info string includes `xfail`); `prose` = the
//!   remaining file bytes with formal/test/`decisions` block spans
//!   (fences included) and the frontmatter `name:` line elided.
//! - **Decisions** are their own per-entry class (tr-grammar §5.2,
//!   resolved 2026-08-24): each entry hashes over its raw lines from
//!   the label line through its `rejected:` lines; the scope hash
//!   covers the sorted labels, NUL-separated.
//! - **`soil_hash`** (design §6.1): a format tag, the definition's
//!   hash-form text (soil-syntax-spec §9.1), and one
//!   `name NUL referent-hash` line per free reference, sorted by name
//!   — callees by `soil_hash`, private helpers by the same recipe
//!   (the fold-in), types by `formal_hash` (cycle members by the
//!   cycle hash), builtins by the literal `soil0-builtin/<major>`.

use sha2::{Digest, Sha256};

use crate::trfile::{Block, FileKind, TrFile};

/// SHA-256 over length-prefixed parts, rendered `sha256:<64 hex>`.
pub fn digest(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    let out = hasher.finalize();
    let mut hex = String::with_capacity(71);
    hex.push_str("sha256:");
    for byte in out {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecHashes {
    pub formal: String,
    /// `None` for type, module, and project files (lock-schema §2).
    pub test: Option<String>,
    pub prose: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Formal,
    Test,
    Decisions,
}

fn class(lang: &str) -> Class {
    match lang {
        "soil-sig" | "requires" | "ensures" | "soil-type" | "invariant" | "exports" | "allow" => {
            Class::Formal
        }
        "test" | "property" | "cram" | "reference" => Class::Test,
        "decisions" => Class::Decisions,
        _ => unreachable!("unreserved language"),
    }
}

/// The block's info string in canonical modifier order — the hash
/// input (the parser enforces this order on the way in, so it equals
/// the source text).
pub fn info_string(block: &Block) -> String {
    let mut info = block.id.clone();
    if block.xfail {
        info.push_str(" xfail");
    }
    if block.agent {
        info.push_str(" @agent");
    }
    info
}

/// The three-part hash of a parsed `.tr` file (`src` must be the text
/// it was parsed from).
pub fn spec_hashes(src: &str, file: &TrFile) -> SpecHashes {
    let infos: Vec<(Class, String)> = file
        .blocks
        .iter()
        .map(|b| (class(&b.lang), info_string(b)))
        .collect();

    let mut formal_parts: Vec<&[u8]> = vec![file.name.as_bytes()];
    let mut test_parts: Vec<&[u8]> = Vec::new();
    for (block, (class, info)) in file.blocks.iter().zip(&infos) {
        match class {
            Class::Formal => {
                formal_parts.push(info.as_bytes());
                formal_parts.push(block.content.as_bytes());
            }
            Class::Test => {
                test_parts.push(info.as_bytes());
                test_parts.push(block.content.as_bytes());
            }
            Class::Decisions => {}
        }
    }

    // Prose: everything else, with formal/test/decisions block spans
    // (fences included) and the `name:` line elided.
    let mut elide: Vec<std::ops::Range<usize>> = file
        .blocks
        .iter()
        .map(|b| b.span.clone())
        .chain(std::iter::once(file.name_span.clone()))
        .collect();
    elide.sort_by_key(|r| r.start);
    debug_assert!(
        elide.windows(2).all(|w| w[0].end <= w[1].start),
        "elided spans overlap"
    );
    let mut prose = Vec::with_capacity(src.len());
    let mut cursor = 0;
    for range in &elide {
        prose.extend_from_slice(&src.as_bytes()[cursor..range.start]);
        cursor = range.end.min(src.len());
    }
    prose.extend_from_slice(&src.as_bytes()[cursor..]);

    SpecHashes {
        formal: digest(&formal_parts),
        test: (file.kind == FileKind::Function).then(|| digest(&test_parts)),
        prose: digest(&[&prose]),
    }
}

/// Per-entry decision hashes over a `decisions` block's raw content:
/// each entry is its label line plus its `rejected:` lines, exactly as
/// written, joined by `\n`. Returns `(label, hash)` in document order.
pub fn decision_entry_hashes(content: &str) -> Vec<(String, String)> {
    let mut entries: Vec<(String, Vec<&str>)> = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if line.trim_start().starts_with("rejected:") {
            if let Some((_, lines)) = entries.last_mut() {
                lines.push(line);
            }
        } else if let Some((label, _)) = line.split_once(':') {
            entries.push((label.trim().to_string(), vec![line]));
        }
    }
    entries
        .into_iter()
        .map(|(label, lines)| {
            let bytes = lines.join("\n");
            (label, digest(&[bytes.as_bytes()]))
        })
        .collect()
}

/// The scope-membership hash: sorted entry labels, NUL-separated
/// (lock-schema §3 — additions and removals move this, text edits do
/// not).
pub fn decisions_scope_hash(labels: &[String]) -> String {
    let mut sorted: Vec<&str> = labels.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    digest(&[sorted.join("\0").as_bytes()])
}

/// A free reference of a definition: `referent` is the callee's
/// `soil_hash`, a private helper's recursive hash, a type's
/// `formal_hash` (or cycle hash), or the literal builtin tag.
pub type HashRef = (String, String);

/// The builtin referent tag (impl plan §8.6).
pub fn builtin_referent() -> String {
    format!("soil0-builtin/{}", crate::config::SOIL0_CLI_MAJOR)
}

/// `soil_hash` over a definition's hash-form text and its resolved
/// reference edges (design §6.1: free variables replaced by the hashes
/// of what they refer to).
pub fn soil_hash(hash_form: &str, refs: &[HashRef]) -> String {
    let mut sorted: Vec<&HashRef> = refs.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut edges = String::new();
    for (name, referent) in sorted {
        edges.push_str(name);
        edges.push('\0');
        edges.push_str(referent);
        edges.push('\n');
    }
    digest(&[b"soil-hash/1", hash_form.as_bytes(), edges.as_bytes()])
}

/// The combined hash of a recursive type cycle: the members' canonical
/// `soil-type` texts, sorted by type name (design §3.8, §6.1).
pub fn cycle_hash(members: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = members.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut parts: Vec<&[u8]> = vec![b"soil-cycle/1"];
    for (_, text) in &sorted {
        parts.push(text.as_bytes());
    }
    digest(&parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_length_prefixed() {
        // ("ab", "c") and ("a", "bc") must differ.
        assert_ne!(digest(&[b"ab", b"c"]), digest(&[b"a", b"bc"]));
        assert_eq!(digest(&[b"ab", b"c"]), digest(&[b"ab", b"c"]));
    }

    #[test]
    fn scope_hash_is_order_independent() {
        let a = decisions_scope_hash(&["b".into(), "a".into()]);
        let b = decisions_scope_hash(&["a".into(), "b".into()]);
        assert_eq!(a, b);
    }

    #[test]
    fn soil_hash_refs_order_independent_value_sensitive() {
        let form = "f : I64 -> I64\nf %0 = g %0\n";
        let ab = soil_hash(
            form,
            &[
                ("a".into(), "sha256:11".into()),
                ("b".into(), "sha256:22".into()),
            ],
        );
        let ba = soil_hash(
            form,
            &[
                ("b".into(), "sha256:22".into()),
                ("a".into(), "sha256:11".into()),
            ],
        );
        assert_eq!(ab, ba);
        let changed = soil_hash(
            form,
            &[
                ("a".into(), "sha256:99".into()),
                ("b".into(), "sha256:22".into()),
            ],
        );
        assert_ne!(ab, changed);
    }

    #[test]
    fn cycle_hash_order_independent() {
        let a = cycle_hash(&[
            ("B".into(), "type B = A".into()),
            ("A".into(), "type A = B".into()),
        ]);
        let b = cycle_hash(&[
            ("A".into(), "type A = B".into()),
            ("B".into(), "type B = A".into()),
        ]);
        assert_eq!(a, b);
    }
}
