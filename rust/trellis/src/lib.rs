//! The Trellis daemon and thin CLI (impl plan 03).
//!
//! The library exists for the crate's own unit and integration tests;
//! its API is unstable and non-contractual (the contractual surfaces
//! are `docs/contracts/trellis-daemon.md`'s). Step 2: skeleton,
//! daemon lifecycle, JSON-RPC over the Unix socket, `soil.toml`
//! loading and the toolchain pin.

// `ErrorReport` carries registry-enriched diagnostics and sits in
// cold error paths of a CLI/daemon; boxing every result for clippy's
// size heuristic would be noise.
#![allow(clippy::result_large_err)]

pub mod cli;
pub mod config;
pub mod cram;
pub mod daemon;
pub mod diag;
pub mod envgen;
pub mod hash;
pub mod lock;
pub mod manifest;
pub mod oracle;
pub mod preflight;
pub mod propgen;
pub mod registry;
pub mod rpc;
pub mod state;
pub mod testrun;
pub mod trfile;
