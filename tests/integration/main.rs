//! Integration test suite for `js-beautify-rs`.
//!
//! Mirrors the layout used by `cargo` itself (`tests/testsuite/main.rs`): a
//! single integration-test *binary* that `mod`-includes every test file. This
//! compiles far faster than one `tests/*.rs` file per topic (each of which is
//! otherwise built and linked as its own separate crate).
//!
//! Add a new area by dropping a `tests/integration/<area>.rs` file and listing
//! it in the `mod` block below.

mod beautify;
mod cli;
mod deobfuscate;
mod fixtures;
mod pipeline;
mod webpack_modules;
