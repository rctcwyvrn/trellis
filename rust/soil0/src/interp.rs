//! The interpreter (impl plan 02 step 8) and the `run`/`test` commands
//! (step 9): a strict tree-walk over `soil-rt` values.
//!
//! Type registration is two-level: **erased** descriptors, one per type
//! *name* (type-variable positions as `Unit` — encode and structural ops
//! are value-directed and never read them), stamped on constructed
//! values; and **mangled ground** descriptors, one per ground instance,
//! used only for type-directed decode of `run`/`test` inputs. Canonical
//! JSON never contains type names (only variant and field names), so the
//! two id spaces byte-compare equal, and pattern matching is name/index
//! based, never id based. Inline record payloads get synthesized
//! `Type.Variant` descriptors.

use crate::ast::{self, ArithOp, CmpOp, Expr, Lit, Pattern, Type as AType};
use crate::diag::{Code, Diagnostic, Note, Opt};
use crate::infer::{conv_sig_prog, OpTable};
use crate::kernel;
use crate::manifest::{IgnoredDefault as MIgnored, LoadedDef, Program, TypeBody};
use crate::parser;
use crate::span::{Span, Spanned};
use crate::types::Ty;
use serde::{Deserialize, Serialize};
use soil_rt::{
    BigInt, Closure, FieldDesc, IgnoredDefault, PanicKind, Record, Ref, Runtime, SoilError,
    Strategy, SumVal, TraceFrame, TypeBody as RtBody, TypeDesc, TypeId, TypeShape, Value,
    VariantDesc,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const CAPS: &[&str] = &["World", "Fs", "Net", "Clock", "Env", "Proc", "Rand", "Py"];

// ---- capability payloads ----

enum FsImpl {
    Real,
    Fake(Vec<(String, String)>),
}

enum ClockImpl {
    Real,
    Fake(i64),
}

enum RandImpl {
    Split(std::cell::Cell<u64>),
}

fn splitmix_next(state: &std::cell::Cell<u64>) -> u64 {
    let mut z = state.get().wrapping_add(0x9E3779B97F4A7C15);
    state.set(z);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

struct WorldTag;

// ---- the interpreter core ----

pub struct Core {
    pub prog: Rc<Program>,
    pub rt: RefCell<Runtime>,
    ops: Vec<OpTable>,
    erased: RefCell<HashMap<String, TypeId>>,
    ground: RefCell<HashMap<String, TypeId>>,
    def_cache: RefCell<HashMap<usize, Value>>,
}

pub type I = Rc<Core>;

fn internal(msg: impl Into<String>) -> SoilError {
    SoilError::new(PanicKind::Internal, msg)
}

impl Core {
    pub fn new(prog: Rc<Program>, ops: Vec<OpTable>) -> Result<I, Diagnostic> {
        let core = Rc::new(Core {
            prog,
            rt: RefCell::new(Runtime::new()),
            ops,
            erased: RefCell::new(HashMap::new()),
            ground: RefCell::new(HashMap::new()),
            def_cache: RefCell::new(HashMap::new()),
        });
        core.register_erased().map_err(|e| {
            Diagnostic::bare(
                Code::Internal,
                format!("type registration failed: {}", e.message),
            )
        })?;
        Ok(core)
    }

    /// Registers every named type (and synthesized inline-payload
    /// records) once, with type-variable positions erased to `Unit`.
    fn register_erased(self: &Rc<Self>) -> Result<(), SoilError> {
        let mut rt = self.rt.borrow_mut();
        let mut erased = self.erased.borrow_mut();
        // Declare pass (recursive types).
        for td in self.prog.types.values() {
            if td.name == "Bool" || matches!(td.body, TypeBody::Alias { .. }) {
                continue;
            }
            erased.insert(td.name.clone(), rt.registry.declare(&td.name)?);
            if let TypeBody::Sum { variants } = &td.body {
                for v in variants {
                    if v.record_fields.is_some() {
                        let syn = format!("{}.{}", td.name, v.name);
                        erased.insert(syn.clone(), rt.registry.declare(&syn)?);
                    }
                }
            }
        }
        erased.insert("Bool".to_string(), rt.registry.bool_id());
        // Define pass.
        for td in self.prog.types.values() {
            if td.name == "Bool" || matches!(td.body, TypeBody::Alias { .. }) {
                continue;
            }
            let id = erased[&td.name];
            let (strategy, body) = match &td.body {
                TypeBody::OpaqueBody => (Strategy::Opaque, RtBody::Opaque),
                TypeBody::Record { fields } => (
                    Strategy::Structural,
                    RtBody::Record(self.erased_fields(&mut rt, &erased, fields)?),
                ),
                TypeBody::Sum { variants } => {
                    let mut out = Vec::new();
                    for v in variants {
                        let payload = if let Opt(Some(shape)) = &v.payload {
                            Some(self.erased_shape_sig(&erased, shape))
                        } else if v.record_fields.is_some() {
                            let syn = format!("{}.{}", td.name, v.name);
                            Some(TypeShape::Named(erased[&syn]))
                        } else {
                            None
                        };
                        out.push(VariantDesc {
                            name: v.name.clone(),
                            payload,
                        });
                    }
                    (Strategy::Structural, RtBody::Sum(out))
                }
                TypeBody::Alias { .. } => unreachable!(),
            };
            rt.registry.define(
                id,
                TypeDesc {
                    name: td.name.clone(),
                    strategy,
                    body,
                },
            )?;
            if let TypeBody::Sum { variants } = &td.body {
                for v in variants {
                    if let Some(fields) = &v.record_fields {
                        let syn = format!("{}.{}", td.name, v.name);
                        let fs = self.erased_fields(&mut rt, &erased, fields)?;
                        rt.registry.define(
                            erased[&syn],
                            TypeDesc {
                                name: syn.clone(),
                                strategy: Strategy::Structural,
                                body: RtBody::Record(fs),
                            },
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn erased_fields(
        &self,
        _rt: &mut Runtime,
        erased: &HashMap<String, TypeId>,
        fields: &[crate::manifest::FieldD],
    ) -> Result<Vec<FieldDesc>, SoilError> {
        let non_ignored: Vec<&str> = fields
            .iter()
            .filter(|f| f.ignored.0.is_none())
            .map(|f| f.name.as_str())
            .collect();
        fields
            .iter()
            .map(|f| {
                let ignored = match &f.ignored.0 {
                    None => None,
                    Some(MIgnored::CopyField { field }) => {
                        let index = non_ignored
                            .iter()
                            .position(|n| n == field)
                            .ok_or_else(|| internal("bad CopyField"))?;
                        Some(IgnoredDefault::CopyField {
                            name: field.clone(),
                            index,
                        })
                    }
                    Some(MIgnored::Const { value }) => {
                        Some(IgnoredDefault::Const(scalar_const(value)?))
                    }
                };
                Ok(FieldDesc {
                    name: f.name.clone(),
                    shape: self.erased_shape_sig(erased, &f.shape),
                    ignored,
                })
            })
            .collect()
    }

    fn erased_shape_sig(
        &self,
        erased: &HashMap<String, TypeId>,
        s: &crate::types::SigType,
    ) -> TypeShape {
        let ty = conv_sig_prog(&self.prog, s, &HashMap::new());
        self.erased_shape_ty(erased, &ty)
    }

    #[allow(clippy::only_used_in_recursion)]
    fn erased_shape_ty(&self, erased: &HashMap<String, TypeId>, ty: &Ty) -> TypeShape {
        match ty {
            Ty::Var(_) | Ty::Rigid(_) => TypeShape::Unit,
            Ty::Arrow { .. } => TypeShape::Closure,
            Ty::Con { name, args } => match name.as_str() {
                "I64" => TypeShape::I64,
                "U64" => TypeShape::U64,
                "I32" => TypeShape::I32,
                "U32" => TypeShape::U32,
                "I16" => TypeShape::I16,
                "U16" => TypeShape::U16,
                "I8" => TypeShape::I8,
                "U8" => TypeShape::U8,
                "BigInt" => TypeShape::BigInt,
                "F64" => TypeShape::F64,
                "Utf8" => TypeShape::Utf8,
                "Bytes" => TypeShape::Bytes,
                "Unit" => TypeShape::Unit,
                "List" => TypeShape::List(Box::new(self.erased_shape_ty(erased, &args[0]))),
                "Map" => TypeShape::Map(
                    Box::new(self.erased_shape_ty(erased, &args[0])),
                    Box::new(self.erased_shape_ty(erased, &args[1])),
                ),
                _ => erased
                    .get(name)
                    .map(|id| TypeShape::Named(*id))
                    .unwrap_or(TypeShape::Unit),
            },
        }
    }

    fn erased_id(&self, name: &str) -> Result<TypeId, SoilError> {
        self.erased
            .borrow()
            .get(name)
            .copied()
            .ok_or_else(|| internal(format!("unregistered type `{name}`")))
    }

    // ---- ground registration (decode only) ----

    fn mangle(ty: &Ty) -> String {
        match ty {
            Ty::Con { name, args } => {
                if args.is_empty() {
                    name.clone()
                } else {
                    let inner: Vec<String> = args.iter().map(Self::mangle).collect();
                    format!("{name}<{}>", inner.join(","))
                }
            }
            _ => "?".to_string(),
        }
    }

    pub fn ground_shape(self: &Rc<Self>, ty: &Ty) -> Result<TypeShape, SoilError> {
        Ok(match ty {
            Ty::Var(_) | Ty::Rigid(_) => {
                return Err(internal("entry and test argument types must be ground"))
            }
            Ty::Arrow { .. } => {
                return Err(internal("function values cannot cross the CLI boundary"))
            }
            Ty::Con { name, args } => match name.as_str() {
                "I64" => TypeShape::I64,
                "U64" => TypeShape::U64,
                "I32" => TypeShape::I32,
                "U32" => TypeShape::U32,
                "I16" => TypeShape::I16,
                "U16" => TypeShape::U16,
                "I8" => TypeShape::I8,
                "U8" => TypeShape::U8,
                "BigInt" => TypeShape::BigInt,
                "F64" => TypeShape::F64,
                "Utf8" => TypeShape::Utf8,
                "Bytes" => TypeShape::Bytes,
                "Unit" => TypeShape::Unit,
                "Bool" => TypeShape::Named(self.rt.borrow().registry.bool_id()),
                "List" => TypeShape::List(Box::new(self.ground_shape(&args[0])?)),
                "Map" => TypeShape::Map(
                    Box::new(self.ground_shape(&args[0])?),
                    Box::new(self.ground_shape(&args[1])?),
                ),
                _ => TypeShape::Named(self.ground_id(ty)?),
            },
        })
    }

    fn ground_id(self: &Rc<Self>, ty: &Ty) -> Result<TypeId, SoilError> {
        let key = Self::mangle(ty);
        if let Some(id) = self.ground.borrow().get(&key) {
            return Ok(*id);
        }
        let Ty::Con { name, args } = ty else {
            return Err(internal("ground_id on non-con"));
        };
        let td = self
            .prog
            .types
            .get(name)
            .ok_or_else(|| internal(format!("unknown type `{name}`")))?
            .clone();
        let subst: HashMap<String, Ty> = td.params.iter().cloned().zip(args.clone()).collect();
        let id = self
            .rt
            .borrow_mut()
            .registry
            .declare(&format!("ground:{key}"))?;
        self.ground.borrow_mut().insert(key.clone(), id);
        let (strategy, body) = match &td.body {
            TypeBody::OpaqueBody => {
                return Err(internal(format!("opaque `{name}` cannot be decoded")))
            }
            TypeBody::Alias { .. } => return Err(internal("aliases are expanded before decode")),
            TypeBody::Record { fields } => {
                let mut fs = Vec::new();
                for f in fields {
                    if f.ignored.0.is_some() {
                        // Reuse erased conversion for the default.
                        let erased = self.erased.borrow();
                        let all = self.erased_fields(&mut self.rt.borrow_mut(), &erased, fields)?;
                        drop(erased);
                        let mut out = Vec::new();
                        for (fd, orig) in all.into_iter().zip(fields) {
                            let shape = if fd.ignored.is_some() {
                                fd.shape
                            } else {
                                let t = conv_sig_prog(&self.prog, &orig.shape, &subst);
                                self.ground_shape(&t)?
                            };
                            out.push(FieldDesc { shape, ..fd });
                        }
                        fs = out;
                        break;
                    }
                    let t = conv_sig_prog(&self.prog, &f.shape, &subst);
                    fs.push(FieldDesc {
                        name: f.name.clone(),
                        shape: self.ground_shape(&t)?,
                        ignored: None,
                    });
                }
                (Strategy::Structural, RtBody::Record(fs))
            }
            TypeBody::Sum { variants } => {
                let mut out = Vec::new();
                for v in variants {
                    let payload = if let Opt(Some(shape)) = &v.payload {
                        let t = conv_sig_prog(&self.prog, shape, &subst);
                        Some(self.ground_shape(&t)?)
                    } else if let Some(fields) = &v.record_fields {
                        // Synthesized ground payload record.
                        let pid = self
                            .rt
                            .borrow_mut()
                            .registry
                            .declare(&format!("ground:{key}.{}", v.name))?;
                        let mut fs = Vec::new();
                        for f in fields {
                            let t = conv_sig_prog(&self.prog, &f.shape, &subst);
                            fs.push(FieldDesc {
                                name: f.name.clone(),
                                shape: self.ground_shape(&t)?,
                                ignored: None,
                            });
                        }
                        self.rt.borrow_mut().registry.define(
                            pid,
                            TypeDesc {
                                name: format!("ground:{key}.{}", v.name),
                                strategy: Strategy::Structural,
                                body: RtBody::Record(fs),
                            },
                        )?;
                        Some(TypeShape::Named(pid))
                    } else {
                        None
                    };
                    out.push(VariantDesc {
                        name: v.name.clone(),
                        payload,
                    });
                }
                (Strategy::Structural, RtBody::Sum(out))
            }
        };
        self.rt.borrow_mut().registry.define(
            id,
            TypeDesc {
                name: format!("ground:{key}"),
                strategy,
                body,
            },
        )?;
        Ok(id)
    }
}

fn scalar_const(value: &serde_json::Value) -> Result<Value, SoilError> {
    Ok(match value {
        serde_json::Value::Number(n) if n.is_i64() => Value::I64(n.as_i64().unwrap()),
        serde_json::Value::String(s) => Value::Utf8(Ref::from(s.as_str())),
        _ => return Err(internal("unsupported Const ignored-default shape in v0")),
    })
}

// ---- environments ----

struct EnvNode {
    name: String,
    cell: RefCell<Value>,
    next: Env,
}

type Env = Option<Rc<EnvNode>>;

fn bind(env: &Env, name: &str, v: Value) -> Env {
    Some(Rc::new(EnvNode {
        name: name.to_string(),
        cell: RefCell::new(v),
        next: env.clone(),
    }))
}

fn lookup(env: &Env, name: &str) -> Option<Value> {
    let mut cur = env.clone();
    while let Some(node) = cur {
        if node.name == name {
            return Some(node.cell.borrow().clone());
        }
        cur = node.next.clone();
    }
    None
}

// ---- values helpers ----

fn bool_value(i: &I, b: bool) -> Value {
    let id = i.rt.borrow().registry.bool_id();
    Value::Sum(Ref::new(SumVal {
        type_id: id,
        variant: u32::from(!b),
        payload: None,
    }))
}

fn as_bool(i: &I, v: &Value) -> Result<bool, SoilError> {
    let id = i.rt.borrow().registry.bool_id();
    match v {
        Value::Sum(s) if s.type_id == id => Ok(s.variant == 0),
        _ => Err(internal("expected Bool")),
    }
}

fn opaque(i: &I, ty: &str, payload: Box<dyn std::any::Any>) -> Result<Value, SoilError> {
    Ok(Value::Opaque(Ref::new(soil_rt::OpaqueVal {
        type_id: i.erased_id(ty)?,
        payload,
    })))
}

// ---- evaluation ----

fn eval(i: &I, d: usize, env: &Env, e: &Spanned<Expr>) -> Result<Value, SoilError> {
    match &e.item {
        Expr::Literal { lit } => literal_value(i, d, lit, e.span),
        Expr::Annot { expr, ty } => {
            if let Expr::RecordE {
                update: Opt(None),
                fields,
            } = &expr.item
            {
                if let ast::Type::Con { name, .. } = &strip_refined(ty).item {
                    if i.prog.types.contains_key(name) {
                        return record_literal(i, d, env, name, fields);
                    }
                }
            }
            eval(i, d, env, expr)
        }
        Expr::Path { root, fields } => {
            let mut v = match lookup(env, root) {
                Some(v) => v,
                None => name_value(i, d, root)?,
            };
            for f in fields {
                v = project(i, &v, f)?;
            }
            Ok(v)
        }
        Expr::Qualified { space, name } => qualified_value(i, d, space, name),
        Expr::CtorE { name } => ctor_value(i, d, env, None, name, None),
        Expr::App { r#fn, arg } => {
            match &r#fn.item {
                Expr::CtorE { name } => return ctor_value(i, d, env, None, name, Some(arg)),
                Expr::Qualified { space, name }
                    if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) =>
                {
                    return ctor_value(i, d, env, Some(space), name, Some(arg));
                }
                _ => {}
            }
            let f = eval(i, d, env, r#fn)?;
            let a = eval(i, d, env, arg)?;
            match f {
                Value::Closure(c) => c.call(&[a]),
                _ => Err(internal("applied a non-function")),
            }
        }
        Expr::Let {
            is_rec,
            bindings,
            body,
        } => {
            if *is_rec {
                let mut new_env = env.clone();
                let mut nodes = Vec::new();
                for b in bindings {
                    let name = &b.binder.0.as_ref().expect("no `_` in let rec").name;
                    new_env = bind(&new_env, name, Value::Unit);
                    nodes.push(new_env.clone().unwrap());
                }
                for (b, node) in bindings.iter().zip(&nodes) {
                    let v = eval(i, d, &new_env, &b.value)?;
                    *node.cell.borrow_mut() = v;
                }
                eval(i, d, &new_env, body)
            } else {
                let mut vals = Vec::new();
                for b in bindings {
                    vals.push(eval(i, d, env, &b.value)?);
                }
                let mut new_env = env.clone();
                for (b, v) in bindings.iter().zip(vals) {
                    if let Opt(Some(binder)) = &b.binder {
                        new_env = bind(&new_env, &binder.name, v);
                    }
                }
                eval(i, d, &new_env, body)
            }
        }
        Expr::Fun { params, body } => Ok(make_lambda(
            i,
            d,
            env.clone(),
            params.clone(),
            Rc::new((**body).clone()),
            0,
        )),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            let c = eval(i, d, env, cond)?;
            if as_bool(i, &c)? {
                eval(i, d, env, then_branch)
            } else {
                eval(i, d, env, else_branch)
            }
        }
        Expr::Match { scrutinee, arms } => {
            let v = eval(i, d, env, scrutinee)?;
            for arm in arms {
                if let Some(bs) = match_pattern(i, &arm.item.pattern, &v)? {
                    let mut new_env = env.clone();
                    for (n, bv) in bs {
                        new_env = bind(&new_env, &n, bv);
                    }
                    return eval(i, d, &new_env, &arm.item.body);
                }
            }
            Err(SoilError::new(
                PanicKind::Internal,
                "non-exhaustive match reached at runtime",
            ))
        }
        Expr::OrE { lhs, rhs } => {
            let l = eval(i, d, env, lhs)?;
            if as_bool(i, &l)? {
                Ok(bool_value(i, true))
            } else {
                eval(i, d, env, rhs)
            }
        }
        Expr::AndE { lhs, rhs } => {
            let l = eval(i, d, env, lhs)?;
            if as_bool(i, &l)? {
                eval(i, d, env, rhs)
            } else {
                Ok(bool_value(i, false))
            }
        }
        Expr::NotE { operand } => {
            let v = eval(i, d, env, operand)?;
            Ok(bool_value(i, !as_bool(i, &v)?))
        }
        Expr::Cmp { op, lhs, rhs } => {
            let l = eval(i, d, env, lhs)?;
            let r = eval(i, d, env, rhs)?;
            let rt = i.rt.borrow();
            let out = match op {
                CmpOp::Eq => soil_rt::ops::eq(&rt, &l, &r)?,
                CmpOp::Ne => !soil_rt::ops::eq(&rt, &l, &r)?,
                _ => {
                    let ord = soil_rt::ops::compare(&rt, &l, &r)?;
                    match op {
                        CmpOp::Lt => ord.is_lt(),
                        CmpOp::Le => ord.is_le(),
                        CmpOp::Gt => ord.is_gt(),
                        CmpOp::Ge => ord.is_ge(),
                        _ => unreachable!(),
                    }
                }
            };
            drop(rt);
            Ok(bool_value(i, out))
        }
        Expr::Arith { op, lhs, rhs } => {
            let l = eval(i, d, env, lhs)?;
            let r = eval(i, d, env, rhs)?;
            let ty = i.ops[d]
                .get(&(e.span.start, e.span.end))
                .cloned()
                .unwrap_or_else(|| "I64".to_string());
            arith(&ty, *op, &l, &r)
        }
        Expr::Neg { operand } => {
            let v = eval(i, d, env, operand)?;
            let ty = i.ops[d]
                .get(&(e.span.start, e.span.end))
                .cloned()
                .unwrap_or_else(|| "I64".to_string());
            negate(&ty, &v)
        }
        Expr::RecordE {
            update: Opt(Some(base)),
            fields,
        } => {
            let mut v = match lookup(env, &base.root) {
                Some(v) => v,
                None => name_value(i, d, &base.root)?,
            };
            for f in &base.fields {
                v = project(i, &v, f)?;
            }
            let Value::Record(rec) = &v else {
                return Err(internal("update of non-record"));
            };
            let desc_fields = record_field_names(i, rec.type_id)?;
            let mut new_fields: Vec<Value> = rec.fields.to_vec();
            for init in fields {
                let idx = desc_fields
                    .iter()
                    .position(|n| *n == init.name)
                    .ok_or_else(|| internal("unknown field in update"))?;
                new_fields[idx] = eval(i, d, env, &init.value)?;
            }
            Ok(Value::Record(Ref::new(Record {
                type_id: rec.type_id,
                fields: new_fields.into_boxed_slice(),
            })))
        }
        Expr::RecordE {
            update: Opt(None),
            fields,
        } => {
            // Unique field-set resolution (the checker guaranteed it).
            let mut names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
            names.sort_unstable();
            let ty = i
                .prog
                .types
                .values()
                .find(|td| match &td.body {
                    TypeBody::Record { fields: fs } => {
                        let mut have: Vec<&str> = fs
                            .iter()
                            .filter(|f| f.ignored.0.is_none())
                            .map(|f| f.name.as_str())
                            .collect();
                        have.sort_unstable();
                        have == names
                    }
                    _ => false,
                })
                .map(|td| td.name.clone())
                .ok_or_else(|| internal("no record type for literal"))?;
            record_literal(i, d, env, &ty, fields)
        }
        Expr::Hole { name } => Err(internal(format!("unfilled hole `?{name}`"))),
    }
}

fn strip_refined(ty: &Spanned<AType>) -> &Spanned<AType> {
    match &ty.item {
        AType::Refined { base, .. } => strip_refined(base),
        _ => ty,
    }
}

fn literal_value(i: &I, d: usize, lit: &Lit, span: Span) -> Result<Value, SoilError> {
    match lit {
        Lit::LInt { digits } => {
            let ty = i.ops[d]
                .get(&(span.start, span.end))
                .cloned()
                .unwrap_or_else(|| "I64".to_string());
            let v: i128 = digits.parse().map_err(|_| internal("bad literal"))?;
            Ok(int_value(&ty, v).expect("checker range-checked"))
        }
        Lit::LFloat { text } => Ok(Value::F64(text.parse().map_err(|_| internal("bad float"))?)),
        Lit::LStr { value } => Ok(Value::Utf8(Ref::from(value.as_str()))),
    }
}

fn int_value(ty: &str, v: i128) -> Option<Value> {
    Some(match ty {
        "I64" => Value::I64(i64::try_from(v).ok()?),
        "U64" => Value::U64(u64::try_from(v).ok()?),
        "I32" => Value::I32(i32::try_from(v).ok()?),
        "U32" => Value::U32(u32::try_from(v).ok()?),
        "I16" => Value::I16(i16::try_from(v).ok()?),
        "U16" => Value::U16(u16::try_from(v).ok()?),
        "I8" => Value::I8(i8::try_from(v).ok()?),
        "U8" => Value::U8(u8::try_from(v).ok()?),
        "BigInt" => Value::BigInt(Ref::new(BigInt::from(v))),
        _ => return None,
    })
}

fn int_of(v: &Value) -> Option<i128> {
    Some(match v {
        Value::I64(x) => *x as i128,
        Value::U64(x) => *x as i128,
        Value::I32(x) => *x as i128,
        Value::U32(x) => *x as i128,
        Value::I16(x) => *x as i128,
        Value::U16(x) => *x as i128,
        Value::I8(x) => *x as i128,
        Value::U8(x) => *x as i128,
        _ => return None,
    })
}

/// Floor division and floor modulus (Python semantics, syntax spec §5.6).
fn floor_div_mod(a: i128, b: i128) -> (i128, i128) {
    let q = a / b;
    let r = a % b;
    if r != 0 && ((r < 0) != (b < 0)) {
        (q - 1, r + b)
    } else {
        (q, r)
    }
}

fn arith(ty: &str, op: ArithOp, l: &Value, r: &Value) -> Result<Value, SoilError> {
    if ty == "F64" {
        let (Value::F64(a), Value::F64(b)) = (l, r) else {
            return Err(internal("F64 op"));
        };
        let out = match op {
            ArithOp::Add => a + b,
            ArithOp::Sub => a - b,
            ArithOp::Mul => a * b,
            ArithOp::Div => a / b,
            ArithOp::Mod => {
                // Floor modulus on F64 too, matching integers.
                let m = a % b;
                if m != 0.0 && (m < 0.0) != (*b < 0.0) {
                    m + b
                } else {
                    m
                }
            }
        };
        return Ok(Value::F64(out));
    }
    if ty == "BigInt" {
        let (Value::BigInt(a), Value::BigInt(b)) = (l, r) else {
            return Err(internal("BigInt op"));
        };
        use num_traits_shim::*;
        let a: &BigInt = a;
        let b: &BigInt = b;
        let out = match op {
            ArithOp::Add => a + b,
            ArithOp::Sub => a - b,
            ArithOp::Mul => a * b,
            ArithOp::Div | ArithOp::Mod => {
                if is_zero(b) {
                    return Err(SoilError::new(PanicKind::DivideByZero, "division by zero"));
                }
                let (q, m) = big_floor_div_mod(a, b);
                if matches!(op, ArithOp::Div) {
                    q
                } else {
                    m
                }
            }
        };
        return Ok(Value::BigInt(Ref::new(out)));
    }
    let (Some(a), Some(b)) = (int_of(l), int_of(r)) else {
        return Err(internal("int op"));
    };
    let out = match op {
        ArithOp::Add => a + b,
        ArithOp::Sub => a - b,
        ArithOp::Mul => a.checked_mul(b).ok_or_else(overflow)?,
        ArithOp::Div | ArithOp::Mod => {
            if b == 0 {
                return Err(SoilError::new(PanicKind::DivideByZero, "division by zero"));
            }
            let (q, m) = floor_div_mod(a, b);
            if matches!(op, ArithOp::Div) {
                q
            } else {
                m
            }
        }
    };
    int_value(ty, out).ok_or_else(overflow)
}

fn overflow() -> SoilError {
    SoilError::new(PanicKind::Overflow, "integer overflow")
}

fn negate(ty: &str, v: &Value) -> Result<Value, SoilError> {
    match v {
        Value::F64(x) => Ok(Value::F64(-x)),
        Value::BigInt(b) => {
            let b: &BigInt = b;
            Ok(Value::BigInt(Ref::new(-b.clone())))
        }
        _ => {
            let a = int_of(v).ok_or_else(|| internal("neg"))?;
            int_value(ty, -a).ok_or_else(overflow)
        }
    }
}

/// BigInt helpers via soil-rt's re-export.
mod num_traits_shim {
    use soil_rt::BigInt;

    pub fn is_zero(b: &BigInt) -> bool {
        *b == BigInt::from(0)
    }

    pub fn big_floor_div_mod(a: &BigInt, b: &BigInt) -> (BigInt, BigInt) {
        let q = a / b;
        let r = a % b;
        let zero = BigInt::from(0);
        if r != zero && ((r < zero) != (*b < zero)) {
            (q - BigInt::from(1), r + b)
        } else {
            (q, r)
        }
    }
}

// ---- names, defs, builtins ----

/// A non-local name: definition, private, or builtin.
fn name_value(i: &I, d: usize, name: &str) -> Result<Value, SoilError> {
    let here = &i.prog.defs[d];
    let candidate = i
        .prog
        .defs
        .iter()
        .position(|x| x.name() == name && (!x.is_private() || x.dir == here.dir));
    if let Some(idx) = candidate {
        return def_value(i, idx);
    }
    if kernel::is_builtin(name) {
        return builtin_value(i, name);
    }
    Err(internal(format!("unbound name `{name}`")))
}

fn qualified_value(i: &I, d: usize, space: &str, name: &str) -> Result<Value, SoilError> {
    if space.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        let idx = i
            .prog
            .defs
            .iter()
            .position(|x| x.name() == name && x.module == space)
            .ok_or_else(|| internal(format!("unbound `{space}::{name}`")))?;
        return def_value(i, idx);
    }
    if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return ctor_value(i, d, &None, Some(space), name, None);
    }
    derived_or_numeric(i, space, name)
}

fn derived_or_numeric(i: &I, space: &str, name: &str) -> Result<Value, SoilError> {
    let ii = i.clone();
    match name {
        "eq" => Ok(curry2(move |a, b| {
            let rt = ii.rt.borrow();
            let r = soil_rt::ops::eq(&rt, &a, &b)?;
            drop(rt);
            Ok(bool_value(&ii, r))
        })),
        "compare" => Ok(curry2(move |a, b| {
            let rt = ii.rt.borrow();
            let ord = soil_rt::ops::compare(&rt, &a, &b)?;
            Ok(Value::I64(match ord {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }))
        })),
        "show" => Ok(Value::Closure(Ref::new(Closure::native(move |args| {
            let rt = ii.rt.borrow();
            let s = soil_rt::json::encode(&rt, &args[0])?;
            Ok(Value::Utf8(Ref::from(s)))
        })))),
        "hash" => Ok(Value::Closure(Ref::new(Closure::native(move |args| {
            let rt = ii.rt.borrow();
            Ok(Value::U64(soil_rt::ops::hash(&rt, &args[0])?))
        })))),
        op if kernel::NUMERIC_OPS.contains(&op) => {
            let ty = space.to_string();
            if op == "neg" {
                Ok(Value::Closure(Ref::new(Closure::native(move |args| {
                    negate(&ty, &args[0])
                }))))
            } else {
                let aop = match op {
                    "add" => ArithOp::Add,
                    "sub" => ArithOp::Sub,
                    "mul" => ArithOp::Mul,
                    "div" => ArithOp::Div,
                    _ => ArithOp::Mod,
                };
                Ok(curry2(move |a, b| arith(&ty, aop, &a, &b)))
            }
        }
        _ => Err(internal(format!("unknown qualified `{space}::{name}`"))),
    }
}

fn curry2(f: impl Fn(Value, Value) -> Result<Value, SoilError> + Clone + 'static) -> Value {
    Value::Closure(Ref::new(Closure::native(move |args| {
        let a = args[0].clone();
        let f = f.clone();
        Ok(Value::Closure(Ref::new(Closure::native(move |args2| {
            f(a.clone(), args2[0].clone())
        }))))
    })))
}

/// The (cached) value of a definition: a curried closure, or the body's
/// value for zero-parameter definitions (which are effect-free by
/// construction — rows attach only to arrows).
pub fn def_value(i: &I, idx: usize) -> Result<Value, SoilError> {
    if let Some(v) = i.def_cache.borrow().get(&idx) {
        return Ok(v.clone());
    }
    let def = &i.prog.defs[idx];
    let ast = &def.ast.item;
    let v = if ast.params.is_empty() {
        eval(i, idx, &None, &ast.body).map_err(|e| push_frame(e, i, idx))?
    } else {
        let params: Vec<String> = ast.params.iter().map(|p| p.name.clone()).collect();
        make_def_closure(i.clone(), idx, params, Vec::new())
    };
    i.def_cache.borrow_mut().insert(idx, v.clone());
    Ok(v)
}

fn push_frame(mut e: SoilError, i: &I, idx: usize) -> SoilError {
    let def = &i.prog.defs[idx];
    e.trace.push(TraceFrame {
        definition: format!("{}::{}", def.module, def.name()),
    });
    e
}

fn make_def_closure(i: I, idx: usize, params: Vec<String>, collected: Vec<Value>) -> Value {
    Value::Closure(Ref::new(Closure::native(move |args| {
        let mut c = collected.clone();
        c.push(args[0].clone());
        if c.len() == params.len() {
            let ast = i.prog.defs[idx].ast.item.clone();
            let mut env: Env = None;
            for (p, v) in params.iter().zip(&c) {
                env = bind(&env, p, v.clone());
            }
            eval(&i, idx, &env, &ast.body).map_err(|e| push_frame(e, &i, idx))
        } else {
            Ok(make_def_closure(i.clone(), idx, params.clone(), c))
        }
    })))
}

fn make_lambda(
    i: &I,
    d: usize,
    env: Env,
    params: Vec<ast::Binder>,
    body: Rc<Spanned<Expr>>,
    next: usize,
) -> Value {
    let i = i.clone();
    Value::Closure(Ref::new(Closure::native(move |args| {
        let new_env = bind(&env, &params[next].name, args[0].clone());
        if next + 1 == params.len() {
            eval(&i, d, &new_env, &body)
        } else {
            Ok(make_lambda(
                &i,
                d,
                new_env,
                params.clone(),
                body.clone(),
                next + 1,
            ))
        }
    })))
}

// ---- records and constructors ----

fn record_field_names(i: &I, id: TypeId) -> Result<Vec<String>, SoilError> {
    let rt = i.rt.borrow();
    let desc = rt.registry.get(id)?;
    match &desc.body {
        RtBody::Record(fields) => Ok(fields.iter().map(|f| f.name.clone()).collect()),
        _ => Err(internal("not a record type")),
    }
}

fn project(i: &I, v: &Value, field: &str) -> Result<Value, SoilError> {
    let Value::Record(rec) = v else {
        return Err(internal("field access on non-record"));
    };
    let names = record_field_names(i, rec.type_id)?;
    let idx = names
        .iter()
        .position(|n| n == field)
        .ok_or_else(|| internal(format!("no field `{field}`")))?;
    Ok(rec.fields[idx].clone())
}

/// Builds a record of registered type `id` from name→value pairs,
/// refilling ignored fields from their defaults.
fn construct_record(i: &I, id: TypeId, pairs: &[(String, Value)]) -> Result<Value, SoilError> {
    let rt = i.rt.borrow();
    let desc = rt.registry.get(id)?;
    let RtBody::Record(fields) = &desc.body else {
        return Err(internal("not a record"));
    };
    let mut non_ignored = Vec::new();
    for f in fields {
        if f.ignored.is_none() {
            let v = pairs
                .iter()
                .find(|(n, _)| n == &f.name)
                .map(|(_, v)| v.clone())
                .ok_or_else(|| internal(format!("missing field `{}`", f.name)))?;
            non_ignored.push(v);
        }
    }
    let mut out = Vec::new();
    let mut ni = non_ignored.iter();
    for f in fields {
        match &f.ignored {
            None => out.push(ni.next().unwrap().clone()),
            Some(default) => out.push(default.call(&non_ignored)?),
        }
    }
    Ok(Value::Record(Ref::new(Record {
        type_id: id,
        fields: out.into_boxed_slice(),
    })))
}

fn record_literal(
    i: &I,
    d: usize,
    env: &Env,
    ty_name: &str,
    inits: &[ast::FieldInit],
) -> Result<Value, SoilError> {
    let mut pairs = Vec::new();
    for init in inits {
        pairs.push((init.name.clone(), eval(i, d, env, &init.value)?));
    }
    construct_record(i, i.erased_id(ty_name)?, &pairs)
}

fn ctor_value(
    i: &I,
    d: usize,
    env: &Env,
    qualifier: Option<&str>,
    name: &str,
    arg: Option<&Spanned<Expr>>,
) -> Result<Value, SoilError> {
    let owner = match qualifier {
        Some(t) => t.to_string(),
        None => i
            .prog
            .variant_owners
            .get(name)
            .and_then(|o| o.first())
            .cloned()
            .ok_or_else(|| internal(format!("unknown constructor `{name}`")))?,
    };
    if owner == "Bool" {
        return Ok(bool_value(i, name == "True"));
    }
    let td = i
        .prog
        .types
        .get(&owner)
        .ok_or_else(|| internal("unknown type"))?;
    let TypeBody::Sum { variants } = &td.body else {
        return Err(internal("not a sum"));
    };
    let variant = variants
        .iter()
        .position(|v| v.name == name)
        .ok_or_else(|| internal("unknown variant"))?;
    let vd = &variants[variant];
    let payload = if let Opt(Some(_)) = &vd.payload {
        let a = arg.ok_or_else(|| internal("missing payload"))?;
        Some(eval(i, d, env, a)?)
    } else if vd.record_fields.is_some() {
        let a = arg.ok_or_else(|| internal("missing payload"))?;
        let Expr::RecordE {
            update: Opt(None),
            fields,
        } = &a.item
        else {
            return Err(internal("inline payload expects a record literal"));
        };
        let syn = format!("{owner}.{name}");
        let mut pairs = Vec::new();
        for init in fields {
            pairs.push((init.name.clone(), eval(i, d, env, &init.value)?));
        }
        Some(construct_record(i, i.erased_id(&syn)?, &pairs)?)
    } else {
        None
    };
    Ok(Value::Sum(Ref::new(SumVal {
        type_id: i.erased_id(&owner)?,
        variant: variant as u32,
        payload,
    })))
}

// ---- pattern matching ----

fn variant_name_of(i: &I, v: &soil_rt::SumVal) -> Result<String, SoilError> {
    let rt = i.rt.borrow();
    let desc = rt.registry.get(v.type_id)?;
    match &desc.body {
        RtBody::Sum(variants) => Ok(variants[v.variant as usize].name.clone()),
        _ => Err(internal("not a sum")),
    }
}

fn match_pattern(
    i: &I,
    pat: &Spanned<Pattern>,
    v: &Value,
) -> Result<Option<Vec<(String, Value)>>, SoilError> {
    match &pat.item {
        Pattern::PWild => Ok(Some(Vec::new())),
        Pattern::PBind { binder } => Ok(Some(vec![(binder.name.clone(), v.clone())])),
        Pattern::PLit { lit } => {
            let matches = match (lit, v) {
                (Lit::LInt { digits }, _) => {
                    let want: i128 = digits.parse().map_err(|_| internal("bad literal"))?;
                    int_of(v) == Some(want)
                }
                (Lit::LFloat { text }, Value::F64(x)) => {
                    let want: f64 = text.parse().map_err(|_| internal("bad float"))?;
                    *x == want
                }
                (Lit::LStr { value }, Value::Utf8(s)) => &**s == value.as_str(),
                _ => false,
            };
            Ok(if matches { Some(Vec::new()) } else { None })
        }
        Pattern::PCtor { name, arg, .. } => {
            let Value::Sum(sv) = v else {
                return Err(internal("ctor pattern on non-sum"));
            };
            if variant_name_of(i, sv)? != *name {
                return Ok(None);
            }
            match (&arg.0, &sv.payload) {
                (None, _) => Ok(Some(Vec::new())),
                (Some(sub), Some(p)) => match_pattern(i, sub, p),
                (Some(sub), None) => {
                    if matches!(sub.item, Pattern::PWild) {
                        Ok(Some(Vec::new()))
                    } else {
                        Err(internal("payload pattern on nullary variant"))
                    }
                }
            }
        }
        Pattern::PRecord { fields, .. } => {
            let Value::Record(rec) = v else {
                return Err(internal("record pattern on non-record"));
            };
            let names = record_field_names(i, rec.type_id)?;
            let mut out = Vec::new();
            for fp in fields {
                let idx = names
                    .iter()
                    .position(|n| *n == fp.name)
                    .ok_or_else(|| internal("no such field"))?;
                let fv = rec.fields[idx].clone();
                match &fp.pattern {
                    Opt(Some(sub)) => match match_pattern(i, sub, &fv)? {
                        Some(mut bs) => out.append(&mut bs),
                        None => return Ok(None),
                    },
                    Opt(None) => out.push((fp.name.clone(), fv)),
                }
            }
            Ok(Some(out))
        }
    }
}

// ---- builtins ----

fn fs_error(i: &I, variant: &str, pairs: &[(String, Value)]) -> Result<Value, SoilError> {
    let syn = format!("FsError.{variant}");
    let payload = construct_record(i, i.erased_id(&syn)?, pairs)?;
    let td = i.prog.types.get("FsError").unwrap();
    let TypeBody::Sum { variants } = &td.body else {
        unreachable!()
    };
    let idx = variants.iter().position(|v| v.name == variant).unwrap();
    Ok(Value::Sum(Ref::new(SumVal {
        type_id: i.erased_id("FsError")?,
        variant: idx as u32,
        payload: Some(payload),
    })))
}

fn result_value(i: &I, ok: bool, payload: Value) -> Result<Value, SoilError> {
    Ok(Value::Sum(Ref::new(SumVal {
        type_id: i.erased_id("Result")?,
        variant: u32::from(!ok),
        payload: Some(payload),
    })))
}

fn utf8_of(v: &Value) -> Result<String, SoilError> {
    match v {
        Value::Utf8(s) => Ok(s.to_string()),
        _ => Err(internal("expected Utf8")),
    }
}

pub fn builtin_value(i: &I, name: &str) -> Result<Value, SoilError> {
    let ii = i.clone();
    Ok(match name {
        "unit" => Value::Unit,
        "world_fs" => Value::Closure(Ref::new(Closure::native(move |_| {
            opaque(&ii, "Fs", Box::new(FsImpl::Real))
        }))),
        "world_clock" => Value::Closure(Ref::new(Closure::native(move |_| {
            opaque(&ii, "Clock", Box::new(ClockImpl::Real))
        }))),
        "world_rand" => Value::Closure(Ref::new(Closure::native(move |_| {
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
                .unwrap_or(0x9E3779B97F4A7C15)
                ^ (std::process::id() as u64) << 32;
            opaque(
                &ii,
                "Rand",
                Box::new(RandImpl::Split(std::cell::Cell::new(seed))),
            )
        }))),
        "fake_fs" => Value::Closure(Ref::new(Closure::native(move |args| {
            let Value::Map(m) = &args[0] else {
                return Err(internal("fake_fs takes a Map"));
            };
            let mut files = Vec::new();
            for (k, v) in m.entries() {
                files.push((utf8_of(k)?, utf8_of(v)?));
            }
            opaque(&ii, "Fs", Box::new(FsImpl::Fake(files)))
        }))),
        "fake_clock" => Value::Closure(Ref::new(Closure::native(move |args| {
            let Value::I64(t) = args[0] else {
                return Err(internal("fake_clock takes I64"));
            };
            opaque(&ii, "Clock", Box::new(ClockImpl::Fake(t)))
        }))),
        "fake_rand" => Value::Closure(Ref::new(Closure::native(move |args| {
            let Value::U64(seed) = args[0] else {
                return Err(internal("fake_rand takes U64"));
            };
            opaque(
                &ii,
                "Rand",
                Box::new(RandImpl::Split(std::cell::Cell::new(seed))),
            )
        }))),
        "fs_read_bytes" => curry2(move |fs, path| {
            let Value::Opaque(o) = &fs else {
                return Err(internal("expected Fs"));
            };
            let p = utf8_of(&path)?;
            let imp = o
                .payload
                .downcast_ref::<FsImpl>()
                .ok_or_else(|| internal("expected Fs"))?;
            match imp {
                FsImpl::Real => match std::fs::read(&p) {
                    Ok(bytes) => result_value(&ii, true, Value::Bytes(Ref::from(bytes))),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        let err = fs_error(
                            &ii,
                            "NotFound",
                            &[("path".to_string(), Value::Utf8(Ref::from(p.as_str())))],
                        )?;
                        result_value(&ii, false, err)
                    }
                    Err(e) => {
                        let err = fs_error(
                            &ii,
                            "ReadFailed",
                            &[
                                ("path".to_string(), Value::Utf8(Ref::from(p.as_str()))),
                                ("message".to_string(), Value::Utf8(Ref::from(e.to_string()))),
                            ],
                        )?;
                        result_value(&ii, false, err)
                    }
                },
                FsImpl::Fake(files) => match files.iter().find(|(k, _)| *k == p) {
                    Some((_, contents)) => {
                        result_value(&ii, true, Value::Bytes(Ref::from(contents.as_bytes())))
                    }
                    None => {
                        let err = fs_error(
                            &ii,
                            "NotFound",
                            &[("path".to_string(), Value::Utf8(Ref::from(p.as_str())))],
                        )?;
                        result_value(&ii, false, err)
                    }
                },
            }
        }),
        "clock_now" => Value::Closure(Ref::new(Closure::native(move |args| {
            let Value::Opaque(o) = &args[0] else {
                return Err(internal("expected Clock"));
            };
            let imp = o
                .payload
                .downcast_ref::<ClockImpl>()
                .ok_or_else(|| internal("expected Clock"))?;
            Ok(Value::I64(match imp {
                ClockImpl::Real => std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
                ClockImpl::Fake(t) => *t,
            }))
        }))),
        "rand_u64" => Value::Closure(Ref::new(Closure::native(move |args| {
            let Value::Opaque(o) = &args[0] else {
                return Err(internal("expected Rand"));
            };
            let RandImpl::Split(state) = o
                .payload
                .downcast_ref::<RandImpl>()
                .ok_or_else(|| internal("expected Rand"))?;
            Ok(Value::U64(splitmix_next(state)))
        }))),
        "utf8_decode" => Value::Closure(Ref::new(Closure::native(move |args| {
            let Value::Bytes(b) = &args[0] else {
                return Err(internal("expected Bytes"));
            };
            match std::str::from_utf8(b) {
                Ok(s) => result_value(&ii, true, Value::Utf8(Ref::from(s))),
                Err(e) => {
                    let at = Value::U64(e.valid_up_to() as u64);
                    let payload = construct_record(
                        &ii,
                        ii.erased_id("Utf8Error.InvalidUtf8")?,
                        &[("at".to_string(), at)],
                    )?;
                    let err = Value::Sum(Ref::new(SumVal {
                        type_id: ii.erased_id("Utf8Error")?,
                        variant: 0,
                        payload: Some(payload),
                    }));
                    result_value(&ii, false, err)
                }
            }
        }))),
        "utf8_encode" => Value::Closure(Ref::new(Closure::native(move |args| {
            let s = utf8_of(&args[0])?;
            Ok(Value::Bytes(Ref::from(s.into_bytes())))
        }))),
        "list_len" => Value::Closure(Ref::new(Closure::native(move |args| {
            let Value::List(l) = &args[0] else {
                return Err(internal("expected List"));
            };
            Ok(Value::I64(l.len() as i64))
        }))),
        "list_empty" => Value::List(Ref::new(Vec::new())),
        "list_nth" => curry2(move |l, idx| {
            let Value::List(l) = &l else {
                return Err(internal("expected List"));
            };
            let Value::I64(n) = idx else {
                return Err(internal("expected I64 index"));
            };
            usize::try_from(n)
                .ok()
                .and_then(|n| l.get(n))
                .cloned()
                .ok_or_else(|| {
                    SoilError::new(
                        PanicKind::IndexOutOfBounds,
                        format!("index {n} out of bounds"),
                    )
                })
        }),
        "list_append" => curry2(move |l, x| {
            let Value::List(l) = &l else {
                return Err(internal("expected List"));
            };
            let mut out = l.to_vec();
            out.push(x);
            Ok(Value::List(Ref::new(out)))
        }),
        _ => return Err(internal(format!("unknown builtin `{name}`"))),
    })
}

// ---- run / test (step 9) ----

fn ground_ty_of_ast(prog: &Program, ty: &Spanned<AType>) -> Result<Ty, Diagnostic> {
    match &ty.item {
        AType::Refined { base, .. } => ground_ty_of_ast(prog, base),
        AType::TVar { name } => Err(Diagnostic::bare(
            Code::MalformedInput,
            format!("entry parameter type variable `{name}` is not ground (contract §8.6)"),
        )),
        AType::Arrow { .. } => Err(Diagnostic::bare(
            Code::MalformedInput,
            "function-typed entry parameters cannot cross the CLI boundary".to_string(),
        )),
        AType::Con { name, args } => {
            let mut converted = Vec::new();
            for a in args {
                converted.push(ground_ty_of_ast(prog, a)?);
            }
            // Expand aliases like the checker does.
            if let Some(td) = prog.types.get(name) {
                if let TypeBody::Alias { ty: target } = &td.body {
                    let subst: HashMap<String, Ty> =
                        td.params.iter().cloned().zip(converted).collect();
                    return Ok(conv_sig_prog(prog, target, &subst));
                }
            }
            Ok(Ty::Con {
                name: name.clone(),
                args: converted,
            })
        }
    }
}

fn runtime_panic(e: SoilError, file: Option<String>) -> Diagnostic {
    Diagnostic {
        code: Code::RuntimePanic,
        message: format!("{}: {}", e.kind.name(), e.message),
        file: Opt(file),
        span: Opt(None),
        notes: e
            .trace
            .iter()
            .rev()
            .map(|f| Note {
                message: format!("in {}", f.definition),
                file: Opt(None),
                span: Opt(None),
            })
            .collect(),
    }
}

/// Peeled parameter types of a definition (declared signature, aliases
/// expanded, refinements erased).
fn param_and_result_tys(prog: &Program, def: &LoadedDef) -> Result<(Vec<Ty>, Ty), Diagnostic> {
    let mut cur = &def.ast.item.sig;
    let mut params = Vec::new();
    for _ in &def.ast.item.params {
        let stripped = strip_refined(cur);
        let AType::Arrow { dom, cod, .. } = &stripped.item else {
            return Err(Diagnostic::bare(
                Code::Internal,
                "sig/param mismatch".to_string(),
            ));
        };
        params.push(ground_ty_of_ast(prog, dom)?);
        cur = cod;
    }
    Ok((params, ground_ty_of_ast(prog, cur)?))
}

fn resolve_def(prog: &Program, name: &str) -> Result<usize, Diagnostic> {
    let matches: Vec<usize> = prog
        .defs
        .iter()
        .enumerate()
        .filter(|(_, d)| d.name() == name)
        .map(|(i, _)| i)
        .collect();
    match matches.as_slice() {
        [one] => Ok(*one),
        [] => Err(Diagnostic::bare(
            Code::Usage,
            format!("no definition named `{name}`"),
        )),
        _ => Err(Diagnostic::bare(
            Code::Usage,
            format!("`{name}` names several definitions; qualify the manifest"),
        )),
    }
}

/// Transitive hole check (contract §8.6): entry or any reachable callee.
fn check_no_holes(
    prog: &Program,
    rename_out: &crate::rename::RenameOutput,
    infer_out: &crate::infer::InferOutput,
    entry: usize,
) -> Result<(), Diagnostic> {
    let index_of = |spelling: &str| -> Option<usize> {
        if let Some((module, name)) = spelling.split_once("::") {
            prog.defs
                .iter()
                .position(|d| d.name() == name && d.module == module)
        } else {
            prog.defs.iter().position(|d| d.name() == spelling)
        }
    };
    let mut seen = vec![false; prog.defs.len()];
    let mut stack = vec![entry];
    while let Some(idx) = stack.pop() {
        if seen[idx] {
            continue;
        }
        seen[idx] = true;
        if !infer_out.defs[idx].checks.holes.is_empty() {
            return Err(Diagnostic::bare(
                Code::UnfilledHole,
                format!(
                    "`{}` (or a callee) contains typed holes; fill them before running",
                    prog.defs[idx].name()
                ),
            ));
        }
        let refs = &rename_out.defs[idx].refs;
        for spelling in refs.defs.iter().chain(refs.privates.iter()) {
            if let Some(t) = index_of(spelling) {
                stack.push(t);
            }
        }
    }
    Ok(())
}

pub struct Session {
    pub core: I,
    pub rename_out: crate::rename::RenameOutput,
    pub infer_out: crate::infer::InferOutput,
}

/// Loads, renames, checks (incl. exhaustiveness), and builds the
/// interpreter core.
pub fn session(prog: Program) -> Result<Session, Diagnostic> {
    let prog = Rc::new(prog);
    let rename_out = crate::rename::rename_program(&prog)?;
    let mut ops = Vec::new();
    let mut infos = Vec::new();
    for index in 0..prog.defs.len() {
        let checked = crate::infer::check_def_all(&prog, index)?;
        for site in &checked.matches {
            crate::exhaust::check_match_site(&prog, &prog.defs[index].path, site)?;
        }
        ops.push(checked.ops);
        infos.push(checked.info);
    }
    let infer_out = crate::infer::InferOutput { defs: infos };
    let core = Core::new(prog, ops)?;
    Ok(Session {
        core,
        rename_out,
        infer_out,
    })
}

pub fn cmd_run(prog: Program, entry: &str, args_json: &str) -> Result<String, Diagnostic> {
    let s = session(prog)?;
    let entry_idx = resolve_def(&s.core.prog, entry)?;
    check_no_holes(&s.core.prog, &s.rename_out, &s.infer_out, entry_idx)?;
    let (param_tys, _) = param_and_result_tys(&s.core.prog, &s.core.prog.defs[entry_idx])?;

    let args: Vec<serde_json::Value> = serde_json::from_str(args_json)
        .map_err(|e| Diagnostic::bare(Code::Usage, format!("--args must be a JSON array: {e}")))?;
    let mut arg_iter = args.into_iter();
    let mut values = Vec::new();
    for ty in &param_tys {
        if let Ty::Con { name, .. } = ty {
            if CAPS.contains(&name.as_str()) {
                values.push(real_capability(&s.core, name).map_err(|e| runtime_panic(e, None))?);
                continue;
            }
        }
        let json = arg_iter.next().ok_or_else(|| {
            Diagnostic::bare(Code::Usage, "too few arguments for the entry".to_string())
        })?;
        values.push(decode_arg(&s.core, ty, &json)?);
    }
    if arg_iter.next().is_some() {
        return Err(Diagnostic::bare(
            Code::Usage,
            "too many arguments for the entry".to_string(),
        ));
    }

    let file = Some(s.core.prog.defs[entry_idx].path.clone());
    let mut v = def_value(&s.core, entry_idx).map_err(|e| runtime_panic(e, file.clone()))?;
    for a in values {
        let Value::Closure(c) = &v else {
            return Err(Diagnostic::bare(
                Code::Internal,
                "arity mismatch".to_string(),
            ));
        };
        v = c.call(&[a]).map_err(|e| runtime_panic(e, file.clone()))?;
    }
    let rt = s.core.rt.borrow();
    soil_rt::json::encode(&rt, &v).map_err(|e| runtime_panic(e.clone(), file))
}

fn real_capability(i: &I, name: &str) -> Result<Value, SoilError> {
    match name {
        "World" => opaque(i, "World", Box::new(WorldTag)),
        "Fs" => opaque(i, "Fs", Box::new(FsImpl::Real)),
        "Clock" => opaque(i, "Clock", Box::new(ClockImpl::Real)),
        "Rand" => {
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
                .unwrap_or(1);
            opaque(
                i,
                "Rand",
                Box::new(RandImpl::Split(std::cell::Cell::new(seed))),
            )
        }
        _ => Err(internal(format!("no real capability for `{name}` in v1"))),
    }
}

fn decode_arg(i: &I, ty: &Ty, json: &serde_json::Value) -> Result<Value, Diagnostic> {
    let shape = i.ground_shape(ty).map_err(|e| runtime_panic(e, None))?;
    let text = serde_json::to_string(json).expect("re-serialize");
    let rt = i.rt.borrow();
    soil_rt::json::decode(&rt, &shape, &text).map_err(|e| Diagnostic {
        code: Code::MalformedInput,
        message: format!("argument does not decode: {}", e.message),
        file: Opt(None),
        span: Opt(None),
        notes: vec![],
    })
}

// ---- test bundles (contract §10) ----

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bundle {
    program: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    def: String,
    binds: Vec<Bind>,
    args: Vec<Arg>,
    expect: Expect,
    xfail: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bind {
    bind: String,
    ctor: String,
    args: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(tag = "tag", content = "value")]
enum Arg {
    Json(serde_json::Value),
    Binding { name: String },
}

#[derive(Deserialize)]
#[serde(tag = "tag", content = "value")]
enum Expect {
    Value(serde_json::Value),
    Panic,
}

#[derive(Serialize)]
pub struct TestOutput {
    cases: Vec<CaseOut>,
}

#[derive(Serialize)]
struct CaseOut {
    name: String,
    result: &'static str,
    details: Opt<Details>,
}

#[derive(Serialize)]
struct Details {
    expected: String,
    actual: String,
}

pub fn cmd_test(bundle_path: &str) -> Result<(String, i32), Diagnostic> {
    let text = std::fs::read_to_string(bundle_path)
        .map_err(|e| Diagnostic::bare(Code::Io, format!("cannot read `{bundle_path}`: {e}")))?;
    let bundle: Bundle = serde_json::from_str(&text)
        .map_err(|e| Diagnostic::bare(Code::MalformedInput, format!("{bundle_path}: {e}")))?;
    let dir = std::path::Path::new(bundle_path)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let manifest = dir.join(&bundle.program);
    let prog = crate::manifest::load_program(manifest.to_str().unwrap_or(&bundle.program))?;
    let s = session(prog)?;

    let mut out = Vec::new();
    let mut failing = false;
    for case in &bundle.cases {
        let def_idx = resolve_def(&s.core.prog, &case.def)?;
        check_no_holes(&s.core.prog, &s.rename_out, &s.infer_out, def_idx)?;
        let (param_tys, result_ty) =
            param_and_result_tys(&s.core.prog, &s.core.prog.defs[def_idx])?;

        // Fakes.
        let mut binds: Vec<(String, Value)> = Vec::new();
        for b in &case.binds {
            if !kernel::is_builtin(&b.ctor) {
                return Err(Diagnostic::bare(
                    Code::MalformedInput,
                    format!("`{}` is not a fake constructor", b.ctor),
                ));
            }
            let sig = parser::parse_type_str(kernel::builtin_sig(&b.ctor).unwrap())
                .expect("builtin sigs parse");
            let mut cur = &sig;
            let mut v = builtin_value(&s.core, &b.ctor).map_err(|e| runtime_panic(e, None))?;
            for a in &b.args {
                let stripped = strip_refined(cur);
                let AType::Arrow { dom, cod, .. } = &stripped.item else {
                    return Err(Diagnostic::bare(
                        Code::MalformedInput,
                        "too many fake args".to_string(),
                    ));
                };
                let ty = ground_ty_of_ast(&s.core.prog, dom)?;
                let av = decode_arg(&s.core, &ty, a)?;
                let Value::Closure(c) = &v else {
                    return Err(Diagnostic::bare(Code::Internal, "fake arity".to_string()));
                };
                v = c.call(&[av]).map_err(|e| runtime_panic(e, None))?;
                cur = cod;
            }
            binds.push((b.bind.clone(), v));
        }

        if case.args.len() != param_tys.len() {
            return Err(Diagnostic::bare(
                Code::MalformedInput,
                format!("case `{}`: wrong argument count", case.name),
            ));
        }
        let mut values = Vec::new();
        for (arg, ty) in case.args.iter().zip(&param_tys) {
            values.push(match arg {
                Arg::Binding { name } => binds
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, v)| v.clone())
                    .ok_or_else(|| {
                        Diagnostic::bare(Code::MalformedInput, format!("unknown binding `{name}`"))
                    })?,
                Arg::Json(j) => decode_arg(&s.core, ty, j)?,
            });
        }

        // Call.
        let outcome: Result<Value, SoilError> = (|| {
            let mut v = def_value(&s.core, def_idx)?;
            for a in values {
                let Value::Closure(c) = &v else {
                    return Err(internal("arity"));
                };
                v = c.call(&[a])?;
            }
            Ok(v)
        })();

        let (passed, expected_s, actual_s) = match (&case.expect, &outcome) {
            (Expect::Panic, Err(_)) => (true, "panic".to_string(), "panic".to_string()),
            (Expect::Panic, Ok(v)) => {
                let rt = s.core.rt.borrow();
                let enc =
                    soil_rt::json::encode(&rt, v).unwrap_or_else(|_| "<unencodable>".to_string());
                (false, "panic".to_string(), enc)
            }
            (Expect::Value(_), Err(e)) => (
                false,
                expected_canonical(&s.core, &case.expect, &result_ty)?,
                format!("panic: {}: {}", e.kind.name(), e.message),
            ),
            (Expect::Value(_), Ok(v)) => {
                let expected = expected_canonical(&s.core, &case.expect, &result_ty)?;
                let rt = s.core.rt.borrow();
                let actual =
                    soil_rt::json::encode(&rt, v).map_err(|e| runtime_panic(e.clone(), None))?;
                (expected == actual, expected, actual)
            }
        };

        let result = match (passed, case.xfail) {
            (true, false) => "pass",
            (true, true) => "xpass",
            (false, true) => "xfail",
            (false, false) => "fail",
        };
        if matches!(result, "fail" | "xpass") {
            failing = true;
        }
        out.push(CaseOut {
            name: case.name.clone(),
            result,
            details: if matches!(result, "fail" | "xpass") {
                Opt(Some(Details {
                    expected: expected_s,
                    actual: actual_s,
                }))
            } else {
                Opt(None)
            },
        });
    }
    let doc = serde_json::to_string(&TestOutput { cases: out })
        .expect("test output serialization cannot fail");
    Ok((doc, i32::from(failing)))
}

fn expected_canonical(i: &I, expect: &Expect, result_ty: &Ty) -> Result<String, Diagnostic> {
    let Expect::Value(json) = expect else {
        return Ok("panic".to_string());
    };
    let v = decode_arg(i, result_ty, json)?;
    let rt = i.rt.borrow();
    soil_rt::json::encode(&rt, &v)
        .map_err(|e| Diagnostic::bare(Code::MalformedInput, e.message.clone()))
}
