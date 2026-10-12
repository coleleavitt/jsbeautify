//! Passes for esbuild and Bun bundle output, which share one runtime helper set.

#[allow(dead_code)]
pub mod alphabet;
pub mod cjs_flatten;
#[allow(dead_code, clippy::redundant_pub_crate)]
pub mod export_renamer;
#[allow(dead_code)]
pub mod local_renamer;
pub mod module_annotator;
pub mod module_handles;
#[allow(dead_code, clippy::trivially_copy_pass_by_ref)]
pub mod runtime;
pub mod syntax_normalizer;

pub use cjs_flatten::flatten_top_level_cjs_wrapper;
pub use export_renamer::{ExportNameCollector, ExportNameRenamer};
pub use module_annotator::annotate_runtime;
pub use module_handles::{BunModuleHandleCollector, BunModuleHandleRenamer};
pub use runtime::{
    BunHelper, BunHelperMap, BunRuntimeCollector, BunRuntimeRenamer, BundleRuntime, Bundler, detect_bundle_runtime,
    with_bundle_runtime,
};
pub use syntax_normalizer::BunSyntaxNormalizer;
