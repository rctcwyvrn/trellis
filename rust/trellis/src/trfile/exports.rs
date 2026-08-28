//! `exports` (tr-grammar §5.1) and `decisions` (§5.2) block grammars
//! — both line-oriented.

use crate::diag::Diag;

#[derive(Debug)]
pub enum ExportLine {
    Def(String),
    Type(String),
    AbstractType(String),
}

pub fn parse_exports(content: &str) -> Result<Vec<ExportLine>, Diag> {
    let mut lines = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry = if let Some(name) = line.strip_prefix("abstract type ") {
            ExportLine::AbstractType(type_name(name.trim())?)
        } else if let Some(name) = line.strip_prefix("type ") {
            ExportLine::Type(type_name(name.trim())?)
        } else {
            let ok = line
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !ok || line.is_empty() {
                return Err(Diag::new(
                    "tr-exports-syntax",
                    format!("`{line}` is not an export line (tr-grammar §5.1)"),
                ));
            }
            ExportLine::Def(line.to_string())
        };
        lines.push(entry);
    }
    Ok(lines)
}

fn type_name(text: &str) -> Result<String, Diag> {
    let ok = text.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && text.chars().all(|c| c.is_ascii_alphanumeric());
    if ok {
        Ok(text.to_string())
    } else {
        Err(Diag::new(
            "tr-exports-syntax",
            format!("`{text}` is not a PascalCase type name"),
        ))
    }
}

#[derive(Debug)]
pub struct DecisionEntry {
    pub label: String,
    pub text: String,
    pub rejected: Vec<String>,
}

pub fn parse_decisions(content: &str) -> Result<Vec<DecisionEntry>, Diag> {
    let mut entries: Vec<DecisionEntry> = Vec::new();
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(rejected) = line.trim_start().strip_prefix("rejected:") {
            match entries.last_mut() {
                Some(entry) => entry.rejected.push(rejected.trim().to_string()),
                None => {
                    return Err(decisions_err("a `rejected:` line must follow an entry"));
                }
            }
            continue;
        }
        let (label, text) = line.split_once(':').ok_or_else(|| {
            decisions_err(format!(
                "`{line}` is not `label: decision` (tr-grammar §5.2)"
            ))
        })?;
        let label = label.trim();
        if label.is_empty() {
            return Err(decisions_err("a decision label may not be empty"));
        }
        if entries.iter().any(|e| e.label == label) {
            // Labels are identity (per-entry hashing, tr-grammar §5.2).
            return Err(decisions_err(format!("duplicate decision label `{label}`")));
        }
        entries.push(DecisionEntry {
            label: label.to_string(),
            text: text.trim().to_string(),
            rejected: Vec::new(),
        });
    }
    if entries.is_empty() {
        return Err(decisions_err("a decisions block needs at least one entry"));
    }
    Ok(entries)
}

fn decisions_err(message: impl Into<String>) -> Diag {
    Diag::new("tr-decisions-syntax", message)
}
