use std::collections::HashMap;

use crate::error::{PanicKind, SoilError};
use crate::value::Closure;

/// A dense, per-runtime-instance index into the type registry (resolved
/// decision, impl plan §1). Ids are not stable across runs; anything
/// persistent identifies types by *name* (filename is identity, design
/// §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub(crate) u32);

impl TypeId {
    pub fn index(self) -> u32 {
        self.0
    }

    /// Rebuild an id from its index — the C ABI's spelling of a
    /// `TypeId`. A forged index is caught by `Registry::get`, never UB.
    pub fn from_index(index: u32) -> Self {
        TypeId(index)
    }
}

/// The type of a field, payload, or element, as the JSON layer and the
/// derived operations need to see it. `Named` covers records, sums, and
/// opaque types — including recursive references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeShape {
    I64,
    U64,
    I32,
    U32,
    I16,
    U16,
    I8,
    U8,
    BigInt,
    F64,
    Utf8,
    Bytes,
    Unit,
    List(Box<TypeShape>),
    Map(Box<TypeShape>, Box<TypeShape>),
    /// A function type. Legal in a shape, but poisons derivation:
    /// deriving `eq` on a type containing an arrow is a type error
    /// (design §3.7).
    Closure,
    Named(TypeId),
}

/// Per-type derivation strategy (design §3.7). `ignored` is per-field,
/// not per-type: see [`FieldDesc::ignored`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    Structural,
    Opaque,
}

/// The default of an `ignored` field — how the field is refilled
/// wherever a value is materialized without it (design §3.7).
///
/// `Const` and `CopyField` are the serializable vocabulary of the
/// descriptor JSON (fixtures are data, not code — impl plan §4 step 6);
/// `Native` is an arbitrary thunk supplied by the embedder, which is
/// what a Soil-level default expression eventually compiles to and is
/// not serializable.
#[derive(Debug)]
pub enum IgnoredDefault {
    /// A constant of the field's own (scalar) shape.
    Const(crate::value::Value),
    /// Copy another (non-ignored, same-shaped) field's value.
    CopyField {
        name: String,
        /// Index into the thunk-argument list — the record's
        /// non-ignored fields in declaration order (micro-pin §8.6).
        /// Resolved at registration.
        index: usize,
    },
    Native(Closure),
}

impl IgnoredDefault {
    /// `args` are the record's non-ignored field values in declaration
    /// order (micro-pin §8.6).
    pub fn call(&self, args: &[crate::value::Value]) -> Result<crate::value::Value, SoilError> {
        match self {
            IgnoredDefault::Const(value) => Ok(value.clone()),
            IgnoredDefault::CopyField { index, .. } => Ok(args[*index].clone()),
            IgnoredDefault::Native(closure) => closure.call(args),
        }
    }
}

#[derive(Debug)]
pub struct FieldDesc {
    pub name: String,
    pub shape: TypeShape,
    /// `Some(default)` marks the field `ignored` (design §3.7): skipped
    /// by `eq`/`compare`, omitted by `show`, refilled by this default on
    /// decode.
    pub ignored: Option<IgnoredDefault>,
}

#[derive(Debug)]
pub struct VariantDesc {
    pub name: String,
    /// At most one payload (design §3.1); its fields are a record if
    /// more is needed.
    pub payload: Option<TypeShape>,
}

#[derive(Debug)]
pub enum TypeBody {
    Record(Vec<FieldDesc>),
    Sum(Vec<VariantDesc>),
    /// No visible structure at all — FFI handles, abstract types. Only
    /// legal with [`Strategy::Opaque`].
    Opaque,
}

#[derive(Debug)]
pub struct TypeDesc {
    /// The cased name (`"SummaryRow"`), the stable identity.
    pub name: String,
    pub strategy: Strategy,
    pub body: TypeBody,
}

enum Slot {
    /// Declared but not yet defined — the target of recursive
    /// references during two-phase registration.
    Declared {
        name: String,
    },
    Defined {
        desc: TypeDesc,
    },
}

/// The per-runtime-instance table of type descriptors. Descriptors
/// drive the derived operations and type-directed JSON decode;
/// registration is used by the interpreter now and compiled code later
/// (plan 01).
///
/// Two-phase registration (`declare`, then `define`) exists because
/// recursive types across definitions are legal (design §3.8): declare
/// every member of a cycle first, then define each against the declared
/// ids. `register` is the one-shot spelling for the common case.
pub struct Registry {
    slots: Vec<Slot>,
    by_name: HashMap<String, TypeId>,
    bool_id: TypeId,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub fn new() -> Self {
        let mut registry = Registry {
            slots: Vec::new(),
            by_name: HashMap::new(),
            bool_id: TypeId(0),
        };
        // `Bool` is the prelude sum `True | False` (plan 01): not a
        // `Value` variant, but pre-registered so the JSON layer can
        // special-case its encoding to JSON booleans (tr-grammar §7).
        registry.bool_id = registry
            .register(TypeDesc {
                name: "Bool".to_string(),
                strategy: Strategy::Structural,
                body: TypeBody::Sum(vec![
                    VariantDesc {
                        name: "True".to_string(),
                        payload: None,
                    },
                    VariantDesc {
                        name: "False".to_string(),
                        payload: None,
                    },
                ]),
            })
            .expect("Bool registration cannot fail on an empty registry");
        registry
    }

    pub fn bool_id(&self) -> TypeId {
        self.bool_id
    }

    pub fn declare(&mut self, name: &str) -> Result<TypeId, SoilError> {
        if self.by_name.contains_key(name) {
            return Err(SoilError::new(
                PanicKind::CapiMisuse,
                format!("type name already registered: {name}"),
            ));
        }
        let id = TypeId(
            u32::try_from(self.slots.len())
                .map_err(|_| SoilError::new(PanicKind::CapiMisuse, "type registry full"))?,
        );
        self.slots.push(Slot::Declared {
            name: name.to_string(),
        });
        self.by_name.insert(name.to_string(), id);
        Ok(id)
    }

    pub fn define(&mut self, id: TypeId, desc: TypeDesc) -> Result<(), SoilError> {
        let slot = self.slots.get(id.0 as usize).ok_or_else(|| {
            SoilError::new(PanicKind::CapiMisuse, format!("undefined type id {}", id.0))
        })?;
        match slot {
            Slot::Defined { .. } => {
                return Err(SoilError::new(
                    PanicKind::CapiMisuse,
                    format!("type id {} is already defined", id.0),
                ));
            }
            Slot::Declared { name } => {
                if *name != desc.name {
                    return Err(SoilError::new(
                        PanicKind::CapiMisuse,
                        format!(
                            "type id {} was declared as {name} but defined as {}",
                            id.0, desc.name
                        ),
                    ));
                }
            }
        }
        self.validate(&desc)?;
        self.slots[id.0 as usize] = Slot::Defined { desc };
        Ok(())
    }

    /// `declare` + `define` for the non-recursive common case. Unlike a
    /// bare `declare`, a failed `register` leaves no trace: the declared
    /// name is rolled back so the caller can retry with a fixed
    /// descriptor.
    pub fn register(&mut self, desc: TypeDesc) -> Result<TypeId, SoilError> {
        let name = desc.name.clone();
        let id = self.declare(&name)?;
        match self.define(id, desc) {
            Ok(()) => Ok(id),
            Err(err) => {
                self.slots.pop();
                self.by_name.remove(&name);
                Err(err)
            }
        }
    }

    pub fn get(&self, id: TypeId) -> Result<&TypeDesc, SoilError> {
        match self.slots.get(id.0 as usize) {
            Some(Slot::Defined { desc }) => Ok(desc),
            Some(Slot::Declared { name }) => Err(SoilError::new(
                PanicKind::CapiMisuse,
                format!("type {name} is declared but not defined"),
            )),
            None => Err(SoilError::new(
                PanicKind::CapiMisuse,
                format!("undefined type id {}", id.0),
            )),
        }
    }

    pub fn lookup(&self, name: &str) -> Option<TypeId> {
        self.by_name.get(name).copied()
    }

    /// Whether the derived operations exist for this type: false iff its
    /// shape transitively contains a function type ("deriving `eq` on a
    /// type containing an arrow is a type error", design §3.7).
    ///
    /// The impl plan places this check at `define` time; it is computed
    /// on demand instead because a member of a type cycle can be defined
    /// while its cycle-mates are still only declared, so the transitive
    /// walk is not possible until the whole cycle is in. The walk is
    /// cycle-safe and cheap at registry scale.
    pub fn derivable(&self, id: TypeId) -> Result<bool, SoilError> {
        let mut visited = vec![false; self.slots.len()];
        self.derivable_walk(id, &mut visited)
    }

    fn derivable_walk(&self, id: TypeId, visited: &mut [bool]) -> Result<bool, SoilError> {
        if visited[id.0 as usize] {
            // A cycle back to a type already on the walk adds nothing
            // new; closures on the cycle are found by the other paths.
            return Ok(true);
        }
        visited[id.0 as usize] = true;
        let desc = self.get(id)?;
        match &desc.body {
            // Opaque types derive by identity regardless of payload.
            TypeBody::Opaque => Ok(true),
            TypeBody::Record(fields) => {
                for field in fields {
                    if !self.shape_derivable(&field.shape, visited)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            TypeBody::Sum(variants) => {
                for variant in variants {
                    if let Some(shape) = &variant.payload {
                        if !self.shape_derivable(shape, visited)? {
                            return Ok(false);
                        }
                    }
                }
                Ok(true)
            }
        }
    }

    fn shape_derivable(&self, shape: &TypeShape, visited: &mut [bool]) -> Result<bool, SoilError> {
        match shape {
            TypeShape::Closure => Ok(false),
            TypeShape::List(elem) => self.shape_derivable(elem, visited),
            TypeShape::Map(key, value) => {
                Ok(self.shape_derivable(key, visited)? && self.shape_derivable(value, visited)?)
            }
            TypeShape::Named(id) => self.derivable_walk(*id, visited),
            _ => Ok(true),
        }
    }

    fn validate(&self, desc: &TypeDesc) -> Result<(), SoilError> {
        if matches!(desc.body, TypeBody::Opaque) && desc.strategy != Strategy::Opaque {
            return Err(SoilError::new(
                PanicKind::CapiMisuse,
                format!(
                    "opaque-bodied type {} must use the opaque strategy",
                    desc.name
                ),
            ));
        }
        match &desc.body {
            TypeBody::Record(fields) => {
                let mut seen = std::collections::HashSet::new();
                for field in fields {
                    if !seen.insert(field.name.as_str()) {
                        return Err(SoilError::new(
                            PanicKind::CapiMisuse,
                            format!("duplicate field {} in type {}", field.name, desc.name),
                        ));
                    }
                    self.validate_shape(&field.shape, &desc.name)?;
                    if let Some(IgnoredDefault::CopyField { name, index }) = &field.ignored {
                        let target = fields.iter().find(|f| f.name == *name).ok_or_else(|| {
                            SoilError::new(
                                PanicKind::CapiMisuse,
                                format!(
                                    "ignored field {} of {} copies unknown field {name}",
                                    field.name, desc.name
                                ),
                            )
                        })?;
                        if target.ignored.is_some() {
                            return Err(SoilError::new(
                                PanicKind::CapiMisuse,
                                format!(
                                    "ignored field {} of {} may not copy another ignored field",
                                    field.name, desc.name
                                ),
                            ));
                        }
                        if target.shape != field.shape {
                            return Err(SoilError::new(
                                PanicKind::CapiMisuse,
                                format!(
                                    "ignored field {} of {} copies a field of a different shape",
                                    field.name, desc.name
                                ),
                            ));
                        }
                        let expected = fields
                            .iter()
                            .filter(|f| f.ignored.is_none())
                            .position(|f| f.name == *name)
                            .expect("target is non-ignored");
                        if *index != expected {
                            return Err(SoilError::new(
                                PanicKind::CapiMisuse,
                                format!(
                                    "ignored field {} of {}: CopyField index {index} does not \
                                     match target position {expected}",
                                    field.name, desc.name
                                ),
                            ));
                        }
                    }
                }
            }
            TypeBody::Sum(variants) => {
                if variants.is_empty() {
                    return Err(SoilError::new(
                        PanicKind::CapiMisuse,
                        format!("sum type {} has no variants", desc.name),
                    ));
                }
                let mut seen = std::collections::HashSet::new();
                for variant in variants {
                    if !seen.insert(variant.name.as_str()) {
                        return Err(SoilError::new(
                            PanicKind::CapiMisuse,
                            format!("duplicate variant {} in type {}", variant.name, desc.name),
                        ));
                    }
                    if let Some(shape) = &variant.payload {
                        self.validate_shape(shape, &desc.name)?;
                    }
                }
            }
            TypeBody::Opaque => {}
        }
        Ok(())
    }

    fn validate_shape(&self, shape: &TypeShape, context: &str) -> Result<(), SoilError> {
        match shape {
            TypeShape::List(elem) => self.validate_shape(elem, context),
            TypeShape::Map(key, value) => {
                self.validate_shape(key, context)?;
                self.validate_shape(value, context)
            }
            TypeShape::Named(id) => {
                if self.slots.get(id.0 as usize).is_none() {
                    return Err(SoilError::new(
                        PanicKind::CapiMisuse,
                        format!("type {context} references undeclared type id {}", id.0),
                    ));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, fields: Vec<FieldDesc>) -> TypeDesc {
        TypeDesc {
            name: name.to_string(),
            strategy: Strategy::Structural,
            body: TypeBody::Record(fields),
        }
    }

    fn field(name: &str, shape: TypeShape) -> FieldDesc {
        FieldDesc {
            name: name.to_string(),
            shape,
            ignored: None,
        }
    }

    #[test]
    fn bool_is_preregistered() {
        let registry = Registry::new();
        let id = registry.lookup("Bool").unwrap();
        assert_eq!(id, registry.bool_id());
        let desc = registry.get(id).unwrap();
        match &desc.body {
            TypeBody::Sum(variants) => {
                let names: Vec<&str> = variants.iter().map(|v| v.name.as_str()).collect();
                assert_eq!(names, vec!["True", "False"]);
            }
            other => panic!("Bool should be a sum, got {other:?}"),
        }
    }

    #[test]
    fn register_and_lookup_round_trip() {
        let mut registry = Registry::new();
        let id = registry
            .register(record(
                "SummaryRow",
                vec![
                    field("count", TypeShape::U64),
                    field("mean", TypeShape::F64),
                ],
            ))
            .unwrap();
        assert_eq!(registry.lookup("SummaryRow"), Some(id));
        assert_eq!(registry.get(id).unwrap().name, "SummaryRow");
        assert!(registry.derivable(id).unwrap());
    }

    #[test]
    fn duplicate_names_rejected() {
        let mut registry = Registry::new();
        registry.register(record("A", vec![])).unwrap();
        assert!(registry.register(record("A", vec![])).is_err());
        assert!(registry.declare("Bool").is_err());
    }

    #[test]
    fn recursive_type_via_declare_define() {
        let mut registry = Registry::new();
        let tree = registry.declare("Tree").unwrap();
        registry
            .define(
                tree,
                TypeDesc {
                    name: "Tree".to_string(),
                    strategy: Strategy::Structural,
                    body: TypeBody::Sum(vec![
                        VariantDesc {
                            name: "Leaf".to_string(),
                            payload: Some(TypeShape::I64),
                        },
                        VariantDesc {
                            name: "Node".to_string(),
                            payload: Some(TypeShape::List(Box::new(TypeShape::Named(tree)))),
                        },
                    ]),
                },
            )
            .unwrap();
        assert!(registry.derivable(tree).unwrap());
        // Reading a declared-but-undefined type is an error, not a panic.
        let pending = registry.declare("Pending").unwrap();
        assert!(registry.get(pending).is_err());
    }

    #[test]
    fn define_checks_name_and_double_definition() {
        let mut registry = Registry::new();
        let id = registry.declare("A").unwrap();
        assert!(registry.define(id, record("B", vec![])).is_err());
        registry.define(id, record("A", vec![])).unwrap();
        assert!(registry.define(id, record("A", vec![])).is_err());
    }

    #[test]
    fn closure_poisons_derivability_transitively() {
        let mut registry = Registry::new();
        let inner = registry
            .register(record("Inner", vec![field("callback", TypeShape::Closure)]))
            .unwrap();
        let outer = registry
            .register(record(
                "Outer",
                vec![field(
                    "items",
                    TypeShape::List(Box::new(TypeShape::Named(inner))),
                )],
            ))
            .unwrap();
        assert!(!registry.derivable(inner).unwrap());
        assert!(!registry.derivable(outer).unwrap());
    }

    #[test]
    fn opaque_body_requires_opaque_strategy_and_derives() {
        let mut registry = Registry::new();
        assert!(registry
            .register(TypeDesc {
                name: "Handle".to_string(),
                strategy: Strategy::Structural,
                body: TypeBody::Opaque,
            })
            .is_err());
        let id = registry
            .register(TypeDesc {
                name: "Handle".to_string(),
                strategy: Strategy::Opaque,
                body: TypeBody::Opaque,
            })
            .unwrap();
        assert!(registry.derivable(id).unwrap());
    }

    #[test]
    fn sum_must_have_variants_and_unique_names() {
        let mut registry = Registry::new();
        assert!(registry
            .register(TypeDesc {
                name: "Empty".to_string(),
                strategy: Strategy::Structural,
                body: TypeBody::Sum(vec![]),
            })
            .is_err());
        assert!(registry
            .register(TypeDesc {
                name: "Dup".to_string(),
                strategy: Strategy::Structural,
                body: TypeBody::Sum(vec![
                    VariantDesc {
                        name: "X".to_string(),
                        payload: None
                    },
                    VariantDesc {
                        name: "X".to_string(),
                        payload: None
                    },
                ]),
            })
            .is_err());
    }
}
