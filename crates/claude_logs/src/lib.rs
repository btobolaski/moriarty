//! Strongly typed serde models for Claude Code session logs.
//!
//! This crate extracts the Claude Code JSONL schema previously housed inside
//! the `moriarty` binary crate so other workspace crates can depend on the
//! shared log types directly.

pub mod parser;

pub use parser::*;
