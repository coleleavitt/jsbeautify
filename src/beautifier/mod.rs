use crate::ast_deobfuscate::AstDeobfuscator;
use crate::chunk_detector::ChunkDetector;
use crate::chunk_splitter::ChunkSplitter;
use crate::options::Options;
use crate::tokenizer::Tokenizer;
use crate::{BeautifyError, Result};

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn oxc_beautify(code: &str) -> Result<String> {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs();
    let parse_result = Parser::new(&allocator, code, source_type).parse();

    if !parse_result.errors.is_empty() {
        return Err(BeautifyError::BeautificationFailed(format!(
            "Parse failed: {:?}",
            parse_result.errors.first()
        )));
    }

    Ok(Codegen::new().build(&parse_result.program).code)
}

/// # Errors
/// Returns an error if the operation fails.
pub fn beautify(code: &str, options: &Options) -> Result<String> {
    if options.deobfuscate {
        let mut deobfuscator = AstDeobfuscator::new()
            .with_skip_annotations(options.skip_annotations)
            .with_readable_names(options.readable_names);
        return deobfuscator.deobfuscate(code);
    }

    if options.split_chunks {
        eprintln!("[BEAUTIFY] Chunk splitting enabled, detecting chunks...");

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

    oxc_beautify(code)
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
}
