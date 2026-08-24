//! Semantic types, contract §5: the span-free signature encoding shared
//! by `infer`/`check` output and `env.json` field/payload/alias shapes
//! (contract §6, resolved 2026-08-22). Checker-internal representations
//! (union-find variables) arrive with step 6.

use crate::ast::Row;
use crate::diag::Opt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tag", content = "value")]
pub enum SigType {
    SArrow {
        param: Opt<String>,
        dom: Box<SigType>,
        row: Row,
        cod: Box<SigType>,
    },
    SCon {
        name: String,
        args: Vec<SigType>,
    },
    SVar {
        name: String,
    },
}

impl SigType {
    pub fn con(name: &str) -> Self {
        SigType::SCon {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    pub fn var(name: &str) -> Self {
        SigType::SVar {
            name: name.to_string(),
        }
    }

    /// Does any `SArrow` occur in this shape? (Rejected in `env.json`
    /// v1, contract §6.)
    pub fn contains_arrow(&self) -> bool {
        match self {
            SigType::SArrow { .. } => true,
            SigType::SCon { args, .. } => args.iter().any(SigType::contains_arrow),
            SigType::SVar { .. } => false,
        }
    }

    /// Every `SVar` name in this shape.
    pub fn vars<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            SigType::SArrow { dom, cod, .. } => {
                dom.vars(out);
                cod.vars(out);
            }
            SigType::SCon { args, .. } => {
                for a in args {
                    a.vars(out);
                }
            }
            SigType::SVar { name } => out.push(name),
        }
    }

    /// Every `SCon` name in this shape.
    pub fn con_names<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            SigType::SArrow { dom, cod, .. } => {
                dom.con_names(out);
                cod.con_names(out);
            }
            SigType::SCon { name, args } => {
                out.push(name);
                for a in args {
                    a.con_names(out);
                }
            }
            SigType::SVar { .. } => {}
        }
    }
}

// ---- checker-internal types (step 6) ----

use crate::ast::Effect;

pub type TvId = u32;
pub type RvId = u32;

/// Effect sets as bitflags; `ffi` implies `panic` at construction from
/// declared rows (spec §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffSet(u8);

impl EffSet {
    pub const EMPTY: EffSet = EffSet(0);

    fn bit(e: Effect) -> u8 {
        match e {
            Effect::Div => 1,
            Effect::Panic => 2,
            Effect::Io => 4,
            Effect::Ffi => 8,
        }
    }

    pub fn single(e: Effect) -> EffSet {
        EffSet(Self::bit(e))
    }

    pub fn from_effects(effects: &[Effect]) -> EffSet {
        let mut s = 0;
        for e in effects {
            s |= Self::bit(*e);
        }
        if s & Self::bit(Effect::Ffi) != 0 {
            s |= Self::bit(Effect::Panic);
        }
        EffSet(s)
    }

    pub fn contains(self, e: Effect) -> bool {
        self.0 & Self::bit(e) != 0
    }

    pub fn union(self, o: EffSet) -> EffSet {
        EffSet(self.0 | o.0)
    }

    pub fn minus(self, o: EffSet) -> EffSet {
        EffSet(self.0 & !o.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn is_subset(self, o: EffSet) -> bool {
        self.0 & !o.0 == 0
    }

    pub fn effects(self) -> Vec<Effect> {
        [Effect::Div, Effect::Panic, Effect::Io, Effect::Ffi]
            .into_iter()
            .filter(|e| self.contains(*e))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tail {
    Closed,
    Open(RvId),
    /// A rigid row variable from the definition's own signature.
    Skolem(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowT {
    pub effects: EffSet,
    pub tail: Tail,
}

impl RowT {
    pub fn closed(effects: EffSet) -> RowT {
        RowT {
            effects,
            tail: Tail::Closed,
        }
    }

    pub fn total() -> RowT {
        RowT::closed(EffSet::EMPTY)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Var(TvId),
    /// A rigid type variable from the definition's own signature.
    Rigid(String),
    Con {
        name: String,
        args: Vec<Ty>,
    },
    Arrow {
        dom: Box<Ty>,
        row: RowT,
        cod: Box<Ty>,
    },
}

#[derive(Debug)]
pub enum UnifyErr {
    Mismatch(Ty, Ty),
    RowMismatch,
    Occurs,
}

/// The union-find store (impl plan 02 §1: mutable union-find, no
/// levels).
#[derive(Default)]
pub struct Store {
    tys: Vec<Option<Ty>>,
    /// A bound row variable stands for the *rest* beyond its position:
    /// resolving follows the chain, unioning effect sets.
    rows: Vec<Option<RowT>>,
}

impl Store {
    pub fn fresh_ty(&mut self) -> Ty {
        self.tys.push(None);
        Ty::Var((self.tys.len() - 1) as TvId)
    }

    pub fn fresh_row(&mut self) -> RowT {
        self.rows.push(None);
        RowT {
            effects: EffSet::EMPTY,
            tail: Tail::Open((self.rows.len() - 1) as RvId),
        }
    }

    /// Follows variable bindings one level (head resolution).
    pub fn shallow(&self, ty: &Ty) -> Ty {
        let mut t = ty.clone();
        while let Ty::Var(v) = t {
            match &self.tys[v as usize] {
                Some(bound) => t = bound.clone(),
                None => return Ty::Var(v),
            }
        }
        t
    }

    pub fn resolve_row(&self, row: &RowT) -> RowT {
        let mut effects = row.effects;
        let mut tail = row.tail.clone();
        while let Tail::Open(v) = tail {
            match &self.rows[v as usize] {
                Some(rest) => {
                    effects = effects.union(rest.effects);
                    tail = rest.tail.clone();
                }
                None => {
                    return RowT {
                        effects,
                        tail: Tail::Open(v),
                    }
                }
            }
        }
        RowT { effects, tail }
    }

    pub fn zonk(&self, ty: &Ty) -> Ty {
        match self.shallow(ty) {
            Ty::Var(v) => Ty::Var(v),
            Ty::Rigid(n) => Ty::Rigid(n),
            Ty::Con { name, args } => Ty::Con {
                name,
                args: args.iter().map(|a| self.zonk(a)).collect(),
            },
            Ty::Arrow { dom, row, cod } => Ty::Arrow {
                dom: Box::new(self.zonk(&dom)),
                row: self.resolve_row(&row),
                cod: Box::new(self.zonk(&cod)),
            },
        }
    }

    fn occurs(&self, v: TvId, ty: &Ty) -> bool {
        match self.shallow(ty) {
            Ty::Var(w) => w == v,
            Ty::Rigid(_) => false,
            Ty::Con { args, .. } => args.iter().any(|a| self.occurs(v, a)),
            Ty::Arrow { dom, cod, .. } => self.occurs(v, &dom) || self.occurs(v, &cod),
        }
    }

    pub fn unify(&mut self, a: &Ty, b: &Ty) -> Result<(), UnifyErr> {
        let (a, b) = (self.shallow(a), self.shallow(b));
        match (&a, &b) {
            (Ty::Var(x), Ty::Var(y)) if x == y => Ok(()),
            (Ty::Var(v), other) | (other, Ty::Var(v)) => {
                if self.occurs(*v, other) {
                    return Err(UnifyErr::Occurs);
                }
                self.tys[*v as usize] = Some(other.clone());
                Ok(())
            }
            (Ty::Rigid(x), Ty::Rigid(y)) if x == y => Ok(()),
            (Ty::Con { name: n1, args: a1 }, Ty::Con { name: n2, args: a2 })
                if n1 == n2 && a1.len() == a2.len() =>
            {
                for (x, y) in a1.iter().zip(a2) {
                    self.unify(x, y)?;
                }
                Ok(())
            }
            (
                Ty::Arrow {
                    dom: d1,
                    row: r1,
                    cod: c1,
                },
                Ty::Arrow {
                    dom: d2,
                    row: r2,
                    cod: c2,
                },
            ) => {
                self.unify(d1, d2)?;
                self.unify_row(r1, r2)?;
                self.unify(c1, c2)
            }
            _ => Err(UnifyErr::Mismatch(a.clone(), b.clone())),
        }
    }

    /// Row unification with set semantics (impl plan 02 step 6).
    pub fn unify_row(&mut self, a: &RowT, b: &RowT) -> Result<(), UnifyErr> {
        let ra = self.resolve_row(a);
        let rb = self.resolve_row(b);
        match (&ra.tail, &rb.tail) {
            (Tail::Closed, Tail::Closed)
            | (Tail::Skolem(_), Tail::Closed)
            | (Tail::Closed, Tail::Skolem(_)) => {
                if ra.effects == rb.effects && ra.tail == rb.tail {
                    Ok(())
                } else {
                    Err(UnifyErr::RowMismatch)
                }
            }
            (Tail::Skolem(s1), Tail::Skolem(s2)) => {
                if s1 == s2 && ra.effects == rb.effects {
                    Ok(())
                } else {
                    Err(UnifyErr::RowMismatch)
                }
            }
            (Tail::Open(x), Tail::Open(y)) => {
                if x == y {
                    if ra.effects == rb.effects {
                        Ok(())
                    } else {
                        Err(UnifyErr::RowMismatch)
                    }
                } else {
                    let fresh = self.fresh_row();
                    let Tail::Open(z) = fresh.tail else {
                        unreachable!()
                    };
                    self.rows[*x as usize] = Some(RowT {
                        effects: rb.effects.minus(ra.effects),
                        tail: Tail::Open(z),
                    });
                    self.rows[*y as usize] = Some(RowT {
                        effects: ra.effects.minus(rb.effects),
                        tail: Tail::Open(z),
                    });
                    Ok(())
                }
            }
            (Tail::Open(x), _) => {
                if ra.effects.is_subset(rb.effects) {
                    self.rows[*x as usize] = Some(RowT {
                        effects: rb.effects.minus(ra.effects),
                        tail: rb.tail.clone(),
                    });
                    Ok(())
                } else {
                    Err(UnifyErr::RowMismatch)
                }
            }
            (_, Tail::Open(y)) => {
                if rb.effects.is_subset(ra.effects) {
                    self.rows[*y as usize] = Some(RowT {
                        effects: ra.effects.minus(rb.effects),
                        tail: ra.tail.clone(),
                    });
                    Ok(())
                } else {
                    Err(UnifyErr::RowMismatch)
                }
            }
        }
    }

    /// Binds an unbound open tail to `(effects, fresh open tail)` —
    /// the growth operation of the effect-fit fixpoint.
    pub fn grow_row(&mut self, tail_var: RvId, effects: EffSet) {
        debug_assert!(self.rows[tail_var as usize].is_none());
        let fresh = self.fresh_row();
        self.rows[tail_var as usize] = Some(RowT {
            effects,
            tail: fresh.tail,
        });
    }

    /// Binds an unbound open tail to a skolem rest.
    pub fn skolemize_row(&mut self, tail_var: RvId, skolem: String) {
        debug_assert!(self.rows[tail_var as usize].is_none());
        self.rows[tail_var as usize] = Some(RowT {
            effects: EffSet::EMPTY,
            tail: Tail::Skolem(skolem),
        });
    }

    /// Closes every still-unbound row variable (defaulting to no further
    /// effects), run once after solving.
    pub fn close_open_rows(&mut self) {
        for slot in &mut self.rows {
            if slot.is_none() {
                *slot = Some(RowT::total());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Effect;

    #[test]
    fn sigtype_round_trips() {
        let json = r#"{"tag":"SCon","value":{"name":"List","args":[{"tag":"SVar","value":{"name":"a"}}]}}"#;
        let t: SigType = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&t).unwrap(), json);
    }

    #[test]
    fn unify_binds_vars() {
        let mut s = Store::default();
        let v = s.fresh_ty();
        s.unify(
            &v,
            &Ty::Con {
                name: "I64".into(),
                args: vec![],
            },
        )
        .unwrap();
        assert_eq!(
            s.zonk(&v),
            Ty::Con {
                name: "I64".into(),
                args: vec![]
            }
        );
    }

    #[test]
    fn occurs_check() {
        let mut s = Store::default();
        let v = s.fresh_ty();
        let list = Ty::Con {
            name: "List".into(),
            args: vec![v.clone()],
        };
        assert!(matches!(s.unify(&v, &list), Err(UnifyErr::Occurs)));
    }

    #[test]
    fn row_unification() {
        let mut s = Store::default();
        // open ~ closed{io}: tail absorbs io.
        let open = s.fresh_row();
        let io = RowT::closed(EffSet::single(Effect::Io));
        s.unify_row(&open, &io).unwrap();
        assert_eq!(s.resolve_row(&open), io);
        // closed{io} ~ closed{} fails.
        assert!(s.unify_row(&io, &RowT::total()).is_err());
        // two opens with different prefixes meet in the middle.
        let a = s.fresh_row();
        let b = RowT {
            effects: EffSet::single(Effect::Panic),
            tail: s.fresh_row().tail,
        };
        s.unify_row(&a, &b).unwrap();
        assert!(s.resolve_row(&a).effects.contains(Effect::Panic));
    }
}
