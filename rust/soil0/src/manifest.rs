//! `program.json` and `env.json` loading, contract §6–§7. Malformed
//! inputs are class-2 diagnostics; the definitions themselves go through
//! the normal parser and report class-1 diagnostics with their file.

use crate::ast::Def;
use crate::diag::{Code, Diagnostic, Opt};
use crate::kernel;
use crate::parser;
use crate::span::Spanned;
use crate::types::SigType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---- env.json schema (contract §6) ----

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvFile {
    pub types: Vec<TypeDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeDef {
    pub name: String,
    pub params: Vec<String>,
    pub strategy: Strategy,
    pub body: TypeBody,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tag", content = "value")]
pub enum Strategy {
    Structural,
    Opaque,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tag", content = "value")]
pub enum TypeBody {
    Record { fields: Vec<FieldD> },
    Sum { variants: Vec<VariantD> },
    OpaqueBody,
    Alias { ty: SigType },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldD {
    pub name: String,
    pub shape: SigType,
    pub ignored: Opt<IgnoredDefault>,
}

/// The serializable ignored-default subset (impl plan 01 §8.14).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "tag", content = "value")]
pub enum IgnoredDefault {
    Const { value: serde_json::Value },
    CopyField { field: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VariantD {
    pub name: String,
    pub payload: Opt<SigType>,
    /// Inline record payloads (tr-grammar §4.1) — kernel-internal, not
    /// yet expressible in `env.json` (contract §13.5).
    #[serde(skip)]
    pub record_fields: Option<Vec<FieldD>>,
}

// ---- program.json (contract §7) ----

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    types: String,
    defs: Vec<String>,
}

/// One definition from the manifest, position-ordered (callee-first).
#[derive(Debug, Clone)]
pub struct LoadedDef {
    /// The manifest-relative path, for diagnostics and output.
    pub path: String,
    /// The parent directory *name* — module identity for `module::def`.
    pub module: String,
    /// The parent directory path — module identity for `_private`
    /// visibility (contract §7).
    pub dir: PathBuf,
    pub ast: Spanned<Def>,
}

impl LoadedDef {
    pub fn name(&self) -> &str {
        &self.ast.item.name
    }

    pub fn is_private(&self) -> bool {
        self.ast.item.name.starts_with('_')
    }
}

#[derive(Debug)]
pub struct Program {
    /// Kernel plus env types, by name.
    pub types: BTreeMap<String, TypeDef>,
    /// Variant name → owning sum-type names (for §5.9 resolution).
    pub variant_owners: BTreeMap<String, Vec<String>>,
    /// Flattened definitions in manifest order.
    pub defs: Vec<LoadedDef>,
}

fn malformed(message: String) -> Diagnostic {
    Diagnostic::bare(Code::MalformedInput, message)
}

pub fn load_program(manifest_path: &str) -> Result<Program, Diagnostic> {
    let manifest_dir = Path::new(manifest_path).parent().unwrap_or(Path::new("."));
    let text = read(manifest_path)?;
    let manifest: ManifestFile =
        serde_json::from_str(&text).map_err(|e| malformed(format!("{manifest_path}: {e}")))?;
    let env_path = manifest_dir.join(&manifest.types);
    let env_text = read(env_path.to_str().unwrap_or(&manifest.types))?;
    let env: EnvFile = serde_json::from_str(&env_text)
        .map_err(|e| malformed(format!("{}: {e}", env_path.display())))?;

    let mut sources = Vec::new();
    for rel in &manifest.defs {
        let full = manifest_dir.join(rel);
        let src = read(full.to_str().unwrap_or(rel))?;
        sources.push((rel.clone(), src));
    }
    from_parts(env, &sources)
}

/// Library entry used by tests: manifest-relative paths plus sources.
pub fn from_parts(env: EnvFile, files: &[(String, String)]) -> Result<Program, Diagnostic> {
    let mut types: BTreeMap<String, TypeDef> = BTreeMap::new();
    for td in kernel::kernel_typedefs() {
        types.insert(td.name.clone(), td);
    }
    let kernel_names: Vec<String> = types.keys().cloned().collect();
    for td in env.types {
        validate_typedef(&td, &kernel_names)?;
        if types.insert(td.name.clone(), td.clone()).is_some() {
            return Err(malformed(format!(
                "env.json: type `{}` is already declared (kernel types cannot be redeclared)",
                td.name
            )));
        }
    }
    // Second pass: every Named reference resolves.
    for td in types.values() {
        for shape in shapes_of(td) {
            let mut cons = Vec::new();
            shape.con_names(&mut cons);
            for con in cons {
                if kernel::builtin_type_arity(con).is_none() && !types.contains_key(con) {
                    return Err(malformed(format!(
                        "env.json: type `{}` references unknown type `{con}`",
                        td.name
                    )));
                }
            }
        }
    }

    let mut defs = Vec::new();
    for (rel, src) in files {
        let rel_path = Path::new(rel);
        let file = parser::parse_file(src, rel).map_err(|mut d| {
            d.file = Opt(Some(rel.clone()));
            d
        })?;
        let dir = rel_path.parent().unwrap_or(Path::new("")).to_path_buf();
        let module = dir
            .file_name()
            .map(|m| m.to_string_lossy().into_owned())
            .unwrap_or_default();
        let is_private_file = rel_path.file_name().is_some_and(|f| f == "_private.soil");
        for def in file.defs {
            if !is_private_file {
                let stem = rel_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned());
                if stem.as_deref() != Some(def.item.name.as_str()) {
                    return Err(Diagnostic {
                        code: Code::NameMismatch,
                        message: format!(
                            "definition `{}` does not match its filename `{rel}` (filename is identity)",
                            def.item.name
                        ),
                        file: Opt(Some(rel.clone())),
                        span: Opt(Some(def.span)),
                        notes: Vec::new(),
                    });
                }
            }
            defs.push(LoadedDef {
                path: rel.clone(),
                module: module.clone(),
                dir: dir.clone(),
                ast: def,
            });
        }
    }

    let mut variant_owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for td in types.values() {
        if let TypeBody::Sum { variants } = &td.body {
            for v in variants {
                variant_owners
                    .entry(v.name.clone())
                    .or_default()
                    .push(td.name.clone());
            }
        }
    }

    Ok(Program {
        types,
        variant_owners,
        defs,
    })
}

fn shapes_of(td: &TypeDef) -> Vec<&SigType> {
    let mut out = Vec::new();
    match &td.body {
        TypeBody::Record { fields } => out.extend(fields.iter().map(|f| &f.shape)),
        TypeBody::Sum { variants } => {
            for v in variants {
                if let Opt(Some(p)) = &v.payload {
                    out.push(p);
                }
                if let Some(fields) = &v.record_fields {
                    out.extend(fields.iter().map(|f| &f.shape));
                }
            }
        }
        TypeBody::OpaqueBody => {}
        TypeBody::Alias { ty } => out.push(ty),
    }
    out
}

fn validate_typedef(td: &TypeDef, kernel_names: &[String]) -> Result<(), Diagnostic> {
    if kernel_names.contains(&td.name) {
        return Err(malformed(format!(
            "env.json: `{}` redeclares a kernel type",
            td.name
        )));
    }
    if kernel::builtin_type_arity(&td.name).is_some() {
        return Err(malformed(format!(
            "env.json: `{}` redeclares a built-in type constructor",
            td.name
        )));
    }
    for shape in shapes_of(td) {
        if shape.contains_arrow() {
            return Err(malformed(format!(
                "env.json: type `{}` has a function-typed component; SArrow shapes are rejected in v1 (contract §6)",
                td.name
            )));
        }
        let mut vars = Vec::new();
        shape.vars(&mut vars);
        for v in vars {
            if !td.params.iter().any(|p| p == v) {
                return Err(malformed(format!(
                    "env.json: type `{}` uses undeclared type variable `{v}`",
                    td.name
                )));
            }
        }
    }
    Ok(())
}

fn read(path: &str) -> Result<String, Diagnostic> {
    std::fs::read_to_string(path).map_err(|e| {
        let code = if e.kind() == std::io::ErrorKind::InvalidData {
            Code::MalformedInput
        } else {
            Code::Io
        };
        Diagnostic::bare(code, format!("cannot read `{path}`: {e}"))
    })
}
