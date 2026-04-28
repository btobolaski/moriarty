//! Strongly typed parser for pi session logs.
//!
//! Each pi session is persisted as a newline-delimited JSON file under
//! `~/.pi/agent/sessions`. This crate models those lines as typed Rust
//! values and exposes helpers for parsing individual lines and whole files.
//!
//! Parsing is deliberately strict: most concrete structs carry
//! `#[serde(deny_unknown_fields)]` and all discriminators are closed enums,
//! so upstream format changes surface as loud parse errors rather than
//! silent data loss. Two categories of exceptions exist:
//!
//! First, structs that `#[serde(flatten)]` an *internally-tagged* enum
//! (one with `tag` but no `content`) cannot use `deny_unknown_fields`,
//! because the inner enum's tag field appears at the outer JSON level and
//! serde's flatten codegen does not register it as claimed; the strict
//! outer struct then rejects it as unknown. Only [`WebSearchResultsData`]
//! is in this category; it relies on the closed-enum discriminator of the
//! flattened payload to enforce the contract. Structs that flatten an
//! *adjacently-tagged* enum (one with both `tag` and `content`), including
//! [`CustomLine`], [`CustomMessageLine`], and [`ToolCallContent`], do not
//! hit this collision and remain fully strict via `deny_unknown_fields`.
//!
//! Second, a small number of tool-argument structs ([`EditArgs`],
//! [`EditReplacement`], [`GrepArgs`]) deliberately omit
//! `deny_unknown_fields` to tolerate completed-but-corrupted or hallucinated
//! assistant streams that emit malformed sibling keys. The same goal is
//! also met at finer granularity by two untagged fallback enums:
//! [`EditEntry`] absorbs interspersed JSON fragments inside an `edits`
//! array, and [`MaybeU32`] absorbs string-typed corruption of numeric
//! tool-call arguments. Each such exception carries an inline comment
//! naming the observed failure mode.

pub mod parser;

pub use parser::*;
