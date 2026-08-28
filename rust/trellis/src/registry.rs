//! The diagnostic registry (contract §6, adopted from design §4.6):
//! `registry/diagnostics.json` is the single source mapping every
//! stable code to a typed repair class and a `spec_ref` anchor. The
//! *format* is contractual; the *contents* grow additively under the
//! drift gate (the coverage test in `tests/state_tests.rs` is its
//! first half).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::diag::Diag;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub registry_format: u64,
    pub repair_classes: Vec<RepairClass>,
    pub codes: Vec<CodeEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairClass {
    pub name: String,
    pub summary: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeEntry {
    pub code: String,
    pub source: String,
    pub repair_class: String,
    pub spec_ref: String,
}

const REGISTRY_JSON: &str = include_str!("../registry/diagnostics.json");

pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let registry: Registry =
            serde_json::from_str(REGISTRY_JSON).expect("registry/diagnostics.json parses");
        let classes: Vec<&str> = registry
            .repair_classes
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        for entry in &registry.codes {
            assert!(
                classes.contains(&entry.repair_class.as_str()),
                "registry: code `{}` names unknown repair class `{}`",
                entry.code,
                entry.repair_class
            );
        }
        registry
    })
}

pub fn lookup(code: &str) -> Option<&'static CodeEntry> {
    static BY_CODE: OnceLock<BTreeMap<&'static str, &'static CodeEntry>> = OnceLock::new();
    BY_CODE
        .get_or_init(|| {
            registry()
                .codes
                .iter()
                .map(|e| (e.code.as_str(), e))
                .collect()
        })
        .get(code)
        .copied()
}

/// Attach `repair_class` and `spec_ref` from the registry — every
/// diagnostic leaves the daemon through this (contract §6).
pub fn enrich(mut diag: Diag) -> Diag {
    if let Some(entry) = lookup(&diag.code) {
        diag.repair_class = Some(entry.repair_class.clone());
        diag.spec_ref = Some(entry.spec_ref.clone());
    }
    diag
}
