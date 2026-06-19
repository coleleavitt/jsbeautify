//! Shared helpers + noise-suppression patterns.
//!
//! All regexes recovered from jxscout-pro-v2 v2.1.1 binary
//! (`crates/core/analyzers/src/helpers.rs`).
//!
//! These are used by the path/hostname/URL analyzers to filter out common
//! false positives (timezone strings, locale codes, NPM relative imports,
//! Excel/PDF internals, …).

mod filters;
mod patterns;
#[cfg(test)]
mod tests;

pub use filters::{is_hostname_noise, is_path_noise, passes_start_pattern};
pub use patterns::*;
