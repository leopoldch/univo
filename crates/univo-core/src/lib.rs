//! Core types and matching logic for Univo.
//!
//! This crate has no dependency on Python, on network clients or on an
//! inference runtime. Bindings and the engine build on top of it.

/// Version of the Univo core, shared by every binding.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
