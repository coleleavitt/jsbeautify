//! Cross-version alignment for producing stable diffs between minified bundle versions.
//!
//! This module implements the "ultimate alignment" algorithm that:
//! 1. Parses sourcemaps to extract original name mappings
//! 2. Matches statements between versions by AST structure hash
//! 3. Applies three-tier canonical naming:
//!    - Tier 1: Original names from sourcemaps (best)
//!    - Tier 2: Slot-based names `sN` using Bun alphabet (semantic, stable)
//!    - Tier 3: Statement-hash-based names `_rN` (fallback)
//!
//! Result: 76% diff reduction between Bun-minified bundles.

mod aligner;
pub mod ast_matcher;
pub mod canonical_namer;
mod config;
mod replacements;
pub mod sourcemap_parser;
#[cfg(test)]
mod tests;

pub use aligner::CrossVersionAligner;
pub use ast_matcher::{StatementInfo, StatementMatcher};
pub use canonical_namer::CanonicalNamer;
pub use config::AlignConfig;
pub use replacements::AlignmentStats;
pub use sourcemap_parser::{NameMapping, SourcemapParser};
