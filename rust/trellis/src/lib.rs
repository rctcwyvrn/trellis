//! The Trellis daemon and thin CLI (impl plan 03).
//!
//! The library exists for the crate's own unit and integration tests;
//! its API is unstable and non-contractual (the contractual surfaces
//! are `docs/contracts/trellis-daemon.md`'s). Step 2: skeleton,
//! daemon lifecycle, JSON-RPC over the Unix socket, `soil.toml`
//! loading and the toolchain pin.

pub mod cli;
pub mod config;
pub mod daemon;
pub mod diag;
pub mod hash;
pub mod lock;
pub mod manifest;
pub mod rpc;
pub mod trfile;
