use crate::token::Token;
use std::collections::HashMap;
use thiserror::Error;

#[cfg(debug_assertions)]
macro_rules! trace_chunk {
    ($($arg:tt)*) => {
        eprintln!("[CHUNK] {}", format!($($arg)*));
    };
}

#[cfg(not(debug_assertions))]
macro_rules! trace_chunk {
    ($($arg:tt)*) => {
        let _ = format_args!($($arg)*);
    };
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChunkMetadata {
    pub id: usize,
    pub name: String,
    pub hash: String,
    pub filename: String,
    pub modules: Vec<usize>,
    pub start_pos: usize,
    pub end_pos: usize,
}

impl ChunkMetadata {
    /// # Panics
    /// Panics if `name` or `hash` is empty.
    #[must_use]
    pub fn new(id: usize, name: String, hash: String) -> Self {
        assert!(!name.is_empty(), "chunk name must not be empty");
        assert!(!hash.is_empty(), "chunk hash must not be empty");

        let filename = format!("{name}.chunk.{hash}.js");
        trace_chunk!("created ChunkMetadata: id={}, name={}, hash={}", id, name, hash);

        Self {
            id,
            name,
            hash,
            filename,
            modules: Vec::new(),
            start_pos: 0,
            end_pos: 0,
        }
    }

    /// # Panics
    /// Panics if `module_id` is already in this chunk.
    pub fn add_module(&mut self, module_id: usize) {
        assert!(
            !self.modules.contains(&module_id),
            "duplicate module {} in chunk {}",
            module_id,
            self.id
        );
        self.modules.push(module_id);
        trace_chunk!("added module {} to chunk {}", module_id, self.id);
    }

    /// # Errors
    /// Returns [`ChunkDetectorError::BoundaryDetectionFailed`] if `start >= end`.
    pub fn set_bounds(&mut self, start: usize, end: usize) -> Result<(), ChunkDetectorError> {
        if start >= end {
            return Err(ChunkDetectorError::BoundaryDetectionFailed {
                reason: format!("invalid bounds: start={start} >= end={end}"),
            });
        }
        self.start_pos = start;
        self.end_pos = end;
        trace_chunk!("set bounds for chunk {}: {}..{}", self.id, start, end);
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ChunkDetectorError {
    #[error("invalid chunk ID map format at position {pos}")]
    InvalidIdMapFormat { pos: usize },

    #[error("chunk ID {chunk_id} appears in multiple locations")]
    DuplicateChunkId { chunk_id: usize },

    #[error("chunk boundary detection failed: {reason}")]
    BoundaryDetectionFailed { reason: String },

    #[error("no chunk map found in token stream")]
    NoChunkMapFound,
}

pub struct ChunkDetector {
    pub chunks: HashMap<usize, ChunkMetadata>,
    detected_boundaries: bool,
}

impl ChunkDetector {
    #[must_use]
    pub fn new() -> Self {
        trace_chunk!("initializing ChunkDetector");
        Self {
            chunks: HashMap::new(),
            detected_boundaries: false,
        }
    }

    /// # Errors
    /// Returns an error if the operation fails.
    ///
    /// # Panics
    /// Panics if `tokens` is empty.
    pub fn detect_chunks(&mut self, tokens: &[Token]) -> Result<(), ChunkDetectorError> {
        assert!(!tokens.is_empty(), "token stream must not be empty");
        trace_chunk!("=== STARTING CHUNK DETECTION ===");
        trace_chunk!("total tokens: {}", tokens.len());

        trace_chunk!("step 1: detecting chunk ID map");
        self.detect_chunk_id_map(tokens)?;
        trace_chunk!("✓ detected {} chunks from ID map", self.chunks.len());

        if self.chunks.is_empty() {
            trace_chunk!("✗ no chunks found in ID map");
            return Err(ChunkDetectorError::NoChunkMapFound);
        }

        trace_chunk!("step 2: detecting chunk boundaries (webpack push patterns)");
        self.detect_chunk_boundaries(tokens)?;

        let chunks_with_bounds = self.chunks.values().filter(|c| c.start_pos < c.end_pos).count();

        debug_assert!(
            chunks_with_bounds <= self.chunks.len(),
            "chunks_with_bounds must not exceed total chunks"
        );

        trace_chunk!(
            "chunks with valid boundaries: {}/{}",
            chunks_with_bounds,
            self.chunks.len()
        );

        if chunks_with_bounds == 0 {
            trace_chunk!("⚠ no chunk boundaries found - this is likely a code-split bundle");
            trace_chunk!("  (chunks are already in separate files, not embedded in this bundle)");
            self.detected_boundaries = false;
        } else {
            trace_chunk!("✓ detected boundaries for {} chunks", chunks_with_bounds);
            self.detected_boundaries = true;
        }

        trace_chunk!("=== CHUNK DETECTION COMPLETE ===");
        debug_assert!(!self.chunks.is_empty(), "must have at least one chunk");

        Ok(())
    }

    fn detect_chunk_id_map(&mut self, tokens: &[Token]) -> Result<(), ChunkDetectorError> {
        const MAX_SEARCH_TOKENS: usize = 100_000;
        let search_limit = tokens.len().min(MAX_SEARCH_TOKENS);

        trace_chunk!("=== DETECTING CHUNK ID MAP ===");
        trace_chunk!("total tokens: {}", tokens.len());
        trace_chunk!("search limit: {}", search_limit);

        debug_assert!(!tokens.is_empty(), "tokens must not be empty");
        debug_assert!(search_limit <= tokens.len(), "search_limit must be <= tokens.len()");

        let mut checked_positions: usize = 0;
        for i in 0..search_limit {
            assert!(i < tokens.len(), "index {} out of bounds (len={})", i, tokens.len());

            checked_positions =
                checked_positions
                    .checked_add(1)
                    .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                        reason: "position counter overflow".to_string(),
                    })?;

            if checked_positions.is_multiple_of(10000) {
                trace_chunk!("checked {} positions so far...", checked_positions);
            }

            if self.is_chunk_map_start(&tokens[i..]) {
                trace_chunk!("✓ found potential chunk map at position {}", i);
                self.parse_chunk_maps(&tokens[i..])?;
                trace_chunk!("✓ successfully parsed chunk maps");
                return Ok(());
            }
        }

        trace_chunk!("✗ no chunk map found after checking {} positions", checked_positions);
        Err(ChunkDetectorError::NoChunkMapFound)
    }

    fn is_chunk_map_start(&self, tokens: &[Token]) -> bool {
        const MIN_TOKENS_REQUIRED: usize = 30;
        const MAX_SKIP: usize = 10;

        fn next_significant(tokens: &[Token], start: usize, max_skip: usize) -> Option<usize> {
            debug_assert!(start < tokens.len(), "start position out of bounds");

            for offset in 0..max_skip {
                let Some(pos) = start.checked_add(offset) else {
                    break;
                };
                if pos >= tokens.len() {
                    break;
                }

                let text = tokens[pos].text.trim();
                if !text.is_empty() {
                    return Some(pos);
                }
            }
            None
        }

        trace_chunk!("checking if chunk map starts at current position");
        trace_chunk!("token count: {}", tokens.len());

        if tokens.len() < MIN_TOKENS_REQUIRED {
            trace_chunk!("not enough tokens: {} < {}", tokens.len(), MIN_TOKENS_REQUIRED);
            return false;
        }

        // 1. Check for "return" at position 0 (skipping initial whitespace)
        let return_pos = match next_significant(tokens, 0, MAX_SKIP) {
            Some(pos) if tokens[pos].text == "return" => {
                trace_chunk!("✓ found 'return' at position {}", pos);
                pos
            }
            Some(pos) => {
                trace_chunk!("token[{}] is not 'return': '{}'", pos, tokens[pos].text);
                return false;
            }
            None => {
                trace_chunk!("no significant token found at start");
                return false;
            }
        };

        // 2. Find "(" (skipping whitespace after "return")
        let Some(next_start) = return_pos.checked_add(1) else {
            return false;
        };
        let paren_pos = match next_significant(tokens, next_start, MAX_SKIP) {
            Some(pos) if tokens[pos].text == "(" => {
                trace_chunk!("✓ found '(' at position {}", pos);
                pos
            }
            Some(pos) => {
                trace_chunk!("expected '(' at position {}, found '{}'", pos, tokens[pos].text);
                return false;
            }
            None => {
                trace_chunk!("no '(' found within {} tokens", MAX_SKIP);
                return false;
            }
        };

        // 3. Find "{" (skipping whitespace after "(")
        let Some(next_start) = paren_pos.checked_add(1) else {
            return false;
        };
        let brace_pos = match next_significant(tokens, next_start, MAX_SKIP) {
            Some(pos) if tokens[pos].text == "{" => {
                trace_chunk!("✓ found '{{' at position {}", pos);
                pos
            }
            Some(pos) => {
                trace_chunk!("expected '{{' at position {}, found '{}'", pos, tokens[pos].text);
                return false;
            }
            None => {
                trace_chunk!("no '{{' found within {} tokens", MAX_SKIP);
                return false;
            }
        };

        // 4. Find ".chunk." string literal - scan 200 tokens to cover large chunk maps
        // The token is a string literal like '".chunk."' so we check if it contains .chunk.
        // Webpack chunk maps can have 30+ entries, each with 4 tokens, so we need ~150+ range
        let has_chunk = tokens
            .iter()
            .skip(brace_pos)
            .take(200)
            .any(|t| t.text.contains(".chunk."));

        if has_chunk {
            trace_chunk!("✓ found '.chunk.' string literal after position {}", brace_pos);
        } else {
            trace_chunk!("'.chunk.' not found in next 200 tokens after position {}", brace_pos);
        }

        let result = has_chunk;
        trace_chunk!("is_chunk_map_start result: {}", result);

        result
    }

    fn parse_chunk_maps(&mut self, tokens: &[Token]) -> Result<(), ChunkDetectorError> {
        trace_chunk!("=== PARSING CHUNK MAPS ===");
        trace_chunk!("total tokens: {}", tokens.len());

        trace_chunk!("step 1: extracting name map from position 2");
        let name_map = self.extract_object_literal(tokens, 2)?;
        trace_chunk!("✓ extracted {} name mappings", name_map.len());

        debug_assert!(!name_map.is_empty(), "name map must not be empty");

        trace_chunk!("step 2: finding hash map start position");
        let hash_map_start = self.find_hash_map_start(tokens)?;
        trace_chunk!("✓ hash map starts at position {}", hash_map_start);

        trace_chunk!("step 3: extracting hash map");
        let hash_map = self.extract_object_literal(tokens, hash_map_start)?;
        trace_chunk!("✓ extracted {} hash mappings", hash_map.len());

        debug_assert!(!hash_map.is_empty(), "hash map must not be empty");

        trace_chunk!("step 4: validating map sizes");
        trace_chunk!("name_map.len()={}, hash_map.len()={}", name_map.len(), hash_map.len());

        // Hash map is authoritative - all chunks have hashes
        // Name map is optional - only named chunks have entries
        debug_assert!(
            name_map.len() <= hash_map.len(),
            "name map cannot have more entries than hash map"
        );

        if name_map.len() < hash_map.len() {
            let diff = hash_map.len().checked_sub(name_map.len()).ok_or_else(|| {
                ChunkDetectorError::BoundaryDetectionFailed {
                    reason: "size difference calculation overflow".to_string(),
                }
            })?;
            trace_chunk!("⚠ {} unnamed chunks detected (hash entries without names)", diff);
        } else {
            trace_chunk!("✓ all chunks have names");
        }

        trace_chunk!("step 5: creating chunk metadata");
        let mut created_count: usize = 0;

        // Iterate through hash map (authoritative source)
        for (chunk_id, hash) in &hash_map {
            debug_assert!(!hash.is_empty(), "hash must not be empty");
            trace_chunk!("processing chunk_id={}, hash='{}'", chunk_id, hash);

            // Check if chunk has a friendly name
            let name = name_map.get(chunk_id).map_or_else(
                || {
                    let generated_name = format!("chunk_{chunk_id}");
                    trace_chunk!("⚠ no friendly name, using: '{}'", generated_name);
                    debug_assert!(!generated_name.is_empty(), "generated name must not be empty");
                    generated_name
                },
                |friendly_name| {
                    trace_chunk!("✓ found friendly name: '{}'", friendly_name);
                    debug_assert!(!friendly_name.is_empty(), "name must not be empty");
                    friendly_name.clone()
                },
            );

            trace_chunk!("creating metadata: id={}, name='{}', hash='{}'", chunk_id, name, hash);

            let metadata = ChunkMetadata::new(*chunk_id, name, hash.clone());

            debug_assert_eq!(metadata.id, *chunk_id, "metadata ID must match");
            debug_assert_eq!(&metadata.hash, hash, "metadata hash must match");
            debug_assert!(!metadata.filename.is_empty(), "filename must not be empty");

            if self.chunks.insert(*chunk_id, metadata).is_some() {
                trace_chunk!("✗ duplicate chunk_id: {}", chunk_id);
                return Err(ChunkDetectorError::DuplicateChunkId { chunk_id: *chunk_id });
            }

            created_count =
                created_count
                    .checked_add(1)
                    .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                        reason: "created count overflow".to_string(),
                    })?;

            debug_assert!(
                created_count <= hash_map.len(),
                "created count must not exceed hash map size"
            );

            trace_chunk!("✓ created chunk metadata {}/{}", created_count, hash_map.len());
        }

        debug_assert_eq!(created_count, hash_map.len(), "must create metadata for all chunks");
        debug_assert_eq!(
            self.chunks.len(),
            hash_map.len(),
            "chunks map size must match hash map size"
        );

        trace_chunk!("=== PARSING COMPLETE ===");
        trace_chunk!("successfully parsed {} chunks", self.chunks.len());

        debug_assert_eq!(
            self.chunks.len(),
            created_count,
            "chunks.len() must equal created_count"
        );

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn extract_object_literal(
        &self,
        tokens: &[Token],
        start: usize,
    ) -> Result<HashMap<usize, String>, ChunkDetectorError> {
        const MAX_ITERATIONS: usize = 10_000;

        assert!(
            start < tokens.len(),
            "start position {} out of bounds (len={})",
            start,
            tokens.len()
        );

        trace_chunk!("=== EXTRACTING OBJECT LITERAL ===");
        trace_chunk!("start position: {}", start);
        trace_chunk!("token at start: '{}'", tokens[start].text);

        let mut result = HashMap::new();
        let mut depth: usize = 0;
        let mut i = start;
        let mut iterations: usize = 0;
        let mut extracted_count: usize = 0;

        while i < tokens.len() {
            assert!(iterations < MAX_ITERATIONS, "max iterations {MAX_ITERATIONS} exceeded");
            iterations = iterations
                .checked_add(1)
                .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                    reason: "iteration counter overflow".to_string(),
                })?;

            debug_assert!(i < tokens.len(), "i must be < tokens.len() in loop");

            let token = &tokens[i];

            if iterations <= 10 || iterations.is_multiple_of(100) {
                trace_chunk!(
                    "iter {}: pos={}, token='{}', depth={}",
                    iterations,
                    i,
                    token.text,
                    depth
                );
            }

            if token.text == "{" {
                trace_chunk!("found opening brace at pos {}, depth {} -> {}", i, depth, depth + 1);
                depth = depth
                    .checked_add(1)
                    .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                        reason: "depth counter overflow".to_string(),
                    })?;
            } else if token.text == "}" {
                trace_chunk!("found closing brace at pos {}, depth={}", i, depth);
                if depth == 0 {
                    trace_chunk!("depth already 0, breaking");
                    break;
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                        reason: "depth counter underflow".to_string(),
                    })?;
                trace_chunk!("depth after decrement: {}", depth);
                if depth == 0 {
                    trace_chunk!("depth reached 0, breaking");
                    break;
                }
            }

            if depth == 1 {
                let has_next_3 = i
                    .checked_add(3)
                    .ok_or(ChunkDetectorError::InvalidIdMapFormat { pos: i })?
                    < tokens.len();

                if has_next_3 {
                    let colon_idx = i
                        .checked_add(1)
                        .ok_or(ChunkDetectorError::InvalidIdMapFormat { pos: i })?;
                    let value_idx = i
                        .checked_add(2)
                        .ok_or(ChunkDetectorError::InvalidIdMapFormat { pos: i })?;

                    debug_assert!(colon_idx < tokens.len(), "colon_idx must be in bounds");
                    debug_assert!(value_idx < tokens.len(), "value_idx must be in bounds");

                    if tokens[colon_idx].text == ":" {
                        trace_chunk!(
                            "found colon at pos {}, checking if '{}' is numeric",
                            colon_idx,
                            token.text
                        );
                        if let Ok(key) = token.text.parse::<usize>() {
                            let value = tokens[value_idx].text.trim_matches('"').to_string();
                            trace_chunk!("✓ extracted mapping: {} -> '{}'", key, &value);

                            extracted_count = extracted_count.checked_add(1).ok_or_else(|| {
                                ChunkDetectorError::BoundaryDetectionFailed {
                                    reason: "extracted count overflow".to_string(),
                                }
                            })?;

                            result.insert(key, value);
                        } else {
                            trace_chunk!("token '{}' is not a valid number", token.text);
                        }
                    }
                }
            }

            i = i
                .checked_add(1)
                .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                    reason: "position counter overflow".to_string(),
                })?;
        }

        debug_assert!(!result.is_empty(), "extracted empty object literal");
        trace_chunk!("=== EXTRACTION COMPLETE ===");
        trace_chunk!("total iterations: {}", iterations);
        trace_chunk!("extracted entries: {}", extracted_count);
        trace_chunk!("final result size: {}", result.len());
        trace_chunk!("final depth: {}", depth);

        assert_eq!(extracted_count, result.len(), "extracted count must match result size");

        Ok(result)
    }

    fn find_hash_map_start(&self, tokens: &[Token]) -> Result<usize, ChunkDetectorError> {
        const MAX_SEARCH: usize = 1000;
        let search_limit = tokens.len().min(MAX_SEARCH);

        trace_chunk!("=== FINDING HASH MAP START ===");
        trace_chunk!("search limit: {}", search_limit);

        for i in 0..search_limit {
            assert!(i < tokens.len(), "index {} out of bounds (len={})", i, tokens.len());

            if tokens[i].text.contains(".chunk.") {
                trace_chunk!("found '.chunk.' string at position {}", i);

                if let Some(next_brace) = tokens[i..].iter().position(|t| t.text == "{") {
                    let result = i
                        .checked_add(next_brace)
                        .ok_or(ChunkDetectorError::InvalidIdMapFormat { pos: i })?;

                    debug_assert!(result < tokens.len(), "result position must be in bounds");
                    trace_chunk!(
                        "✓ found opening brace {} positions after, hash map starts at {}",
                        next_brace,
                        result
                    );

                    return Ok(result);
                }
                trace_chunk!("no opening brace found after '.chunk.' at pos {}", i);
            }
        }

        trace_chunk!("✗ hash map not found in first {} tokens", search_limit);

        Err(ChunkDetectorError::BoundaryDetectionFailed {
            reason: "hash map not found after .chunk.".to_string(),
        })
    }

    #[must_use]
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    #[must_use]
    pub fn get_chunk(&self, id: usize) -> Option<&ChunkMetadata> {
        self.chunks.get(&id)
    }

    #[must_use]
    pub const fn has_boundaries(&self) -> bool {
        self.detected_boundaries
    }

    fn detect_chunk_boundaries(&mut self, tokens: &[Token]) -> Result<(), ChunkDetectorError> {
        const MAX_SEARCH_TOKENS: usize = 100_000;

        trace_chunk!("=== DETECTING CHUNK BOUNDARIES ===");
        trace_chunk!("searching for webpack push patterns");

        let search_limit = tokens.len().min(MAX_SEARCH_TOKENS);
        let mut found_boundaries: usize = 0;

        for i in 0..search_limit {
            assert!(i < tokens.len(), "index {} out of bounds (len={})", i, tokens.len());

            if self.is_webpack_push_pattern(&tokens[i..]) {
                trace_chunk!("found webpack push pattern at position {}", i);

                if let Some((chunk_id, start, end)) = self.extract_chunk_boundary(&tokens[i..], i)? {
                    trace_chunk!(
                        "✓ extracted chunk boundary: id={}, start={}, end={}",
                        chunk_id,
                        start,
                        end
                    );

                    if let Some(chunk) = self.chunks.get_mut(&chunk_id) {
                        chunk.set_bounds(start, end)?;
                        found_boundaries = found_boundaries.checked_add(1).ok_or_else(|| {
                            ChunkDetectorError::BoundaryDetectionFailed {
                                reason: "found boundaries counter overflow".to_string(),
                            }
                        })?;
                        trace_chunk!("set bounds for chunk {}: {}..{}", chunk_id, start, end);
                    } else {
                        trace_chunk!("chunk_id {} not found in chunks map, skipping", chunk_id);
                    }
                }
            }

            if found_boundaries > 0 && found_boundaries.is_multiple_of(10) {
                trace_chunk!("progress: found {} chunk boundaries so far", found_boundaries);
            }
        }

        trace_chunk!("=== BOUNDARY DETECTION COMPLETE ===");
        trace_chunk!("found {} chunk boundaries", found_boundaries);

        Ok(())
    }

    fn is_webpack_push_pattern(&self, tokens: &[Token]) -> bool {
        const MIN_TOKENS: usize = 10;
        if tokens.len() < MIN_TOKENS {
            return false;
        }

        let has_self_or_window = tokens[0].text == "self" || tokens[0].text == "window";
        let has_webpack_chunk = tokens.iter().take(6).any(|t| t.text.contains("webpackChunk"));
        let has_push = tokens.iter().take(10).any(|t| t.text == "push");

        has_self_or_window && has_webpack_chunk && has_push
    }

    fn extract_chunk_boundary(
        &self,
        tokens: &[Token],
        base_pos: usize,
    ) -> Result<Option<(usize, usize, usize)>, ChunkDetectorError> {
        const MAX_SEARCH: usize = 100;

        trace_chunk!("extracting chunk boundary from position {}", base_pos);
        let mut push_pos = None;

        for (i, token) in tokens.iter().enumerate().take(MAX_SEARCH) {
            if token.text == "push" {
                push_pos = Some(i);
                trace_chunk!("found 'push' at offset {}", i);
                break;
            }
        }

        let Some(push_pos) = push_pos else {
            trace_chunk!("no 'push' found in first {} tokens", MAX_SEARCH);
            return Ok(None);
        };

        let array_start = push_pos
            .checked_add(2)
            .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                reason: "position overflow while finding array start".to_string(),
            })?;

        if array_start >= tokens.len() || tokens[array_start].text != "[" {
            trace_chunk!("expected '[' at position {}, skipping", array_start);
            return Ok(None);
        }

        let chunk_id_pos = array_start
            .checked_add(1)
            .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                reason: "position overflow while finding chunk ID".to_string(),
            })?;

        if chunk_id_pos >= tokens.len() || tokens[chunk_id_pos].text != "[" {
            trace_chunk!("expected nested '[' at position {}, skipping", chunk_id_pos);
            return Ok(None);
        }

        let id_pos = chunk_id_pos
            .checked_add(1)
            .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                reason: "position overflow while finding ID value".to_string(),
            })?;

        if id_pos >= tokens.len() {
            return Ok(None);
        }

        let chunk_id = if let Ok(id) = tokens[id_pos].text.parse::<usize>() {
            trace_chunk!("parsed chunk_id: {}", id);
            id
        } else {
            trace_chunk!("failed to parse chunk_id from '{}'", tokens[id_pos].text);
            return Ok(None);
        };

        let modules_obj_pos = self.find_modules_object(tokens, id_pos)?;
        let obj_end = self.find_object_end(tokens, modules_obj_pos)?;

        let start = base_pos;
        let end = base_pos
            .checked_add(obj_end)
            .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                reason: "position overflow calculating chunk end".to_string(),
            })?;

        debug_assert!(start < end, "chunk start must be before end");
        trace_chunk!("chunk boundary: start={}, end={} (size={})", start, end, end - start);

        Ok(Some((chunk_id, start, end)))
    }

    fn find_modules_object(&self, tokens: &[Token], start: usize) -> Result<usize, ChunkDetectorError> {
        const MAX_SEARCH: usize = 20;
        let search_limit = tokens.len().min(start.checked_add(MAX_SEARCH).ok_or_else(|| {
            ChunkDetectorError::BoundaryDetectionFailed {
                reason: "overflow calculating search limit".to_string(),
            }
        })?);

        for (i, token) in tokens.iter().enumerate().take(search_limit).skip(start) {
            if token.text == "{" {
                trace_chunk!("found modules object at position {}", i);
                return Ok(i);
            }
        }

        Err(ChunkDetectorError::BoundaryDetectionFailed {
            reason: format!("modules object not found after position {start}"),
        })
    }

    fn find_object_end(&self, tokens: &[Token], start: usize) -> Result<usize, ChunkDetectorError> {
        const MAX_ITERATIONS: usize = 100_000;

        let mut depth: usize = 0;
        let mut i = start;
        let mut iterations: usize = 0;

        while i < tokens.len() {
            assert!(iterations < MAX_ITERATIONS, "max iterations exceeded");
            iterations = iterations
                .checked_add(1)
                .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                    reason: "iteration counter overflow".to_string(),
                })?;

            if tokens[i].text == "{" {
                depth = depth
                    .checked_add(1)
                    .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                        reason: "depth overflow".to_string(),
                    })?;
            } else if tokens[i].text == "}" {
                if depth == 0 {
                    break;
                }
                depth = depth
                    .checked_sub(1)
                    .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                        reason: "depth underflow".to_string(),
                    })?;
                if depth == 0 {
                    trace_chunk!("found object end at position {} (relative to start)", i - start);
                    return Ok(i);
                }
            }

            i = i
                .checked_add(1)
                .ok_or_else(|| ChunkDetectorError::BoundaryDetectionFailed {
                    reason: "position overflow".to_string(),
                })?;
        }

        Err(ChunkDetectorError::BoundaryDetectionFailed {
            reason: format!("object end not found starting from position {start}"),
        })
    }
}

impl Default for ChunkDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_token(text: &str) -> Token {
        use crate::token::TokenType;
        Token::new(TokenType::Unknown, text.to_string())
    }

    #[test]
    fn test_chunk_metadata_creation() {
        let chunk = ChunkMetadata::new(937, "LoginView".to_string(), "363842".to_string());

        assert_eq!(chunk.id, 937);
        assert_eq!(chunk.name, "LoginView");
        assert_eq!(chunk.hash, "363842");
        assert_eq!(chunk.filename, "LoginView.chunk.363842.js");
        assert!(chunk.modules.is_empty());
    }

    #[test]
    fn test_add_module() {
        let mut chunk = ChunkMetadata::new(937, "LoginView".to_string(), "363842".to_string());

        chunk.add_module(65277);
        chunk.add_module(12345);

        assert_eq!(chunk.modules.len(), 2);
        assert!(chunk.modules.contains(&65277));
        assert!(chunk.modules.contains(&12345));
    }

    #[test]
    fn set_bounds_accepts_valid_range() {
        let mut chunk = ChunkMetadata::new(1, "A".to_string(), "h".to_string());
        chunk.set_bounds(10, 20).expect("valid bounds should be accepted");
        assert_eq!(chunk.start_pos, 10);
        assert_eq!(chunk.end_pos, 20);
    }

    #[test]
    fn set_bounds_rejects_inverted_and_empty_range() {
        let mut chunk = ChunkMetadata::new(1, "A".to_string(), "h".to_string());

        // start == end (empty) is rejected.
        let eq = chunk.set_bounds(5, 5);
        assert!(matches!(eq, Err(ChunkDetectorError::BoundaryDetectionFailed { .. })));

        // start > end (inverted) is rejected.
        let inv = chunk.set_bounds(20, 10);
        assert!(matches!(inv, Err(ChunkDetectorError::BoundaryDetectionFailed { .. })));

        // A rejected call must not mutate the chunk.
        assert_eq!(chunk.start_pos, 0);
        assert_eq!(chunk.end_pos, 0);
    }

    #[test]
    fn test_detector_initialization() {
        let detector = ChunkDetector::new();

        assert_eq!(detector.chunk_count(), 0);
        assert!(!detector.detected_boundaries);
    }

    #[test]
    fn test_is_chunk_map_start() {
        let detector = ChunkDetector::new();

        let tokens = vec![
            make_token("return"),
            make_token("("),
            make_token("{"),
            make_token("14"),
            make_token(":"),
            make_token("\"LoginView\""),
            make_token(","),
            make_token("68"),
            make_token(":"),
            make_token("\"SearchView\""),
            make_token("}"),
            make_token("["),
            make_token("A"),
            make_token("]"),
            make_token("||"),
            make_token("A"),
            make_token(")"),
            make_token("+"),
            make_token("\".chunk.\""),
            make_token("+"),
            make_token("{"),
            make_token("14"),
            make_token(":"),
            make_token("\"abc123\""),
            make_token(","),
            make_token("68"),
            make_token(":"),
            make_token("\"def456\""),
            make_token("}"),
            make_token("["),
            make_token("A"),
            make_token("]"),
        ];

        debug_assert!(tokens.len() >= 30, "test needs at least 30 tokens");
        assert!(detector.is_chunk_map_start(&tokens));
    }

    #[test]
    fn test_extract_simple_object_literal() {
        let detector = ChunkDetector::new();

        let tokens = vec![
            make_token("{"),
            make_token("14"),
            make_token(":"),
            make_token("\"LoginView\""),
            make_token(","),
            make_token("68"),
            make_token(":"),
            make_token("\"SearchResultsView\""),
            make_token("}"),
        ];

        let result = detector.extract_object_literal(&tokens, 0).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result.get(&14), Some(&"LoginView".to_string()));
        assert_eq!(result.get(&68), Some(&"SearchResultsView".to_string()));
    }
}
