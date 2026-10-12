mod blank_lines;
mod layout;

pub use layout::Layout;

use crate::ast_deobfuscate::AstDeobfuscator;
use crate::chunk_detector::ChunkDetector;
use crate::chunk_splitter::ChunkSplitter;
use crate::options::Options;
use crate::tokenizer::Tokenizer;
use crate::turbopack_extract::TurbopackChunk;
use crate::{BeautifyError, Result};

use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_parser::Parser;

fn oxc_beautify(code: &str, options: &Options) -> Result<String> {
    let allocator = Allocator::default();
    let parse_result = Parser::new(&allocator, code, options.parse_goal.resolve(code, false)).parse();

    // oxc performs error recovery and still produces a usable (partial) AST
    // even when `errors` is non-empty. Rather than bail — which left huge or
    // slightly-malformed bundles with no beautified output at all — we codegen
    // the recovered program and only treat an *empty* result as a hard failure.
    if !parse_result.diagnostics.is_empty() {
        eprintln!(
            "[beautify] {} parse diagnostic(s); using recovered AST (first: {:?})",
            parse_result.diagnostics.len(),
            parse_result.diagnostics.first().map(ToString::to_string),
        );
    }

    let preserve = options.preserve_newlines && options.max_preserve_newlines != 1 && has_blank_line(code);
    let codegen_options = CodegenOptions {
        source_map_path: preserve.then(|| "input.js".into()),
        ..CodegenOptions::default()
    };
    let generated = Codegen::new()
        .with_options(codegen_options)
        .build(&parse_result.program);
    let out = generated.code;
    if out.trim().is_empty() && !code.trim().is_empty() {
        return Err(BeautifyError::BeautificationFailed(
            "codegen produced empty output from recovered AST".to_string(),
        ));
    }
    let blank_lines = generated.map.map_or_else(Vec::new, |map| {
        blank_lines::blank_lines(
            code,
            &parse_result.program,
            &out,
            map.get_tokens()
                .map(|t| (t.get_dst_line(), t.get_dst_col(), t.get_src_line(), t.get_src_col())),
            options.max_preserve_newlines,
        )
    });
    Layout::new(options, code).apply(&out, parse_result.program.source_type, &blank_lines)
}

/// `split-chunks` on a Turbopack chunk: one file per module factory (and per
/// scope-hoisted module) in the chunk directory, plus the registrations as the
/// chunk map.
fn split_turbopack_chunk(code: &str, chunk: &TurbopackChunk, options: &Options) -> Result<String> {
    eprintln!(
        "[BEAUTIFY] ✓ Turbopack chunk: {} registration(s), {} module factories, {} scope-hoisted modules",
        chunk.registrations.len(),
        chunk.module_count(),
        chunk.hoisted_count()
    );
    let written = chunk
        .write_modules(code, &options.chunk_dir)
        .map_err(|e| BeautifyError::BeautificationFailed(format!("failed to write Turbopack modules: {e}")))?;
    if options.generate_source_map {
        eprintln!("[BEAUTIFY] ⚠ Source maps are not generated for Turbopack modules");
    }
    if let Some(map_path) = &options.chunk_map_output {
        std::fs::write(map_path, serde_json::to_string_pretty(chunk)?)
            .map_err(|e| BeautifyError::BeautificationFailed(format!("failed to write chunk map: {e}")))?;
        eprintln!("[BEAUTIFY] ✓ Chunk map written to: {}", map_path.display());
    }
    Ok(format!(
        "// Turbopack modules written to: {}\n// Total files: {}\n",
        options.chunk_dir.display(),
        written.len()
    ))
}

fn has_blank_line(code: &str) -> bool {
    code.lines().skip(1).any(|line| line.trim().is_empty())
}

/// Runs on a worker whose stack is sized from the input length, so input of
/// any nesting depth is processed.
///
/// # Errors
/// Returns an error if the operation fails.
pub fn beautify(code: &str, options: &Options) -> Result<String> {
    crate::stack::run_on_sized_stack(code.len(), || beautify_on_current_stack(code, options))
}

/// Deobfuscate one file of a project with its part of the project naming
/// plan (see [`crate::ast_deobfuscate::project`]); `options.deobfuscate` is
/// implied.
///
/// # Errors
/// Returns an error if deobfuscation fails.
pub fn deobfuscate_with_project_seeds(
    code: &str,
    options: &Options,
    seeds: crate::ast_deobfuscate::project::ProjectSeeds,
) -> Result<String> {
    crate::stack::run_on_sized_stack(code.len(), || {
        deobfuscator(code, options).with_project_seeds(seeds).deobfuscate(code)
    })
}

fn deobfuscator(code: &str, options: &Options) -> AstDeobfuscator {
    AstDeobfuscator::new()
        .with_skip_annotations(options.skip_annotations)
        .with_readable_names(options.readable_names)
        .with_parse_goal(options.parse_goal)
        .with_isolated(options.isolated)
        .with_layout(Layout::new(options, code))
}

fn beautify_on_current_stack(code: &str, options: &Options) -> Result<String> {
    if options.deobfuscate {
        return deobfuscator(code, options).deobfuscate(code);
    }

    if options.split_chunks {
        eprintln!("[BEAUTIFY] Chunk splitting enabled, detecting chunks...");
        if let Some(chunk) = crate::turbopack_extract::detect_turbopack_chunk(code) {
            return split_turbopack_chunk(code, &chunk, options);
        }

        let mut tokenizer = Tokenizer::new(code);
        let tokens = tokenizer.tokenize()?;

        let mut detector = ChunkDetector::new();
        match detector.detect_chunks(&tokens) {
            Ok(()) => {
                eprintln!("[BEAUTIFY] ✓ Detected {} chunks", detector.chunk_count());

                if detector.chunk_count() > 0 {
                    if detector.has_boundaries() {
                        let splitter = ChunkSplitter::new(detector);

                        eprintln!("[BEAUTIFY] Splitting chunks to: {}", options.chunk_dir.display());

                        let manifest = splitter.split_and_write(&tokens, options)?;

                        eprintln!("[BEAUTIFY] ✓ Successfully wrote {} chunk files", manifest.total_chunks);

                        if let Some(ref map_path) = options.chunk_map_output {
                            eprintln!("[BEAUTIFY] ✓ Chunk map written to: {}", map_path.display());
                        }

                        return Ok(format!(
                            "// Chunks written to: {}\n// Total chunks: {}\n",
                            options.chunk_dir.display(),
                            manifest.total_chunks
                        ));
                    }
                    eprintln!("[BEAUTIFY] ⚠ Chunks detected but no embedded code found");
                    eprintln!(
                        "[BEAUTIFY]   This appears to be a code-split bundle (chunks are already separate files)"
                    );
                    eprintln!(
                        "[BEAUTIFY]   Chunk metadata extracted: {} chunks",
                        detector.chunk_count()
                    );

                    if let Some(ref map_path) = options.chunk_map_output {
                        use crate::chunk_splitter::ChunkManifest;
                        use std::fs;

                        let manifest = ChunkManifest::from_detector(&detector);
                        let json = serde_json::to_string_pretty(&manifest).map_err(|e| {
                            BeautifyError::BeautificationFailed(format!("failed to serialize manifest: {e}"))
                        })?;
                        fs::write(map_path, json).map_err(|e| {
                            BeautifyError::BeautificationFailed(format!("failed to write manifest: {e}"))
                        })?;

                        eprintln!("[BEAUTIFY] ✓ Chunk metadata written to: {}", map_path.display());
                    }

                    eprintln!("[BEAUTIFY] Proceeding with Oxc beautification");
                } else {
                    eprintln!("[BEAUTIFY] ⚠ No chunks detected, proceeding with Oxc beautification");
                }
            }
            Err(e) => {
                eprintln!("[BEAUTIFY] ⚠ Chunk detection failed: {e}");
                eprintln!("[BEAUTIFY] Falling back to Oxc beautification");
            }
        }
    }

    oxc_beautify(code, options)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod basic_tests {
    use super::*;

    #[test]
    fn test_beautify_simple() {
        let code = "function test(){return 42;}";
        let options = Options::default();
        let result = beautify(code, &options).unwrap();
        assert!(result.contains("function"));
        assert!(result.contains("return"));
    }

    #[test]
    fn test_beautify_with_spaces() {
        let code = "function test(){return 42;}";
        let options = Options::default();
        let result = beautify(code, &options).unwrap();
        assert!(result.contains("function test"));
    }

    #[test]
    fn recovers_output_from_parse_error() {
        // A stray token mid-file previously made oxc_beautify bail entirely,
        // leaving the file with no beautified output. oxc's error recovery
        // still yields a usable partial AST here, so we should get non-empty,
        // codegen'd output back rather than an error.
        let code = "function ok(){return 1;} ### var y = 2;";
        let options = Options::default();
        let result = beautify(code, &options).expect("recovered AST should beautify");
        assert!(!result.trim().is_empty(), "expected non-empty recovered output");
        assert!(
            result.contains("function"),
            "recovered output should retain valid functions"
        );
    }

    #[test]
    fn empty_recovery_is_an_error() {
        // When oxc cannot recover *any* statements it codegens an empty
        // program; we surface that as an error so the pipeline can fall back
        // to writing the raw source instead of an empty file.
        let code = "function ok(){return 1;} @@@ function also(){return 2;}";
        let options = Options::default();
        assert!(
            beautify(code, &options).is_err(),
            "fully-unrecoverable input should error so the caller can fall back",
        );
    }
}
