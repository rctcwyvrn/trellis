//! The C ABI (impl plan §4 step 8). Verbatim rules: no global state
//! (`SoilRuntime*` is the instance); explicit init/teardown; no
//! unwinding across the boundary — every entry point is wrapped in
//! `catch_unwind`, and a caught panic becomes a `SoilError` of kind
//! `Internal`.
//!
//! Threading: a runtime and every handle created under it belong to one
//! thread (non-atomic refcounts by resolved decision). Parallel hosts
//! run one runtime per thread.
//!
//! Ownership: constructors return owned handles; every input handle is
//! *borrowed* (internally refcount-bumped as needed), so the caller
//! frees exactly what it was returned — values with `soil_value_free`,
//! strings with `soil_string_free`, errors with `soil_error_free`.
//! Closures are not constructible over this ABI yet (`soil_closure_new`
//! arrives with compiled code, plan 05).

use std::ffi::{c_char, c_int, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use crate::descriptor::{TypeBody, TypeId};
use crate::error::{PanicKind, SoilError};
use crate::value::{Record, Ref, SumVal, Value};
use crate::{descriptor_json, json, ops, Runtime};

/// Opaque C-side names. `SoilRuntime*` is a `Runtime`, `SoilValue*` a
/// `Value`, `SoilError*` a `SoilError`; the internal layouts are not
/// part of the ABI.
pub struct SoilRuntime(Runtime);

fn internal(payload: Box<dyn std::any::Any + Send>) -> SoilError {
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "runtime panic".to_string());
    SoilError::new(PanicKind::Internal, message)
}

/// Store `err` through the out-parameter, if one was provided.
fn put_err(out: *mut *mut SoilError, err: SoilError) {
    if !out.is_null() {
        unsafe { *out = Box::into_raw(Box::new(err)) };
    }
}

/// Run `body` with unwinds caught; on success return its value, on
/// error store the error and return `fail`.
fn guard<T>(
    out_err: *mut *mut SoilError,
    fail: T,
    body: impl FnOnce() -> Result<T, SoilError>,
) -> T {
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(Ok(value)) => value,
        Ok(Err(err)) => {
            put_err(out_err, err);
            fail
        }
        Err(payload) => {
            put_err(out_err, internal(payload));
            fail
        }
    }
}

unsafe fn cstr<'a>(ptr: *const c_char) -> Result<&'a str, SoilError> {
    if ptr.is_null() {
        return Err(SoilError::new(PanicKind::CapiMisuse, "null string"));
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map_err(|_| SoilError::new(PanicKind::CapiMisuse, "string is not valid UTF-8"))
}

unsafe fn rt_ref<'a>(runtime: *mut SoilRuntime) -> Result<&'a mut Runtime, SoilError> {
    unsafe { runtime.as_mut() }
        .map(|r| &mut r.0)
        .ok_or_else(|| SoilError::new(PanicKind::CapiMisuse, "null runtime"))
}

unsafe fn value_ref<'a>(value: *const Value) -> Result<&'a Value, SoilError> {
    unsafe { value.as_ref() }.ok_or_else(|| SoilError::new(PanicKind::CapiMisuse, "null value"))
}

fn owned(value: Value) -> *mut Value {
    Box::into_raw(Box::new(value))
}

/// Create a runtime instance. Never fails; returns null only on an
/// internal panic.
#[no_mangle]
pub extern "C" fn soil_init() -> *mut SoilRuntime {
    catch_unwind(|| Box::into_raw(Box::new(SoilRuntime(Runtime::new())))).unwrap_or(ptr::null_mut())
}

/// Tear down a runtime. Values created under it must already be freed.
///
/// # Safety
/// `runtime` must be a pointer returned by `soil_init`, not yet torn
/// down.
#[no_mangle]
pub unsafe extern "C" fn soil_teardown(runtime: *mut SoilRuntime) {
    if !runtime.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(runtime) })));
    }
}

/// Register types from descriptor JSON (see `descriptor_json`). Returns
/// 0 on success, nonzero with `*err` set on failure.
///
/// # Safety
/// `runtime` is a live runtime; `desc_json` is a NUL-terminated UTF-8
/// string; `err` is null or a valid out-pointer.
#[no_mangle]
pub unsafe extern "C" fn soil_register_types(
    runtime: *mut SoilRuntime,
    desc_json: *const c_char,
    err: *mut *mut SoilError,
) -> c_int {
    guard(err, 1, || {
        let rt = unsafe { rt_ref(runtime) }?;
        let json = unsafe { cstr(desc_json) }?;
        descriptor_json::load_descriptors(rt, json)?;
        Ok(0)
    })
}

/// Look up a type id by name. Returns `UINT32_MAX` if unknown.
///
/// # Safety
/// `runtime` is a live runtime; `name` is a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn soil_type_lookup(runtime: *mut SoilRuntime, name: *const c_char) -> u32 {
    guard(ptr::null_mut(), u32::MAX, || {
        let rt = unsafe { rt_ref(runtime) }?;
        let name = unsafe { cstr(name) }?;
        Ok(rt.registry.lookup(name).map_or(u32::MAX, TypeId::index))
    })
}

#[no_mangle]
pub extern "C" fn soil_i64_new(n: i64) -> *mut Value {
    catch_unwind(|| owned(Value::I64(n))).unwrap_or(ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn soil_f64_new(x: f64) -> *mut Value {
    catch_unwind(|| owned(Value::F64(x))).unwrap_or(ptr::null_mut())
}

#[no_mangle]
pub extern "C" fn soil_unit_new() -> *mut Value {
    catch_unwind(|| owned(Value::Unit)).unwrap_or(ptr::null_mut())
}

/// # Safety
/// `s` is a NUL-terminated UTF-8 string; `err` is null or valid.
#[no_mangle]
pub unsafe extern "C" fn soil_utf8_new(s: *const c_char, err: *mut *mut SoilError) -> *mut Value {
    guard(err, ptr::null_mut(), || {
        let s = unsafe { cstr(s) }?;
        Ok(owned(Value::Utf8(Ref::from(s))))
    })
}

/// Build a record from its **non-ignored** fields in declaration order;
/// `ignored` fields are refilled from their defaults, exactly as in
/// JSON decode (micro-pin §8.6). Field values are borrowed.
///
/// # Safety
/// `runtime` is live; `fields` points to `n` valid value handles; `err`
/// is null or valid.
#[no_mangle]
pub unsafe extern "C" fn soil_record_new(
    runtime: *mut SoilRuntime,
    type_id: u32,
    fields: *const *const Value,
    n: usize,
    err: *mut *mut SoilError,
) -> *mut Value {
    guard(err, ptr::null_mut(), || {
        let rt = unsafe { rt_ref(runtime) }?;
        let id = TypeId::from_index(type_id);
        let desc = rt.registry.get(id)?;
        let TypeBody::Record(field_descs) = &desc.body else {
            return Err(SoilError::new(
                PanicKind::CapiMisuse,
                format!("type {} is not a record", desc.name),
            ));
        };
        let expected = field_descs.iter().filter(|f| f.ignored.is_none()).count();
        if n != expected {
            return Err(SoilError::new(
                PanicKind::CapiMisuse,
                format!(
                    "record {} takes {expected} non-ignored fields, got {n}",
                    desc.name
                ),
            ));
        }
        let inputs = if n == 0 {
            &[]
        } else if fields.is_null() {
            return Err(SoilError::new(PanicKind::CapiMisuse, "null field array"));
        } else {
            unsafe { std::slice::from_raw_parts(fields, n) }
        };
        let mut supplied = Vec::with_capacity(n);
        for &field in inputs {
            supplied.push(unsafe { value_ref(field) }?.clone());
        }
        let mut values = Vec::with_capacity(field_descs.len());
        let mut next = 0usize;
        for field_desc in field_descs {
            match &field_desc.ignored {
                None => {
                    values.push(supplied[next].clone());
                    next += 1;
                }
                Some(default) => values.push(default.call(&supplied)?),
            }
        }
        Ok(owned(Value::Record(Ref::new(Record {
            type_id: id,
            fields: values.into_boxed_slice(),
        }))))
    })
}

/// Build a sum value by variant name. `payload` is null for nullary
/// variants, borrowed otherwise.
///
/// # Safety
/// `runtime` is live; `variant` is a NUL-terminated UTF-8 string;
/// `payload` is null or a valid value handle; `err` is null or valid.
#[no_mangle]
pub unsafe extern "C" fn soil_sum_new(
    runtime: *mut SoilRuntime,
    type_id: u32,
    variant: *const c_char,
    payload: *const Value,
    err: *mut *mut SoilError,
) -> *mut Value {
    guard(err, ptr::null_mut(), || {
        let rt = unsafe { rt_ref(runtime) }?;
        let id = TypeId::from_index(type_id);
        let desc = rt.registry.get(id)?;
        let variant_name = unsafe { cstr(variant) }?;
        let TypeBody::Sum(variants) = &desc.body else {
            return Err(SoilError::new(
                PanicKind::CapiMisuse,
                format!("type {} is not a sum", desc.name),
            ));
        };
        let Some(index) = variants.iter().position(|v| v.name == variant_name) else {
            return Err(SoilError::new(
                PanicKind::CapiMisuse,
                format!("type {} has no variant {variant_name}", desc.name),
            ));
        };
        let payload_value = match (&variants[index].payload, payload.is_null()) {
            (Some(_), false) => Some(unsafe { value_ref(payload) }?.clone()),
            (None, true) => None,
            _ => {
                return Err(SoilError::new(
                    PanicKind::CapiMisuse,
                    format!("variant {variant_name} payload mismatch"),
                ));
            }
        };
        Ok(owned(Value::Sum(Ref::new(SumVal {
            type_id: id,
            variant: u32::try_from(index).expect("validated at registration"),
            payload: payload_value,
        }))))
    })
}

/// Build a list from borrowed elements.
///
/// # Safety
/// `items` points to `n` valid value handles; `err` is null or valid.
#[no_mangle]
pub unsafe extern "C" fn soil_list_new(
    items: *const *const Value,
    n: usize,
    err: *mut *mut SoilError,
) -> *mut Value {
    guard(err, ptr::null_mut(), || {
        let inputs = if n == 0 {
            &[]
        } else if items.is_null() {
            return Err(SoilError::new(PanicKind::CapiMisuse, "null item array"));
        } else {
            unsafe { std::slice::from_raw_parts(items, n) }
        };
        let mut values = Vec::with_capacity(n);
        for &item in inputs {
            values.push(unsafe { value_ref(item) }?.clone());
        }
        Ok(owned(Value::List(Ref::new(values))))
    })
}

/// # Safety
/// `value` is a valid handle.
#[no_mangle]
pub unsafe extern "C" fn soil_value_clone(value: *const Value) -> *mut Value {
    guard(ptr::null_mut(), ptr::null_mut(), || {
        Ok(owned(unsafe { value_ref(value) }?.clone()))
    })
}

/// # Safety
/// `value` was returned by this ABI and is freed exactly once.
#[no_mangle]
pub unsafe extern "C" fn soil_value_free(value: *mut Value) {
    if !value.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(value) })));
    }
}

/// Read an `I64` out of a value handle. Returns 0 on success.
///
/// # Safety
/// All pointers valid; `out` is a valid out-pointer.
#[no_mangle]
pub unsafe extern "C" fn soil_i64_get(
    value: *const Value,
    out: *mut i64,
    err: *mut *mut SoilError,
) -> c_int {
    guard(err, 1, || match unsafe { value_ref(value) }? {
        Value::I64(n) => {
            unsafe { *out = *n };
            Ok(0)
        }
        _ => Err(SoilError::new(PanicKind::TypeError, "value is not an I64")),
    })
}

/// Derived structural equality (design §3.7). Returns 1, 0, or -1 with
/// `*err` set.
///
/// # Safety
/// `runtime` is live; `a` and `b` are valid handles; `err` null or valid.
#[no_mangle]
pub unsafe extern "C" fn soil_eq(
    runtime: *mut SoilRuntime,
    a: *const Value,
    b: *const Value,
    err: *mut *mut SoilError,
) -> c_int {
    guard(err, -1, || {
        let rt = unsafe { rt_ref(runtime) }?;
        let equal = ops::eq(rt, unsafe { value_ref(a) }?, unsafe { value_ref(b) }?)?;
        Ok(c_int::from(equal))
    })
}

/// Canonical JSON of a value — `show` (design §3.7). Returns an owned
/// NUL-terminated string; free with `soil_string_free`.
///
/// # Safety
/// `runtime` is live; `value` is a valid handle; `err` null or valid.
#[no_mangle]
pub unsafe extern "C" fn soil_show(
    runtime: *mut SoilRuntime,
    value: *const Value,
    err: *mut *mut SoilError,
) -> *mut c_char {
    guard(err, ptr::null_mut(), || {
        let rt = unsafe { rt_ref(runtime) }?;
        let encoded = json::encode(rt, unsafe { value_ref(value) }?)?;
        CString::new(encoded)
            .map(CString::into_raw)
            .map_err(|_| SoilError::new(PanicKind::Internal, "canonical JSON contained NUL"))
    })
}

/// Type-directed decode — `parse` (design §3.7). `shape_json` is a
/// shape in the descriptor-JSON encoding (`{"tag": "Named", "value":
/// "Doc"}`, `{"tag": "List", "value": {"tag": "I64"}}`, …).
///
/// # Safety
/// `runtime` is live; both strings are NUL-terminated UTF-8; `err` null
/// or valid.
#[no_mangle]
pub unsafe extern "C" fn soil_decode(
    runtime: *mut SoilRuntime,
    shape_json: *const c_char,
    value_json: *const c_char,
    err: *mut *mut SoilError,
) -> *mut Value {
    guard(err, ptr::null_mut(), || {
        let rt = unsafe { rt_ref(runtime) }?;
        let shape = descriptor_json::parse_shape_json(rt, unsafe { cstr(shape_json) }?)?;
        Ok(owned(json::decode(rt, &shape, unsafe {
            cstr(value_json)
        }?)?))
    })
}

/// # Safety
/// `s` was returned by `soil_show` and is freed exactly once.
#[no_mangle]
pub unsafe extern "C" fn soil_string_free(s: *mut c_char) {
    if !s.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { CString::from_raw(s) })));
    }
}

/// # Safety
/// `err` is a valid error handle.
#[no_mangle]
pub unsafe extern "C" fn soil_error_message(err: *const SoilError) -> *mut c_char {
    guard(ptr::null_mut(), ptr::null_mut(), || {
        let err = unsafe { err.as_ref() }
            .ok_or_else(|| SoilError::new(PanicKind::CapiMisuse, "null error"))?;
        CString::new(err.to_string())
            .map(CString::into_raw)
            .map_err(|_| SoilError::new(PanicKind::Internal, "error message contained NUL"))
    })
}

/// The `PanicKind` as a stable small integer (order of declaration).
///
/// # Safety
/// `err` is a valid error handle.
#[no_mangle]
pub unsafe extern "C" fn soil_error_kind(err: *const SoilError) -> c_int {
    match unsafe { err.as_ref() } {
        None => -1,
        Some(e) => e.kind as c_int,
    }
}

/// # Safety
/// `err` was returned through an out-parameter and is freed exactly once.
#[no_mangle]
pub unsafe extern "C" fn soil_error_free(err: *mut SoilError) {
    if !err.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(unsafe { Box::from_raw(err) })));
    }
}

/// Live runtime allocations on this thread — debug builds only (always
/// 0 in release). The leak-check hook.
#[no_mangle]
pub extern "C" fn soil_debug_live_values() -> usize {
    crate::value::debug_live_values()
}

/// The trivial `main` wrapper (design §3.10): init, hand the runtime to
/// the host's entry function, tear down, return its exit code.
#[no_mangle]
pub extern "C" fn soil_main(entry: Option<extern "C" fn(*mut SoilRuntime) -> c_int>) -> c_int {
    let Some(entry) = entry else { return 2 };
    let runtime = soil_init();
    if runtime.is_null() {
        return 2;
    }
    let code = entry(runtime);
    unsafe { soil_teardown(runtime) };
    code
}
