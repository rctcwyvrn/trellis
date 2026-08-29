//! Incremental state (impl plan 03 step 6): scan a Soil root, run the
//! static pipeline through the linked soil0, compute content-addressed
//! hashes, derive the status ladder and every staleness flag from the
//! lock-schema §8 invalidation table, and apply the refresh writes.
//! Explicit refresh (plan 03 resolved): hashes are re-checked at
//! request boundaries; nothing watches.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::Config;
use crate::diag::{Diag, ErrorReport};
use crate::hash;
use crate::lock::{self, Lock};
use crate::registry;
use crate::trfile::{self, FileKind, Payload, TrFile};

pub struct Root {
    pub root: PathBuf,
    /// Definition path (root-relative, extensionless) → entry.
    pub entries: BTreeMap<String, Entry>,
    /// Module → `_private.soil` source.
    pub privates: BTreeMap<String, String>,
}

pub struct Entry {
    pub rel: String,
    pub kind: FileKind,
    pub tr: TrFile,
    pub spec: hash::SpecHashes,
    pub lock: Option<Lock>,
    pub soil: Option<String>,
}

fn module_of(rel: &str) -> String {
    rel.rsplit_once('/')
        .map(|(m, _)| m.to_string())
        .unwrap_or_default()
}

fn soil0_diag(d: &soil0::diag::Diagnostic) -> Diag {
    let mut diag = Diag::new(d.code.as_str(), d.message.clone());
    diag.file = d.file.0.clone();
    diag.span = d.span.0.map(|s| crate::diag::Span {
        start: s.start,
        end: s.end,
        line: s.line,
        col: s.col,
    });
    registry::enrich(diag)
}

pub fn scan(root: &Path, config: &Config) -> Result<Root, ErrorReport> {
    let vocabulary: BTreeSet<String> = config.tags.keys().cloned().collect();
    let mut entries = BTreeMap::new();
    let mut privates = BTreeMap::new();
    let mut errors: Vec<Diag> = Vec::new();

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let listing = std::fs::read_dir(&dir)
            .map_err(|e| ErrorReport::one("io", format!("cannot read {}: {e}", dir.display())))?;
        for item in listing {
            let path = item
                .map_err(|e| ErrorReport::one("io", format!("scan: {e}")))?
                .path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if path.is_dir() {
                if !name.starts_with('.') && name != "target" {
                    stack.push(path);
                }
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .expect("under root")
                .to_string_lossy()
                .into_owned();
            if name == "_private.soil" {
                let src = std::fs::read_to_string(&path)
                    .map_err(|e| ErrorReport::one("io", format!("{rel}: {e}")))?;
                privates.insert(module_of(&rel), src);
                continue;
            }
            if path.extension().is_none_or(|e| e != "tr") {
                continue;
            }
            let src = std::fs::read_to_string(&path)
                .map_err(|e| ErrorReport::one("io", format!("{rel}: {e}")))?;
            match trfile::parse_tr(&src, &path, Some(&vocabulary)) {
                Err(report) => errors.extend(report.errors),
                Ok(tr) => {
                    let spec = hash::spec_hashes(&src, &tr);
                    let rel_stem = rel.trim_end_matches(".tr").to_string();
                    let lock = match std::fs::read_to_string(path.with_extension("lock")) {
                        Err(_) => None,
                        Ok(text) => match lock::read(&text) {
                            Ok(lock) => Some(lock),
                            Err(report) => {
                                errors.extend(report.errors.into_iter().map(|mut d| {
                                    d.file = Some(format!("{rel_stem}.lock"));
                                    d
                                }));
                                None
                            }
                        },
                    };
                    let soil = std::fs::read_to_string(path.with_extension("soil")).ok();
                    entries.insert(
                        rel_stem.clone(),
                        Entry {
                            rel: rel_stem,
                            kind: tr.kind,
                            tr,
                            spec,
                            lock,
                            soil,
                        },
                    );
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(ErrorReport {
            errors: errors.into_iter().map(registry::enrich).collect(),
        });
    }
    Ok(Root {
        root: root.to_path_buf(),
        entries,
        privates,
    })
}

// ---- the program: dependency order, env, soil0 pipeline ----

pub struct Compiled {
    pub program: soil0::manifest::Program,
    /// Definition name (bare for functions, `module/_name` for
    /// privates) → reference sets from rename.
    pub refs: BTreeMap<String, soil0::rename::RefSets>,
    /// Same keys → computed content-addressed hash (design §6.1).
    pub soil_hashes: BTreeMap<String, String>,
}

/// The assembled program inputs: the environment plus the sources in
/// dependency order — shared by `compile` and the test runner (which
/// appends synthesized wrapper files, step 7).
pub struct Assembly {
    pub env: soil0::manifest::EnvFile,
    /// (rel `.soil` path, source), callee-first.
    pub ordered: Vec<(String, String)>,
}

/// Build the environment from the root's type entries.
pub fn env_of_root(root: &Root) -> Result<soil0::manifest::EnvFile, ErrorReport> {
    let typedefs: Vec<&trfile::typedef::TypeDef> = root
        .entries
        .values()
        .filter_map(|e| {
            e.tr.blocks.iter().find_map(|b| match &b.payload {
                Payload::TypeDef(td) => Some(td),
                _ => None,
            })
        })
        .collect();
    crate::envgen::env_of(&typedefs).map_err(|d| ErrorReport {
        errors: vec![registry::enrich(d)],
    })
}

/// Assemble and rename the root's program; compute every definition's
/// `soil_hash` bottom-up.
pub fn compile(root: &Root) -> Result<Compiled, ErrorReport> {
    let Assembly { env, ordered } = assemble(root)?;
    compile_assembly(root, env, ordered)
}

/// Order the root's sources callee-first (the daemon owns the tree —
/// contract §7) and build the environment.
pub fn assemble(root: &Root) -> Result<Assembly, ErrorReport> {
    let env = env_of_root(root)?;

    // The files: every lowered function plus the private files.
    let mut sources: Vec<(String, String)> = Vec::new(); // (rel .soil path, src)
    for entry in root.entries.values() {
        if let Some(soil) = &entry.soil {
            sources.push((format!("{}.soil", entry.rel), soil.clone()));
        }
    }
    for (module, src) in &root.privates {
        let rel = if module.is_empty() {
            "_private.soil".to_string()
        } else {
            format!("{module}/_private.soil")
        };
        sources.push((rel, src.clone()));
    }

    // Per-file surface references (a light pre-pass; rename is the
    // authority afterwards, but needs callee-first order to run).
    let mut file_defs: BTreeMap<String, Vec<String>> = BTreeMap::new(); // file -> def names
    let mut def_names: BTreeSet<String> = BTreeSet::new();
    let mut parsed: BTreeMap<String, soil0::ast::File> = BTreeMap::new();
    for (rel, src) in &sources {
        let file = soil0::parser::parse_file(src, rel).map_err(|d| ErrorReport {
            errors: vec![soil0_diag(&d)],
        })?;
        let names: Vec<String> = file.defs.iter().map(|d| d.item.name.clone()).collect();
        def_names.extend(names.iter().cloned());
        file_defs.insert(rel.clone(), names);
        parsed.insert(rel.clone(), file);
    }
    let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new(); // file -> files
    let name_to_file: BTreeMap<String, String> = file_defs
        .iter()
        .flat_map(|(f, names)| names.iter().map(move |n| (n.clone(), f.clone())))
        .collect();
    for (rel, file) in &parsed {
        let mut used = BTreeSet::new();
        for def in &file.defs {
            surface_names(&def.item, &mut used);
        }
        let entry = deps.entry(rel.clone()).or_default();
        for name in used {
            if let Some(target) = name_to_file.get(&name) {
                if target != rel {
                    entry.insert(target.clone());
                }
            }
        }
    }
    let order = topo_sort(&deps).map_err(|cycle| ErrorReport {
        errors: vec![registry::enrich(Diag::new(
            "state-cycle",
            format!(
                "definitions are mutually recursive across files: {} (design §3.8)",
                cycle.join(" -> ")
            ),
        ))],
    })?;
    let by_rel: BTreeMap<&str, &str> = sources
        .iter()
        .map(|(r, s)| (r.as_str(), s.as_str()))
        .collect();
    let ordered: Vec<(String, String)> = order
        .iter()
        .map(|rel| (rel.clone(), by_rel[rel.as_str()].to_string()))
        .collect();
    Ok(Assembly { env, ordered })
}

fn compile_assembly(
    root: &Root,
    env: soil0::manifest::EnvFile,
    ordered: Vec<(String, String)>,
) -> Result<Compiled, ErrorReport> {
    let program = soil0::manifest::from_parts(env, &ordered).map_err(|d| ErrorReport {
        errors: vec![soil0_diag(&d)],
    })?;
    let rename = soil0::rename::rename_program(&program).map_err(|d| ErrorReport {
        errors: vec![soil0_diag(&d)],
    })?;

    // Reference sets keyed like the manifest's defs.
    let mut refs: BTreeMap<String, soil0::rename::RefSets> = BTreeMap::new();
    let mut key_of: BTreeMap<String, String> = BTreeMap::new(); // bare/priv name -> key
    for (def, out) in program.defs.iter().zip(rename.defs) {
        let module = module_of(&def.path);
        let key = if out.name.starts_with('_') {
            if module.is_empty() {
                out.name.clone()
            } else {
                format!("{module}/{}", out.name)
            }
        } else {
            out.name.clone()
        };
        key_of.insert(out.name.clone(), key.clone());
        refs.insert(key, out.refs);
    }

    // Hashes bottom-up over the manifest order (callee-first).
    let mut forms: BTreeMap<String, String> = BTreeMap::new(); // key -> hash form
    for (rel, src) in &ordered {
        for (name, form) in
            soil0::hashform::hash_forms_per_def(src, rel).map_err(|d| ErrorReport {
                errors: vec![soil0_diag(&d)],
            })?
        {
            forms.insert(key_of[&name].clone(), form);
        }
    }
    let type_formal: BTreeMap<&str, &str> = root
        .entries
        .values()
        .filter(|e| e.kind == FileKind::Type)
        .map(|e| (e.tr.name.as_str(), e.spec.formal.as_str()))
        .collect();
    let mut soil_hashes: BTreeMap<String, String> = BTreeMap::new();
    for def in &program.defs {
        let module = module_of(&def.path);
        let bare = def.name().to_string();
        let key = key_of[&bare].clone();
        let sets = &refs[&key];
        let mut edges: Vec<hash::HashRef> = Vec::new();
        for name in &sets.defs {
            edges.push((name.clone(), soil_hashes[&key_of[name]].clone()));
        }
        for name in &sets.privates {
            let pkey = if module.is_empty() {
                name.clone()
            } else {
                format!("{module}/{name}")
            };
            edges.push((name.clone(), soil_hashes[&pkey].clone()));
        }
        for name in &sets.builtins {
            edges.push((name.clone(), hash::builtin_referent()));
        }
        for name in &sets.types {
            // User types edge by formal_hash; kernel types are
            // contract surface and edge nowhere (step-5 rule).
            if let Some(formal) = type_formal.get(name.as_str()) {
                edges.push((name.clone(), (*formal).to_string()));
            }
        }
        soil_hashes.insert(key.clone(), hash::soil_hash(&forms[&key], &edges));
    }

    Ok(Compiled {
        program,
        refs,
        soil_hashes,
    })
}

fn surface_names(def: &soil0::ast::Def, out: &mut BTreeSet<String>) {
    fn walk(e: &soil0::ast::Expr, out: &mut BTreeSet<String>) {
        use soil0::ast::Expr::*;
        match e {
            Path { root, .. } => {
                out.insert(root.clone());
            }
            Qualified { name, .. } => {
                out.insert(name.clone());
            }
            Let { bindings, body, .. } => {
                for b in bindings {
                    walk(&b.value.item, out);
                }
                walk(&body.item, out);
            }
            Fun { body, .. } => walk(&body.item, out),
            If {
                cond,
                then_branch,
                else_branch,
            } => {
                walk(&cond.item, out);
                walk(&then_branch.item, out);
                walk(&else_branch.item, out);
            }
            Match { scrutinee, arms } => {
                walk(&scrutinee.item, out);
                for arm in arms {
                    walk(&arm.item.body.item, out);
                }
            }
            OrE { lhs, rhs }
            | AndE { lhs, rhs }
            | Cmp { lhs, rhs, .. }
            | Arith { lhs, rhs, .. } => {
                walk(&lhs.item, out);
                walk(&rhs.item, out);
            }
            NotE { operand } | Neg { operand } => walk(&operand.item, out),
            App { r#fn, arg } => {
                walk(&r#fn.item, out);
                walk(&arg.item, out);
            }
            RecordE { update, fields } => {
                if let soil0::diag::Opt(Some(base)) = update {
                    out.insert(base.root.clone());
                }
                for f in fields {
                    walk(&f.value.item, out);
                }
            }
            Annot { expr, .. } => walk(&expr.item, out),
            CtorE { .. } | Literal { .. } | Hole { .. } => {}
        }
    }
    walk(&def.body.item, out);
    if let soil0::diag::Opt(Some(d)) = &def.decreases {
        walk(&d.item, out);
    }
}

fn topo_sort(deps: &BTreeMap<String, BTreeSet<String>>) -> Result<Vec<String>, Vec<String>> {
    let mut order = Vec::new();
    let mut state: BTreeMap<&str, u8> = BTreeMap::new(); // 0 unvisited, 1 visiting, 2 done
    fn visit<'a>(
        node: &'a str,
        deps: &'a BTreeMap<String, BTreeSet<String>>,
        state: &mut BTreeMap<&'a str, u8>,
        order: &mut Vec<String>,
    ) -> Result<(), Vec<String>> {
        match state.get(node) {
            Some(2) => return Ok(()),
            Some(1) => return Err(vec![node.to_string()]),
            _ => {}
        }
        state.insert(node, 1);
        if let Some(targets) = deps.get(node) {
            for target in targets {
                visit(target, deps, state, order).map_err(|mut cycle| {
                    cycle.push(node.to_string());
                    cycle
                })?;
            }
        }
        state.insert(node, 2);
        order.push(node.to_string());
        Ok(())
    }
    for node in deps.keys() {
        visit(node, deps, &mut state, &mut order)?;
    }
    Ok(order)
}

/// The lowering's reference edges for one definition, from the
/// compiled rename output: `calls` (public defs by `soil_hash`) and
/// `private_helpers` (module privates by their folded hash).
pub fn lowering_edges(
    compiled: &Compiled,
    name: &str,
    module: &str,
) -> (Vec<crate::lock::NamedHash>, Vec<crate::lock::NamedHash>) {
    let Some(sets) = compiled.refs.get(name) else {
        return (Vec::new(), Vec::new());
    };
    let calls = sets
        .defs
        .iter()
        .map(|n| crate::lock::NamedHash {
            name: n.clone(),
            hash: compiled.soil_hashes[n].clone(),
        })
        .collect();
    let helpers = sets
        .privates
        .iter()
        .map(|n| {
            let key = if module.is_empty() {
                n.clone()
            } else {
                format!("{module}/{n}")
            };
            crate::lock::NamedHash {
                name: n.clone(),
                hash: compiled.soil_hashes[&key].clone(),
            }
        })
        .collect();
    (calls, helpers)
}

// ---- statuses ----

#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub defs: Vec<DefStatus>,
    /// Pipeline errors that kept content hashes from being computed
    /// (`status` still reports what the spec side knows).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<Diag>,
}

#[derive(Debug, Serialize)]
pub struct DefStatus {
    pub path: String,
    pub kind: String,
    pub status: String,
    pub flags: Vec<String>,
}

/// Current decision entries per scope key (`""` = project, else the
/// module), from the module/project headers.
fn decision_entries(root: &Root) -> BTreeMap<String, Vec<(String, String)>> {
    let mut scopes = BTreeMap::new();
    for entry in root.entries.values() {
        if !matches!(entry.kind, FileKind::Module | FileKind::Project) {
            continue;
        }
        let scope = match entry.kind {
            FileKind::Project => String::new(),
            _ => module_of(&entry.rel),
        };
        if let Some(content) = entry.tr.blocks.iter().find_map(|b| match &b.payload {
            Payload::Decisions(_) => Some(&b.content),
            _ => None,
        }) {
            scopes.insert(scope, hash::decision_entry_hashes(content));
        }
    }
    scopes
}

fn scope_labels(scopes: &BTreeMap<String, Vec<(String, String)>>, module: &str) -> Vec<String> {
    let mut labels = Vec::new();
    for key in [String::new(), module.to_string()] {
        if let Some(entries) = scopes.get(&key) {
            labels.extend(entries.iter().map(|(l, _)| l.clone()));
        }
    }
    labels
}

pub fn statuses(
    root: &Root,
    soil_hashes: &BTreeMap<String, String>,
    compiled_refs: &BTreeMap<String, soil0::rename::RefSets>,
) -> StatusReport {
    let scopes = decision_entries(root);
    let type_formal = type_formal_hashes(root);
    let mut defs = Vec::new();
    for entry in root.entries.values() {
        let mut flags = Vec::new();
        let module = module_of(&entry.rel);
        let status = match &entry.lock {
            None => {
                flags.push("no-lock".into());
                "unlocked".to_string()
            }
            Some(lock) => {
                if lock.spec.hashes.formal != entry.spec.formal {
                    flags.push("formal-stale".into());
                }
                if lock.spec.hashes.test != entry.spec.test {
                    flags.push("test-stale".into());
                }
                if lock.spec.hashes.prose != entry.spec.prose {
                    flags.push("prose-stale".into());
                }
                if lock.spec.prose_state == "review-suggested" {
                    flags.push("review-suggested".into());
                }
                if let Some(lowering) = &lock.lowering {
                    match (soil_hashes.get(&entry.tr.name), &entry.soil) {
                        (_, None) => flags.push("soil-missing".into()),
                        (Some(computed), Some(soil)) if *computed != lowering.soil_hash => {
                            // Own edit vs callee drift: recompute with
                            // the lock's *stored* edges — if that still
                            // matches, only callees moved (content
                            // addressing propagating, design §6.1).
                            if own_form_changed(
                                soil,
                                &entry.rel,
                                lowering,
                                compiled_refs,
                                &type_formal,
                            ) {
                                flags.push("soil-drift".into());
                            } else {
                                flags.push("stale-callees".into());
                            }
                        }
                        _ => {}
                    }
                    // Decision staleness — derived, never stored
                    // (lock-schema §8, resolved 2026-08-24).
                    let labels = scope_labels(&scopes, &module);
                    if hash::decisions_scope_hash(&labels) != lowering.decisions.scope_hash {
                        flags.push("decision-scope-stale".into());
                    }
                    for applied in &lowering.decisions.applied {
                        let scope_key = match applied.scope.as_str() {
                            "project" => String::new(),
                            _ => module.clone(),
                        };
                        let current = scopes
                            .get(&scope_key)
                            .and_then(|entries| entries.iter().find(|(l, _)| *l == applied.label));
                        if current.map(|(_, h)| h.as_str()) != Some(applied.hash.as_str()) {
                            flags.push(format!("decision-stale:{}", applied.label));
                        }
                    }
                } else if entry.kind == FileKind::Function && entry.soil.is_some() {
                    flags.push("soil-untracked".into());
                }
                ladder(lock)
            }
        };
        defs.push(DefStatus {
            path: entry.rel.clone(),
            kind: format!("{:?}", entry.kind).to_lowercase(),
            status,
            flags,
        });
    }
    StatusReport {
        defs,
        errors: Vec::new(),
    }
}

/// The lock-schema §8 ladder, derived, never stored.
fn ladder(lock: &Lock) -> String {
    use crate::lock::Checks;
    if lock.accepted {
        return "accepted".into();
    }
    match &lock.checks {
        Some(Checks::Function {
            types,
            holes,
            refinements,
            ..
        }) => {
            if *holes > 0 {
                return "partial".into();
            }
            if types != "ok" {
                return "error".into();
            }
            let rows = lock.tests.as_deref().unwrap_or(&[]);
            let tested = !rows.is_empty()
                && rows
                    .iter()
                    .all(|r| r.result == "pass" || r.result == "xfail");
            if tested {
                if !refinements.is_empty() && refinements.values().all(|a| a == "proven") {
                    "verified".into()
                } else {
                    "tested".into()
                }
            } else {
                "typed".into()
            }
        }
        Some(Checks::Type { types, .. }) => {
            if types == "ok" {
                "typed".into()
            } else {
                "error".into()
            }
        }
        Some(Checks::Module { exports }) => {
            if exports == "ok" {
                "ok".into()
            } else {
                "error".into()
            }
        }
        None => {
            if lock.lowering.is_none() {
                "unlowered".into()
            } else {
                "unchecked".into()
            }
        }
    }
}

/// Whether the definition's own hash-form changed, as opposed to a
/// callee's hash moving under it: recompute against the lock's
/// *stored* definition/private edges (plus the current builtin and
/// user-type edges, which are stable unless those specs changed) —
/// a match means only callees drifted (content addressing
/// propagating, design §6.1).
fn own_form_changed(
    soil: &str,
    rel: &str,
    lowering: &crate::lock::Lowering,
    refs: &BTreeMap<String, soil0::rename::RefSets>,
    type_formal: &BTreeMap<String, String>,
) -> bool {
    let name = rel.rsplit_once('/').map(|(_, n)| n).unwrap_or(rel);
    let Ok(forms) = soil0::hashform::hash_forms_per_def(soil, rel) else {
        return true;
    };
    let Some((_, form)) = forms.into_iter().find(|(n, _)| n == name) else {
        return true;
    };
    let mut edges: Vec<crate::hash::HashRef> = lowering
        .calls
        .iter()
        .chain(&lowering.private_helpers)
        .map(|nh| (nh.name.clone(), nh.hash.clone()))
        .collect();
    if let Some(sets) = refs.get(name) {
        for b in &sets.builtins {
            edges.push((b.clone(), hash::builtin_referent()));
        }
        for t in &sets.types {
            if let Some(formal) = type_formal.get(t) {
                edges.push((t.clone(), formal.clone()));
            }
        }
    }
    crate::hash::soil_hash(&form, &edges) != lowering.soil_hash
}

// ---- the full static pipeline ----

pub fn check(root: &Root) -> Result<serde_json::Value, ErrorReport> {
    let compiled = compile(root)?;
    let out = soil0::exhaust::check_program(&compiled.program).map_err(|d| ErrorReport {
        errors: vec![soil0_diag(&d)],
    })?;
    Ok(serde_json::to_value(&out).expect("check output serializes"))
}

// ---- refresh: the invalidation-table writes ----

fn type_formal_hashes(root: &Root) -> BTreeMap<String, String> {
    root.entries
        .values()
        .filter(|e| e.kind == FileKind::Type)
        .map(|e| (e.tr.name.clone(), e.spec.formal.clone()))
        .collect()
}

pub fn refresh(root_path: &Path, config: &Config) -> Result<StatusReport, ErrorReport> {
    let root = scan(root_path, config)?;
    let compiled = compile(&root)?;
    let scopes = decision_entries(&root);
    let type_formal = type_formal_hashes(&root);

    for entry in root.entries.values() {
        let lock_path = root_path.join(format!("{}.lock", entry.rel));
        let mut lock = match &entry.lock {
            Some(lock) => lock.clone(),
            // A definition without a sidecar gets one (the daemon
            // owns lock creation).
            None => new_lock(entry),
        };

        // prose change: flag + re-anchor (lock-schema §8 row 3).
        if lock.spec.hashes.prose != entry.spec.prose {
            lock.spec.prose_state = "review-suggested".into();
            lock.spec.hashes.prose = entry.spec.prose.clone();
        }
        // Soil hand-edit: soil_hash updated, provenance hand-edited
        // (row 5); reference edges re-anchored with it.
        if let Some(lowering) = &mut lock.lowering {
            if let Some(computed) = compiled.soil_hashes.get(&entry.tr.name) {
                if *computed != lowering.soil_hash {
                    let own = entry.soil.as_deref().is_none_or(|soil| {
                        own_form_changed(soil, &entry.rel, lowering, &compiled.refs, &type_formal)
                    });
                    lowering.soil_hash = computed.clone();
                    if own {
                        lowering.provenance = "hand-edited".into();
                    }
                    let module = module_of(&entry.rel);
                    let (calls, helpers) = lowering_edges(&compiled, &entry.tr.name, &module);
                    lowering.calls = calls;
                    lowering.private_helpers = helpers;
                }
            }
        }
        // Module/project decisions snapshot (informational; staleness
        // is derived from the reliance edges).
        if matches!(entry.kind, FileKind::Module | FileKind::Project) {
            let scope = match entry.kind {
                FileKind::Project => String::new(),
                _ => module_of(&entry.rel),
            };
            lock.spec.decisions = scopes
                .get(&scope)
                .map(|entries| entries.iter().cloned().collect());
        }

        let rendered = lock::write(&lock);
        let current = std::fs::read_to_string(&lock_path).ok();
        if current.as_deref() != Some(rendered.as_str()) {
            std::fs::write(&lock_path, rendered)
                .map_err(|e| ErrorReport::one("io", format!("{}: {e}", lock_path.display())))?;
        }
    }

    let root = scan(root_path, config)?;
    Ok(statuses(&root, &compiled.soil_hashes, &compiled.refs))
}

fn new_lock(entry: &Entry) -> Lock {
    use crate::lock::*;
    Lock {
        lock_format: LOCK_FORMAT,
        name: entry.tr.name.clone(),
        kind: match entry.kind {
            FileKind::Function => "function",
            FileKind::Type => "type",
            FileKind::Module => "module",
            FileKind::Project => "project",
        }
        .into(),
        versions: Versions {
            trellis: crate::config::trellis_version().into(),
            soil: "0.1".into(),
        },
        spec: Spec {
            provenance: "human".into(),
            hashes: Hashes {
                formal: entry.spec.formal.clone(),
                test: entry.spec.test.clone(),
                prose: entry.spec.prose.clone(),
            },
            prose_state: "fresh".into(),
            blocks: entry
                .tr
                .blocks
                .iter()
                .map(|b| BlockProv {
                    block: b.id.clone(),
                    author: if b.agent { "agent" } else { "human" }.into(),
                })
                .collect(),
            pinned: false,
            escape_hatches: entry
                .tr
                .blocks
                .iter()
                .find_map(|b| match &b.payload {
                    Payload::Allow(hatches) => Some(hatches.clone()),
                    _ => None,
                })
                .unwrap_or_default(),
            decisions: None,
        },
        lowering: None,
        checks: None,
        tests: matches!(entry.kind, FileKind::Function | FileKind::Type).then(Vec::new),
        oracles: (entry.kind == FileKind::Function).then(Vec::new),
        cycle_hash: None,
        accepted: false,
    }
}

/// `trellis decisions editorial <label>`: the human declares a
/// detected decision edit meaning-preserving; re-stamp every citing
/// reliance edge to the current entry hash (tr-grammar §5.2).
pub fn editorial(root_path: &Path, config: &Config, label: &str) -> Result<usize, ErrorReport> {
    let root = scan(root_path, config)?;
    let scopes = decision_entries(&root);
    let exists = scopes
        .values()
        .any(|entries| entries.iter().any(|(l, _)| l == label));
    if !exists {
        return Err(ErrorReport {
            errors: vec![registry::enrich(Diag::new(
                "unknown-decision",
                format!("no decision entry labelled `{label}` exists in this root"),
            ))],
        });
    }
    let mut restamped = 0;
    for entry in root.entries.values() {
        let Some(lock) = &entry.lock else { continue };
        let mut lock = lock.clone();
        let module = module_of(&entry.rel);
        let mut changed = false;
        if let Some(lowering) = &mut lock.lowering {
            for applied in &mut lowering.decisions.applied {
                if applied.label != label {
                    continue;
                }
                let scope_key = match applied.scope.as_str() {
                    "project" => String::new(),
                    _ => module.clone(),
                };
                if let Some((_, current)) = scopes
                    .get(&scope_key)
                    .and_then(|entries| entries.iter().find(|(l, _)| l == label))
                {
                    if applied.hash != *current {
                        applied.hash = current.clone();
                        changed = true;
                    }
                }
            }
        }
        if changed {
            let lock_path = root_path.join(format!("{}.lock", entry.rel));
            std::fs::write(&lock_path, lock::write(&lock))
                .map_err(|e| ErrorReport::one("io", format!("{}: {e}", lock_path.display())))?;
            restamped += 1;
        }
    }
    Ok(restamped)
}
