//! Context-bundle assembly and the budgeted packer (impl plan 03
//! step 9, contract §4–§5): the fixed directory layout every lowering
//! job reads, also written to disk by `trellis context` for humans.
//!
//! Priority is fixed (design §4.6): spec > tests > callee signatures >
//! corpus examples > module prose; `previous.soil` and `reference.py`
//! sit between tests and callees (an executable spec and the body
//! under revision outrank the callable menu). The packer keeps the
//! longest priority-order prefix that fits the budget — the first
//! over-budget item and everything after it drop, which makes the
//! packed set and the estimate monotone in the budget and the drop
//! order predictable. `spec.md`, `decisions.md`, `tests.json`, and
//! the manifest itself are never dropped: a budget too small for them
//! is `budget-too-small`, not a silent truncation.

use std::path::Path;

use serde::Serialize;

use crate::diag::{Diag, ErrorReport};
use crate::lock::Checks;
use crate::registry;
use crate::state::{Entry, Root};
use crate::trfile::{tests_blk, FileKind, Payload};

/// Tokens ≈ bytes/4, rounded up (micro-pin §8.10).
pub fn tokens(bytes: usize) -> u64 {
    (bytes as u64).div_ceil(4)
}

#[derive(Serialize)]
struct Manifest {
    definition: String,
    budget: u64,
    estimate: u64,
    packed: Vec<String>,
    dropped: Vec<Dropped>,
    oracle: Option<OracleStanza>,
    spec_hashes: SpecHashes,
}

#[derive(Serialize, Clone)]
struct Dropped {
    item: String,
    reason: String,
}

#[derive(Serialize, Clone)]
#[serde(tag = "tag", content = "value")]
enum OracleStanza {
    Reference { path: String, symbol: String },
    Cli { command: String },
}

#[derive(Serialize)]
struct SpecHashes {
    formal: String,
    test: String,
    prose: String,
}

pub struct Bundle {
    /// (bundle-relative path, contents), `bundle.json` first.
    pub files: Vec<(String, String)>,
    /// The §4.1 manifest, also present in `files`.
    pub manifest: serde_json::Value,
    /// `bundle.json` exactly as written (struct field order; the
    /// `manifest` Value alphabetizes keys).
    pub manifest_text: String,
}

struct Candidate {
    rel: String,
    contents: String,
    /// Whole-unit companions (a corpus pair drops as one).
    companions: Vec<(String, String)>,
}

impl Candidate {
    fn bytes(&self) -> usize {
        self.contents.len() + self.companions.iter().map(|(_, c)| c.len()).sum::<usize>()
    }
    fn one(rel: impl Into<String>, contents: String) -> Candidate {
        Candidate {
            rel: rel.into(),
            contents,
            companions: Vec::new(),
        }
    }
}

fn read_rel(root: &Root, rel: &str) -> Result<String, ErrorReport> {
    std::fs::read_to_string(root.root.join(rel))
        .map_err(|e| ErrorReport::one("io", format!("{rel}: {e}")))
}

fn module_of(rel: &str) -> String {
    rel.rsplit_once('/')
        .map(|(m, _)| m.to_string())
        .unwrap_or_default()
}

/// Nearest-first: the target's module before others, then path order.
fn nearness(target_module: &str, rel: &str) -> (bool, String) {
    (module_of(rel) != target_module, rel.to_string())
}

pub fn assemble(root: &Root, entry: &Entry, budget: u64) -> Result<Bundle, ErrorReport> {
    if entry.kind != FileKind::Function {
        return Err(ErrorReport::one(
            "usage",
            format!(
                "`{}` is not a function definition; only functions lower",
                entry.tr.name
            ),
        ));
    }
    let target_module = module_of(&entry.rel);

    // ---- the never-dropped set ----
    let spec_md = read_rel(root, &format!("{}.tr", entry.rel))?;
    let decisions_md = render_decisions(root, &target_module);
    let tests_json = render_tests(entry);

    // ---- droppable candidates, keep-priority order ----
    let mut candidates: Vec<Candidate> = Vec::new();
    if let Some(soil) = &entry.soil {
        candidates.push(Candidate::one("previous.soil", soil.clone()));
    }
    let oracle = entry.tr.blocks.iter().find_map(|b| match &b.payload {
        Payload::Reference(r) => Some(r),
        _ => None,
    });
    let oracle_stanza = match oracle {
        Some(tests_blk::Reference::Python { path, symbol }) => {
            candidates.push(Candidate::one("reference.py", read_rel(root, path)?));
            Some(OracleStanza::Reference {
                path: path.clone(),
                symbol: symbol.clone(),
            })
        }
        Some(tests_blk::Reference::Cli { command }) => Some(OracleStanza::Cli {
            command: command.clone(),
        }),
        None => None,
    };

    // The callable surface (resolved 2026-09-14): one signature file
    // per *lowered* function in the root, nearest-first — the serial
    // lowering order makes exactly these available, and a first
    // lowering has no known edges to narrow by.
    let mut callees: Vec<&Entry> = root
        .entries
        .values()
        .filter(|e| e.kind == FileKind::Function && e.soil.is_some() && e.rel != entry.rel)
        .collect();
    callees.sort_by_key(|e| nearness(&target_module, &e.rel));
    for callee in &callees {
        candidates.push(Candidate::one(
            format!("callees/{}.md", callee.tr.name),
            render_callee(callee),
        ));
    }

    // The corpus: the root's `.tr`/`.soil` pairs (the prelude after
    // plan 04), whole pairs at a time, nearest-first.
    for example in &callees {
        let tr = read_rel(root, &format!("{}.tr", example.rel))?;
        let soil = read_rel(root, &format!("{}.soil", example.rel))?;
        candidates.push(Candidate {
            rel: format!("examples/{}.tr", example.rel),
            contents: tr,
            companions: vec![(format!("examples/{}.soil", example.rel), soil)],
        });
    }

    // Module prose, lowest tier.
    if !target_module.is_empty() {
        let module_rel = format!("{target_module}/_module.tr");
        if root
            .entries
            .contains_key(&format!("{target_module}/_module"))
        {
            candidates.push(Candidate::one("module.md", read_rel(root, &module_rel)?));
        }
    }

    // ---- the prefix packer ----
    let mut mandatory_bytes = spec_md.len() + tests_json.len();
    if let Some(d) = &decisions_md {
        mandatory_bytes += d.len();
    }
    if tokens(mandatory_bytes) > budget {
        return Err(ErrorReport {
            errors: vec![registry::enrich(Diag::new(
                "budget-too-small",
                format!(
                    "budget {budget} cannot hold the never-dropped set \
                     (spec.md, decisions.md, tests.json ≈ {} tokens) — contract §5",
                    tokens(mandatory_bytes)
                ),
            ))],
        });
    }

    let mut files: Vec<(String, String)> = Vec::new();
    files.push(("spec.md".into(), spec_md));
    if let Some(d) = decisions_md {
        files.push(("decisions.md".into(), d));
    }
    files.push(("tests.json".into(), tests_json));

    let mut used = tokens(mandatory_bytes);
    let mut dropped: Vec<Dropped> = Vec::new();
    let mut over = false;
    for candidate in candidates {
        let cost = tokens(candidate.bytes());
        if !over && used + cost <= budget {
            used += cost;
            files.push((candidate.rel, candidate.contents));
            files.extend(candidate.companions);
        } else {
            // The first over-budget item ends the prefix: everything
            // after it drops too, keeping the packed set and the
            // estimate monotone in the budget.
            over = true;
            dropped.push(Dropped {
                item: candidate.rel,
                reason: "budget".into(),
            });
        }
    }

    // ---- the manifest (rendered once to measure, once final) ----
    let manifest_value = |estimate: u64| Manifest {
        definition: entry.rel.clone(),
        budget,
        estimate,
        packed: files.iter().map(|(rel, _)| rel.clone()).collect(),
        dropped: dropped.clone(),
        oracle: oracle_stanza.clone(),
        spec_hashes: SpecHashes {
            formal: entry.spec.formal.clone(),
            test: entry
                .spec
                .test
                .clone()
                .unwrap_or_else(|| entry.spec.formal.clone()),
            prose: entry.spec.prose.clone(),
        },
    };
    let packed_bytes: usize = files.iter().map(|(_, c)| c.len()).sum();
    let provisional = render_manifest(&manifest_value(0));
    let estimate = tokens(packed_bytes + provisional.len());
    let manifest = manifest_value(estimate);
    let rendered = render_manifest(&manifest);
    let manifest_json = serde_json::to_value(&manifest).expect("manifest serializes");

    let mut all = vec![("bundle.json".to_string(), rendered.clone())];
    all.extend(files);
    Ok(Bundle {
        files: all,
        manifest: manifest_json,
        manifest_text: rendered,
    })
}

fn render_manifest(manifest: &Manifest) -> String {
    let mut s = serde_json::to_string_pretty(manifest).expect("manifest serializes");
    s.push('\n');
    s
}

/// §4.3: the decisions in scope, project section first, then the
/// module's. `None` when no decisions exist in scope.
fn render_decisions(root: &Root, target_module: &str) -> Option<String> {
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    for entry in root.entries.values() {
        let scope = match entry.kind {
            FileKind::Project => String::new(),
            FileKind::Module => module_of(&entry.rel),
            _ => continue,
        };
        if !scope.is_empty() && scope != target_module {
            continue;
        }
        let Some(Payload::Decisions(entries)) = entry
            .tr
            .blocks
            .iter()
            .find(|b| matches!(b.payload, Payload::Decisions(_)))
            .map(|b| &b.payload)
        else {
            continue;
        };
        let heading = if scope.is_empty() {
            "## project".to_string()
        } else {
            format!("## module {scope}")
        };
        let mut lines = Vec::new();
        for d in entries {
            lines.push(format!("- **{}** — {}", d.label, d.text));
            for r in &d.rejected {
                lines.push(format!("  rejected: {r}"));
            }
        }
        sections.push((heading, lines));
    }
    if sections.is_empty() {
        return None;
    }
    // Project first.
    sections.sort_by_key(|(h, _)| h != "## project");
    let mut out = String::from("# Decisions\n");
    for (heading, lines) in sections {
        out.push('\n');
        out.push_str(&heading);
        out.push_str("\n\n");
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
    }
    Some(out)
}

/// §4.5: the soil0 §10 case shape minus the `program` key — every
/// sandboxed expect case (generated property inputs are not bundle
/// content; the property text lives in `spec.md`).
fn render_tests(entry: &Entry) -> String {
    let bundle = crate::testrun::build_bundle(entry);
    let mut value = serde_json::to_value(&bundle).expect("bundle serializes");
    value.as_object_mut().expect("object").remove("program");
    let mut s = serde_json::to_string_pretty(&value).expect("tests serialize");
    s.push('\n');
    s
}

/// §4.4: signatures only, never bodies; every clause annotated with
/// its lock assurance, `runtime` shown as demoted so the agent cannot
/// rely on an unproven claim.
fn render_callee(callee: &Entry) -> String {
    let mut out = format!("# {}\n", callee.tr.name);
    if let Some(sig) = callee.tr.blocks.iter().find(|b| b.id == "soil-sig") {
        out.push_str("\n```soil-sig\n");
        out.push_str(sig.content.trim_end());
        out.push_str("\n```\n");
    }
    let assurances: std::collections::BTreeMap<String, String> =
        match callee.lock.as_ref().and_then(|l| l.checks.as_ref()) {
            Some(Checks::Function { refinements, .. }) => refinements
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            _ => Default::default(),
        };
    let mut clause_lines = Vec::new();
    for block in &callee.tr.blocks {
        if block.id != "requires" && block.id != "ensures" {
            continue;
        }
        for line in block.content.lines() {
            let Some((label, pred)) = line.split_once(':') else {
                continue;
            };
            let (label, pred) = (label.trim(), pred.trim());
            if label.is_empty() {
                continue;
            }
            let assurance = assurances
                .get(label)
                .map(String::as_str)
                .unwrap_or("runtime");
            let note = match assurance {
                "proven" => "(proven)".to_string(),
                other => format!(
                    "({other} — demoted, guard active; do not rely on this claim \
                     when reasoning about your own obligations)"
                ),
            };
            clause_lines.push(format!("- {} `{label}` {note}: {pred}", block.id));
        }
    }
    if !clause_lines.is_empty() {
        out.push('\n');
        for line in clause_lines {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// Write the bundle under `out_dir` (created; wiped only when
/// `allow_wipe` — the daemon's own `.trellis/context/…` default).
pub fn write_to(bundle: &Bundle, out_dir: &Path, allow_wipe: bool) -> Result<(), ErrorReport> {
    if out_dir.exists() {
        let occupied = std::fs::read_dir(out_dir)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
        if occupied && !allow_wipe {
            return Err(ErrorReport::one(
                "usage",
                format!("--out {} exists and is not empty", out_dir.display()),
            ));
        }
        if occupied {
            std::fs::remove_dir_all(out_dir)
                .map_err(|e| ErrorReport::one("io", format!("{}: {e}", out_dir.display())))?;
        }
    }
    for (rel, contents) in &bundle.files {
        let path = out_dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ErrorReport::one("io", format!("{}: {e}", parent.display())))?;
        }
        std::fs::write(&path, contents)
            .map_err(|e| ErrorReport::one("io", format!("{}: {e}", path.display())))?;
    }
    Ok(())
}
