//! Passes specific to esbuild-style bundle output.

#[allow(dead_code, clippy::enum_variant_names)]
pub mod helper;

pub use helper::{EsbuildHelperCollector, EsbuildHelperKind, annotate_esbuild_modules};
