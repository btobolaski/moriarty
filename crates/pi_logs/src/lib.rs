//! Strongly typed parser for pi session logs.
//!
//! Each pi session is persisted as a newline-delimited JSON file under
//! `~/.pi/agent/sessions`. This crate models those lines as typed Rust
//! values and exposes helpers for parsing individual lines and whole files.
//!
//! Parsing is deliberately strict: most concrete structs carry
//! `#[serde(deny_unknown_fields)]` and all discriminators are closed enums,
//! so upstream format changes surface as loud parse errors rather than
//! silent data loss. The exception is structs that participate in a
//! `#[serde(flatten)]` relationship (e.g. [`CustomLine`], [`CustomMessageLine`])
//! — serde does not allow `deny_unknown_fields` together with `flatten`, so
//! those types rely on the closed-enum discriminator of their flattened
//! payload to enforce the contract.

pub mod parser;

pub use parser::*;
