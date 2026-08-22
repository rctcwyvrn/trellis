//! The canonical JSON bridge (tr-grammar §7).
//!
//! One value format serves `show`/`parse`, tests, the REPL, FFI
//! marshalling, and host stubs. Encoding is canonical — equal values
//! produce byte-equal output — and decoding is always type-directed.

mod decode;
pub(crate) mod encode;

pub use decode::decode;
pub use encode::encode;
