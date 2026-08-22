use std::any::Any;
use std::cmp::Ordering;
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

use num_bigint::BigInt;

use crate::descriptor::TypeId;
use crate::error::{PanicKind, SoilError};

/// A reference-counted, immutable heap allocation.
///
/// Non-atomic by decision: a runtime instance and every value it owns
/// belong to a single thread. Cycles are unreachable through the public
/// API today but would leak; the cycle-collection strategy is explicitly
/// deferred (design §10).
///
/// Debug builds maintain a thread-local count of live allocations,
/// exposed through [`debug_live_values`], which backs the leak-check
/// exit criterion of plan 01.
pub struct Ref<T: ?Sized>(Rc<T>);

#[cfg(debug_assertions)]
mod live {
    use std::cell::Cell;

    thread_local! {
        static LIVE: Cell<usize> = const { Cell::new(0) };
    }

    pub fn inc() {
        LIVE.with(|c| c.set(c.get() + 1));
    }

    pub fn dec() {
        LIVE.with(|c| c.set(c.get() - 1));
    }

    pub fn count() -> usize {
        LIVE.with(|c| c.get())
    }
}

/// The number of live runtime allocations on this thread.
///
/// Meaningful in debug builds only; release builds compile the counter
/// out and always return 0.
pub fn debug_live_values() -> usize {
    #[cfg(debug_assertions)]
    {
        live::count()
    }
    #[cfg(not(debug_assertions))]
    {
        0
    }
}

impl<T> Ref<T> {
    pub fn new(value: T) -> Self {
        #[cfg(debug_assertions)]
        live::inc();
        Ref(Rc::new(value))
    }
}

impl<T: ?Sized> Ref<T> {
    /// The allocation's address — the identity used by the `opaque`
    /// derivation strategy (design §3.7).
    pub fn addr(&self) -> usize {
        Rc::as_ptr(&self.0) as *const () as usize
    }

    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Rc::ptr_eq(&a.0, &b.0)
    }
}

impl From<&str> for Ref<str> {
    fn from(s: &str) -> Self {
        #[cfg(debug_assertions)]
        live::inc();
        Ref(Rc::from(s))
    }
}

impl From<String> for Ref<str> {
    fn from(s: String) -> Self {
        #[cfg(debug_assertions)]
        live::inc();
        Ref(Rc::from(s.into_boxed_str()))
    }
}

impl From<&[u8]> for Ref<[u8]> {
    fn from(b: &[u8]) -> Self {
        #[cfg(debug_assertions)]
        live::inc();
        Ref(Rc::from(b))
    }
}

impl From<Vec<u8>> for Ref<[u8]> {
    fn from(b: Vec<u8>) -> Self {
        #[cfg(debug_assertions)]
        live::inc();
        Ref(Rc::from(b.into_boxed_slice()))
    }
}

impl<T: ?Sized> Clone for Ref<T> {
    fn clone(&self) -> Self {
        Ref(self.0.clone())
    }
}

impl<T: ?Sized> Drop for Ref<T> {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        if Rc::strong_count(&self.0) == 1 {
            live::dec();
        }
    }
}

impl<T: ?Sized> Deref for Ref<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Ref<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A Soil value.
///
/// Scalars are stored inline; everything else is behind a [`Ref`], so
/// `clone` is at most a refcount bump. `Bool` is deliberately absent: it
/// is the prelude sum `True | False` (design §3.12 / plan 01), which the
/// registry pre-registers and the JSON layer special-cases.
#[derive(Clone, Debug)]
pub enum Value {
    I64(i64),
    U64(u64),
    I32(i32),
    U32(u32),
    I16(i16),
    U16(u16),
    I8(i8),
    U8(u8),
    F64(f64),
    BigInt(Ref<BigInt>),
    Utf8(Ref<str>),
    Bytes(Ref<[u8]>),
    Unit,
    Record(Ref<Record>),
    Sum(Ref<SumVal>),
    List(Ref<Vec<Value>>),
    Map(Ref<MapVal>),
    Closure(Ref<Closure>),
    Opaque(Ref<OpaqueVal>),
}

/// A record value: its type plus field values in declaration order.
/// Field names live in the type's descriptor, not in the value.
#[derive(Debug)]
pub struct Record {
    pub type_id: TypeId,
    pub fields: Box<[Value]>,
}

/// A sum value: its type, the variant's declaration index, and at most
/// one payload (design §3.1). Variant names live in the descriptor.
#[derive(Debug)]
pub struct SumVal {
    pub type_id: TypeId,
    pub variant: u32,
    pub payload: Option<Value>,
}

/// An opaque handle (FFI values, abstract types). Identity — for `eq`,
/// `compare`, and `hash` under the `opaque` strategy — is the address of
/// the containing allocation, so two handles are equal iff they are the
/// same allocation.
pub struct OpaqueVal {
    pub type_id: TypeId,
    pub payload: Box<dyn Any>,
}

impl fmt::Debug for OpaqueVal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<handle:{:?}>", self.type_id)
    }
}

/// A Soil closure, as a boxed Rust callable (resolved decision, impl
/// plan §1): plan-01 tests and builtins pass Rust closures, `soil0`'s
/// interpreter captures AST + environment, compiled code later wraps an
/// `extern "C"` function and its environment. One invocation path.
pub type ClosureFn = Box<dyn Fn(&[Value]) -> Result<Value, SoilError>>;

pub struct Closure {
    f: ClosureFn,
}

impl Closure {
    pub fn native(f: impl Fn(&[Value]) -> Result<Value, SoilError> + 'static) -> Self {
        Closure { f: Box::new(f) }
    }

    pub fn call(&self, args: &[Value]) -> Result<Value, SoilError> {
        (self.f)(args)
    }
}

impl fmt::Debug for Closure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<closure>")
    }
}

/// How a map orders its keys.
///
/// `Custom` carries a program-supplied comparator in OCaml's protocol
/// (design §3.6): a closure taking `[k1, k2]` and returning an `I64`
/// that is negative, zero, or positive. `Structural` is the derived
/// total order (`ops::compare`) — the order used at the JSON boundary,
/// where a decoded map has no program closure to carry.
#[derive(Debug)]
pub enum MapOrder {
    Structural,
    Custom(Ref<Closure>),
}

/// A Soil map: entries kept sorted by the carried order (a sorted vec
/// by resolved decision; swap for a tree behind this API only if
/// profiling demands it — plan 01).
///
/// Operations take the [`Runtime`](crate::Runtime) because the
/// structural order consults the registry, and they are fallible
/// because a custom comparator is a Soil closure. A comparator that is
/// not a total order silently corrupts the sorted invariant; detecting
/// that is the refinement checker's job, not the runtime's (impl plan
/// §7). The order is structure, not content: derived `eq`/`compare`
/// look only at the entries.
#[derive(Debug)]
pub struct MapVal {
    order: MapOrder,
    entries: Vec<(Value, Value)>,
}

impl MapVal {
    pub fn new(order: MapOrder) -> Self {
        MapVal {
            order,
            entries: Vec::new(),
        }
    }

    pub fn order(&self) -> &MapOrder {
        &self.order
    }

    /// Entries in comparator order — the order `show` and JSON emit.
    pub fn entries(&self) -> &[(Value, Value)] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn compare_keys(
        &self,
        runtime: &crate::Runtime,
        a: &Value,
        b: &Value,
    ) -> Result<Ordering, SoilError> {
        match &self.order {
            MapOrder::Structural => crate::ops::compare(runtime, a, b),
            MapOrder::Custom(comparator) => {
                let result = comparator.call(&[a.clone(), b.clone()])?;
                match result {
                    Value::I64(n) => Ok(n.cmp(&0)),
                    other => Err(SoilError::new(
                        PanicKind::TypeError,
                        format!("map comparator returned {other:?}, expected I64"),
                    )),
                }
            }
        }
    }

    /// Binary search for `key`: `Ok(index)` if present, `Err(insertion
    /// point)` if absent — or a comparator failure.
    fn search(
        &self,
        runtime: &crate::Runtime,
        key: &Value,
    ) -> Result<Result<usize, usize>, SoilError> {
        let mut lo = 0usize;
        let mut hi = self.entries.len();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.compare_keys(runtime, &self.entries[mid].0, key)? {
                Ordering::Less => lo = mid + 1,
                Ordering::Greater => hi = mid,
                Ordering::Equal => return Ok(Ok(mid)),
            }
        }
        Ok(Err(lo))
    }

    pub fn get(&self, runtime: &crate::Runtime, key: &Value) -> Result<Option<&Value>, SoilError> {
        Ok(self.search(runtime, key)?.ok().map(|i| &self.entries[i].1))
    }

    /// Insert or replace. Mutation is a construction-time affair: a
    /// `MapVal` already shared inside a `Value::Map` is immutable, and
    /// Soil-level insertion clones the map (no mutation, design §3.13).
    pub fn insert(
        &mut self,
        runtime: &crate::Runtime,
        key: Value,
        value: Value,
    ) -> Result<(), SoilError> {
        match self.search(runtime, &key)? {
            Ok(i) => self.entries[i].1 = value,
            Err(i) => self.entries.insert(i, (key, value)),
        }
        Ok(())
    }

    pub fn remove(
        &mut self,
        runtime: &crate::Runtime,
        key: &Value,
    ) -> Result<Option<Value>, SoilError> {
        match self.search(runtime, key)? {
            Ok(i) => Ok(Some(self.entries.remove(i).1)),
            Err(_) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn i64_comparator() -> Ref<Closure> {
        Ref::new(Closure::native(|args| match (&args[0], &args[1]) {
            (Value::I64(a), Value::I64(b)) => Ok(Value::I64(match a.cmp(b) {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            })),
            _ => Err(SoilError::new(PanicKind::TypeError, "expected I64 keys")),
        }))
    }

    #[test]
    fn value_is_at_most_three_words() {
        assert!(std::mem::size_of::<Value>() <= 24);
    }

    #[test]
    fn refcounting_returns_to_zero() {
        let before = debug_live_values();
        {
            let a = Ref::new(vec![Value::I64(1), Value::Unit]);
            let b = a.clone();
            assert!(Ref::ptr_eq(&a, &b));
            assert_eq!(debug_live_values(), before + 1);
            let s: Ref<str> = Ref::from("hello");
            assert_eq!(&*s, "hello");
            assert_eq!(debug_live_values(), before + 2);
        }
        assert_eq!(debug_live_values(), before);
    }

    #[test]
    fn nested_values_release_on_drop() {
        let before = debug_live_values();
        {
            let inner: Ref<str> = Ref::from("payload");
            let list = Value::List(Ref::new(vec![Value::Utf8(inner)]));
            let copy = list.clone();
            drop(list);
            assert_eq!(debug_live_values(), before + 2);
            drop(copy);
        }
        assert_eq!(debug_live_values(), before);
    }

    #[test]
    fn closure_invocation() {
        let double = Closure::native(|args| match args {
            [Value::I64(n)] => Ok(Value::I64(n * 2)),
            _ => Err(SoilError::new(PanicKind::TypeError, "expected one I64")),
        });
        match double.call(&[Value::I64(21)]) {
            Ok(Value::I64(42)) => {}
            other => panic!("unexpected result: {other:?}"),
        }
        assert!(double.call(&[Value::Unit]).is_err());
    }

    #[test]
    fn map_insert_get_remove_in_comparator_order() {
        let rt = crate::Runtime::new();
        let mut map = MapVal::new(MapOrder::Custom(i64_comparator()));
        for k in [30i64, 10, 20, 10] {
            map.insert(&rt, Value::I64(k), Value::I64(k * 100)).unwrap();
        }
        assert_eq!(map.len(), 3);
        let keys: Vec<i64> = map
            .entries()
            .iter()
            .map(|(k, _)| match k {
                Value::I64(n) => *n,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(keys, vec![10, 20, 30]);

        match map.get(&rt, &Value::I64(20)).unwrap() {
            Some(Value::I64(2000)) => {}
            other => panic!("unexpected: {other:?}"),
        }
        assert!(map.get(&rt, &Value::I64(99)).unwrap().is_none());

        match map.remove(&rt, &Value::I64(10)).unwrap() {
            Some(Value::I64(1000)) => {}
            other => panic!("unexpected: {other:?}"),
        }
        assert_eq!(map.len(), 2);
        assert!(map.remove(&rt, &Value::I64(10)).unwrap().is_none());
    }

    #[test]
    fn map_structural_order_uses_derived_compare() {
        let rt = crate::Runtime::new();
        let mut map = MapVal::new(MapOrder::Structural);
        for k in [30i64, 10, 20] {
            map.insert(&rt, Value::I64(k), Value::Unit).unwrap();
        }
        let keys: Vec<i64> = map
            .entries()
            .iter()
            .map(|(k, _)| match k {
                Value::I64(n) => *n,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(keys, vec![10, 20, 30]);
    }

    #[test]
    fn map_comparator_errors_propagate() {
        let rt = crate::Runtime::new();
        let mut map = MapVal::new(MapOrder::Custom(Ref::new(Closure::native(|_| {
            Err(SoilError::new(PanicKind::TypeError, "always fails"))
        }))));
        assert!(map.insert(&rt, Value::I64(1), Value::Unit).is_ok());
        assert!(map.insert(&rt, Value::I64(2), Value::Unit).is_err());
    }

    #[test]
    fn opaque_identity_is_address() {
        let a = Ref::new(OpaqueVal {
            type_id: TypeId(0),
            payload: Box::new(7u8),
        });
        let b = a.clone();
        let c = Ref::new(OpaqueVal {
            type_id: TypeId(0),
            payload: Box::new(7u8),
        });
        assert_eq!(a.addr(), b.addr());
        assert_ne!(a.addr(), c.addr());
    }
}
