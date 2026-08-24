//! `soil-rt` — the Soil runtime.
//!
//! Owns the value model, memory management (non-atomic reference
//! counting; a runtime instance and all its values belong to one
//! thread), derived operations, the canonical JSON bridge, and the
//! C ABI for embedding. Everything else — `soil0`'s interpreter,
//! compiled code, hosts — manipulates Soil values only through this
//! crate. See `docs/plans/impls/01-soil-rt-impl.md`.

pub mod capi;
pub mod descriptor;
pub mod descriptor_json;
pub mod error;
pub mod json;
pub mod ops;
pub mod value;

pub use descriptor::{
    FieldDesc, IgnoredDefault, Registry, Strategy, TypeBody, TypeDesc, TypeId, TypeShape,
    VariantDesc,
};
pub use descriptor_json::{descriptors_to_json, load_descriptors};
pub use error::{PanicKind, SoilError, TraceFrame};
pub use value::{
    debug_live_values, Closure, MapOrder, MapVal, OpaqueVal, Record, Ref, SumVal, Value,
};

/// One Soil runtime instance: the type registry and (in debug builds)
/// the allocation accounting. No global state — embedders hold this and
/// pass it explicitly; over the C ABI it is the `SoilRuntime*` handle.
/// An instance and every value created under it belong to one thread.
pub use num_bigint::BigInt;

#[derive(Default)]
pub struct Runtime {
    pub registry: Registry,
}

impl Runtime {
    pub fn new() -> Self {
        Runtime {
            registry: Registry::new(),
        }
    }
}
