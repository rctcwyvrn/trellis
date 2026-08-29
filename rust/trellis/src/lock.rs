//! Lock sidecars (impl plan 03 step 5): the model per
//! `docs/lock-schema.md`, the canonical writer (approved micro-pin
//! §8.7: 2-space pretty JSON, key order per the schema tables,
//! one-object-per-line rows, one trailing newline), a strict reader,
//! and the schema invariants. The regeneration round-trip
//! (read → write byte-identical) is the conformance gate.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::diag::ErrorReport;

pub const LOCK_FORMAT: u64 = 1;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lock {
    pub lock_format: u64,
    pub name: String,
    /// `function` | `type` | `module` | `project` (lock-schema §1;
    /// `project` added with tr-grammar's project headers).
    pub kind: String,
    pub versions: Versions,
    pub spec: Spec,
    /// Function entries only; `null` before the first lowering.
    #[serde(default)]
    pub lowering: Option<Lowering>,
    #[serde(default)]
    pub checks: Option<Checks>,
    #[serde(default)]
    pub tests: Option<Vec<TestRow>>,
    #[serde(default)]
    pub oracles: Option<Vec<Oracle>>,
    /// Type entries only; `null` when acyclic.
    #[serde(default)]
    pub cycle_hash: Option<String>,
    pub accepted: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Versions {
    pub trellis: String,
    pub soil: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    /// `human` | `agent` — the vibing tier (design §1.2).
    pub provenance: String,
    pub hashes: Hashes,
    /// `fresh` | `review-suggested`.
    pub prose_state: String,
    pub blocks: Vec<BlockProv>,
    pub pinned: bool,
    pub escape_hatches: Vec<String>,
    /// Module/project entries only: per-entry decision hashes
    /// (lock-schema §2, resolved 2026-08-24).
    #[serde(default)]
    pub decisions: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hashes {
    pub formal: String,
    /// `null` for type, module, and project files.
    #[serde(default)]
    pub test: Option<String>,
    pub prose: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockProv {
    pub block: String,
    /// `human` | `agent`.
    pub author: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lowering {
    pub soil_hash: String,
    /// `agent` | `human-verified` | `hand-edited` | `prelude-fork`.
    pub provenance: String,
    /// `null` unless an agent actually ran (resolved 2026-08-28).
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub private_helpers: Vec<NamedHash>,
    pub calls: Vec<NamedHash>,
    /// Reliance edges (lock-schema §3, resolved 2026-08-24).
    pub decisions: LoweringDecisions,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedHash {
    pub name: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoweringDecisions {
    pub scope_hash: String,
    pub applied: Vec<AppliedDecision>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedDecision {
    /// `project` | `module`.
    pub scope: String,
    pub label: String,
    pub hash: String,
}

/// The kind-shaped `checks` section (lock-schema §4; untagged — the
/// key sets are disjoint).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Checks {
    Function {
        types: String,
        termination: String,
        refinements: BTreeMap<String, String>,
        holes: u64,
    },
    Type {
        types: String,
        invariants: BTreeMap<String, String>,
    },
    Module {
        exports: String,
    },
}

#[derive(Debug, Clone, PartialEq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct TestRow {
    pub name: String,
    /// lock-schema §5 tiers.
    pub tier: String,
    /// `sandboxed` | `real`.
    pub mode: String,
    /// `spec` | `derived`.
    pub origin: String,
    /// `pass` | `fail` | `xfail` | `xpass`.
    pub result: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Oracle {
    Reference {
        kind: String, // "reference"
        path: String,
        hash: String,
    },
    Definition {
        kind: String, // "definition"
        name: String,
        formal_hash: String,
    },
    Cli {
        kind: String, // "cli"
        command: String,
        hash: String,
    },
}

// ---- reading ----

pub fn read(text: &str) -> Result<Lock, ErrorReport> {
    let lock: Lock = serde_json::from_str(text)
        .map_err(|e| ErrorReport::one("lock-malformed", format!("lock does not parse: {e}")))?;
    validate(&lock)?;
    Ok(lock)
}

/// The lock-schema §8 invariants a single sidecar can check on its
/// own (cross-lock invariants — oracle acceptance — live in the
/// manifest merge).
pub fn validate(lock: &Lock) -> Result<(), ErrorReport> {
    let fail = |code: &str, msg: String| Err(ErrorReport::one(code, msg));
    if lock.lock_format != LOCK_FORMAT {
        return fail(
            "lock-format",
            format!("lock_format {} is not {LOCK_FORMAT}", lock.lock_format),
        );
    }
    match (lock.kind.as_str(), &lock.checks) {
        ("function", None | Some(Checks::Function { .. }))
        | ("type", None | Some(Checks::Type { .. }))
        | ("module", None | Some(Checks::Module { .. }))
        | ("project", None) => {}
        (kind, _) => {
            return fail(
                "lock-kind-shape",
                format!("checks section does not match kind `{kind}`"),
            )
        }
    }
    if lock.kind != "function" && lock.lowering.is_some() {
        return fail(
            "lock-kind-shape",
            format!("a {} entry cannot carry a lowering", lock.kind),
        );
    }
    if lock.kind != "type" && lock.cycle_hash.is_some() {
        return fail("lock-kind-shape", "cycle_hash is type-only".into());
    }
    // Invariant 1 + 4 (function entries): accepted requires tested —
    // types ok, no holes, every result pass (xfail blocks accepted).
    if lock.accepted && lock.kind == "function" {
        let (types_ok, holes) = match &lock.checks {
            Some(Checks::Function { types, holes, .. }) => (types == "ok", *holes),
            _ => (false, 0),
        };
        let rows = lock.tests.as_deref().unwrap_or(&[]);
        let all_pass = rows.iter().all(|r| r.result == "pass");
        if !types_ok || holes > 0 || !all_pass {
            return fail(
                "lock-accepted-untested",
                "accepted requires typed (no holes) and every result pass (lock-schema §8)".into(),
            );
        }
    }
    Ok(())
}

// ---- the canonical writer (micro-pin §8.7) ----

fn json_str(s: &str) -> String {
    serde_json::to_string(s).expect("string serializes")
}

fn row_named(nh: &NamedHash) -> String {
    format!(
        "{{ \"name\": {}, \"hash\": {} }}",
        json_str(&nh.name),
        json_str(&nh.hash)
    )
}

fn write_rows<T>(out: &mut String, indent: &str, rows: &[T], render: impl Fn(&T) -> String) {
    if rows.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push_str("[\n");
    for (i, row) in rows.iter().enumerate() {
        out.push_str(indent);
        out.push_str("  ");
        out.push_str(&render(row));
        out.push_str(if i + 1 < rows.len() { ",\n" } else { "\n" });
    }
    out.push_str(indent);
    out.push(']');
}

fn write_map(out: &mut String, indent: &str, map: &BTreeMap<String, String>) {
    if map.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push_str("{\n");
    for (i, (k, v)) in map.iter().enumerate() {
        out.push_str(indent);
        out.push_str("  ");
        out.push_str(&format!("{}: {}", json_str(k), json_str(v)));
        out.push_str(if i + 1 < map.len() { ",\n" } else { "\n" });
    }
    out.push_str(indent);
    out.push('}');
}

fn opt_str(v: &Option<String>) -> String {
    match v {
        Some(s) => json_str(s),
        None => "null".into(),
    }
}

pub fn write(lock: &Lock) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"lock_format\": {},\n", lock.lock_format));
    out.push_str(&format!("  \"name\": {},\n", json_str(&lock.name)));
    out.push_str(&format!("  \"kind\": {},\n", json_str(&lock.kind)));
    out.push_str(&format!(
        "  \"versions\": {{ \"trellis\": {}, \"soil\": {} }},\n",
        json_str(&lock.versions.trellis),
        json_str(&lock.versions.soil)
    ));

    // spec
    out.push_str("  \"spec\": {\n");
    out.push_str(&format!(
        "    \"provenance\": {},\n",
        json_str(&lock.spec.provenance)
    ));
    out.push_str("    \"hashes\": {\n");
    out.push_str(&format!(
        "      \"formal\": {},\n",
        json_str(&lock.spec.hashes.formal)
    ));
    out.push_str(&format!(
        "      \"test\": {},\n",
        opt_str(&lock.spec.hashes.test)
    ));
    out.push_str(&format!(
        "      \"prose\": {}\n",
        json_str(&lock.spec.hashes.prose)
    ));
    out.push_str("    },\n");
    out.push_str(&format!(
        "    \"prose_state\": {},\n",
        json_str(&lock.spec.prose_state)
    ));
    out.push_str("    \"blocks\": ");
    write_rows(&mut out, "    ", &lock.spec.blocks, |b| {
        format!(
            "{{ \"block\": {}, \"author\": {} }}",
            json_str(&b.block),
            json_str(&b.author)
        )
    });
    out.push_str(",\n");
    out.push_str(&format!("    \"pinned\": {},\n", lock.spec.pinned));
    out.push_str("    \"escape_hatches\": ");
    write_rows(&mut out, "    ", &lock.spec.escape_hatches, |h| json_str(h));
    if let Some(decisions) = &lock.spec.decisions {
        out.push_str(",\n    \"decisions\": ");
        write_map(&mut out, "    ", decisions);
    }
    out.push_str("\n  },\n");

    // lowering (function entries always carry the key)
    if lock.kind == "function" {
        match &lock.lowering {
            None => out.push_str("  \"lowering\": null,\n"),
            Some(l) => {
                out.push_str("  \"lowering\": {\n");
                out.push_str(&format!("    \"soil_hash\": {},\n", json_str(&l.soil_hash)));
                out.push_str(&format!(
                    "    \"provenance\": {},\n",
                    json_str(&l.provenance)
                ));
                out.push_str(&format!("    \"provider\": {},\n", opt_str(&l.provider)));
                out.push_str(&format!("    \"model\": {},\n", opt_str(&l.model)));
                out.push_str("    \"private_helpers\": ");
                write_rows(&mut out, "    ", &l.private_helpers, row_named);
                out.push_str(",\n    \"calls\": ");
                write_rows(&mut out, "    ", &l.calls, row_named);
                out.push_str(",\n    \"decisions\": {\n");
                out.push_str(&format!(
                    "      \"scope_hash\": {},\n",
                    json_str(&l.decisions.scope_hash)
                ));
                out.push_str("      \"applied\": ");
                write_rows(&mut out, "      ", &l.decisions.applied, |a| {
                    format!(
                        "{{ \"scope\": {}, \"label\": {}, \"hash\": {} }}",
                        json_str(&a.scope),
                        json_str(&a.label),
                        json_str(&a.hash)
                    )
                });
                out.push_str("\n    }\n  },\n");
            }
        }
    }

    // checks
    match &lock.checks {
        None => {}
        Some(Checks::Function {
            types,
            termination,
            refinements,
            holes,
        }) => {
            out.push_str("  \"checks\": {\n");
            out.push_str(&format!("    \"types\": {},\n", json_str(types)));
            out.push_str(&format!(
                "    \"termination\": {},\n",
                json_str(termination)
            ));
            out.push_str("    \"refinements\": ");
            write_map(&mut out, "    ", refinements);
            out.push_str(&format!(",\n    \"holes\": {holes}\n  }},\n"));
        }
        Some(Checks::Type { types, invariants }) => {
            out.push_str("  \"checks\": {\n");
            out.push_str(&format!("    \"types\": {},\n", json_str(types)));
            out.push_str("    \"invariants\": ");
            write_map(&mut out, "    ", invariants);
            out.push_str("\n  },\n");
        }
        Some(Checks::Module { exports }) => {
            out.push_str(&format!(
                "  \"checks\": {{\n    \"exports\": {}\n  }},\n",
                json_str(exports)
            ));
        }
    }

    // tests
    if let Some(tests) = &lock.tests {
        out.push_str("  \"tests\": ");
        write_rows(&mut out, "  ", tests, |t| {
            format!(
                "{{ \"name\": {}, \"tier\": {}, \"mode\": {}, \"origin\": {}, \"result\": {} }}",
                json_str(&t.name),
                json_str(&t.tier),
                json_str(&t.mode),
                json_str(&t.origin),
                json_str(&t.result)
            )
        });
        out.push_str(",\n");
    }

    // oracles
    if let Some(oracles) = &lock.oracles {
        out.push_str("  \"oracles\": ");
        write_rows(&mut out, "  ", oracles, |o| match o {
            Oracle::Reference { kind, path, hash } => format!(
                "{{ \"kind\": {}, \"path\": {}, \"hash\": {} }}",
                json_str(kind),
                json_str(path),
                json_str(hash)
            ),
            Oracle::Definition {
                kind,
                name,
                formal_hash,
            } => format!(
                "{{ \"kind\": {}, \"name\": {}, \"formal_hash\": {} }}",
                json_str(kind),
                json_str(name),
                json_str(formal_hash)
            ),
            Oracle::Cli {
                kind,
                command,
                hash,
            } => format!(
                "{{ \"kind\": {}, \"command\": {}, \"hash\": {} }}",
                json_str(kind),
                json_str(command),
                json_str(hash)
            ),
        });
        out.push_str(",\n");
    }

    // cycle_hash (type entries always carry the key)
    if lock.kind == "type" {
        out.push_str(&format!(
            "  \"cycle_hash\": {},\n",
            opt_str(&lock.cycle_hash)
        ));
    }

    out.push_str(&format!("  \"accepted\": {}\n", lock.accepted));
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_function() -> Lock {
        Lock {
            lock_format: 1,
            name: "f".into(),
            kind: "function".into(),
            versions: Versions {
                trellis: "0.1.0".into(),
                soil: "0.1".into(),
            },
            spec: Spec {
                provenance: "human".into(),
                hashes: Hashes {
                    formal: "sha256:aa".into(),
                    test: Some("sha256:bb".into()),
                    prose: "sha256:cc".into(),
                },
                prose_state: "fresh".into(),
                blocks: vec![BlockProv {
                    block: "test t".into(),
                    author: "human".into(),
                }],
                pinned: false,
                escape_hatches: vec![],
                decisions: None,
            },
            lowering: None,
            checks: None,
            tests: Some(vec![]),
            oracles: Some(vec![]),
            cycle_hash: None,
            accepted: false,
        }
    }

    #[test]
    fn round_trips_byte_identically() {
        let text = write(&minimal_function());
        let back = read(&text).unwrap();
        assert_eq!(write(&back), text);
    }

    #[test]
    fn lowering_round_trips() {
        let mut lock = minimal_function();
        lock.lowering = Some(Lowering {
            soil_hash: "sha256:aa".into(),
            provenance: "human-verified".into(),
            provider: None,
            model: None,
            private_helpers: vec![],
            calls: vec![],
            decisions: LoweringDecisions {
                scope_hash: "sha256:bb".into(),
                applied: vec![],
            },
        });
        let text = write(&lock);
        let back = read(&text).unwrap_or_else(|e| panic!("{}\n{text}", e.render()));
        assert_eq!(write(&back), text);
    }

    #[test]
    fn accepted_gating_enforced() {
        let mut lock = minimal_function();
        lock.accepted = true; // no checks -> not typed
        let err = validate(&lock).unwrap_err();
        assert_eq!(err.errors[0].code, "lock-accepted-untested");

        lock.checks = Some(Checks::Function {
            types: "ok".into(),
            termination: "verified".into(),
            refinements: BTreeMap::new(),
            holes: 0,
        });
        validate(&lock).unwrap();

        lock.tests = Some(vec![TestRow {
            name: "t#1".into(),
            tier: "expect".into(),
            mode: "sandboxed".into(),
            origin: "spec".into(),
            result: "xfail".into(),
        }]);
        let err = validate(&lock).unwrap_err();
        assert_eq!(err.errors[0].code, "lock-accepted-untested");
    }

    #[test]
    fn kind_shapes_enforced() {
        // A module entry cannot carry a lowering.
        let mut lock = minimal_function();
        lock.kind = "module".into();
        lock.lowering = Some(Lowering {
            soil_hash: "sha256:aa".into(),
            provenance: "human-verified".into(),
            provider: None,
            model: None,
            private_helpers: vec![],
            calls: vec![],
            decisions: LoweringDecisions {
                scope_hash: "sha256:bb".into(),
                applied: vec![],
            },
        });
        let err = validate(&lock).unwrap_err();
        assert_eq!(err.errors[0].code, "lock-kind-shape");

        // cycle_hash is type-only.
        let mut lock = minimal_function();
        lock.cycle_hash = Some("sha256:cc".into());
        let err = validate(&lock).unwrap_err();
        assert_eq!(err.errors[0].code, "lock-kind-shape");

        // Mismatched checks shape.
        let mut lock = minimal_function();
        lock.checks = Some(Checks::Module {
            exports: "ok".into(),
        });
        let err = validate(&lock).unwrap_err();
        assert_eq!(err.errors[0].code, "lock-kind-shape");
    }
}
