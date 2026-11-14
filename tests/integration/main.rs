//! Integration test suite for `jsbeautify`.
//!
//! Mirrors the layout used by `cargo` itself (`tests/testsuite/main.rs`): a
//! single integration-test *binary* that `mod`-includes every test file. This
//! compiles far faster than one `tests/*.rs` file per topic (each of which is
//! otherwise built and linked as its own separate crate).
//!
//! Add a new area by dropping a `tests/integration/<area>.rs` file and listing
//! it in the `mod` block below.

mod angular;
mod annex_b_functions;
mod beautify;
mod cli;
mod dead_declarations;
mod deobfuscate;
mod esbuild_bundles;
mod fixtures;
mod format_cli;
mod inspect;
mod nesting;
mod project;
mod script_mode;
mod turbopack;
mod webpack_modules;
