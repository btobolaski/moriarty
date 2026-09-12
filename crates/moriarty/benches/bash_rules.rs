// Cargo enables cfg(test) for benches but removes #[test] functions with harness=false,
// leaving their imports/helpers unused. Only this target relaxes those two lints;
// the ordinary Nextest/Clippy targets still check the same source without allowances.
#![allow(dead_code, unused_imports)]

// Reuse the binary's private modules instead of creating a public library just for benchmarks.
include!("../src/main.rs");
