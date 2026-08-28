//! `soil0`: the minimal Soil implementation (impl plan 02).
//!
//! The CLI (`docs/contracts/soil0-cli.md`) is the frozen compatibility
//! contract; this library API is unstable and exists for the daemon's
//! in-process use (plan 02 resolved decision). Conformance is defined by
//! the binary alone.

pub mod ast;
pub mod cli;
pub mod diag;
pub mod exhaust;
pub mod hashform;
pub mod infer;
pub mod interp;
pub mod kernel;
pub mod lexer;
pub mod manifest;
pub mod parser;
pub mod print;
pub mod rename;
pub mod span;
pub mod token;
pub mod types;
