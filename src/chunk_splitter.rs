use crate::chunk_detector::{ChunkDetector, ChunkMetadata};
use crate::options::Options;
use crate::sourcemap::SourceMap;
use crate::token::Token;
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[cfg(debug_assertions)]
macro_rules! trace_split {
    ($($arg:tt)*) => {
        eprintln!("[SPLIT] {}", format!($($arg)*));
    };
}

#[cfg(not(debug_assertions))]
macro_rules! trace_split {
    ($($arg:tt)*) => {
        let _ = format_args!($($arg)*);
    };
}

#[derive(Debug, Error)]
pub enum ChunkSplitterError {
    #[error("failed to create chunk directory {path}: {source}")]
    DirectoryCreationFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write chunk file {path}: {source}")]
    ChunkWriteFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write chunk map {path}: {source}")]
    ChunkMapWriteFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("chunk {chunk_id} has invalid bounds: start={start}, end={end}")]
    InvalidChunkBounds { chunk_id: usize, start: usize, end: usize },

    #[error("chunk {chunk_id} bounds exceed token stream length: end={end}, len={len}")]
    ChunkBoundsOutOfRange { chunk_id: usize, end: usize, len: usize },
}

pub struct ChunkSplitter {
    detector: ChunkDetector,
}

impl ChunkSplitter {
    /// # Panics
    /// Panics if detector has no detected chunks.
    #[must_use]
    pub fn new(detector: ChunkDetector) -> Self {
        assert!(detector.chunk_count() > 0, "detector must have detected chunks");
        trace_split!("initializing ChunkSplitter with {} chunks", detector.chunk_count());

        Self { detector }
    }

    /// # Errors
    /// Returns an error if the operation fails.
    ///
    /// # Panics
    /// Panics if `tokens` is empty or `split_chunks` is not enabled.
    pub fn split_and_write(&self, tokens: &[Token], options: &Options) -> Result<ChunkManifest, ChunkSplitterError> {
        assert!(!tokens.is_empty(), "token stream must not be empty");
        assert!(options.split_chunks, "split_chunks option must be enabled");

        trace_split!("=== STARTING CHUNK SPLITTING ===");
        trace_split!("total tokens: {}", tokens.len());
        trace_split!("chunk directory: {}", options.chunk_dir.display());

        self.create_chunk_directory(options)?;

        let mut manifest = ChunkManifest::new(options.chunk_dir.clone());
        let mut written_count: usize = 0;

        for (chunk_id, chunk) in &self.detector.chunks {
            trace_split!("processing chunk {}: {}", chunk_id, chunk.name);

            let chunk_tokens = self.extract_chunk_tokens(tokens, chunk)?;
            trace_split!("extracted {} tokens for chunk {}", chunk_tokens.len(), chunk_id);

            let output_path = options.chunk_dir.join(&chunk.filename);
            self.write_chunk_file(&chunk_tokens, &output_path, chunk)?;

            if options.generate_source_map {
                trace_split!("generating source map for chunk {}", chunk_id);
                self.write_chunk_source_map(&chunk_tokens, &output_path, chunk, options)?;
            }

            manifest.add_chunk(*chunk_id, chunk.clone());

            written_count = written_count
                .checked_add(1)
                .ok_or_else(|| ChunkSplitterError::ChunkWriteFailed {
                    path: output_path.clone(),
                    source: std::io::Error::other("written count overflow"),
                })?;

            trace_split!(
                "✓ wrote chunk {} ({}/{})",
                chunk.filename,
                written_count,
                self.detector.chunk_count()
            );
        }

        if let Some(map_path) = &options.chunk_map_output {
            trace_split!("writing chunk map to {}", map_path.display());
            self.write_chunk_map(&manifest, map_path)?;
            trace_split!("✓ wrote chunk map");
        }

        trace_split!("=== CHUNK SPLITTING COMPLETE ===");
        trace_split!("wrote {} chunk files", written_count);

        debug_assert_eq!(written_count, self.detector.chunk_count(), "must write all chunks");

        Ok(manifest)
    }

    fn create_chunk_directory(&self, options: &Options) -> Result<(), ChunkSplitterError> {
        trace_split!("creating chunk directory: {}", options.chunk_dir.display());

        fs::create_dir_all(&options.chunk_dir).map_err(|source| ChunkSplitterError::DirectoryCreationFailed {
            path: options.chunk_dir.clone(),
            source,
        })?;

        trace_split!("✓ chunk directory ready");
        Ok(())
    }

    fn extract_chunk_tokens(&self, tokens: &[Token], chunk: &ChunkMetadata) -> Result<Vec<Token>, ChunkSplitterError> {
        trace_split!(
            "extracting tokens for chunk {} ({}..{})",
            chunk.id,
            chunk.start_pos,
            chunk.end_pos
        );

        if chunk.start_pos >= chunk.end_pos {
            return Err(ChunkSplitterError::InvalidChunkBounds {
                chunk_id: chunk.id,
                start: chunk.start_pos,
                end: chunk.end_pos,
            });
        }

        if chunk.end_pos > tokens.len() {
            return Err(ChunkSplitterError::ChunkBoundsOutOfRange {
                chunk_id: chunk.id,
                end: chunk.end_pos,
                len: tokens.len(),
            });
        }

        debug_assert!(chunk.start_pos < tokens.len(), "start must be in bounds");
        debug_assert!(chunk.end_pos <= tokens.len(), "end must be in bounds");

        let chunk_tokens = tokens[chunk.start_pos..chunk.end_pos].to_vec();

        debug_assert!(!chunk_tokens.is_empty(), "chunk must have tokens");
        trace_split!("extracted {} tokens", chunk_tokens.len());

        Ok(chunk_tokens)
    }

    fn write_chunk_file(
        &self,
        tokens: &[Token],
        path: &Path,
        _chunk: &ChunkMetadata,
    ) -> Result<(), ChunkSplitterError> {
        assert!(!tokens.is_empty(), "tokens must not be empty");
        trace_split!("writing chunk file: {}", path.display());

        let mut file = fs::File::create(path).map_err(|source| ChunkSplitterError::ChunkWriteFailed {
            path: path.to_path_buf(),
            source,
        })?;

        let mut written_bytes: usize = 0;
        let mut written_tokens: usize = 0;

        for token in tokens {
            let bytes = file
                .write(token.text.as_bytes())
                .map_err(|source| ChunkSplitterError::ChunkWriteFailed {
                    path: path.to_path_buf(),
                    source,
                })?;

            written_bytes = written_bytes
                .checked_add(bytes)
                .ok_or_else(|| ChunkSplitterError::ChunkWriteFailed {
                    path: path.to_path_buf(),
                    source: std::io::Error::other("byte counter overflow"),
                })?;

            written_tokens = written_tokens
                .checked_add(1)
                .ok_or_else(|| ChunkSplitterError::ChunkWriteFailed {
                    path: path.to_path_buf(),
                    source: std::io::Error::other("token counter overflow"),
                })?;
        }

        debug_assert_eq!(written_tokens, tokens.len(), "must write all tokens");
        trace_split!("✓ wrote {} tokens, {} bytes", written_tokens, written_bytes);

        Ok(())
    }

    fn write_chunk_map(&self, manifest: &ChunkManifest, path: &Path) -> Result<(), ChunkSplitterError> {
        trace_split!("writing chunk map to {}", path.display());

        let json = serde_json::to_string_pretty(manifest).map_err(|e| ChunkSplitterError::ChunkMapWriteFailed {
            path: path.to_path_buf(),
            source: std::io::Error::other(e.to_string()),
        })?;

        let json_len = json.len();

        fs::write(path, &json).map_err(|source| ChunkSplitterError::ChunkMapWriteFailed {
            path: path.to_path_buf(),
            source,
        })?;

        trace_split!("✓ wrote chunk map ({} bytes)", json_len);
        Ok(())
    }

    fn write_chunk_source_map(
        &self,
        tokens: &[Token],
        chunk_path: &Path,
        chunk: &ChunkMetadata,
        options: &Options,
    ) -> Result<(), ChunkSplitterError> {
        assert!(!tokens.is_empty(), "tokens must not be empty");
        trace_split!("writing source map for chunk {}", chunk.id);

        let line_count = tokens
            .iter()
            .filter(|t| t.text == "\n")
            .count()
            .checked_add(1)
            .ok_or_else(|| ChunkSplitterError::ChunkWriteFailed {
                path: chunk_path.to_path_buf(),
                source: std::io::Error::other("line count overflow"),
            })?;

        let source_map = SourceMap::for_chunk(&chunk.name, &chunk.filename, line_count);

        let map_filename = format!("{}.map", chunk.filename);
        let map_path = options.chunk_dir.join(&map_filename);

        let json = source_map.to_json().map_err(|e| ChunkSplitterError::ChunkWriteFailed {
            path: map_path.clone(),
            source: std::io::Error::other(e.to_string()),
        })?;

        fs::write(&map_path, &json).map_err(|source| ChunkSplitterError::ChunkWriteFailed {
            path: map_path.clone(),
            source,
        })?;

        trace_split!("✓ wrote source map: {} ({} bytes)", map_filename, json.len());
        Ok(())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChunkManifest {
    pub chunk_dir: PathBuf,
    pub chunks: HashMap<usize, ChunkMetadata>,
    pub total_chunks: usize,
}

impl ChunkManifest {
    fn new(chunk_dir: PathBuf) -> Self {
        Self {
            chunk_dir,
            chunks: HashMap::new(),
            total_chunks: 0,
        }
    }

    #[must_use]
    pub fn from_detector(detector: &ChunkDetector) -> Self {
        let total_chunks = detector.chunk_count();
        debug_assert_eq!(detector.chunks.len(), total_chunks, "chunk count mismatch");

        Self {
            chunk_dir: PathBuf::from("."),
            chunks: detector.chunks.clone(),
            total_chunks,
        }
    }

    fn add_chunk(&mut self, id: usize, metadata: ChunkMetadata) {
        assert!(!self.chunks.contains_key(&id), "duplicate chunk ID: {id}");

        self.chunks.insert(id, metadata);
        self.total_chunks = self.total_chunks.checked_add(1).expect("chunk count overflow");

        debug_assert_eq!(self.chunks.len(), self.total_chunks, "count must match map size");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_manifest_creation() {
        let manifest = ChunkManifest::new(PathBuf::from("./chunks"));

        assert_eq!(manifest.chunk_dir, PathBuf::from("./chunks"));
        assert_eq!(manifest.total_chunks, 0);
        assert!(manifest.chunks.is_empty());
    }

    #[test]
    fn test_chunk_manifest_add_chunk() {
        let mut manifest = ChunkManifest::new(PathBuf::from("./chunks"));
        let chunk = ChunkMetadata::new(937, "LoginView".to_string(), "363842".to_string());

        manifest.add_chunk(937, chunk);

        assert_eq!(manifest.total_chunks, 1);
        assert!(manifest.chunks.contains_key(&937));
    }
}
