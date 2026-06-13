//! # js-beautify-rs
//!
//! A Rust port of js-beautify with webpack-specific improvements.
//!
//! ## Example
//!
//! ```rust
//! use js_beautify_rs::{beautify, Options};
//!
//! let code = "function test(){console.log('hello');}";
//! let options = Options::default();
//! let beautified = beautify(code, &options).expect("beautification failed");
//! ```

pub mod analyzers;
pub mod ast_deobfuscate;
pub mod beautifier;
pub mod bun_extract;
pub mod cross_version;
pub mod db;
pub mod diff;
pub mod domain_recon;
pub mod fuzz;
pub mod hooks;
pub mod monitoring;
pub mod nextjs_discovery;
pub mod oxc_opts;
pub mod pipeline;
pub mod proxy;
pub mod recon_engine;
pub mod relationships;
pub mod repeater;
pub mod scope;
pub mod server;
pub mod sourcemap;
pub mod token;
pub mod tokenizer;
pub mod webpack_module_extractor;
pub mod websocket;
pub mod wordlist;

mod chunk_detector;
mod chunk_splitter;
mod options;

pub use ast_deobfuscate::AstDeobfuscator;
pub use beautifier::beautify;
pub use bun_extract::{
    BundleSplit, ExportMapping, MappingSource, ModuleInfo, ModuleType, extract_all_names,
    extract_display_name_patterns, extract_mr_exports, extract_this_name_patterns, find_module_wrappers, split_bundle,
};
pub use chunk_detector::{ChunkDetector, ChunkMetadata};
pub use chunk_splitter::{ChunkManifest, ChunkSplitter};
pub use cross_version::{AlignConfig, AlignmentStats, CrossVersionAligner, StatementMatcher};
pub use options::Options;
pub use oxc_opts::OxcOptimizer;
pub use pipeline::{AssetKind, IngestRequest, IngestResult, Pipeline, PipelineConfig, PipelineIngestor};
pub use token::{Token, TokenType};

#[derive(Debug, thiserror::Error)]
pub enum BeautifyError {
    #[error("tokenization failed at line {line}, column {column}: {message}")]
    TokenizationFailed {
        message: String,
        line: usize,
        column: usize,
    },

    #[error("beautification failed: {0}")]
    BeautificationFailed(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("chunk detection failed: {0}")]
    ChunkDetectionFailed(#[from] chunk_detector::ChunkDetectorError),

    #[error("chunk splitting failed: {0}")]
    ChunkSplittingFailed(#[from] chunk_splitter::ChunkSplitterError),

    #[error("JSON parsing failed: {0}")]
    JsonParsingFailed(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, BeautifyError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_beautify() {
        let code = "function test(){return 42;}";
        let options = Options::default();
        let result = beautify(code, &options).expect("beautification failed");

        assert!(result.contains("function test()"));
        assert!(result.contains("return 42;"));
    }
}
