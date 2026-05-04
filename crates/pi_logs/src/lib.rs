//! Strongly typed parser for pi session logs.
//!
//! Each pi session is persisted as a newline-delimited JSON file under
//! `~/.pi/agent/sessions`. This crate models those lines as typed Rust
//! values and exposes helpers for parsing individual lines and whole files.
//!
//! Parsing is deliberately strict: most concrete structs carry
//! `#[serde(deny_unknown_fields)]` and all discriminators are closed enums,
//! so upstream format changes surface as loud parse errors rather than
//! silent data loss.
//!
//! A small number of narrowly documented exceptions exist for shapes that
//! serde cannot express strictly with a derive alone, or for specific
//! corrupt-stream patterns observed in real logs. The authoritative detail
//! lives in `parser.rs`; this crate-level overview stays intentionally brief
//! so those rules are described in one place only.

pub mod parser;

pub use parser::*;
