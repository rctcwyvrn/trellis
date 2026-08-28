//! The derived global manifest `soil.lock` (lock-schema §7): a pure
//! merge of the sidecars — never separately maintained — plus the
//! nodes and aggregates that only exist globally. Gitignored,
//! regenerated on demand.

use std::collections::BTreeMap;

use crate::diag::ErrorReport;
use crate::lock::{Lock, Oracle};

pub struct Manifest {
    /// Definition path (root-relative, extensionless — e.g.
    /// `csvstats/median`) → its sidecar.
    pub definitions: BTreeMap<String, Lock>,
    /// Derived from every `lowering.private_helpers` list; a helper
    /// with no remaining owner simply never appears (the GC).
    pub soil_private: Vec<PrivateNode>,
    /// Definition path → its `allow` hatches (the audit view).
    pub escape_hatches: BTreeMap<String, Vec<String>>,
    /// Package name → hash, from `soil.toml` (empty until plan 04).
    pub trusted_packages: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrivateNode {
    /// `module/_name`.
    pub name: String,
    pub hash: String,
    pub owners: Vec<String>,
}

/// Merge sidecars into the manifest, enforcing the cross-lock
/// invariants (lock-schema §8): only `accepted` definitions may
/// appear in another entry's `oracles`.
pub fn merge(
    entries: Vec<(String, Lock)>,
    trusted_packages: BTreeMap<String, String>,
) -> Result<Manifest, ErrorReport> {
    let definitions: BTreeMap<String, Lock> = entries.into_iter().collect();

    // Invariant 3: definition oracles must be accepted.
    for (path, lock) in &definitions {
        for oracle in lock.oracles.as_deref().unwrap_or(&[]) {
            if let Oracle::Definition { name, .. } = oracle {
                let accepted = definitions
                    .iter()
                    .any(|(p, l)| l.accepted && (p == name || p.ends_with(&format!("/{name}"))));
                if !accepted {
                    return Err(ErrorReport::one(
                        "lock-oracle-unaccepted",
                        format!(
                            "{path} uses `{name}` as a test oracle, but no accepted \
                             definition of that name exists (lock-schema §8)"
                        ),
                    ));
                }
            }
        }
    }

    // soil_private: helper nodes with their owners, keyed by
    // module-qualified name; ownerless helpers never materialize.
    let mut private: BTreeMap<(String, String), PrivateNode> = BTreeMap::new();
    let mut escape_hatches = BTreeMap::new();
    for (path, lock) in &definitions {
        let module = match path.rsplit_once('/') {
            Some((module, _)) => module.to_string(),
            None => String::new(),
        };
        if let Some(lowering) = &lock.lowering {
            for helper in &lowering.private_helpers {
                let qualified = if module.is_empty() {
                    helper.name.clone()
                } else {
                    format!("{module}/{}", helper.name)
                };
                let node = private
                    .entry((qualified.clone(), helper.hash.clone()))
                    .or_insert_with(|| PrivateNode {
                        name: qualified,
                        hash: helper.hash.clone(),
                        owners: Vec::new(),
                    });
                node.owners.push(path.clone());
            }
        }
        if !lock.spec.escape_hatches.is_empty() {
            escape_hatches.insert(path.clone(), lock.spec.escape_hatches.clone());
        }
    }

    Ok(Manifest {
        definitions,
        soil_private: private.into_values().collect(),
        escape_hatches,
        trusted_packages,
    })
}

/// The manifest's surface form (same style as the sidecars).
pub fn write(manifest: &Manifest) -> String {
    let mut out = String::new();
    out.push_str("{\n  \"lock_format\": 1,\n  \"definitions\": {\n");
    for (i, (path, lock)) in manifest.definitions.iter().enumerate() {
        // Each sidecar embeds indented two levels.
        let body = crate::lock::write(lock);
        let indented: String = body
            .trim_end()
            .lines()
            .enumerate()
            .map(|(n, line)| {
                if n == 0 {
                    line.to_string()
                } else {
                    format!("    {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        out.push_str(&format!(
            "    {}: {indented}{}\n",
            serde_json::to_string(path).expect("path serializes"),
            if i + 1 < manifest.definitions.len() {
                ","
            } else {
                ""
            }
        ));
    }
    out.push_str("  },\n  \"soil_private\": ");
    if manifest.soil_private.is_empty() {
        out.push_str("[]");
    } else {
        out.push_str("[\n");
        for (i, node) in manifest.soil_private.iter().enumerate() {
            let owners = node
                .owners
                .iter()
                .map(|o| serde_json::to_string(o).expect("owner"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "    {{ \"name\": {}, \"hash\": {}, \"owners\": [{owners}] }}{}\n",
                serde_json::to_string(&node.name).expect("name"),
                serde_json::to_string(&node.hash).expect("hash"),
                if i + 1 < manifest.soil_private.len() {
                    ","
                } else {
                    ""
                }
            ));
        }
        out.push_str("  ]");
    }
    out.push_str(",\n  \"escape_hatches\": ");
    if manifest.escape_hatches.is_empty() {
        out.push_str("{}");
    } else {
        out.push_str("{\n");
        for (i, (path, hatches)) in manifest.escape_hatches.iter().enumerate() {
            let list = hatches
                .iter()
                .map(|h| serde_json::to_string(h).expect("hatch"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "    {}: [{list}]{}\n",
                serde_json::to_string(path).expect("path"),
                if i + 1 < manifest.escape_hatches.len() {
                    ","
                } else {
                    ""
                }
            ));
        }
        out.push_str("  }");
    }
    out.push_str(",\n  \"trusted_packages\": ");
    if manifest.trusted_packages.is_empty() {
        out.push_str("{}");
    } else {
        out.push_str("{\n");
        for (i, (name, hash)) in manifest.trusted_packages.iter().enumerate() {
            out.push_str(&format!(
                "    {}: {}{}\n",
                serde_json::to_string(name).expect("name"),
                serde_json::to_string(hash).expect("hash"),
                if i + 1 < manifest.trusted_packages.len() {
                    ","
                } else {
                    ""
                }
            ));
        }
        out.push_str("  }");
    }
    out.push_str("\n}\n");
    out
}
